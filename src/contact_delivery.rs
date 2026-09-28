//! Verified contact ownership, pending gifts and durable provider delivery.
use crate::{error, hash_secret, new_session_token, pool, user_id};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_query,
    sql_types::{BigInt, Bool, Jsonb, Nullable, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Deserialize)]
pub(crate) struct Recipient {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Deserialize)]
struct AvatarLookupInput {
    contacts: Vec<Recipient>,
}

#[derive(QueryableByName)]
struct AvatarUrlRow {
    #[diesel(sql_type=Text)]
    avatar_url: String,
}

pub(crate) fn normalize(kind: &str, value: &str) -> Option<String> {
    let value = value.trim();
    match kind {
        "phone" => {
            if value
                .chars()
                .any(|c| !c.is_ascii_digit() && !matches!(c, '+' | ' ' | '-' | '(' | ')'))
            {
                return None;
            }
            let compact: String = value
                .chars()
                .filter(|c| !matches!(c, ' ' | '-' | '(' | ')'))
                .collect();
            let compact = if compact.len() == 11 && compact.starts_with('1') {
                format!("+86{compact}")
            } else {
                compact
            };
            let digits = compact.strip_prefix('+')?;
            (digits.len() >= 8
                && digits.len() <= 15
                && !digits.starts_with('0')
                && digits.bytes().all(|b| b.is_ascii_digit()))
            .then_some(compact)
        }
        "email" => {
            let (local, domain) = value.rsplit_once('@')?;
            if local.is_empty()
                || local.contains('@')
                || !domain.contains('.')
                || domain.starts_with('.')
                || domain.ends_with('.')
                || value.len() > 254
                || value.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return None;
            }
            Some(format!("{local}@{}", domain.to_ascii_lowercase()))
        }
        _ => None,
    }
}
pub(crate) fn validate_config() -> Result<(), String> {
    if let Ok(mode) = std::env::var("LIYU_TEST_DELIVERY") {
        if mode != "true" && mode != "false" {
            return Err("LIYU_TEST_DELIVERY must be true or false".into());
        }
    }
    if let Ok(url) = std::env::var("LIYU_DELIVERY_WEBHOOK") {
        if !url.is_empty() {
            if !(url.starts_with("https://")
                || (test_mode() && url.starts_with("http://127.0.0.1:")))
            {
                return Err(
                    "LIYU_DELIVERY_WEBHOOK requires HTTPS (test mode permits loopback HTTP)".into(),
                );
            }
            let public = std::env::var("LIYU_PUBLIC_URL").unwrap_or_default();
            if !(public.starts_with("https://")
                || (test_mode() && public.starts_with("http://127.0.0.1:")))
            {
                return Err(
                    "Configure LIYU_PUBLIC_URL with the public HTTPS invitation origin".into(),
                );
            }
        }
    }
    Ok(())
}

/// The contact migration may already be recorded as applied on an existing database.
/// Reuse its marked SQL section to add only the new address-book structures.
pub(crate) fn ensure_schema(conn: &mut PgConnection) -> Result<(), String> {
    #[derive(QueryableByName)]
    struct Ready {
        #[diesel(sql_type=Bool)]
        ready: bool,
    }
    const READY_SQL: &str = "SELECT (to_regclass('contact_identities') IS NOT NULL
        AND to_regclass('sender_contacts') IS NOT NULL
        AND to_regclass('sender_contact_methods') IS NOT NULL
        AND to_regclass('contact_identities_user_kind_idx') IS NOT NULL
        AND to_regclass('sender_contact_methods_active_unique') IS NOT NULL
        AND EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='order_items' AND column_name='recipient_bound_user_id')
        AND EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='order_items' AND column_name='recipient_change_confirmed')
        AND NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid='contact_identities'::regclass AND conname='contact_identities_user_id_kind_key')) AS ready";
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock(731129928)").execute(conn)?;
        let ready = sql_query(READY_SQL).get_result::<Ready>(conn)?;
        if ready.ready {
            return Ok(());
        }
        let section = include_str!("../migrations/20260928000000_contact_delivery/up.sql")
            .split_once("-- contact book extension (shared with startup upgrade)")
            .expect("contact book SQL section")
            .1
            .split("-- end contact book extension")
            .next()
            .unwrap();
        conn.batch_execute(section)?;
        if !sql_query(READY_SQL).get_result::<Ready>(conn)?.ready {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        Ok(())
    })
    .map_err(|e| format!("contact book schema upgrade failed: {e}"))
}

pub(crate) fn test_mode() -> bool {
    std::env::var("LIYU_TEST_DELIVERY").as_deref() == Ok("true")
}

#[derive(QueryableByName)]
struct Id {
    #[diesel(sql_type=BigInt)]
    id: i64,
}
#[derive(QueryableByName)]
struct JsonRow {
    #[diesel(sql_type=Jsonb)]
    data: Value,
}

#[derive(QueryableByName)]
struct ContactRow {
    #[diesel(sql_type=BigInt)]
    id: i64,
    #[diesel(sql_type=Nullable<BigInt>)]
    bound_user_id: Option<i64>,
    #[diesel(sql_type=Text)]
    label: String,
}

#[derive(QueryableByName)]
struct MethodRow {
    #[diesel(sql_type=Text)]
    kind: String,
    #[diesel(sql_type=Text)]
    value: String,
}

pub(crate) struct Binding {
    pub current: Option<i64>,
    pub previous: Option<i64>,
    contact_id: Option<i64>,
    label: String,
}

impl Binding {
    pub fn changed(&self) -> bool {
        self.previous.is_some() && self.previous != self.current
    }
    pub fn warning(&self) -> Value {
        json!({"code":"recipient_identity_changed","message":"此联系方式现在对应的用户与之前保存的联系人不同，继续送礼可能送错人。请确认是否继续。"})
    }
}

fn lock_contact_value(conn: &mut PgConnection, kind: &str, value: &str) -> QueryResult<()> {
    sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind::<Text, _>(format!("contact:{kind}:{value}"))
        .execute(conn)?;
    Ok(())
}

pub(crate) fn binding(
    conn: &mut PgConnection,
    owner: i64,
    contact: &Value,
) -> QueryResult<Binding> {
    let kind = contact["kind"].as_str().unwrap_or("");
    let value = contact["value"].as_str().unwrap_or("");
    lock_contact_value(conn, kind, value)?;
    let current = resolve(conn, contact)?;
    let saved = sql_query("SELECT c.id,c.bound_user_id,c.label FROM sender_contacts c JOIN sender_contact_methods m ON m.contact_id=c.id WHERE m.owner_id=$1 AND m.kind=$2 AND m.value=$3 AND m.expired_at IS NULL AND c.expired_at IS NULL")
        .bind::<BigInt,_>(owner).bind::<Text,_>(kind).bind::<Text,_>(value)
        .get_result::<ContactRow>(conn).optional()?;
    Ok(Binding {
        current,
        previous: saved.as_ref().and_then(|r| r.bound_user_id),
        contact_id: saved.as_ref().map(|r| r.id),
        label: saved.map_or_else(
            || contact["label"].as_str().unwrap_or("").to_owned(),
            |r| r.label,
        ),
    })
}

fn insert_contact(
    conn: &mut PgConnection,
    owner: i64,
    label: &str,
    bound: Option<i64>,
    methods: &[(String, String)],
) -> QueryResult<i64> {
    let row = sql_query(
        "INSERT INTO sender_contacts(owner_id,label,bound_user_id) VALUES($1,$2,$3) RETURNING id",
    )
    .bind::<BigInt, _>(owner)
    .bind::<Text, _>(label)
    .bind::<Nullable<BigInt>, _>(bound)
    .get_result::<Id>(conn)?;
    for (kind, value) in methods {
        sql_query("INSERT INTO sender_contact_methods(contact_id,owner_id,kind,value) VALUES($1,$2,$3,$4)")
            .bind::<BigInt,_>(row.id).bind::<BigInt,_>(owner).bind::<Text,_>(kind).bind::<Text,_>(value).execute(conn)?;
    }
    Ok(row.id)
}

// Called in the checkout transaction, after binding() has locked the address.
pub(crate) fn save_binding(
    conn: &mut PgConnection,
    owner: i64,
    contact: &Value,
    b: &Binding,
    confirmed: bool,
) -> QueryResult<()> {
    let kind = contact["kind"].as_str().unwrap_or("").to_owned();
    let value = contact["value"].as_str().unwrap_or("").to_owned();
    if b.changed() {
        if !confirmed {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        let old = b
            .contact_id
            .ok_or(diesel::result::Error::RollbackTransaction)?;
        let remaining=sql_query("SELECT kind,value FROM sender_contact_methods WHERE contact_id=$1 AND expired_at IS NULL AND NOT (kind=$2 AND value=$3) ORDER BY kind,value")
            .bind::<BigInt,_>(old).bind::<Text,_>(&kind).bind::<Text,_>(&value).load::<MethodRow>(conn)?;
        sql_query("UPDATE sender_contacts SET expired_at=now() WHERE id=$1 AND owner_id=$2 AND expired_at IS NULL")
            .bind::<BigInt,_>(old).bind::<BigInt,_>(owner).execute(conn)?;
        sql_query("UPDATE sender_contact_methods SET expired_at=now() WHERE contact_id=$1 AND expired_at IS NULL")
            .bind::<BigInt,_>(old).execute(conn)?;
        if !remaining.is_empty() {
            let methods = remaining
                .into_iter()
                .map(|m| (m.kind, m.value))
                .collect::<Vec<_>>();
            insert_contact(conn, owner, &b.label, b.previous, &methods)?;
        }
        insert_contact(conn, owner, &b.label, b.current, &[(kind, value)])?;
    } else if let Some(id) = b.contact_id {
        if b.previous.is_none() && b.current.is_some() {
            sql_query(
                "UPDATE sender_contacts SET bound_user_id=$2 WHERE id=$1 AND bound_user_id IS NULL",
            )
            .bind::<BigInt, _>(id)
            .bind::<Nullable<BigInt>, _>(b.current)
            .execute(conn)?;
        }
    } else {
        insert_contact(conn, owner, &b.label, b.current, &[(kind, value)])?;
    }
    Ok(())
}

pub(crate) fn resolve(conn: &mut PgConnection, r: &Value) -> QueryResult<Option<i64>> {
    let kind = r["kind"].as_str().unwrap_or("");
    let value = r["value"].as_str().unwrap_or("");
    sql_query("SELECT c.user_id AS id FROM contact_identities c JOIN users u ON u.id=c.user_id AND u.is_active WHERE c.kind=$1 AND c.value=$2")
        .bind::<Text,_>(kind).bind::<Text,_>(value).get_result::<Id>(conn).optional().map(|x|x.map(|r|r.id))
}

pub(crate) fn notify(conn: &mut PgConnection, gift: i64, uid: i64) -> QueryResult<()> {
    sql_query("INSERT INTO notifications(user_id,title,body,expires_at,gift_id,event_key,type) SELECT $2,'有礼物等你拆','一份神秘礼物已经放进你的礼盒。',expires_at,id,'gift:'||id||':received','gift' FROM gifts WHERE id=$1 ON CONFLICT(event_key) DO NOTHING")
        .bind::<BigInt,_>(gift).bind::<BigInt,_>(uid).execute(conn)?;
    Ok(())
}

pub(crate) fn gift_delivery(
    conn: &mut PgConnection,
    gift: i64,
    recipient: Option<i64>,
    contact: Option<&Value>,
) -> QueryResult<()> {
    if let Some(uid) = recipient {
        return notify(conn, gift, uid);
    }
    let Some(contact) = contact else {
        return Err(diesel::result::Error::RollbackTransaction);
    };
    let token = new_session_token();
    let url = std::env::var("LIYU_PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
    let payload = json!({"type":"gift_invitation","title":"有一份礼物等你领取","body":"验证此联系方式并登录礼遇后即可领取。","url":format!("{}/gift-invitations/{token}",url.trim_end_matches('/'))});
    sql_query("UPDATE gifts SET invitation_hash=$2 WHERE id=$1")
        .bind::<BigInt, _>(gift)
        .bind::<Text, _>(hash_secret(&token))
        .execute(conn)?;
    sql_query("INSERT INTO delivery_outbox(event_key,gift_id,kind,destination,payload) VALUES($1,$2,$3,$4,$5) ON CONFLICT(event_key) DO NOTHING")
        .bind::<Text,_>(format!("gift:{gift}:invite")).bind::<BigInt,_>(gift)
        .bind::<Text,_>(contact["kind"].as_str().unwrap_or("")).bind::<Text,_>(contact["value"].as_str().unwrap_or("")).bind::<Jsonb,_>(payload).execute(conn)?;
    if std::env::var("LIYU_DELIVERY_WEBHOOK")
        .ok()
        .filter(|s| !s.is_empty())
        .is_none()
    {
        sql_query("UPDATE delivery_outbox SET last_error='delivery provider is not configured' WHERE gift_id=$1").bind::<BigInt,_>(gift).execute(conn)?;
    }
    Ok(())
}

// Called only in the transaction that verifies a contact. This cannot grant ownership
// from legacy identifiers or fixed-code profile rows.
pub(crate) fn attach(
    conn: &mut PgConnection,
    uid: i64,
    kind: &str,
    value: &str,
) -> QueryResult<()> {
    lock_contact_value(conn, kind, value)?;
    sql_query("INSERT INTO contact_identities(user_id,kind,value) VALUES($1,$2,$3) ON CONFLICT(kind,value) DO UPDATE SET verified_at=now() WHERE contact_identities.user_id=excluded.user_id RETURNING id")
        .bind::<BigInt,_>(uid).bind::<Text,_>(kind).bind::<Text,_>(value).get_result::<Id>(conn)?;
    sql_query("UPDATE sender_contacts c SET bound_user_id=$1 FROM sender_contact_methods m WHERE m.contact_id=c.id AND m.kind=$2 AND m.value=$3 AND m.expired_at IS NULL AND c.expired_at IS NULL AND c.bound_user_id IS NULL")
        .bind::<BigInt,_>(uid).bind::<Text,_>(kind).bind::<Text,_>(value).execute(conn)?;
    claim_pending(conn, uid)
}

#[derive(Deserialize)]
struct ContactInput {
    #[serde(default)]
    label: String,
    #[serde(default)]
    phones: Vec<String>,
    #[serde(default)]
    emails: Vec<String>,
}

#[derive(Deserialize)]
struct ContactValueInput {
    kind: String,
    value: String,
}

#[handler]
async fn release_identity(req: &mut Request, res: &mut Response) {
    let Some(owner) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Ok(input) = req.parse_json::<ContactValueInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid contact");
    };
    let Some(value) = normalize(&input.kind, &input.value) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid contact");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<usize, diesel::result::Error, _>(|conn| {
        lock_contact_value(conn, &input.kind, &value)?;
        sql_query("DELETE FROM contact_identities WHERE user_id=$1 AND kind=$2 AND value=$3")
            .bind::<BigInt, _>(owner)
            .bind::<Text, _>(&input.kind)
            .bind::<Text, _>(&value)
            .execute(conn)
    });
    match result {
        Ok(1) => res.render(Json(json!({"released":true}))),
        Ok(_) => error(res, StatusCode::NOT_FOUND, "contact identity not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "contact release failed",
        ),
    }
}

#[handler]
async fn list_contacts(req: &mut Request, res: &mut Response) {
    let Some(owner) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let rows=sql_query("SELECT jsonb_build_object('id',c.id,'label',c.label,'phones',COALESCE(jsonb_agg(m.value) FILTER(WHERE m.kind='phone'),'[]'::jsonb),'emails',COALESCE(jsonb_agg(m.value) FILTER(WHERE m.kind='email'),'[]'::jsonb)) AS data FROM sender_contacts c JOIN sender_contact_methods m ON m.contact_id=c.id AND m.expired_at IS NULL WHERE c.owner_id=$1 AND c.expired_at IS NULL GROUP BY c.id ORDER BY c.id")
        .bind::<BigInt,_>(owner).load::<JsonRow>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(rows.into_iter().map(|r| r.data).collect::<Vec<_>>())),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "contacts unavailable",
        ),
    }
}

/// Return only avatars actively chosen by verified contact owners. Missing
/// contacts and accounts using the generated avatar have the same response.
#[handler]
async fn contact_avatars(req: &mut Request, res: &mut Response) {
    if user_id(req).is_none() {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    }
    let Ok(input) = req.parse_json::<AvatarLookupInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid contacts");
    };
    if input.contacts.is_empty() || input.contacts.len() > 32 {
        return error(res, StatusCode::BAD_REQUEST, "send 1 to 32 contacts");
    }
    let mut contacts = Vec::new();
    for contact in input.contacts {
        let Some(value) = normalize(&contact.kind, &contact.value) else {
            return error(res, StatusCode::BAD_REQUEST, "invalid contact value");
        };
        if !contacts.contains(&(contact.kind.clone(), value.clone())) {
            contacts.push((contact.kind, value));
        }
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let mut avatars = Vec::new();
    for (kind, value) in contacts {
        // Joining the avatar record excludes stale, external or default URLs.
        let result = sql_query("SELECT p.avatar_url AS avatar_url FROM contact_identities c JOIN users u ON u.id=c.user_id AND u.is_active JOIN user_profiles p ON p.user_id=u.id JOIN avatars a ON a.owner_id=u.id AND p.avatar_url='/api/v1/media/avatars/'||replace(a.id::text,'-','') WHERE c.kind=$1 AND c.value=$2 LIMIT 1")
            .bind::<Text, _>(&kind)
            .bind::<Text, _>(&value)
            .get_result::<AvatarUrlRow>(&mut conn)
            .optional();
        match result {
            Ok(Some(row)) => {
                avatars.push(json!({"kind":kind,"value":value,"avatar_url":row.avatar_url}))
            }
            Ok(None) => {}
            Err(_) => {
                return error(
                    res,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "avatars unavailable",
                )
            }
        }
    }
    res.render(Json(json!({"avatars":avatars})));
}

#[handler]
async fn create_contact(req: &mut Request, res: &mut Response) {
    let Some(owner) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let Ok(input) = req.parse_json::<ContactInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid contact");
    };
    if input.label.chars().count() > 100
        || input.phones.len() + input.emails.len() == 0
        || input.phones.len() + input.emails.len() > 20
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid contact");
    }
    let mut methods = Vec::new();
    for (kind, values) in [("phone", input.phones), ("email", input.emails)] {
        for raw in values {
            let Some(value) = normalize(kind, &raw) else {
                return error(res, StatusCode::BAD_REQUEST, "invalid contact value");
            };
            methods.push((kind.to_owned(), value));
        }
    }
    methods.sort();
    methods.dedup();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<i64>, diesel::result::Error, _>(|conn| {
        let mut bound = None;
        for (kind, value) in &methods {
            lock_contact_value(conn, kind, value)?;
            let current = resolve(conn, &json!({"kind":kind,"value":value}))?;
            if current.is_some() && bound.is_some() && current != bound {
                return Ok(None);
            }
            if current.is_some() {
                bound = current;
            }
        }
        insert_contact(conn, owner, input.label.trim(), bound, &methods).map(Some)
    });
    match result {
        Ok(Some(id)) => res.render(Json(json!({"id":id}))),
        Ok(None) => error(
            res,
            StatusCode::CONFLICT,
            "contact methods belong to different users",
        ),
        Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        )) => error(res, StatusCode::CONFLICT, "contact already saved"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "contact save failed",
        ),
    }
}

pub(crate) fn claim_pending(conn: &mut PgConnection, uid: i64) -> QueryResult<()> {
    let ids=sql_query("SELECT g.id FROM gifts g JOIN contact_identities c ON c.user_id=$1 AND c.kind=g.recipient_contact->>'kind' AND c.value=g.recipient_contact->>'value' WHERE g.recipient_id IS NULL AND g.sender_id<>$1 AND g.state='sealed' AND g.expires_at>now() ORDER BY g.id FOR UPDATE OF g")
        .bind::<BigInt,_>(uid).load::<Id>(conn)?;
    for row in ids {
        sql_query("UPDATE gifts SET recipient_id=$2 WHERE id=$1 AND recipient_id IS NULL")
            .bind::<BigInt, _>(row.id)
            .bind::<BigInt, _>(uid)
            .execute(conn)?;
        sql_query("UPDATE delivery_outbox SET status='cancelled',payload='{}' WHERE gift_id=$1 AND status IN ('pending','failed')").bind::<BigInt,_>(row.id).execute(conn)?;
        notify(conn, row.id, uid)?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct ChallengeInput {
    kind: String,
    value: String,
    purpose: String,
}

#[handler]
async fn challenge(req: &mut Request, res: &mut Response) {
    let Ok(input) = req.parse_json::<ChallengeInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let Some(value) = normalize(&input.kind, &input.value) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid contact");
    };
    let owner = if input.purpose == "bind" {
        let Some(id) = user_id(req) else {
            return error(res, StatusCode::UNAUTHORIZED, "login required");
        };
        Some(id)
    } else if input.purpose == "register" {
        None
    } else {
        return error(res, StatusCode::BAD_REQUEST, "invalid purpose");
    };
    if !test_mode()
        && std::env::var("LIYU_DELIVERY_WEBHOOK")
            .ok()
            .filter(|s| !s.is_empty())
            .is_none()
    {
        return error(
            res,
            StatusCode::SERVICE_UNAVAILABLE,
            "delivery provider is not configured",
        );
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let source = match req.remote_addr() {
        salvo::conn::SocketAddr::IPv4(a) => a.ip().to_string(),
        salvo::conn::SocketAddr::IPv6(a) => a.ip().to_string(),
        _ => "unknown".into(),
    };
    let source_hash = hash_secret(&source);
    let id = new_session_token();
    let random = uuid::Uuid::new_v4();
    let code = if test_mode() {
        "123456".into()
    } else {
        format!(
            "{:06}",
            u32::from_be_bytes(random.as_bytes()[0..4].try_into().unwrap()) % 1_000_000
        )
    };
    let result=conn.transaction::<bool,diesel::result::Error,_>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))").bind::<Text,_>(format!("source:{source_hash}")).execute(conn)?;
        sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))").bind::<Text,_>(format!("{}:{value}",input.kind)).execute(conn)?;
        let recent=sql_query("SELECT count(*) AS id FROM contact_challenges WHERE kind=$1 AND value=$2 AND created_at>now()-interval '1 minute'")
            .bind::<Text,_>(&input.kind).bind::<Text,_>(&value).get_result::<Id>(conn)?;
        let quota=sql_query("SELECT count(*) AS id FROM contact_challenges WHERE created_at>now()-interval '1 hour' AND ((kind=$1 AND value=$2) OR source_hash=$3)")
            .bind::<Text,_>(&input.kind).bind::<Text,_>(&value).bind::<Text,_>(&source_hash).get_result::<Id>(conn)?;
        if recent.id>0 || quota.id>=30 { return Ok(false); }
        sql_query("INSERT INTO contact_challenges(id,kind,value,purpose,owner_id,code_hash,source_hash) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind::<Text,_>(&id).bind::<Text,_>(&input.kind).bind::<Text,_>(&value).bind::<Text,_>(&input.purpose).bind::<Nullable<BigInt>,_>(owner).bind::<Text,_>(hash_secret(&code)).bind::<Text,_>(&source_hash).execute(conn)?;
        if !test_mode() {
            sql_query("INSERT INTO delivery_outbox(event_key,kind,destination,payload) VALUES($1,$2,$3,$4)")
                .bind::<Text,_>(format!("challenge:{id}")).bind::<Text,_>(&input.kind).bind::<Text,_>(&value).bind::<Jsonb,_>(json!({"type":"verification","code":code,"expires_in_seconds":600})).execute(conn)?;
        }
        Ok(true)
    });
    match result { Ok(true)=>res.render(Json(json!({"challenge_id":id,"expires_in_seconds":600,"test_code":if test_mode(){Some(code)}else{None}}))),Ok(false)=>error(res,StatusCode::TOO_MANY_REQUESTS,"please wait before requesting another code"),Err(_)=>error(res,StatusCode::INTERNAL_SERVER_ERROR,"cannot request verification") }
}

// The caller must persist failed attempts even when verification is rejected.
pub(crate) fn verify(
    conn: &mut PgConnection,
    id: &str,
    kind: &str,
    value: &str,
    purpose: &str,
    owner: Option<i64>,
    code: &str,
) -> QueryResult<bool> {
    let row=sql_query("UPDATE contact_challenges SET attempts=attempts+1 WHERE id=$1 AND kind=$2 AND value=$3 AND purpose=$4 AND owner_id IS NOT DISTINCT FROM $5 AND consumed_at IS NULL AND expires_at>now() AND attempts<5 RETURNING jsonb_build_object('correct',code_hash=$6) AS data")
        .bind::<Text,_>(id).bind::<Text,_>(kind).bind::<Text,_>(value).bind::<Text,_>(purpose).bind::<Nullable<BigInt>,_>(owner).bind::<Text,_>(hash_secret(code)).get_result::<JsonRow>(conn).optional()?;
    if !row.is_some_and(|r| r.data["correct"] == true) {
        return Ok(false);
    }
    sql_query("UPDATE contact_challenges SET consumed_at=now() WHERE id=$1")
        .bind::<Text, _>(id)
        .execute(conn)?;
    Ok(true)
}

#[handler]
async fn invitation(req: &mut Request, res: &mut Response) {
    let token = req.param::<String>("token").unwrap_or_default();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let gift = sql_query(
        "SELECT id FROM gifts WHERE invitation_hash=$1 AND state='sealed' AND expires_at>now()",
    )
    .bind::<Text, _>(hash_secret(&token))
    .get_result::<Id>(&mut conn)
    .optional();
    if !matches!(gift, Ok(Some(_))) {
        return error(res, StatusCode::NOT_FOUND, "invitation unavailable");
    }
    res.render(salvo::prelude::Text::Html(include_str!(
        "../web/gift-invitation.html"
    )));
}

#[handler]
async fn claim(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let token = req.param::<String>("token").unwrap_or_default();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Option<i64>,diesel::result::Error,_>(|conn| {
        let row=sql_query("SELECT g.id FROM gifts g JOIN contact_identities c ON c.user_id=$2 AND c.kind=g.recipient_contact->>'kind' AND c.value=g.recipient_contact->>'value' WHERE g.invitation_hash=$1 AND g.sender_id<>$2 AND (g.recipient_id IS NULL OR g.recipient_id=$2) AND g.state='sealed' AND g.expires_at>now() FOR UPDATE OF g")
            .bind::<Text,_>(hash_secret(&token)).bind::<BigInt,_>(uid).get_result::<Id>(conn).optional()?;
        if let Some(row)=row { claim_pending(conn,uid)?; return Ok(Some(row.id)); }
        Ok(None)
    });
    match result {
        Ok(Some(id)) => res.render(Json(json!({"gift_id":id,"claimed":true}))),
        Ok(None) => error(
            res,
            StatusCode::NOT_FOUND,
            "verify the recipient contact or invitation unavailable",
        ),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "claim failed"),
    }
}

pub(crate) fn routes() -> Router {
    Router::new()
        .push(
            Router::with_path("api/v1/contacts")
                .get(list_contacts)
                .post(create_contact),
        )
        .push(Router::with_path("api/v1/contacts/avatars").post(contact_avatars))
        .push(Router::with_path("api/v1/me/contact-identities").delete(release_identity))
        .push(Router::with_path("api/v1/auth/challenges").post(challenge))
        .push(Router::with_path("gift-invitations/{token}").get(invitation))
        .push(Router::with_path("api/v1/gift-invitations/{token}/claim").post(claim))
}

// A small dedicated thread keeps blocking provider IO off the HTTP runtime.
pub(crate) fn start_worker() {
    std::thread::spawn(|| loop {
        if let Ok(mut conn) = pool().get() {
            let _ = maintenance(&mut conn);
            if let Ok(url) = std::env::var("LIYU_DELIVERY_WEBHOOK") {
                if url.starts_with("https://")
                    || (test_mode() && url.starts_with("http://127.0.0.1:"))
                {
                    for _ in 0..16 {
                        if !matches!(dispatch_one(&mut conn, &url), Ok(true)) {
                            break;
                        }
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    });
}

fn maintenance(conn: &mut PgConnection) -> QueryResult<()> {
    conn.transaction::<(),diesel::result::Error,_>(|conn| {
        let ids=sql_query("SELECT id FROM gifts WHERE recipient_id IS NULL AND state='sealed' AND expires_at<=now() ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 100").load::<Id>(conn)?;
        for r in ids {
            crate::benefits::refund(conn,r.id,"expired")?;
            sql_query("UPDATE gifts SET state='expired',settled_at=now() WHERE id=$1").bind::<BigInt,_>(r.id).execute(conn)?;
        }
        // Reconcile registration/payment races without trusting identifiers.
        let users=sql_query("SELECT DISTINCT c.user_id AS id FROM contact_identities c JOIN gifts g ON g.recipient_id IS NULL AND c.kind=g.recipient_contact->>'kind' AND c.value=g.recipient_contact->>'value' JOIN users u ON u.id=c.user_id AND u.is_active WHERE g.state='sealed' AND g.expires_at>now() ORDER BY c.user_id LIMIT 100").load::<Id>(conn)?;
        for u in users { claim_pending(conn,u.id)?; }
        sql_query("UPDATE delivery_outbox o SET status='cancelled',payload='{}' FROM gifts g WHERE o.gift_id=g.id AND (g.state<>'sealed' OR g.expires_at<=now() OR g.recipient_id IS NOT NULL) AND o.status IN ('pending','failed')").execute(conn)?;
        sql_query("UPDATE delivery_outbox SET status='cancelled',payload='{}' WHERE gift_id IS NULL AND created_at<now()-interval '10 minutes' AND status IN ('pending','failed')").execute(conn)?;
        Ok(())
    })
}

fn dispatch_one(conn: &mut PgConnection, url: &str) -> QueryResult<bool> {
    let row=sql_query("UPDATE delivery_outbox SET status='sending',lease_until=now()+interval '30 seconds',attempts=attempts+1 WHERE id=(SELECT id FROM delivery_outbox WHERE ((status IN ('pending','failed') AND next_attempt_at<=now()) OR (status='sending' AND lease_until<now())) AND attempts<8 ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING jsonb_build_object('id',id,'event_key',event_key,'channel',kind,'destination',destination,'payload',payload) AS data")
        .get_result::<JsonRow>(conn).optional()?;
    let Some(row) = row else {
        return Ok(false);
    };
    let id = row.data["id"].as_i64().unwrap();
    use std::io::Write;
    let mut command = std::process::Command::new("curl");
    command
        .args([
            "--silent",
            "--fail",
            "--max-time",
            "10",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
            "--header",
        ])
        .arg(format!(
            "Idempotency-Key: {}",
            row.data["event_key"].as_str().unwrap()
        ));
    // Keep message contents and provider credentials out of process arguments.
    let quote = |s: &str| {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    };
    let mut config = format!("data-binary = \"{}\"\n", quote(&row.data.to_string()));
    if let Ok(token) = std::env::var("LIYU_DELIVERY_TOKEN") {
        config.push_str(&format!(
            "header = \"{}\"\n",
            quote(&format!("Authorization: Bearer {token}"))
        ));
    }
    let result = command
        .args(["--config", "-", url])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            let written = child.stdin.take().unwrap().write_all(config.as_bytes());
            let status = child.wait()?;
            written?;
            Ok(status.success())
        });
    let success = matches!(result, Ok(true));
    sql_query("UPDATE delivery_outbox SET status=CASE WHEN $2 THEN 'sent' ELSE 'failed' END,sent_at=CASE WHEN $2 THEN now() ELSE NULL END,payload=CASE WHEN $2 THEN '{}'::jsonb ELSE payload END,last_error=CASE WHEN $2 THEN NULL ELSE 'provider unavailable or rejected request' END,lease_until=NULL,next_attempt_at=now()+make_interval(secs=>LEAST(3600,30*power(2,attempts)::int)) WHERE id=$1 AND status='sending'")
        .bind::<BigInt,_>(id).bind::<diesel::sql_types::Bool,_>(success).execute(conn)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_preserves_email_aliases_and_rejects_bad_numbers() {
        assert_eq!(
            normalize("phone", "138 0013 8000").as_deref(),
            Some("+8613800138000")
        );
        assert_eq!(
            normalize("phone", "+1 (415) 555-0123").as_deref(),
            Some("+14155550123")
        );
        assert_eq!(
            normalize("email", "A.b+gift@EXAMPLE.COM").as_deref(),
            Some("A.b+gift@example.com")
        );
        for bad in ["+86abc123456", "++123456789", "12345", "+0123456789"] {
            assert!(normalize("phone", bad).is_none());
        }
        assert!(normalize("email", "x@y").is_none());
    }
}
