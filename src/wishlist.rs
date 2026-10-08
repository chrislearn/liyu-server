//! Confirmed-friend wishlists with explicit owner and friend projections.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Integer, Nullable, Text};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{error, pool, user_id};

const FRIEND_VISIBLE: &str = "w.published_at IS NOT NULL AND EXISTS (SELECT 1 FROM friendships f WHERE f.status = 'accepted' \
    AND f.user_low_id = LEAST(w.owner_id, $2) AND f.user_high_id = GREATEST(w.owner_id, $2)) \
    AND (NOT EXISTS (SELECT 1 FROM wishlist_audience a WHERE a.wishlist_id = w.id) \
    OR EXISTS (SELECT 1 FROM wishlist_audience a WHERE a.wishlist_id = w.id AND a.user_id = $2))";

#[derive(Deserialize)]
struct FriendRequest {
    user_id: i64,
}

#[derive(Deserialize)]
struct WishItemInput {
    product_id: Option<i32>,
    kind: Option<String>,
    max_price_cents: Option<i64>,
    wants: Option<String>,
}

#[derive(Deserialize)]
struct WishlistInput {
    title: String,
    note: Option<String>,
    occasion: Option<String>,
    event_on: String,
    items: Vec<WishItemInput>,
    audience_user_ids: Option<Vec<i64>>,
    expires_hours: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WishlistDraftInput {
    title: String,
    note: Option<String>,
    #[serde(default)]
    items: Vec<WishItemInput>,
}

#[derive(Deserialize)]
struct WishlistEdit {
    title: String,
    note: Option<String>,
    occasion: Option<String>,
    event_on: String,
    audience_user_ids: Option<Vec<i64>>,
    expires_hours: Option<i32>,
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct FriendRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    display_name: String,
    #[diesel(sql_type = Text)]
    account_name: String,
    #[diesel(sql_type = Text)]
    nickname: String,
    #[diesel(sql_type = Text)]
    phone: String,
    #[diesel(sql_type = Text)]
    email: String,
    #[diesel(sql_type = Text)]
    relationship: String,
    #[diesel(sql_type = Text)]
    birthday: String,
    #[diesel(sql_type = Text)]
    wedding_date: String,
    #[diesel(sql_type = Text)]
    note: String,
}

#[derive(Deserialize)]
struct FriendDetailsInput {
    nickname: String,
    phone: String,
    email: String,
    relationship: String,
    birthday: String,
    #[serde(default)]
    wedding_date: String,
    note: String,
}

fn valid_occasion_date(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[0..4].parse().unwrap_or(0);
    let month: usize = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..10].parse().unwrap_or(0);
    if !(1900..=2100).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    day >= 1 && day <= days[month - 1]
}

fn valid_friend_details(d: &FriendDetailsInput) -> bool {
    [
        (&d.nickname, 100),
        (&d.phone, 100),
        (&d.email, 254),
        (&d.relationship, 100),
        (&d.birthday, 40),
        (&d.wedding_date, 40),
        (&d.note, 500),
    ]
    .into_iter()
    .all(|(value, max)| value.chars().count() <= max && !value.chars().any(char::is_control))
        && (d.phone.trim().is_empty()
            || crate::contact_delivery::normalize("phone", &d.phone).is_some())
        && (d.email.trim().is_empty()
            || crate::contact_delivery::normalize("email", &d.email).is_some())
        && valid_occasion_date(d.birthday.trim())
        && valid_occasion_date(d.wedding_date.trim())
}

#[derive(QueryableByName)]
struct WishlistRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    owner_id: i64,
    #[diesel(sql_type = Text)]
    owner_name: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    note: String,
    #[diesel(sql_type = Text)]
    occasion: String,
    #[diesel(sql_type = Text)]
    event_on: String,
    #[diesel(sql_type = Bool)]
    is_open: bool,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = BigInt)]
    item_count: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
}

#[derive(QueryableByName)]
struct WishItemRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Nullable<Integer>)]
    product_id: Option<i32>,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = BigInt)]
    max_price_cents: i64,
    #[diesel(sql_type = Text)]
    wants: String,
    #[diesel(sql_type = Bool)]
    claimed: bool,
    #[diesel(sql_type = Bool)]
    by_me: bool,
}

fn user(req: &Request, res: &mut Response) -> Option<i64> {
    match user_id(req) {
        Some(uid) => Some(uid),
        None => {
            error(res, StatusCode::UNAUTHORIZED, "invalid session");
            None
        }
    }
}

fn path_id(req: &Request, key: &str, res: &mut Response) -> Option<i64> {
    match req.param::<i64>(key).filter(|id| *id > 0) {
        Some(id) => Some(id),
        None => {
            error(res, StatusCode::BAD_REQUEST, "invalid id");
            None
        }
    }
}

fn valid_date(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn valid_header(title: &str, note: &str, occasion: &str, event_on: &str) -> bool {
    let n = title.trim().chars().count();
    n > 0
        && n <= 16
        && note.chars().count() <= 40
        && occasion.chars().count() <= 20
        && valid_date(event_on)
}

fn valid_expiry(hours: i32) -> bool {
    (1..=720).contains(&hours)
}

fn valid_item(item: &WishItemInput) -> bool {
    let kind = item.kind.as_deref().unwrap_or("").trim();
    let wants = item.wants.as_deref().unwrap_or("");
    (item.product_id.is_some_and(|id| id >= 0) || !kind.is_empty())
        && kind.chars().count() <= 30
        && wants.chars().count() <= 20
        && item.max_price_cents.unwrap_or(0) >= 0
}

fn summary(row: WishlistRow) -> Value {
    json!({
        "id": row.id,
        "owner": {"id": row.owner_id, "display_name": row.owner_name},
        "title": row.title,
        "note": row.note,
        "occasion": row.occasion,
        "event_on": row.event_on,
        "is_open": row.is_open,
        "status": row.status,
        "item_count": row.item_count,
        "expires_at": row.expires_at,
    })
}

fn item_projection(row: WishItemRow, owner: bool) -> Value {
    let status = if !row.claimed {
        "open"
    } else if !owner && row.by_me {
        "by_me"
    } else {
        "claimed"
    };
    json!({
        "id": row.id,
        "product_id": row.product_id,
        "kind": row.kind,
        "max_price_cents": row.max_price_cents,
        "wants": row.wants,
        "status": status,
    })
}

fn friend_count(conn: &mut PgConnection, uid: i64, other: i64) -> QueryResult<i64> {
    diesel::sql_query(
        "SELECT count(*) AS count FROM friendships WHERE status='accepted' \
         AND user_low_id=LEAST($1,$2) AND user_high_id=GREATEST($1,$2)",
    )
    .bind::<BigInt, _>(uid)
    .bind::<BigInt, _>(other)
    .get_result::<CountRow>(conn)
    .map(|row| row.count)
}

fn validate_audience(conn: &mut PgConnection, owner: i64, audience: &[i64]) -> QueryResult<bool> {
    let mut seen = std::collections::HashSet::new();
    for &other in audience {
        if other <= 0
            || other == owner
            || !seen.insert(other)
            || friend_count(conn, owner, other)? != 1
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn replace_audience(conn: &mut PgConnection, wid: i64, audience: &[i64]) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM wishlist_audience WHERE wishlist_id=$1")
        .bind::<BigInt, _>(wid)
        .execute(conn)?;
    for &other in audience {
        diesel::sql_query("INSERT INTO wishlist_audience (wishlist_id,user_id) VALUES ($1,$2)")
            .bind::<BigInt, _>(wid)
            .bind::<BigInt, _>(other)
            .execute(conn)?;
    }
    Ok(())
}

fn insert_item(
    conn: &mut PgConnection,
    wid: i64,
    ordinal: i32,
    item: &WishItemInput,
) -> QueryResult<i64> {
    diesel::sql_query(
        "INSERT INTO wishlist_items (wishlist_id,ordinal,product_id,kind,max_price_cents,wants) \
         VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
    )
    .bind::<BigInt, _>(wid)
    .bind::<Integer, _>(ordinal)
    .bind::<Nullable<Integer>, _>(item.product_id)
    .bind::<Text, _>(item.kind.as_deref().unwrap_or("").trim())
    .bind::<BigInt, _>(item.max_price_cents.unwrap_or(0))
    .bind::<Text, _>(item.wants.as_deref().unwrap_or("").trim())
    .get_result::<IdRow>(conn)
    .map(|row| row.id)
}

#[handler]
async fn friends(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let rows = diesel::sql_query(
        "SELECT u.id,COALESCE(NULLIF(d.nickname,''),u.display_name) AS display_name, \
         u.display_name AS account_name,COALESCE(d.nickname,'') AS nickname, \
         COALESCE(d.phone,'') AS phone,COALESCE(d.email,'') AS email, \
         COALESCE(d.relationship,'') AS relationship,COALESCE(d.birthday,'') AS birthday, \
         COALESCE(d.wedding_date,'') AS wedding_date, \
         COALESCE(d.note,'') AS note FROM friendships f JOIN users u ON \
         u.id=CASE WHEN f.user_low_id=$1 THEN f.user_high_id ELSE f.user_low_id END \
         LEFT JOIN friend_details d ON d.owner_id=$1 AND d.friend_id=u.id \
         WHERE f.status='accepted' AND (f.user_low_id=$1 OR f.user_high_id=$1) \
         ORDER BY display_name,u.id",
    )
    .bind::<BigInt, _>(uid)
    .load::<FriendRow>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(
            rows.into_iter()
                .map(|r| json!({"id":r.id,"display_name":r.display_name,"account_name":r.account_name,
                    "nickname":r.nickname,"phone":r.phone,"email":r.email,"relationship":r.relationship,
                    "birthday":r.birthday,"wedding_date":r.wedding_date,"note":r.note}))
                .collect::<Vec<_>>(),
        )),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "friends query failed",
        ),
    }
}

#[handler]
async fn put_friend_details(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(friend_id) = req.param::<i64>("id").filter(|id| *id > 0 && *id != uid) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid friend id");
    };
    let Ok(input) = req.parse_json::<FriendDetailsInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid friend details");
    };
    if !valid_friend_details(&input) {
        return error(res, StatusCode::BAD_REQUEST, "invalid friend details");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let changed = diesel::sql_query(
        "INSERT INTO friend_details(owner_id,friend_id,nickname,phone,email,relationship,birthday,wedding_date,note) \
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9 WHERE EXISTS ( \
           SELECT 1 FROM friendships f WHERE f.status='accepted' \
           AND f.user_low_id=LEAST($1,$2) AND f.user_high_id=GREATEST($1,$2)) \
         ON CONFLICT(owner_id,friend_id) DO UPDATE SET \
           nickname=excluded.nickname,phone=excluded.phone,email=excluded.email, \
           relationship=excluded.relationship,birthday=excluded.birthday, \
           wedding_date=excluded.wedding_date,note=excluded.note,updated_at=now()",
    )
    .bind::<BigInt, _>(uid)
    .bind::<BigInt, _>(friend_id)
    .bind::<Text, _>(input.nickname.trim())
    .bind::<Text, _>(input.phone.trim())
    .bind::<Text, _>(input.email.trim())
    .bind::<Text, _>(input.relationship.trim())
    .bind::<Text, _>(input.birthday.trim())
    .bind::<Text, _>(input.wedding_date.trim())
    .bind::<Text, _>(input.note.trim())
    .execute(&mut conn);
    match changed {
        Ok(1) => res.render(Json(json!({"updated":true}))),
        Ok(_) => error(res, StatusCode::NOT_FOUND, "friend not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "friend details update failed",
        ),
    }
}

#[handler]
async fn request_friend(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let body: FriendRequest = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if body.user_id <= 0 || body.user_id == uid {
        return error(res, StatusCode::BAD_REQUEST, "invalid friend");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row = diesel::sql_query(
        "INSERT INTO friendships (user_low_id,user_high_id,requested_by_user_id) \
         SELECT LEAST($1,$2),GREATEST($1,$2),$1 FROM users WHERE id=$2 \
         ON CONFLICT (user_low_id,user_high_id) DO NOTHING RETURNING id",
    )
    .bind::<BigInt, _>(uid)
    .bind::<BigInt, _>(body.user_id)
    .get_result::<IdRow>(&mut conn)
    .optional();
    match row {
        Ok(Some(row)) => res.render(Json(json!({"id":row.id,"status":"pending"}))),
        Ok(None) => error(
            res,
            StatusCode::CONFLICT,
            "friend request already exists or user not found",
        ),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "friend request failed",
        ),
    }
}

#[handler]
async fn accept_friend(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(fid) = path_id(req, "id", res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row = diesel::sql_query(
        "UPDATE friendships SET status='accepted',accepted_at=now() \
         WHERE id=$1 AND status='pending' AND requested_by_user_id<>$2 \
         AND (user_low_id=$2 OR user_high_id=$2) RETURNING id",
    )
    .bind::<BigInt, _>(fid)
    .bind::<BigInt, _>(uid)
    .get_result::<IdRow>(&mut conn)
    .optional();
    match row {
        Ok(Some(row)) => res.render(Json(json!({"id":row.id,"status":"accepted"}))),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "friend request not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "accept friend failed",
        ),
    }
}

#[handler]
async fn my_wishlists(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let rows = diesel::sql_query(
        "SELECT w.id,w.owner_id,u.display_name AS owner_name,w.title,w.note,w.occasion, \
         w.event_on::text AS event_on, extract(epoch from w.expires_at)::bigint AS expires_at, \
         (w.published_at IS NOT NULL AND w.closed_at IS NULL AND w.expires_at>now() AND w.event_on >= CURRENT_DATE - 7) AS is_open, \
         CASE WHEN w.closed_at IS NOT NULL THEN 'closed' WHEN w.published_at IS NULL THEN 'draft' \
         WHEN w.expires_at<=now() OR w.event_on<CURRENT_DATE-7 THEN 'expired' ELSE 'published' END AS status, \
         (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id=w.id) AS item_count \
         FROM wishlists w JOIN users u ON u.id=w.owner_id \
         WHERE w.owner_id=$1 ORDER BY w.event_on DESC,w.id DESC",
    )
    .bind::<BigInt, _>(uid)
    .load::<WishlistRow>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(rows.into_iter().map(summary).collect::<Vec<_>>())),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "wishlists query failed",
        ),
    }
}

#[handler]
async fn friend_wishlists(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let query = format!(
        "SELECT w.id,w.owner_id,u.display_name AS owner_name,w.title,w.note,w.occasion, \
         w.event_on::text AS event_on, extract(epoch from w.expires_at)::bigint AS expires_at, \
         (w.published_at IS NOT NULL AND w.closed_at IS NULL AND w.expires_at>now() AND w.event_on >= CURRENT_DATE - 7) AS is_open, \
         CASE WHEN w.closed_at IS NOT NULL THEN 'closed' WHEN w.published_at IS NULL THEN 'draft' \
         WHEN w.expires_at<=now() OR w.event_on<CURRENT_DATE-7 THEN 'expired' ELSE 'published' END AS status, \
         (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id=w.id) AS item_count \
         FROM wishlists w JOIN users u ON u.id=w.owner_id \
         WHERE w.owner_id<>$1 AND {} ORDER BY w.event_on,w.id",
        FRIEND_VISIBLE.replace("$2", "$1")
    );
    let rows = diesel::sql_query(query)
        .bind::<BigInt, _>(uid)
        .load::<WishlistRow>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(rows.into_iter().map(summary).collect::<Vec<_>>())),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "friend wishlists query failed",
        ),
    }
}

#[handler]
async fn get_wishlist(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let query = format!(
        "SELECT w.id,w.owner_id,u.display_name AS owner_name,w.title,w.note,w.occasion, \
         w.event_on::text AS event_on, extract(epoch from w.expires_at)::bigint AS expires_at, \
         (w.published_at IS NOT NULL AND w.closed_at IS NULL AND w.expires_at>now() AND w.event_on >= CURRENT_DATE - 7) AS is_open, \
         CASE WHEN w.closed_at IS NOT NULL THEN 'closed' WHEN w.published_at IS NULL THEN 'draft' \
         WHEN w.expires_at<=now() OR w.event_on<CURRENT_DATE-7 THEN 'expired' ELSE 'published' END AS status, \
         (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id=w.id) AS item_count \
         FROM wishlists w JOIN users u ON u.id=w.owner_id \
         WHERE w.id=$1 AND (w.owner_id=$2 OR ({}))",
        FRIEND_VISIBLE
    );
    let row = diesel::sql_query(query)
        .bind::<BigInt, _>(wid)
        .bind::<BigInt, _>(uid)
        .get_result::<WishlistRow>(&mut conn)
        .optional();
    let row = match row {
        Ok(Some(row)) => row,
        Ok(None) => return error(res, StatusCode::NOT_FOUND, "wishlist not found"),
        Err(_) => {
            return error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "wishlist query failed",
            );
        }
    };
    let is_owner = row.owner_id == uid;
    let items = diesel::sql_query(
        "SELECT id,product_id,kind,max_price_cents,wants, \
         (claimed_gift_id IS NOT NULL) AS claimed, \
         COALESCE(claimed_by_user_id=$2,false) AS by_me \
         FROM wishlist_items WHERE wishlist_id=$1 ORDER BY ordinal,id",
    )
    .bind::<BigInt, _>(wid)
    .bind::<BigInt, _>(uid)
    .load::<WishItemRow>(&mut conn);
    let items = match items {
        Ok(items) => items,
        Err(_) => {
            return error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "wishlist items query failed",
            );
        }
    };
    let mut output = summary(row);
    output["items"] = Value::Array(
        items
            .into_iter()
            .map(|i| item_projection(i, is_owner))
            .collect(),
    );
    if is_owner {
        let audience = diesel::sql_query(
            "SELECT user_id AS id FROM wishlist_audience WHERE wishlist_id=$1 ORDER BY user_id",
        )
        .bind::<BigInt, _>(wid)
        .load::<IdRow>(&mut conn);
        match audience {
            Ok(audience) => {
                output["audience_user_ids"] =
                    json!(audience.into_iter().map(|a| a.id).collect::<Vec<_>>())
            }
            Err(_) => {
                return error(
                    res,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "wishlist audience query failed",
                );
            }
        }
    }
    res.render(Json(output));
}

fn active_product(conn: &mut PgConnection, pid: i32) -> QueryResult<bool> {
    diesel::sql_query("SELECT count(*) AS count FROM catalog WHERE id=$1 AND is_active")
        .bind::<Integer, _>(pid)
        .get_result::<CountRow>(conn)
        .map(|r| r.count == 1)
}

#[handler]
async fn create_draft(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Ok(body) = req.parse_json::<WishlistDraftInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let note = body.note.as_deref().unwrap_or("").trim();
    let mut seen = std::collections::HashSet::new();
    if body.title.trim().is_empty()
        || body.title.trim().chars().count() > 16
        || note.chars().count() > 40
        || body.items.len() > 8
        || !body
            .items
            .iter()
            .all(|item| valid_item(item) && item.product_id.is_none_or(|pid| seen.insert(pid)))
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid wishlist draft");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Option<i64>,diesel::result::Error,_>(|conn| {
        for item in &body.items { if let Some(pid)=item.product_id { if !active_product(conn,pid)? { return Ok(None) } } }
        let id=diesel::sql_query("INSERT INTO wishlists(owner_id,title,note,event_on,expires_at,published_at) VALUES($1,$2,$3,CURRENT_DATE,now()+interval '24 hours',NULL) RETURNING id")
            .bind::<BigInt,_>(uid).bind::<Text,_>(body.title.trim()).bind::<Text,_>(note).get_result::<IdRow>(conn)?.id;
        for (ordinal,item) in body.items.iter().enumerate() { insert_item(conn,id,ordinal as i32,item)?; }
        Ok(Some(id))
    });
    match result {
        Ok(Some(id)) => res.render(Json(json!({"id":id,"status":"draft"}))),
        Ok(None) => error(res, StatusCode::BAD_REQUEST, "product unavailable"),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "draft create failed"),
    }
}

#[handler]
async fn publish_draft(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let Ok(body) = req.parse_json::<WishlistEdit>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let note = body.note.as_deref().unwrap_or("").trim();
    let occasion = body.occasion.as_deref().unwrap_or("other").trim();
    let hours = body.expires_hours.unwrap_or(24);
    if !valid_header(&body.title, note, occasion, &body.event_on) || !valid_expiry(hours) {
        return error(res, StatusCode::BAD_REQUEST, "invalid wishlist");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Option<&'static str>,diesel::result::Error,_>(|conn| {
        let owned=diesel::sql_query("SELECT id FROM wishlists WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind::<BigInt,_>(wid).bind::<BigInt,_>(uid).get_result::<IdRow>(conn).optional()?;
        if owned.is_none() { return Ok(None) }
        let published=diesel::sql_query("SELECT count(*) AS count FROM wishlists WHERE id=$1 AND published_at IS NOT NULL AND closed_at IS NULL")
            .bind::<BigInt,_>(wid).get_result::<CountRow>(conn)?.count;
        if published==1 { return Ok(Some("published")) }
        let available=diesel::sql_query("SELECT count(*) AS count FROM wishlists w WHERE w.id=$1 AND w.closed_at IS NULL AND $2::date BETWEEN CURRENT_DATE AND CURRENT_DATE+180 AND (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id=w.id) BETWEEN 1 AND 8 AND NOT EXISTS (SELECT 1 FROM wishlist_items i JOIN catalog c ON c.id=i.product_id WHERE i.wishlist_id=w.id AND NOT c.is_active)")
            .bind::<BigInt,_>(wid).bind::<Text,_>(&body.event_on).get_result::<CountRow>(conn)?.count;
        let audience = if let Some(audience) = &body.audience_user_ids { audience.clone() } else {
            diesel::sql_query("SELECT user_id AS id FROM wishlist_audience WHERE wishlist_id=$1 ORDER BY user_id")
                .bind::<BigInt,_>(wid).load::<IdRow>(conn)?.into_iter().map(|r|r.id).collect()
        };
        if available!=1 || !validate_audience(conn,uid,&audience)? { return Ok(Some("invalid")) }
        diesel::sql_query("UPDATE wishlists SET title=$2,note=$3,occasion=$4,event_on=$5::date,expires_at=now()+$6*interval '1 hour',published_at=now() WHERE id=$1")
            .bind::<BigInt,_>(wid).bind::<Text,_>(body.title.trim()).bind::<Text,_>(note).bind::<Text,_>(occasion).bind::<Text,_>(&body.event_on).bind::<Integer,_>(hours).execute(conn)?;
        replace_audience(conn,wid,&audience)?;
        Ok(Some("published"))
    });
    match result {
        Ok(Some("published")) => res.render(Json(json!({"id":wid,"status":"published"}))),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "wishlist not found"),
        Ok(Some(_)) => error(
            res,
            StatusCode::CONFLICT,
            "empty draft, invalid date/audience, or unavailable product",
        ),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "wishlist publish failed"),
    }
}

#[handler]
async fn create_wishlist(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let body: WishlistInput = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let note = body.note.as_deref().unwrap_or("").trim();
    let occasion = body.occasion.as_deref().unwrap_or("other").trim();
    let audience = body.audience_user_ids.as_deref().unwrap_or(&[]);
    if !valid_header(&body.title, note, occasion, &body.event_on)
        || !valid_expiry(body.expires_hours.unwrap_or(24))
        || body.items.is_empty()
        || body.items.len() > 8
        || !body.items.iter().all(valid_item)
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid wishlist");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<i64>, diesel::result::Error, _>(|conn| {
        if !validate_audience(conn, uid, audience)? {
            return Ok(None);
        }
        let row = diesel::sql_query(
            "INSERT INTO wishlists (owner_id,title,note,occasion,event_on,expires_at) \
             SELECT $1,$2,$3,$4,$5::date,now()+$6*interval '1 hour' WHERE $5::date BETWEEN CURRENT_DATE AND CURRENT_DATE+180 \
             RETURNING id",
        )
        .bind::<BigInt, _>(uid)
        .bind::<Text, _>(body.title.trim())
        .bind::<Text, _>(note)
        .bind::<Text, _>(occasion)
        .bind::<Text, _>(&body.event_on)
        .bind::<Integer, _>(body.expires_hours.unwrap_or(24))
        .get_result::<IdRow>(conn)
        .optional()?;
        let Some(row) = row else { return Ok(None) };
        for (ordinal, item) in body.items.iter().enumerate() {
            insert_item(conn, row.id, ordinal as i32, item)?;
        }
        replace_audience(conn, row.id, audience)?;
        Ok(Some(row.id))
    });
    match result {
        Ok(Some(id)) => res.render(Json(json!({"id":id}))),
        Ok(None) => error(res, StatusCode::BAD_REQUEST, "invalid date or audience"),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "wishlist create failed"),
    }
}

#[handler]
async fn edit_wishlist(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let body: WishlistEdit = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let note = body.note.as_deref().unwrap_or("").trim();
    let occasion = body.occasion.as_deref().unwrap_or("other").trim();
    let audience = body.audience_user_ids.as_deref();
    if !valid_header(&body.title, note, occasion, &body.event_on)
        || body.expires_hours.is_some_and(|hours| !valid_expiry(hours))
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid wishlist");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<bool, diesel::result::Error, _>(|conn| {
        if let Some(audience) = audience {
            if !validate_audience(conn, uid, audience)? {
                return Ok(false);
            }
        }
        let updated = diesel::sql_query(
            "UPDATE wishlists SET title=$3,note=$4,occasion=$5,event_on=$6::date, \
             expires_at=CASE WHEN $7::integer IS NULL THEN expires_at ELSE now()+$7*interval '1 hour' END \
             WHERE id=$1 AND owner_id=$2 AND closed_at IS NULL \
             AND (published_at IS NULL OR (expires_at>now() AND event_on >= CURRENT_DATE-7)) \
             AND $6::date BETWEEN CURRENT_DATE AND CURRENT_DATE+180",
        )
        .bind::<BigInt, _>(wid)
        .bind::<BigInt, _>(uid)
        .bind::<Text, _>(body.title.trim())
        .bind::<Text, _>(note)
        .bind::<Text, _>(occasion)
        .bind::<Text, _>(&body.event_on)
        .bind::<Nullable<Integer>, _>(body.expires_hours)
        .execute(conn)?;
        if updated == 0 {
            return Ok(false);
        }
        if let Some(audience) = audience {
            replace_audience(conn, wid, audience)?;
        }
        Ok(true)
    });
    match result {
        Ok(true) => res.render(Json(json!({"id":wid,"updated":true}))),
        Ok(false) => error(
            res,
            StatusCode::CONFLICT,
            "wishlist not editable or audience invalid",
        ),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "wishlist update failed"),
    }
}

#[handler]
async fn close_wishlist(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row = diesel::sql_query(
        "UPDATE wishlists SET closed_at=COALESCE(closed_at,now()) \
         WHERE id=$1 AND owner_id=$2 AND published_at IS NOT NULL RETURNING id",
    )
    .bind::<BigInt, _>(wid)
    .bind::<BigInt, _>(uid)
    .get_result::<IdRow>(&mut conn)
    .optional();
    match row {
        Ok(Some(_)) => res.render(Json(json!({"id":wid,"is_open":false}))),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "wishlist not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "wishlist close failed",
        ),
    }
}

#[handler]
async fn delete_wishlist(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let deleted = diesel::sql_query(
        "DELETE FROM wishlists w WHERE w.id=$1 AND w.owner_id=$2 \
         AND NOT EXISTS (SELECT 1 FROM wishlist_items i WHERE i.wishlist_id=w.id \
         AND i.claimed_gift_id IS NOT NULL)",
    )
    .bind::<BigInt, _>(wid)
    .bind::<BigInt, _>(uid)
    .execute(&mut conn);
    match deleted {
        Ok(1) => {
            res.status_code(StatusCode::NO_CONTENT);
        }
        Ok(_) => error(
            res,
            StatusCode::CONFLICT,
            "wishlist not found or has claimed items",
        ),
        Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::ForeignKeyViolation,
            _,
        )) => error(res, StatusCode::CONFLICT, "wishlist cannot be deleted yet"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "wishlist delete failed",
        ),
    }
}

#[handler]
async fn add_item(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let body: WishItemInput = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if !valid_item(&body) {
        return error(res, StatusCode::BAD_REQUEST, "invalid wish item");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<(i64,bool)>, diesel::result::Error, _>(|conn| {
        let owned = diesel::sql_query(
            "SELECT id FROM wishlists WHERE id=$1 AND owner_id=$2 AND closed_at IS NULL \
             AND published_at IS NULL FOR UPDATE",
        )
        .bind::<BigInt, _>(wid)
        .bind::<BigInt, _>(uid)
        .get_result::<IdRow>(conn)
        .optional()?;
        if owned.is_none() { return Ok(None) }
        if let Some(pid) = body.product_id {
            if !active_product(conn, pid)? { return Ok(None) }
            let existing = diesel::sql_query("SELECT id FROM wishlist_items WHERE wishlist_id=$1 AND product_id=$2")
                .bind::<BigInt,_>(wid).bind::<Integer,_>(pid).get_result::<IdRow>(conn).optional()?;
            if let Some(existing) = existing { return Ok(Some((existing.id,true))) }
        }
        let count = diesel::sql_query("SELECT count(*) AS count FROM wishlist_items WHERE wishlist_id=$1")
            .bind::<BigInt, _>(wid)
            .get_result::<CountRow>(conn)?.count;
        if count >= 8 { return Ok(None) }
        let ordinal = diesel::sql_query("SELECT (COALESCE(max(ordinal),-1)+1)::bigint AS count FROM wishlist_items WHERE wishlist_id=$1")
            .bind::<BigInt, _>(wid)
            .get_result::<CountRow>(conn)?.count;
        insert_item(conn, wid, ordinal as i32, &body).map(|id| Some((id,false)))
    });
    match result {
        Ok(Some((id, already_present))) => {
            res.render(Json(json!({"id":id,"already_present":already_present})))
        }
        Ok(None) => error(
            res,
            StatusCode::CONFLICT,
            "draft unavailable, product inactive, or wishlist full",
        ),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "wish item create failed"),
    }
}

#[handler]
async fn delete_item(req: &mut Request, res: &mut Response) {
    let Some(uid) = user(req, res) else { return };
    let Some(wid) = path_id(req, "id", res) else {
        return;
    };
    let Some(iid) = path_id(req, "item_id", res) else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let deleted = conn.transaction::<usize,diesel::result::Error,_>(|conn| {
        let owned = diesel::sql_query("SELECT id FROM wishlists WHERE id=$1 AND owner_id=$2 AND closed_at IS NULL AND published_at IS NULL FOR UPDATE")
            .bind::<BigInt,_>(wid).bind::<BigInt,_>(uid).get_result::<IdRow>(conn).optional()?;
        if owned.is_none() { return Ok(0) }
        diesel::sql_query("DELETE FROM wishlist_items WHERE id=$1 AND wishlist_id=$2 AND claimed_gift_id IS NULL")
            .bind::<BigInt,_>(iid).bind::<BigInt,_>(wid).execute(conn)
    });
    match deleted {
        Ok(1) => {
            res.status_code(StatusCode::NO_CONTENT);
        }
        Ok(_) => error(
            res,
            StatusCode::CONFLICT,
            "draft item unavailable or claimed",
        ),
        Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::ForeignKeyViolation,
            _,
        )) => error(res, StatusCode::CONFLICT, "item cannot be deleted yet"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "wish item delete failed",
        ),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/friends").get(friends))
        .push(Router::with_path("api/v1/friends/{id}/details").put(put_friend_details))
        .push(Router::with_path("api/v1/friends/requests").post(request_friend))
        .push(Router::with_path("api/v1/friends/requests/{id}/accept").post(accept_friend))
        .push(Router::with_path("api/v1/wishlists/mine").get(my_wishlists))
        .push(Router::with_path("api/v1/wishlists/friends").get(friend_wishlists))
        .push(Router::with_path("api/v1/wishlists").post(create_wishlist))
        .push(Router::with_path("api/v1/wishlists/drafts").post(create_draft))
        .push(Router::with_path("api/v1/wishlists/{id}/publish").post(publish_draft))
        .push(
            Router::with_path("api/v1/wishlists/{id}")
                .get(get_wishlist)
                .put(edit_wishlist)
                .delete(delete_wishlist),
        )
        .push(Router::with_path("api/v1/wishlists/{id}/close").post(close_wishlist))
        .push(Router::with_path("api/v1/wishlists/{id}/items").post(add_item))
        .push(Router::with_path("api/v1/wishlists/{id}/items/{item_id}").delete(delete_item))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occasion_dates_accept_leap_days_and_reject_invalid_calendar_dates() {
        assert!(valid_occasion_date(""));
        assert!(valid_occasion_date("2000-02-29"));
        assert!(!valid_occasion_date("1900-02-29"));
        assert!(!valid_occasion_date("2026-09-31"));
        assert!(!valid_occasion_date("2026/09/30"));
    }

    #[test]
    fn privacy_projection_for_owner_friend_and_other() {
        let make_row = |by_me| WishItemRow {
            id: 7,
            product_id: None,
            kind: "耳机".into(),
            max_price_cents: 100_000,
            wants: "降噪".into(),
            claimed: true,
            by_me,
        };
        let owner = item_projection(make_row(false), true);
        let claiming_friend = item_projection(make_row(true), false);
        let other_friend = item_projection(make_row(false), false);
        assert_eq!(owner["status"], "claimed");
        assert_eq!(claiming_friend["status"], "by_me");
        assert_eq!(other_friend["status"], "claimed");
        for projection in [owner, claiming_friend, other_friend] {
            let body = projection.to_string();
            for private_field in [
                "claimed_by_user_id",
                "claimed_gift_id",
                "given_item",
                "sender_id",
            ] {
                assert!(!body.contains(private_field));
            }
            assert_eq!(projection.as_object().unwrap().len(), 6);
        }
    }

    #[test]
    fn validates_wishlist_shape() {
        assert!(valid_header(
            "生日心愿单",
            "想要惊喜",
            "birthday",
            "2026-10-01"
        ));
        assert!(!valid_header("", "", "birthday", "2026-10-01"));
        assert!(!valid_date("2026/10/01"));
        assert!(valid_item(&WishItemInput {
            product_id: None,
            kind: Some("耳机".into()),
            max_price_cents: Some(100_000),
            wants: Some("降噪".into())
        }));
    }
}
