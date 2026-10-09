//! Mutually confirmed promise dates and explicit, short-lived iCalendar exports.
use crate::{error, hash_secret, new_session_token, pool, user_id};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_types::{BigInt, Bool, Jsonb, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = Jsonb)]
    data: Value,
}

pub(crate) fn ensure_schema(conn: &mut PgConnection) -> Result<(), String> {
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        diesel::sql_query("SELECT pg_advisory_xact_lock(731129940)").execute(conn)?;
        let section = include_str!("../migrations/20260930020000_contract_personal_marks/up.sql")
            .split_once("-- purchase and calendar extension (shared with startup upgrade)")
            .unwrap()
            .1
            .split("-- end purchase and calendar extension")
            .next()
            .unwrap();
        conn.batch_execute(section)
    })
    .map_err(|e| format!("purchase/calendar upgrade failed: {e}"))
}

fn schedule(conn: &mut PgConnection, uid: i64, id: i64) -> QueryResult<Option<Value>> {
    diesel::sql_query("SELECT jsonb_build_object('gift_id',g.id,'title',g.contract_text,'planned_on',COALESCE(to_char(d.confirmed_on,'YYYY-MM-DD'),''),'proposed_on',COALESCE(to_char(d.proposed_on,'YYYY-MM-DD'),''),'revision',COALESCE(d.revision,0),'pending',COALESCE(d.proposer_id IS NOT NULL AND d.confirmed_by IS NULL,false),'proposed_by_me',COALESCE(d.proposer_id=$1,false),'start_ms',(extract(epoch FROM d.confirmed_on::timestamp AT TIME ZONE 'UTC')*1000)::bigint,'end_ms',(extract(epoch FROM (d.confirmed_on+1)::timestamp AT TIME ZONE 'UTC')*1000)::bigint,'end_on',COALESCE(to_char(d.confirmed_on+1,'YYYYMMDD'),''),'status',COALESCE(m.status,'pending'),'stamp',to_char(now() AT TIME ZONE 'UTC','YYYYMMDD\"T\"HH24MISS\"Z\"')) AS data FROM gifts g LEFT JOIN gift_contract_marks m ON m.gift_id=g.id AND m.user_id=$1 LEFT JOIN gift_contract_schedules d ON d.gift_id=g.id WHERE g.id=$2 AND (g.sender_id=$1 OR g.recipient_id=$1) AND g.state='accepted' AND g.contract_text<>''")
        .bind::<BigInt,_>(uid).bind::<BigInt,_>(id).get_result::<Row>(conn).optional().map(|r| r.map(|r|r.data))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleInput {
    proposed_on: String,
    expected_revision: i64,
}

#[handler]
async fn get_schedule(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Some(id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match schedule(&mut conn, uid, id) {
        Ok(Some(row)) => res.render(Json(row)),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "accepted contract not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot read promise date",
        ),
    }
}

#[handler]
async fn put_schedule(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Some(id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let Ok(body) = req.parse_json::<ScheduleInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid date request");
    };
    if !crate::wishlist::valid_occasion_date(&body.proposed_on) || body.expected_revision < 0 {
        return error(
            res,
            StatusCode::BAD_REQUEST,
            "use a real date YYYY-MM-DD, or empty to clear",
        );
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<Value>,diesel::result::Error,_>(|conn| {
        // Serializes date edits with other participant writes on this gift.
        #[derive(QueryableByName)] struct Exists { #[diesel(sql_type = Bool)] present: bool }
        let exists = diesel::sql_query("SELECT true AS present FROM gifts WHERE id=$2 AND (sender_id=$1 OR recipient_id=$1) AND state='accepted' AND contract_text<>'' FOR UPDATE")
            .bind::<BigInt,_>(uid).bind::<BigInt,_>(id).get_result::<Exists>(conn).optional()?;
        if !exists.is_some_and(|r|r.present) { return Ok(None) }
        let prior = schedule(conn,uid,id)?.unwrap();
        if prior["revision"] != body.expected_revision { return Ok(Some(json!({"conflict":true}))) }
        diesel::sql_query("INSERT INTO gift_contract_schedules(gift_id,proposed_on,proposer_id) VALUES($2,NULLIF($3,'')::date,$1) ON CONFLICT(gift_id) DO UPDATE SET proposed_on=excluded.proposed_on,proposer_id=$1,confirmed_by=NULL,revision=gift_contract_schedules.revision+1,updated_at=now()")
            .bind::<BigInt,_>(uid).bind::<BigInt,_>(id).bind::<Text,_>(&body.proposed_on).execute(conn)?;
        schedule(conn,uid,id)
    });
    match result {
        Ok(Some(row)) if row["conflict"] == true => error(
            res,
            StatusCode::CONFLICT,
            "date changed; reload before saving",
        ),
        Ok(Some(row)) => res.render(Json(row)),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "accepted contract not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot propose promise date",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    expected_revision: i64,
}

#[handler]
async fn confirm_schedule(req: &mut Request, res: &mut Response) {
    decide(req, res, false).await;
}
#[handler]
async fn cancel_schedule(req: &mut Request, res: &mut Response) {
    decide(req, res, true).await;
}
async fn decide(req: &mut Request, res: &mut Response, cancel: bool) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Some(id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let Ok(body) = req.parse_json::<Decision>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid revision");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Option<Value>,diesel::result::Error,_>(|conn| {
        #[derive(QueryableByName)] struct Locked { #[diesel(sql_type=BigInt)] id:i64 }
        let found=diesel::sql_query("SELECT id FROM gifts WHERE id=$2 AND (sender_id=$1 OR recipient_id=$1) AND state='accepted' AND contract_text<>'' FOR UPDATE")
            .bind::<BigInt,_>(uid).bind::<BigInt,_>(id).get_result::<Locked>(conn).optional()?;
        if !found.is_some_and(|r|r.id==id) {return Ok(None)}
        let prior=schedule(conn,uid,id)?.unwrap();
        if prior["revision"]!=body.expected_revision || prior["pending"]!=true || (cancel && prior["proposed_by_me"]!=true) || (!cancel && prior["proposed_by_me"]==true) {return Ok(Some(json!({"conflict":true})))}
        let sql=if cancel {"UPDATE gift_contract_schedules SET proposer_id=NULL,proposed_on=NULL,confirmed_by=NULL,revision=revision+1,updated_at=now() WHERE gift_id=$2 AND proposer_id=$1"}
            else {"UPDATE gift_contract_schedules SET confirmed_on=proposed_on,confirmed_by=$1,revision=revision+1,updated_at=now() WHERE gift_id=$2 AND proposer_id<>$1"};
        diesel::sql_query(sql).bind::<BigInt,_>(uid).bind::<BigInt,_>(id).execute(conn)?;
        schedule(conn,uid,id)
    });
    match result {
        Ok(Some(row)) if row["conflict"] == true => error(
            res,
            StatusCode::CONFLICT,
            "proposal changed or this participant cannot approve/cancel it",
        ),
        Ok(Some(row)) => res.render(Json(row)),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "accepted contract not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "date decision failed",
        ),
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

// RFC 5545 folding counts UTF-8 bytes and never splits a code point.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut bytes = 0;
    for c in line.chars() {
        if bytes + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            bytes = 1;
        }
        out.push(c);
        bytes += c.len_utf8();
    }
    out.push_str("\r\n");
    out
}

fn calendar(uid: i64, row: &Value) -> String {
    let id = row["gift_id"].as_i64().unwrap();
    let mut lines = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//LIYU//Confirmed Promise//ZH".into(),
        "CALSCALE:GREGORIAN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:liyu-{uid}-{id}@personal.liyu"),
        format!("DTSTAMP:{}", row["stamp"].as_str().unwrap()),
        format!(
            "DTSTART;VALUE=DATE:{}",
            row["planned_on"].as_str().unwrap().replace('-', "")
        ),
        format!("DTEND;VALUE=DATE:{}", row["end_on"].as_str().unwrap()),
        format!(
            "SUMMARY:礼遇约定：{}",
            escape(row["title"].as_str().unwrap())
        ),
        "DESCRIPTION:双方已确认的约定日期。改期须再次确认并重新导入；不会自动同步或发送日历邀请。"
            .into(),
        "TRANSP:TRANSPARENT".into(),
    ];
    if row["status"] == "pending" {
        lines.extend([
            "BEGIN:VALARM".into(),
            "ACTION:DISPLAY".into(),
            "TRIGGER:-PT12H".into(),
            "DESCRIPTION:礼遇约定提醒".into(),
            "END:VALARM".into(),
        ]);
    }
    lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
    lines.iter().map(|s| fold(s)).collect()
}

struct Export {
    uid: i64,
    gift: i64,
    session: String,
    key: String,
    date: String,
    expires: Instant,
}
fn exports() -> &'static Mutex<HashMap<String, Export>> {
    static MAP: OnceLock<Mutex<HashMap<String, Export>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}

#[handler]
async fn create_export(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Some(id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let Ok(Some(row)) = schedule(&mut conn, uid, id) else {
        return error(res, StatusCode::NOT_FOUND, "accepted contract not found");
    };
    if row["planned_on"] == "" {
        return error(
            res,
            StatusCode::CONFLICT,
            "both participants must confirm a date before exporting",
        );
    }
    let id = uuid::Uuid::new_v4().to_string();
    let key = new_session_token();
    let mut map = exports().lock().unwrap();
    map.retain(|_, v| v.expires > Instant::now());
    if map.len() >= 256 {
        return error(
            res,
            StatusCode::TOO_MANY_REQUESTS,
            "too many pending calendar exports",
        );
    }
    map.insert(
        id.clone(),
        Export {
            uid,
            gift: row["gift_id"].as_i64().unwrap(),
            session: hash_secret(crate::bearer(req).unwrap_or_default()),
            key: hash_secret(&key),
            date: row["planned_on"].as_str().unwrap().into(),
            expires: Instant::now() + Duration::from_secs(300),
        },
    );
    res.render(Json(
        json!({"browser_path":format!("/calendar-export/{id}#{key}"),"expires_in_seconds":300}),
    ));
}

#[handler]
async fn export_page(_req: &mut Request, res: &mut Response) {
    res.headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("Referrer-Policy", "no-referrer".parse().unwrap());
    res.headers_mut().insert("Content-Security-Policy","default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'".parse().unwrap());
    res.render(salvo::prelude::Text::Html(include_str!(
        "calendar_export.html"
    )));
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportKey {
    key: String,
}
#[handler]
async fn download(req: &mut Request, res: &mut Response) {
    res.headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    let id = req.param::<String>("id").unwrap_or_default();
    let Ok(body) = req.parse_json::<ExportKey>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid export key");
    };
    // Wrong guesses do not consume a legitimate export. Correct access is single-use.
    let export = {
        let mut map = exports().lock().unwrap();
        map.retain(|_, v| v.expires > Instant::now());
        if !map
            .get(&id)
            .is_some_and(|v| v.key == hash_secret(&body.key))
        {
            return error(res, StatusCode::GONE, "export expired or unavailable");
        }
        map.remove(&id).unwrap()
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    if !matches!(crate::session_owner(&mut conn,&export.session),Ok(Some(uid)) if uid==export.uid) {
        return error(
            res,
            StatusCode::GONE,
            "session revoked; export again after login",
        );
    }
    match schedule(&mut conn,export.uid,export.gift) {
        Ok(Some(row)) if row["planned_on"]==export.date => res.render(Json(json!({"filename":format!("liyu-promise-{}.ics",export.gift),"content":calendar(export.uid,&row)}))),
        _=>error(res,StatusCode::CONFLICT,"promise date changed; export again"),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(
            Router::with_path("api/v1/contracts/{id}/schedule")
                .get(get_schedule)
                .put(put_schedule),
        )
        .push(Router::with_path("api/v1/contracts/{id}/calendar-export").post(create_export))
        .push(Router::with_path("api/v1/contracts/{id}/schedule/confirm").post(confirm_schedule))
        .push(Router::with_path("api/v1/contracts/{id}/schedule/cancel").post(cancel_schedule))
        .push(Router::with_path("calendar-export/{id}").get(export_page))
        .push(Router::with_path("api/v1/calendar-exports/{id}/download").post(download))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_escapes_injection_and_folds_utf8() {
        let row = json!({"gift_id":1,"planned_on":"2028-02-29","end_on":"20280301","stamp":"20261009T000000Z","status":"pending","title":"电影;咖啡,\\\r\nEND:VEVENT"});
        let ics = calendar(7, &row);
        assert!(ics.contains("DTEND;VALUE=DATE:20280301\r\n"));
        assert!(ics.contains("电影\\;咖啡\\,\\\\\\nEND:VEVENT"));
        assert_eq!(ics.matches("\r\nEND:VEVENT\r\n").count(), 1);
        let folded = fold(&"礼".repeat(80));
        assert!(folded.split("\r\n").all(|s| s.len() <= 75));
        assert_eq!(folded.replace("\r\n ", "").trim_end(), "礼".repeat(80));
    }
}
