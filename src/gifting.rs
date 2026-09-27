//! Gift state machine. Answers stay on the server; each role receives a separate projection.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Integer, Nullable, Text};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{error, pool, user_id};

#[derive(Deserialize)]
struct PuzzleInput {
    unlock_kind: String,
    clue: Option<String>,
    answer: Option<String>,
    message: Option<String>,
    contract_text: Option<String>,
}

#[derive(Deserialize)]
struct AnswerInput {
    answer: String,
}

#[derive(Deserialize)]
struct AcceptInput {
    agree: Option<bool>,
    recipient_name: Option<String>,
    recipient_phone: Option<String>,
    recipient_address: Option<String>,
}

#[derive(QueryableByName)]
struct GiftView {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    sender_id: i64,
    #[diesel(sql_type = Text)]
    sender_name: String,
    #[diesel(sql_type = Integer)]
    product_id: i32,
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
    #[diesel(sql_type = Bool)]
    physical: bool,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Text)]
    unlock_kind: String,
    #[diesel(sql_type = Text)]
    clue: String,
    #[diesel(sql_type = Text)]
    message: String,
    #[diesel(sql_type = Text)]
    contract_text: String,
    #[diesel(sql_type = Integer)]
    attempts: i32,
    #[diesel(sql_type = Bool)]
    identity_known: bool,
    #[diesel(sql_type = Nullable<Text>)]
    voucher_code: Option<String>,
    #[diesel(sql_type = Bool)]
    expired: bool,
}

#[derive(QueryableByName)]
struct SenderView {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    recipient_id: i64,
    #[diesel(sql_type = Text)]
    recipient_name: String,
    #[diesel(sql_type = Text)]
    notification_status: String,
    #[diesel(sql_type = Integer)]
    product_id: i32,
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Bool)]
    expired: bool,
    #[diesel(sql_type = Bool)]
    carrier_delivered: bool,
    #[diesel(sql_type = Bool)]
    recipient_confirmed: bool,
}

const GIFT_VIEW_SELECT: &str = "SELECT g.id,g.sender_id, \
    su.display_name AS sender_name, \
    g.product_id,g.price_cents,c.physical,g.state,g.unlock_kind,g.clue, \
    g.message,g.contract_text,g.attempts,g.identity_known,g.voucher_code, \
    (g.expires_at<=now() AND g.state IN ('sealed','opened')) AS expired \
    FROM gifts g JOIN users su ON su.id=g.sender_id \
    JOIN catalog c ON c.id=g.product_id";

const SENDER_VIEW_SELECT: &str = "SELECT g.id,COALESCE(g.recipient_id,0) AS recipient_id,COALESCE(NULLIF(g.recipient_contact->>'label',''),g.recipient_contact->>'value',ru.display_name,'待领取') AS recipient_name, \
    g.product_id,g.price_cents,g.state,COALESCE((SELECT status FROM delivery_outbox o WHERE o.gift_id=g.id),'in_app') AS notification_status, \
    (g.expires_at<=now() AND g.state IN ('sealed','opened')) AS expired, \
    (s.delivered_at IS NOT NULL) AS carrier_delivered, \
    (s.recipient_confirmed_at IS NOT NULL) AS recipient_confirmed \
    FROM gifts g LEFT JOIN users ru ON ru.id=g.recipient_id \
    LEFT JOIN shipments s ON s.gift_id=g.id";

#[derive(QueryableByName)]
struct GiftLock {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Text)]
    unlock_kind: String,
    #[diesel(sql_type = Nullable<Text>)]
    answer_hash: Option<String>,
    #[diesel(sql_type = Integer)]
    attempts: i32,
    #[diesel(sql_type = Text)]
    contract_text: String,
    #[diesel(sql_type = Bool)]
    physical: bool,
    #[diesel(sql_type = Bool)]
    expired: bool,
}

fn requester(req: &Request, res: &mut Response) -> Option<i64> {
    match user_id(req) {
        Some(uid) => Some(uid),
        None => {
            error(res, StatusCode::UNAUTHORIZED, "invalid session");
            None
        }
    }
}

fn gift_id(req: &Request, res: &mut Response) -> Option<i64> {
    match req.param::<i64>("id").filter(|id| *id > 0) {
        Some(id) => Some(id),
        None => {
            error(res, StatusCode::BAD_REQUEST, "invalid gift id");
            None
        }
    }
}

fn normalized_answer(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn answer_digest(id: i64, raw: &str) -> String {
    let pepper = std::env::var("LIYU_ANSWER_PEPPER").unwrap_or_else(|_| "liyu-local-demo".into());
    let bytes = format!("{pepper}:{id}:{}", normalized_answer(raw));
    format!("{:x}", Sha256::digest(bytes.as_bytes()))
}

fn public_state(g: &GiftView) -> &str {
    if g.expired {
        "expired"
    } else {
        &g.state
    }
}

fn sender_state(g: &SenderView) -> &str {
    let state = if g.expired { "expired" } else { &g.state };
    match state {
        "accepted" | "exchanged" | "cashed_out" => "handled",
        other => other,
    }
}

fn sender_projection(g: SenderView) -> Value {
    json!({
        "id": g.id,
        "recipient": {"display_name":g.recipient_name},
        "delivery_status":if g.recipient_id==0 {"pending_claim"}else{"assigned"},
        "notification_status":g.notification_status,
        "product_id": g.product_id,
        "price_cents": g.price_cents,
        "state": sender_state(&g),
        "carrier_delivered": g.carrier_delivered,
        "recipient_confirmed": g.recipient_confirmed,
    })
}

fn recipient_projection(g: GiftView) -> Value {
    let state = public_state(&g);
    let revealed = matches!(state, "revealed" | "accepted" | "exchanged" | "cashed_out");
    let mut result = json!({
        "id": g.id,
        "state": state,
        "unlock_kind": g.unlock_kind,
        "clue": g.clue,
        "attempts_left": (3 - g.attempts).max(0),
    });
    if revealed {
        result["product_id"] = json!(g.product_id);
        result["price_cents"] = json!(g.price_cents);
        result["physical"] = json!(g.physical);
        result["message"] = json!(g.message);
        result["contract_text"] = json!(g.contract_text);
        if g.identity_known {
            result["sender"] = json!({"id":g.sender_id,"display_name":g.sender_name});
        }
        if let Some(code) = g.voucher_code {
            result["voucher_code"] = json!(code);
        }
    }
    result
}

fn lock_gift(
    conn: &mut PgConnection,
    id: i64,
    uid: i64,
    owner_col: &str,
) -> QueryResult<Option<GiftLock>> {
    let sql = format!(
        "SELECT g.id,g.state,g.unlock_kind,g.answer_hash,g.attempts,g.contract_text, \
         c.physical,(g.expires_at<=now()) AS expired \
         FROM gifts g JOIN catalog c ON c.id=g.product_id \
         WHERE g.id=$1 AND g.{owner_col}=$2 \
         AND (g.available_at<=now() OR g.sender_id=$2) FOR UPDATE OF g"
    );
    diesel::sql_query(sql)
        .bind::<BigInt, _>(id)
        .bind::<BigInt, _>(uid)
        .get_result::<GiftLock>(conn)
        .optional()
}

fn release_wish(conn: &mut PgConnection, id: i64) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE wishlist_items SET claimed_gift_id=NULL,claimed_by_user_id=NULL,claimed_at=NULL \
         WHERE claimed_gift_id=$1",
    )
    .bind::<BigInt, _>(id)
    .execute(conn)?;
    Ok(())
}

#[handler]
async fn inbox(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let sql = format!(
        "{GIFT_VIEW_SELECT} WHERE g.recipient_id=$1 AND g.available_at<=now() \
         AND g.state<>'withdrawn' ORDER BY g.available_at DESC,g.id DESC LIMIT 100"
    );
    let rows = diesel::sql_query(sql)
        .bind::<BigInt, _>(uid)
        .load::<GiftView>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(
            rows.into_iter()
                .map(recipient_projection)
                .collect::<Vec<_>>(),
        )),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "inbox query failed"),
    }
}

#[handler]
async fn outbox(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let sql = format!(
        "{SENDER_VIEW_SELECT} WHERE g.sender_id=$1 ORDER BY g.created_at DESC,g.id DESC LIMIT 100"
    );
    let rows = diesel::sql_query(sql)
        .bind::<BigInt, _>(uid)
        .load::<SenderView>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(
            rows.into_iter().map(sender_projection).collect::<Vec<_>>(),
        )),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "outbox query failed",
        ),
    }
}

#[handler]
async fn detail(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Some(id) = gift_id(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let sender_sql = format!("{SENDER_VIEW_SELECT} WHERE g.id=$1 AND g.sender_id=$2");
    let sender = diesel::sql_query(sender_sql)
        .bind::<BigInt, _>(id)
        .bind::<BigInt, _>(uid)
        .get_result::<SenderView>(&mut conn)
        .optional();
    match sender {
        Ok(Some(row)) => {
            res.render(Json(sender_projection(row)));
            return;
        }
        Ok(None) => {}
        Err(_) => return error(res, StatusCode::INTERNAL_SERVER_ERROR, "gift query failed"),
    }
    let sql = format!(
        "{GIFT_VIEW_SELECT} WHERE g.id=$1 AND g.recipient_id=$2 \
         AND g.available_at<=now() AND g.state<>'withdrawn'"
    );
    let row = diesel::sql_query(sql)
        .bind::<BigInt, _>(id)
        .bind::<BigInt, _>(uid)
        .get_result::<GiftView>(&mut conn)
        .optional();
    match row {
        Ok(Some(row)) => res.render(Json(recipient_projection(row))),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "gift not found"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "gift query failed"),
    }
}

#[handler]
async fn configure_puzzle(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Some(id) = gift_id(req, res) else { return };
    let body: PuzzleInput = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let clue = body.clue.as_deref().unwrap_or("").trim();
    let puzzle_answer = body.answer.as_deref().unwrap_or("").trim();
    let message = body.message.as_deref().unwrap_or("").trim();
    let contract = body.contract_text.as_deref().unwrap_or("").trim();
    let puzzle = matches!(
        body.unlock_kind.as_str(),
        "guess_who" | "question" | "passphrase"
    );
    if !(puzzle || body.unlock_kind == "free")
        || clue.chars().count() > 30
        || message.chars().count() > 40
        || contract.chars().count() > 24
        || (puzzle
            && (normalized_answer(puzzle_answer).is_empty() || puzzle_answer.chars().count() > 20))
        || (puzzle && body.unlock_kind != "guess_who" && clue.is_empty())
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid puzzle");
    }
    let digest = puzzle.then(|| answer_digest(id, puzzle_answer));
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let updated = diesel::sql_query(
        "UPDATE gifts SET unlock_kind=$3,clue=$4,answer_hash=$5,message=$6,contract_text=$7 \
         WHERE id=$1 AND sender_id=$2 AND state='sealed' AND opened_at IS NULL",
    )
    .bind::<BigInt, _>(id)
    .bind::<BigInt, _>(uid)
    .bind::<Text, _>(&body.unlock_kind)
    .bind::<Text, _>(if puzzle { clue } else { "" })
    .bind::<Nullable<Text>, _>(digest)
    .bind::<Text, _>(message)
    .bind::<Text, _>(contract)
    .execute(&mut conn);
    match updated {
        Ok(1) => res.render(Json(json!({"id":id,"configured":true}))),
        Ok(_) => error(res, StatusCode::CONFLICT, "gift not configurable"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "puzzle update failed",
        ),
    }
}

#[handler]
async fn open(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Some(id) = gift_id(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<&'static str>, diesel::result::Error, _>(|conn| {
        let row = lock_gift(conn, id, uid, "recipient_id")?;
        let Some(row) = row else { return Ok(None) };
        if row.state != "sealed" { return Ok(Some("already_open")) }
        if row.expired {
            crate::benefits::refund(conn, row.id, "expired")?;
            diesel::sql_query("UPDATE gifts SET state='expired',settled_at=now() WHERE id=$1")
                .bind::<BigInt, _>(row.id).execute(conn)?;
            release_wish(conn, row.id)?;
            return Ok(Some("expired"));
        }
        if row.unlock_kind == "free" {
            diesel::sql_query("UPDATE gifts SET state='revealed',opened_at=now(),revealed_at=now(),identity_known=true WHERE id=$1")
                .bind::<BigInt, _>(row.id).execute(conn)?;
            Ok(Some("revealed"))
        } else {
            diesel::sql_query("UPDATE gifts SET state='opened',opened_at=now() WHERE id=$1")
                .bind::<BigInt, _>(row.id).execute(conn)?;
            Ok(Some("opened"))
        }
    });
    match result {
        Ok(Some("opened")) => res.render(Json(json!({"id":id,"state":"opened","attempts_left":3}))),
        Ok(Some("revealed")) => res.render(Json(
            json!({"id":id,"state":"revealed","identity_known":true}),
        )),
        Ok(Some("expired")) => error(res, StatusCode::CONFLICT, "gift expired"),
        Ok(Some(_)) => error(res, StatusCode::CONFLICT, "gift already opened"),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "gift not found"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "gift open failed"),
    }
}

#[handler]
async fn answer(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Some(id) = gift_id(req, res) else { return };
    let body: AnswerInput = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if normalized_answer(&body.answer).is_empty() || body.answer.chars().count() > 20 {
        return error(res, StatusCode::BAD_REQUEST, "invalid answer");
    }
    let digest = answer_digest(id, &body.answer);
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<(bool, i32, bool)>, diesel::result::Error, _>(|conn| {
        let row = lock_gift(conn, id, uid, "recipient_id")?;
        let Some(row) = row else { return Ok(None) };
        if row.state != "opened" || row.attempts >= 3 {
            return Ok(Some((false, -1, false)));
        }
        if row.expired {
            crate::benefits::refund(conn, id, "expired")?;
            diesel::sql_query("UPDATE gifts SET state='expired',settled_at=now() WHERE id=$1")
                .bind::<BigInt, _>(id)
                .execute(conn)?;
            release_wish(conn, id)?;
            return Ok(Some((false, -2, false)));
        }
        let correct = row.answer_hash.as_deref() == Some(digest.as_str());
        let attempts = row.attempts + 1;
        let revealed = correct || attempts >= 3;
        diesel::sql_query(
            "UPDATE gifts SET attempts=$2, \
             state=CASE WHEN $3 THEN 'revealed' ELSE 'opened' END, \
             identity_known=CASE WHEN $3 THEN $4 ELSE identity_known END, \
             revealed_at=CASE WHEN $3 THEN now() ELSE revealed_at END WHERE id=$1",
        )
        .bind::<BigInt, _>(id)
        .bind::<Integer, _>(attempts)
        .bind::<Bool, _>(revealed)
        .bind::<Bool, _>(correct)
        .execute(conn)?;
        Ok(Some((correct, 3 - attempts, revealed)))
    });
    match result {
        Ok(Some((correct, left, revealed))) if left >= 0 => res.render(Json(json!({
            "id":id,"correct":correct,"attempts_left":left,
            "state":if revealed {"revealed"} else {"opened"},"identity_known":correct,
        }))),
        Ok(Some((_, -2, _))) => error(res, StatusCode::CONFLICT, "gift expired"),
        Ok(Some(_)) => error(res, StatusCode::CONFLICT, "gift not awaiting answer"),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "gift not found"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "answer failed"),
    }
}

#[handler]
async fn accept(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Some(id) = gift_id(req, res) else { return };
    let body: AcceptInput = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let name = body.recipient_name.as_deref().unwrap_or("").trim();
    let phone = body.recipient_phone.as_deref().unwrap_or("").trim();
    let address = body.recipient_address.as_deref().unwrap_or("").trim();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<&'static str>, diesel::result::Error, _>(|conn| {
        let row = lock_gift(conn, id, uid, "recipient_id")?;
        let Some(row) = row else { return Ok(None) };
        if row.state != "revealed" { return Ok(Some("bad_state")) }
        if !row.contract_text.is_empty() && body.agree != Some(true) { return Ok(Some("contract")) }
        if row.physical && (name.is_empty() || address.is_empty() || phone.len()!=11 || !phone.bytes().all(|b| b.is_ascii_digit())) {
            return Ok(Some("address"));
        }
        if row.physical {
            diesel::sql_query(
                "INSERT INTO shipments (gift_id,carrier,tracking_number,recipient_name,recipient_phone,recipient_address) \
                 VALUES ($1,'礼遇演示快递',$2,$3,$4,$5)",
            )
            .bind::<BigInt, _>(id)
            .bind::<Text, _>(format!("LY-DEMO-{id}"))
            .bind::<Text, _>(name)
            .bind::<Text, _>(phone)
            .bind::<Text, _>(address)
            .execute(conn)?;
            diesel::sql_query(
                "INSERT INTO shipment_events (gift_id,event_at,description) \
                 VALUES ($1,now(),'演示订单已创建，等待揽收')",
            )
            .bind::<BigInt, _>(id)
            .execute(conn)?;
        }
        let voucher = (!row.physical).then(|| format!("LY-{}", uuid::Uuid::new_v4().simple()));
        diesel::sql_query(
            "UPDATE gifts SET state='accepted',settled_at=now(), \
             identity_known=CASE WHEN contract_text<>'' THEN true ELSE identity_known END, \
             voucher_code=$2 WHERE id=$1",
        )
        .bind::<BigInt, _>(id)
        .bind::<Nullable<Text>, _>(voucher)
        .execute(conn)?;
        Ok(Some("accepted"))
    });
    match result {
        Ok(Some("accepted")) => res.render(Json(json!({"id":id,"state":"accepted"}))),
        Ok(Some("contract")) => error(res, StatusCode::CONFLICT, "contract agreement required"),
        Ok(Some("address")) => error(res, StatusCode::BAD_REQUEST, "recipient address required"),
        Ok(Some(_)) => error(res, StatusCode::CONFLICT, "gift not ready to accept"),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "gift not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "gift acceptance failed",
        ),
    }
}

#[handler]
async fn withdraw(req: &mut Request, res: &mut Response) {
    let Some(uid) = requester(req, res) else {
        return;
    };
    let Some(id) = gift_id(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<bool>, diesel::result::Error, _>(|conn| {
        let row = lock_gift(conn, id, uid, "sender_id")?;
        let Some(row) = row else { return Ok(None) };
        if row.state != "sealed" {
            return Ok(Some(false));
        }
        crate::benefits::refund(conn, id, "withdraw")?;
        diesel::sql_query("UPDATE gifts SET state='withdrawn',settled_at=now() WHERE id=$1")
            .bind::<BigInt, _>(id)
            .execute(conn)?;
        release_wish(conn, id)?;
        Ok(Some(true))
    });
    match result {
        Ok(Some(true)) => res.render(Json(json!({"id":id,"state":"withdrawn"}))),
        Ok(Some(false)) => error(res, StatusCode::CONFLICT, "gift already opened"),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "gift not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "gift withdraw failed",
        ),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/gifts/inbox").get(inbox))
        .push(Router::with_path("api/v1/gifts/outbox").get(outbox))
        .push(Router::with_path("api/v1/gifts/{id}").get(detail))
        .push(Router::with_path("api/v1/gifts/{id}/puzzle").put(configure_puzzle))
        .push(Router::with_path("api/v1/gifts/{id}/open").post(open))
        .push(Router::with_path("api/v1/gifts/{id}/answer").post(answer))
        .push(Router::with_path("api/v1/gifts/{id}/accept").post(accept))
        .push(Router::with_path("api/v1/gifts/{id}/withdraw").post(withdraw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(state: &str) -> GiftView {
        GiftView {
            id: 5,
            sender_id: 1,
            sender_name: "神秘送礼人".into(),
            product_id: 17,
            price_cents: 89900,
            physical: true,
            state: state.into(),
            unlock_kind: "question".into(),
            clue: "我们第一次一起看的电影？".into(),
            message: "毕业快乐".into(),
            contract_text: "有空喝杯咖啡".into(),
            attempts: 0,
            identity_known: false,
            voucher_code: Some("PRIVATE-CODE".into()),
            expired: false,
        }
    }

    fn sender_fixture(state: &str) -> SenderView {
        SenderView {
            id: 5,
            recipient_id: 2,
            recipient_name: "林舟".into(),
            notification_status: "in_app".into(),
            product_id: 17,
            price_cents: 89900,
            state: state.into(),
            expired: false,
            carrier_delivered: false,
            recipient_confirmed: false,
        }
    }

    #[test]
    fn sealed_recipient_and_sender_projections_do_not_leak() {
        let recipient = recipient_projection(fixture("sealed"));
        let sender = sender_projection(sender_fixture("exchanged"));
        assert_eq!(recipient["state"], "sealed");
        assert_eq!(sender["state"], "handled");
        for text in ["神秘送礼人", "89900", "17", "PRIVATE-CODE", "毕业快乐"] {
            assert!(!recipient.to_string().contains(text));
        }
        for text in [
            "exchanged",
            "PRIVATE-CODE",
            "contract_text",
            "recipient_address",
            "tracking_number",
        ] {
            assert!(!sender.to_string().contains(text));
        }
    }

    #[test]
    fn accepted_gift_sender_receives_only_original_order_and_delivery_flags() {
        let mut gift = sender_fixture("accepted");
        gift.carrier_delivered = true;
        gift.recipient_confirmed = true;
        let view = sender_projection(gift);
        assert_eq!(
            view,
            json!({
                "id": 5,
                "recipient": {"display_name":"林舟"},
                "delivery_status":"assigned",
                "notification_status":"in_app",
                "product_id": 17,
                "price_cents": 89900,
                "state": "handled",
                "carrier_delivered": true,
                "recipient_confirmed": true,
            })
        );
        for secret in [
            "VERY-PRIVATE-VOUCHER",
            "contract_text",
            "recipient_address",
            "tracking_number",
            "exchanged_item_id",
            "神秘送礼人",
        ] {
            assert!(!view.to_string().contains(secret));
        }
    }

    #[test]
    fn answer_normalization_and_digest_are_stable() {
        assert_eq!(normalized_answer("  电 影！A  "), "电影a");
        assert_eq!(normalized_answer("电影Ａ"), "电影a");
        assert_eq!(answer_digest(7, "电 影！A"), answer_digest(7, "电影a"));
        assert_ne!(answer_digest(7, "电影a"), answer_digest(8, "电影a"));
    }
}
