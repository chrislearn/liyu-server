//! Public native-client PKCE login and app-scoped host operation adapter.
use crate::{error, hash_secret, new_session_token, pool};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_types::{BigInt, Jsonb, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::OnceLock, time::Duration};
const CLIENT: &str = "liyu-mini-host-v1";
#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type=Jsonb)]
    data: Value,
}
pub(crate) fn ensure_schema(conn: &mut PgConnection) -> QueryResult<()> {
    conn.transaction(|conn| {
        diesel::sql_query("SELECT pg_advisory_xact_lock(731129941)").execute(conn)?;
        conn.batch_execute(
            include_str!("../migrations/20261008000000_browser_authorizations/up.sql")
                .split_once("-- host PKCE extension")
                .unwrap()
                .1,
        )
    })
}
fn b64(bytes: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut bits = 0u32;
    let mut n = 0;
    for b in bytes {
        bits = (bits << 8) | u32::from(*b);
        n += 8;
        while n >= 6 {
            n -= 6;
            out.push(C[((bits >> n) & 63) as usize] as char)
        }
    }
    if n > 0 {
        out.push(C[((bits << (6 - n)) & 63) as usize] as char)
    };
    out
}
fn valid_redirect(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    value == "https://octosense.invalid/auth/callback"
        || (url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some_and(|p| p > 0)
            && url.path() == "/oauth/callback")
}
#[handler]
async fn authorize(req: &mut Request, res: &mut Response) {
    let get = |key| req.query::<String>(key).unwrap_or_default();
    let redirect = get("redirect_uri");
    let challenge = get("code_challenge");
    let state = get("state");
    if get("client_id") != CLIENT
        || get("response_type") != "code"
        || get("scope") != "app.session"
        || get("code_challenge_method") != "S256"
        || challenge.len() != 43
        || !challenge
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || state.len() < 16
        || state.len() > 256
        || !state.bytes().all(|b| b.is_ascii_graphic())
        || !valid_redirect(&redirect)
    {
        return error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid native authorization request",
        );
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let key = new_session_token();
    let source = match req.remote_addr() {
        salvo::conn::SocketAddr::IPv4(a) => a.ip().to_string(),
        salvo::conn::SocketAddr::IPv6(a) => a.ip().to_string(),
        _ => "unknown".into(),
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<bool,diesel::result::Error,_>(|conn|{
        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1,1))").bind::<Text,_>(hash_secret(&source)).execute(conn)?;
        let quota=diesel::sql_query("SELECT jsonb_build_object('count',count(*)) AS data FROM browser_authorizations WHERE source_hash=$1 AND created_at>now()-interval '1 hour'").bind::<Text,_>(hash_secret(&source)).get_result::<Row>(conn)?;
        if quota.data["count"].as_i64().unwrap_or(100)>=60 {return Ok(false)}
        diesel::sql_query("INSERT INTO browser_authorizations(id,poll_hash,browser_hash,purpose,source_hash) VALUES($1,$2,$3,'login',$4)").bind::<Text,_>(&id).bind::<Text,_>(hash_secret(&new_session_token())).bind::<Text,_>(hash_secret(&key)).bind::<Text,_>(hash_secret(&source)).execute(conn)?;
        diesel::sql_query("INSERT INTO host_oauth_requests(auth_id,challenge,redirect_uri,state) VALUES($1,$2,$3,$4)").bind::<Text,_>(&id).bind::<Text,_>(&challenge).bind::<Text,_>(&redirect).bind::<Text,_>(&state).execute(conn)?;Ok(true)
    });
    match result {
        Ok(true) => {
            res.headers_mut()
                .insert("Cache-Control", "no-store".parse().unwrap());
            res.headers_mut()
                .insert("Referrer-Policy", "no-referrer".parse().unwrap());
            res.headers_mut().insert("Content-Security-Policy","default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
            let original = "id=location.pathname.split('/').pop(), key=location.hash.slice(1)";
            let html = include_str!("browser_auth.html")
                .replace(original, &format!("id={}, key={}", json!(id), json!(key)));
            res.render(salvo::prelude::Text::Html(html));
        }
        Ok(false) => error(
            res,
            StatusCode::TOO_MANY_REQUESTS,
            "too many authorization requests",
        ),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot start native authorization",
        ),
    }
}
pub(crate) fn approved(conn: &mut PgConnection, id: &str, uid: i64) -> QueryResult<Option<String>> {
    let code = new_session_token();
    let row=diesel::sql_query("UPDATE host_oauth_requests SET code_hash=$2,user_id=$3 WHERE auth_id=$1 AND expires_at>now() AND code_hash IS NULL RETURNING jsonb_build_object('redirect_uri',redirect_uri,'state',state) AS data").bind::<Text,_>(id).bind::<Text,_>(hash_secret(&code)).bind::<BigInt,_>(uid).get_result::<Row>(conn).optional()?;
    Ok(row.map(|row| {
        let mut url = reqwest::Url::parse(row.data["redirect_uri"].as_str().unwrap()).unwrap();
        url.query_pairs_mut()
            .append_pair("code", &code)
            .append_pair("state", row.data["state"].as_str().unwrap());
        url.to_string()
    }))
}
#[derive(Deserialize)]
struct Grant {
    grant_type: String,
    client_id: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    refresh_token: String,
}
fn issue(conn: &mut PgConnection, uid: i64, family: &str) -> QueryResult<Value> {
    let access = new_session_token();
    let refresh = new_session_token();
    diesel::sql_query(
        "INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')",
    )
    .bind::<Text, _>(hash_secret(&access))
    .bind::<BigInt, _>(uid)
    .execute(conn)?;
    diesel::sql_query("INSERT INTO host_oauth_grants(refresh_hash,access_hash,user_id,family,expires_at) VALUES($1,$2,$3,$4,COALESCE((SELECT min(expires_at) FROM host_oauth_grants WHERE family=$4),now()+interval '90 days'))").bind::<Text,_>(hash_secret(&refresh)).bind::<Text,_>(hash_secret(&access)).bind::<BigInt,_>(uid).bind::<Text,_>(family).execute(conn)?;
    Ok(
        json!({"access_token":access,"refresh_token":refresh,"token_type":"Bearer","expires_in":3600,"scope":"app.session"}),
    )
}
#[handler]
async fn token(req: &mut Request, res: &mut Response) {
    let Ok(body) = req.parse_form::<Grant>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid OAuth grant");
    };
    if body.client_id != CLIENT {
        return error(res, StatusCode::BAD_REQUEST, "invalid client");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Option<Value>,diesel::result::Error,_>(|conn|{
        if body.grant_type=="authorization_code" {
            if !(43..=128).contains(&body.code_verifier.len()) || !body.code_verifier.bytes().all(|b|b.is_ascii_alphanumeric()||b"-._~".contains(&b)) {return Ok(None)}
            let row=diesel::sql_query("SELECT jsonb_build_object('challenge',r.challenge,'redirect_uri',r.redirect_uri,'user_id',r.user_id) AS data FROM host_oauth_requests r JOIN browser_authorizations a ON a.id=r.auth_id JOIN users u ON u.id=r.user_id WHERE r.code_hash=$1 AND NOT r.consumed AND r.expires_at>now() AND a.state='approved' AND u.is_active FOR UPDATE OF r,a").bind::<Text,_>(hash_secret(&body.code)).get_result::<Row>(conn).optional()?;
            let Some(row)=row else {return Ok(None)};
            if row.data["challenge"]!=b64(&Sha256::digest(body.code_verifier.as_bytes())) || row.data["redirect_uri"]!=body.redirect_uri {return Ok(None)}
            diesel::sql_query("UPDATE host_oauth_requests SET consumed=true WHERE code_hash=$1").bind::<Text,_>(hash_secret(&body.code)).execute(conn)?;
            issue(conn,row.data["user_id"].as_i64().unwrap(),&new_session_token()).map(Some)
        } else if body.grant_type=="refresh_token" {
            diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended(family,0)) FROM host_oauth_grants WHERE refresh_hash=$1").bind::<Text,_>(hash_secret(&body.refresh_token)).execute(conn)?;
            let row=diesel::sql_query("SELECT jsonb_build_object('user_id',g.user_id,'family',g.family,'used',g.used,'revoked',g.revoked) AS data FROM host_oauth_grants g JOIN users u ON u.id=g.user_id WHERE refresh_hash=$1 AND expires_at>now() AND u.is_active FOR UPDATE OF g").bind::<Text,_>(hash_secret(&body.refresh_token)).get_result::<Row>(conn).optional()?;
            let Some(row)=row else {return Ok(None)};let family=row.data["family"].as_str().unwrap();
            if row.data["used"]==true || row.data["revoked"]==true {revoke_family(conn,family)?;return Ok(None)}
            diesel::sql_query("UPDATE host_oauth_grants SET used=true WHERE refresh_hash=$1").bind::<Text,_>(hash_secret(&body.refresh_token)).execute(conn)?;
            diesel::sql_query("DELETE FROM sessions WHERE token_hash IN(SELECT access_hash FROM host_oauth_grants WHERE family=$1)").bind::<Text,_>(family).execute(conn)?;
            issue(conn,row.data["user_id"].as_i64().unwrap(),family).map(Some)
        } else {Ok(None)}
    });
    res.headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    match result {
        Ok(Some(data)) => res.render(Json(data)),
        Ok(None) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(json!({"error":"invalid_grant"})))
        }
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "native token exchange failed",
        ),
    }
}
fn revoke_family(conn: &mut PgConnection, family: &str) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM sessions WHERE token_hash IN(SELECT access_hash FROM host_oauth_grants WHERE family=$1)").bind::<Text,_>(family).execute(conn)?;
    diesel::sql_query("UPDATE host_oauth_grants SET revoked=true WHERE family=$1")
        .bind::<Text, _>(family)
        .execute(conn)?;
    Ok(())
}
#[handler]
async fn identity(req: &mut Request, res: &mut Response) {
    let Some(uid) = crate::user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match diesel::sql_query("SELECT jsonb_build_object('sub',id::text,'label',display_name) AS data FROM users WHERE id=$1 AND is_active").bind::<BigInt,_>(uid).get_result::<Row>(&mut conn){Ok(row)=>res.render(Json(row.data)),Err(_)=>error(res,StatusCode::UNAUTHORIZED,"account unavailable")}
}
#[handler]
async fn logout(req: &mut Request, res: &mut Response) {
    let hash = hash_secret(crate::bearer(req).unwrap_or_default());
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<(),diesel::result::Error,_>(|conn|{
        let rows=diesel::sql_query("SELECT jsonb_build_object('family',family) AS data FROM host_oauth_grants WHERE access_hash=$1").bind::<Text,_>(&hash).load::<Row>(conn)?;
        for row in rows {
            diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))").bind::<Text,_>(row.data["family"].as_str().unwrap()).execute(conn)?;
            revoke_family(conn,row.data["family"].as_str().unwrap())?;} crate::revoke_session(conn,&hash)?;Ok(())
    });
    match result {
        Ok(()) => res.render(Json(json!({"logged_out":true}))),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot revoke host session",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    path: String,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    expected_total: Option<i64>,
}
fn allowed(path: &str) -> bool {
    if path.len() > 2048 || !path.starts_with('/') || path.starts_with("//") {
        return false;
    }
    let path = path.split('?').next().unwrap();
    let pieces: Vec<_> = path.split('/').skip(1).collect();
    !pieces.is_empty()
        && [
            "me",
            "products",
            "catalog",
            "shipments",
            "browser-authorizations",
            "categories",
            "friends",
            "friend-tags",
            "notifications",
            "wishlists",
            "gifts",
            "contracts",
            "orders",
            "wallet",
            "benefits",
            "coupons",
            "contract-templates",
        ]
        .contains(&pieces[0])
        && pieces.iter().all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}
#[handler]
async fn contact_status(req: &mut Request, res: &mut Response) {
    let path = req.query::<String>("path").unwrap_or_default();
    let key = req.query::<String>("poll_key").unwrap_or_default();
    if !path.starts_with("/browser-authorizations/") || !path.ends_with("/poll") {
        return error(res, StatusCode::BAD_REQUEST, "invalid contact route");
    }
    forward(
        req,
        res,
        Operation {
            path,
            body: Some(json!({"poll_key":key})),
            idempotency_key: None,
            expected_total: None,
        },
        "POST",
    )
    .await;
}
#[handler]
async fn read_operation(req: &mut Request, res: &mut Response) {
    let path = req.query::<String>("path").unwrap_or_default();
    forward(
        req,
        res,
        Operation {
            path,
            body: None,
            idempotency_key: None,
            expected_total: None,
        },
        "GET",
    )
    .await;
}
#[handler]
async fn write_operation(req: &mut Request, res: &mut Response) {
    let method = req
        .param::<String>("method")
        .unwrap_or_default()
        .to_uppercase();
    let Ok(body) = req.parse_json::<Operation>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid host operation");
    };
    forward(req, res, body, &method).await;
}
async fn forward(req: &mut Request, res: &mut Response, body: Operation, method: &str) {
    if crate::user_id(req).is_none() {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    }
    if !allowed(&body.path)
        || !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method)
        || body.idempotency_key.as_ref().is_some_and(|k| {
            k.is_empty() || k.len() > 128 || !k.bytes().all(|c| c.is_ascii_graphic())
        })
        || body.expected_total.is_some_and(|n| n < 0)
    {
        return error(
            res,
            StatusCode::BAD_REQUEST,
            "operation is outside the LIYU business API",
        );
    }
    if body.path.starts_with("/browser-authorizations") {
        if method != "POST"
            || (body.path == "/browser-authorizations"
                && !body
                    .body
                    .as_ref()
                    .is_some_and(|v| v["purpose"] == "email" || v["purpose"] == "phone"))
        {
            return error(res, StatusCode::BAD_REQUEST, "host login is PKCE only");
        }
        if body.path != "/browser-authorizations" {
            let pieces: Vec<_> = body.path.split('?').next().unwrap().split('/').collect();
            if pieces.len() != 4 || pieces[3] != "poll" {
                return error(res, StatusCode::BAD_REQUEST, "invalid contact status route");
            }
            let Ok(mut conn) = pool().get() else {
                return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
            };
            let authorized = diesel::sql_query("SELECT jsonb_build_object('ok',true) AS data FROM browser_authorizations WHERE id=$1 AND owner_id=$2 AND purpose IN ('email','phone')").bind::<Text,_>(pieces[2]).bind::<BigInt,_>(crate::user_id(req).unwrap()).get_result::<Row>(&mut conn).optional();
            if !matches!(authorized, Ok(Some(_))) {
                return error(res, StatusCode::FORBIDDEN, "not your contact flow");
            }
        }
    }
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    let http = HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap()
    });
    let bind = std::env::var("LIYU_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let Ok(addr) = bind.parse::<std::net::SocketAddr>() else {
        return error(
            res,
            StatusCode::SERVICE_UNAVAILABLE,
            "invalid internal listener",
        );
    };
    let mut request = http
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            format!("http://127.0.0.1:{}/api/v1{}", addr.port(), body.path),
        )
        .bearer_auth(crate::bearer(req).unwrap_or_default());
    if let Some(key) = body.idempotency_key {
        request = request.header("Idempotency-Key", key)
    };
    if let Some(total) = body.expected_total {
        request = request.header("X-Expected-Total-Cents", total)
    };
    if let Some(value) = body.body {
        request = request.json(&value)
    }
    let Ok(mut reply) = request.send().await else {
        return error(res, StatusCode::BAD_GATEWAY, "business API did not answer");
    };
    let status = reply.status().as_u16();
    let mut bytes = Vec::new();
    loop {
        match reply.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > 60000 {
                    return error(
                        res,
                        StatusCode::BAD_GATEWAY,
                        "business response exceeds host limit; narrow the request",
                    );
                };
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            Err(_) => return error(res, StatusCode::BAD_GATEWAY, "business response failed"),
        }
    }
    let result = if bytes.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => v,
            Err(_) => {
                return error(
                    res,
                    StatusCode::BAD_GATEWAY,
                    "business API did not return JSON",
                )
            }
        }
    };
    res.render(Json(json!({"status":status,"body":result})))
}
pub(crate) fn routes() -> Router {
    Router::new()
        .push(Router::with_path("oauth/authorize").get(authorize))
        .push(Router::with_path("oauth/token").post(token))
        .push(Router::with_path("oauth/me").get(identity))
        .push(Router::with_path("oauth/logout").post(logout))
        .push(Router::with_path("api/v1/host/read").get(read_operation))
        .push(Router::with_path("api/v1/host/contact-status").get(contact_status))
        .push(Router::with_path("api/v1/host/{method}").post(write_operation))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_and_operation_boundaries() {
        assert!(valid_redirect("https://octosense.invalid/auth/callback"));
        assert!(valid_redirect("http://127.0.0.1:44001/oauth/callback"));
        assert!(!valid_redirect("https://evil.test/auth/callback"));
        assert!(!valid_redirect("http://127.0.0.1:44001/oauth/callback?x=1"));
        assert!(allowed("/gifts/123/puzzle"));
        for p in [
            "//evil.test",
            "/oauth/token",
            "/host/read",
            "/me/../auth/login",
            "/me/%2e%2e/auth",
            "/me\\evil",
        ] {
            assert!(!allowed(p))
        }
        assert_eq!(
            b64(&Sha256::digest(b"0123456789")),
            "hNiYd_DUBB77a_kaFvAkjy_Vc-avBcGflr7bn4gveII"
        );
    }
}
