//! Browser-owned credential entry and single-use, nonce-bound app authorization.
use crate::{error, hash_secret, new_session_token, pool};
use diesel::{
    prelude::*,
    sql_types::{BigInt, Jsonb, Nullable, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type=Jsonb)]
    data: Value,
}
#[derive(Deserialize)]
struct Start {
    purpose: String,
}
#[derive(Deserialize)]
struct Secret {
    #[serde(default)]
    poll_key: String,
    #[serde(default)]
    browser_key: String,
}

fn read(
    conn: &mut PgConnection,
    id: &str,
    hash: &str,
    browser: bool,
) -> QueryResult<Option<Value>> {
    let column = if browser { "browser_hash" } else { "poll_hash" };
    diesel::sql_query(format!("SELECT jsonb_build_object('purpose',purpose,'owner_id',owner_id,'approved_user_id',approved_user_id,'state',state,'expired',expires_at<=now()) AS data FROM browser_authorizations WHERE id=$1 AND {column}=$2 FOR UPDATE"))
        .bind::<Text,_>(id).bind::<Text,_>(hash).get_result::<Row>(conn).optional().map(|r|r.map(|r|r.data))
}
fn valid(row: &Value) -> bool {
    row["expired"] == false && row["state"] == "pending"
}

#[handler]
async fn start(req: &mut Request, res: &mut Response) {
    let Ok(input) = req.parse_json::<Start>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    if !["login", "email", "phone"].contains(&input.purpose.as_str()) {
        return error(res, StatusCode::BAD_REQUEST, "invalid purpose");
    }
    let owner = if input.purpose == "login" {
        None
    } else {
        let Some(uid) = crate::user_id(req) else {
            return error(res, StatusCode::UNAUTHORIZED, "login required");
        };
        Some(uid)
    };
    let source = hash_secret(&req.remote_addr().to_string());
    // Only the IP participates in the quota; a client's ephemeral port must not bypass it.
    let source = match req.remote_addr() {
        salvo::conn::SocketAddr::IPv4(a) => hash_secret(&a.ip().to_string()),
        salvo::conn::SocketAddr::IPv6(a) => hash_secret(&a.ip().to_string()),
        _ => source,
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let poll_key = new_session_token();
    let browser_key = new_session_token();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<bool,diesel::result::Error,_>(|conn| {
        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))").bind::<Text,_>(&source).execute(conn)?;
        let quota = diesel::sql_query("SELECT jsonb_build_object('count',count(*)) AS data FROM browser_authorizations WHERE source_hash=$1 AND created_at>now()-interval '1 hour'").bind::<Text,_>(&source).get_result::<Row>(conn)?;
        if quota.data["count"].as_i64().unwrap_or(60)>=60 { return Ok(false) }
        diesel::sql_query("DELETE FROM browser_authorizations WHERE expires_at<now()-interval '1 day'").execute(conn)?;
        diesel::sql_query("INSERT INTO browser_authorizations(id,poll_hash,browser_hash,purpose,owner_id,source_hash) VALUES($1,$2,$3,$4,$5,$6)")
            .bind::<Text,_>(&id).bind::<Text,_>(hash_secret(&poll_key)).bind::<Text,_>(hash_secret(&browser_key))
            .bind::<Text,_>(&input.purpose).bind::<Nullable<BigInt>,_>(owner).bind::<Text,_>(&source).execute(conn)?;
        Ok(true)
    });
    match result {
        Ok(true)=>res.render(Json(json!({"id":id,"poll_key":poll_key,"browser_path":format!("/authorize/{id}#{}",browser_key),"expires_in_seconds":300,"interval":3}))),
        Ok(false)=>error(res,StatusCode::TOO_MANY_REQUESTS,"too many authorization requests"),
        Err(_)=>error(res,StatusCode::INTERNAL_SERVER_ERROR,"cannot create authorization"),
    }
}

// Browser nonce stays in the URL fragment and request body, never access-log URLs.
#[handler]
async fn info(req: &mut Request, res: &mut Response) {
    let Ok(secret) = req.parse_json::<Secret>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match read(&mut conn, &id, &hash_secret(&secret.browser_key), true) {
        Ok(Some(row)) if valid(&row) => res.render(Json(
            json!({"purpose":row["purpose"],"owner_id":row["owner_id"],"app_name":"LIYU-MINI"}),
        )),
        _ => error(res, StatusCode::GONE, "authorization expired or cancelled"),
    }
}
#[handler]
async fn browser_login(req: &mut Request, res: &mut Response) {
    let Ok(secret) = req.parse_json::<Secret>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let allowed = pool()
        .get()
        .ok()
        .and_then(|mut c| {
            read(&mut c, &id, &hash_secret(&secret.browser_key), true)
                .ok()
                .flatten()
        })
        .is_some_and(|r| valid(&r));
    if !allowed {
        return error(res, StatusCode::GONE, "authorization expired or cancelled");
    }
    let register = req.query::<String>("mode").as_deref() == Some("register");
    let charged = pool().get().ok().and_then(|mut conn| {
        diesel::sql_query("UPDATE browser_authorizations SET login_attempts=login_attempts+1 WHERE id=$1 AND browser_hash=$2 AND state='pending' AND expires_at>now() AND login_attempts<10")
            .bind::<Text,_>(&id).bind::<Text,_>(hash_secret(&secret.browser_key)).execute(&mut conn).ok()
    });
    if charged != Some(1) {
        return error(
            res,
            StatusCode::TOO_MANY_REQUESTS,
            "too many login attempts",
        );
    }
    // Browser sessions expire quickly even if the page is abandoned.
    crate::authenticate_with_ttl(req, res, register, 600).await;
}
#[handler]
async fn approve(req: &mut Request, res: &mut Response) {
    let Some(uid) = crate::user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "login required");
    };
    let session_hash = hash_secret(crate::bearer(req).unwrap_or_default());
    let Ok(secret) = req.parse_json::<Secret>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Option<Option<String>>, diesel::result::Error, _>(|conn| {
        let Some(row) = read(conn, &id, &hash_secret(&secret.browser_key), true)? else {
            return Ok(None);
        };
        if !valid(&row) || row["owner_id"].as_i64().is_some_and(|owner| owner != uid) {
            return Ok(None);
        }
        // Revalidate inside the transaction, so revoked/expired browser tokens cannot approve.
        if crate::session_owner(conn, &session_hash)? != Some(uid) {
            return Ok(None);
        }
        diesel::sql_query(
            "UPDATE browser_authorizations SET state='approved',approved_user_id=$2 WHERE id=$1",
        )
        .bind::<Text, _>(&id)
        .bind::<BigInt, _>(uid)
        .execute(conn)?;
        crate::revoke_session(conn, &session_hash)?;
        Ok(Some(crate::host_auth::approved(conn, &id, uid)?))
    });
    match result {
        Ok(Some(redirect)) => res.render(Json(json!({"ok":true,"redirect_uri":redirect}))),
        Ok(None) => error(
            res,
            StatusCode::CONFLICT,
            "authorization unavailable or wrong account",
        ),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot approve authorization",
        ),
    }
}
#[handler]
async fn poll(req: &mut Request, res: &mut Response) {
    let Ok(secret) = req.parse_json::<Secret>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid JSON");
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let cancel = req.query::<String>("cancel").as_deref() == Some("true");
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Value,diesel::result::Error,_>(|conn|{
        let Some(row)=read(conn,&id,&hash_secret(&secret.poll_key),false)? else { return Ok(json!({"state":"invalid"})) };
        if row["expired"]==true { return Ok(json!({"state":"expired"})) }
        if cancel && (row["state"]=="pending" || row["state"]=="approved") {
            diesel::sql_query("UPDATE browser_authorizations SET state='cancelled' WHERE id=$1").bind::<Text,_>(&id).execute(conn)?;
            return Ok(json!({"state":"cancelled"}))
        }
        if row["state"]!="approved" { return Ok(json!({"state":row["state"]})) }
        let uid=row["approved_user_id"].as_i64().unwrap_or(-1);
        diesel::sql_query("UPDATE browser_authorizations SET state='consumed' WHERE id=$1").bind::<Text,_>(&id).execute(conn)?;
        if row["purpose"]!="login" { return Ok(json!({"state":"complete"})) }
        let token=new_session_token();
        let inserted=diesel::sql_query("INSERT INTO sessions(token_hash,user_id) SELECT $1,id FROM users WHERE id=$2 AND is_active").bind::<Text,_>(hash_secret(&token)).bind::<BigInt,_>(uid).execute(conn)?;
        if inserted!=1 { return Ok(json!({"state":"invalid"})) }
        Ok(json!({"state":"complete","token":token,"expires_in_seconds":crate::SESSION_TTL_SECONDS}))
    });
    match result {
        Ok(data) => res.render(Json(data)),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot read authorization",
        ),
    }
}
#[handler]
async fn page(res: &mut Response) {
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    res.headers_mut().insert("content-security-policy","default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    res.render(salvo::prelude::Text::Html(include_str!(
        "browser_auth.html"
    )));
}
pub(crate) fn routes() -> Router {
    Router::new()
        .push(Router::with_path("authorize/{id}").get(page))
        .push(Router::with_path("api/v1/browser-authorizations").post(start))
        .push(Router::with_path("api/v1/browser-authorizations/{id}/info").post(info))
        .push(Router::with_path("api/v1/browser-authorizations/{id}/login").post(browser_login))
        .push(Router::with_path("api/v1/browser-authorizations/{id}/approve").post(approve))
        .push(Router::with_path("api/v1/browser-authorizations/{id}/poll").post(poll))
}
