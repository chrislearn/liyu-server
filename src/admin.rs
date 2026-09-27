//! Browser-only administration. No user Bearer credentials are accepted.
use crate::{error, hash_secret, new_session_token, pool};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_query,
    sql_types::{Array, BigInt, Bool, Integer, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;

const COOKIE: &str = "liyu_admin_session";
const TTL: u32 = 8 * 60 * 60;

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}
#[derive(QueryableByName)]
struct Id {
    #[diesel(sql_type = BigInt)]
    id: i64,
}
#[derive(QueryableByName)]
struct Admin {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    username: String,
    #[diesel(sql_type = Text)]
    password_hash: String,
}
#[derive(QueryableByName)]
struct Session {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    username: String,
    #[diesel(sql_type = Text)]
    csrf_token: String,
}

fn password_hash(password: &str) -> Result<String, String> {
    let salt =
        SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes()).map_err(|e| e.to_string())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

/// Existing databases have already marked the consolidated init migration applied.
/// Install only its admin section, atomically, without replaying user/catalog seeds.
pub(crate) fn ensure_schema(conn: &mut PgConnection) -> Result<(), String> {
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock(731129927)").execute(conn)?;
        let state: Count = sql_query(
            "SELECT ((to_regclass('administrators') IS NOT NULL)::int + \
             (to_regclass('admin_sessions') IS NOT NULL)::int + \
             (to_regclass('catalog_id_seq') IS NOT NULL)::int)::bigint AS count",
        ).get_result(conn)?;
        match state.count {
            0 => {
                let sql = include_str!("../migrations/20260926000000_init/up.sql")
                    .split_once("-- 12. web administration")
                    .expect("admin section in consolidated migration").1;
                conn.batch_execute(&format!("-- 12. web administration{sql}"))?;
            }
            3 => (),
            _ => return Err(diesel::result::Error::RollbackTransaction),
        }
        conn.batch_execute(
            "ALTER TABLE administrators DROP CONSTRAINT IF EXISTS administrators_username_check",
        )?;
        Ok(())
    }).map_err(|e| match e {
        diesel::result::Error::RollbackTransaction =>
            "partial admin schema detected; inspect administrators, admin_sessions and catalog_id_seq".into(),
        _ => e.to_string(),
    })
}

pub(crate) fn bootstrap(conn: &mut PgConnection) -> Result<(), String> {
    let name = std::env::var("LIYU_ADMIN_USERNAME")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let password = std::env::var("LIYU_ADMIN_PASSWORD").unwrap_or_default();
    if name.is_empty() && password.is_empty() {
        return Ok(());
    }
    sync_configured_admin(conn, &name, &password)
}

fn sync_configured_admin(
    conn: &mut PgConnection,
    name: &str,
    password: &str,
) -> Result<(), String> {
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock(731129927)").execute(conn)?;
        let existing =
            sql_query("SELECT id,username,password_hash FROM administrators WHERE username=$1")
                .bind::<Text, _>(name)
                .get_result::<Admin>(conn)
                .optional()?;
        if let Some(admin) = existing {
            let unchanged = PasswordHash::new(&admin.password_hash)
                .ok()
                .is_some_and(|hash| {
                    Argon2::default()
                        .verify_password(password.as_bytes(), &hash)
                        .is_ok()
                });
            if !unchanged {
                let hash = password_hash(password)
                    .map_err(|_| diesel::result::Error::RollbackTransaction)?;
                sql_query("UPDATE administrators SET password_hash=$1 WHERE id=$2")
                    .bind::<Text, _>(hash)
                    .bind::<BigInt, _>(admin.id)
                    .execute(conn)?;
                // Password changes invalidate this administrator's previous sessions.
                sql_query("DELETE FROM admin_sessions WHERE administrator_id=$1")
                    .bind::<BigInt, _>(admin.id)
                    .execute(conn)?;
            }
        } else {
            let hash =
                password_hash(password).map_err(|_| diesel::result::Error::RollbackTransaction)?;
            sql_query("INSERT INTO administrators(username,password_hash) VALUES ($1,$2)")
                .bind::<Text, _>(name)
                .bind::<Text, _>(hash)
                .execute(conn)?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

fn token(req: &Request) -> Option<&str> {
    req.headers()
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == COOKIE && value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
                .then_some(value)
        })
}

fn same_origin(req: &Request) -> bool {
    if req
        .headers()
        .get("x-admin-request")
        .and_then(|v| v.to_str().ok())
        != Some("1")
    {
        return false;
    }
    match req.headers().get("origin") {
        None => true, // Non-browser tools still need the explicit header and CSRF token.
        Some(origin) => {
            let origin = origin.to_str().unwrap_or("");
            let authority = origin
                .strip_prefix("https://")
                .or_else(|| origin.strip_prefix("http://"));
            let host = req.headers().get("host").and_then(|v| v.to_str().ok());
            authority.is_some() && authority == host
        }
    }
}

fn authorize(req: &Request, res: &mut Response, mutation: bool) -> Option<Session> {
    let Some(secret) = token(req) else {
        error(
            res,
            StatusCode::UNAUTHORIZED,
            "administrator login required",
        );
        return None;
    };
    let Ok(mut conn) = pool().get() else {
        error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
        return None;
    };
    let session = sql_query("SELECT a.id,a.username,s.csrf_token FROM admin_sessions s JOIN administrators a ON a.id=s.administrator_id WHERE s.token_hash=$1 AND s.expires_at>now() AND a.is_active")
        .bind::<Text,_>(hash_secret(secret)).get_result::<Session>(&mut conn).optional();
    match session {
        Ok(Some(s)) => {
            if mutation
                && (!same_origin(req)
                    || req
                        .headers()
                        .get("x-csrf-token")
                        .and_then(|v| v.to_str().ok())
                        != Some(s.csrf_token.as_str()))
            {
                error(res, StatusCode::FORBIDDEN, "invalid CSRF token or origin");
                None
            } else {
                Some(s)
            }
        }
        Ok(None) => {
            error(
                res,
                StatusCode::UNAUTHORIZED,
                "administrator login required",
            );
            None
        }
        Err(_) => {
            error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "administrator session query failed",
            );
            None
        }
    }
}

fn cookie(res: &mut Response, value: &str, max_age: u32) {
    let secure = if crate::config().admin_cookie_secure {
        "; Secure"
    } else {
        ""
    };
    res.headers_mut().insert(
        "set-cookie",
        format!(
            "{COOKIE}={value}; Path=/admin; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}"
        )
        .parse()
        .unwrap(),
    );
}

#[handler]
async fn page(res: &mut Response) {
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("x-frame-options", "DENY".parse().unwrap());
    res.headers_mut().insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    res.render(salvo::prelude::Text::Html(include_str!(
        "../web/admin.html"
    )));
}
#[handler]
async fn script(res: &mut Response) {
    res.headers_mut().insert(
        "content-type",
        "application/javascript; charset=utf-8".parse().unwrap(),
    );
    res.render(include_str!("../web/admin.js"));
}
#[handler]
async fn style(res: &mut Response) {
    res.headers_mut()
        .insert("content-type", "text/css; charset=utf-8".parse().unwrap());
    res.render(include_str!("../web/admin.css"));
}
#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}
#[handler]
async fn login(req: &mut Request, res: &mut Response) {
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    if !same_origin(req) {
        return error(res, StatusCode::FORBIDDEN, "invalid origin");
    }
    let body: Login = match req.parse_json().await {
        Ok(v) => v,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row = sql_query(
        "SELECT id,username,password_hash FROM administrators WHERE username=$1 AND is_active",
    )
    .bind::<Text, _>(body.username.trim().to_lowercase())
    .get_result::<Admin>(&mut conn)
    .optional();
    let admin = match row {
        Ok(Some(a)) => a,
        Ok(None) => {
            return error(
                res,
                StatusCode::UNAUTHORIZED,
                "invalid administrator credentials",
            )
        }
        Err(_) => {
            return error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "administrator login query failed",
            )
        }
    };
    let valid = PasswordHash::new(&admin.password_hash)
        .ok()
        .is_some_and(|h| {
            Argon2::default()
                .verify_password(body.password.as_bytes(), &h)
                .is_ok()
        });
    if !valid {
        return error(
            res,
            StatusCode::UNAUTHORIZED,
            "invalid administrator credentials",
        );
    }
    let secret = new_session_token();
    let csrf = new_session_token();
    let result = conn.transaction::<(), diesel::result::Error, _>(|conn| {
        sql_query("DELETE FROM admin_sessions WHERE expires_at<=now()").execute(conn)?;
        if let Some(old) = token(req) {
            sql_query("DELETE FROM admin_sessions WHERE token_hash=$1")
                .bind::<Text, _>(hash_secret(old))
                .execute(conn)?;
        }
        sql_query(
            "INSERT INTO admin_sessions(token_hash,administrator_id,csrf_token) VALUES ($1,$2,$3)",
        )
        .bind::<Text, _>(hash_secret(&secret))
        .bind::<BigInt, _>(admin.id)
        .bind::<Text, _>(&csrf)
        .execute(conn)?;
        Ok(())
    });
    if result.is_err() {
        return error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "administrator login failed",
        );
    }
    cookie(res, &secret, TTL);
    res.render(Json(
        json!({"id":admin.id,"username":admin.username,"csrf_token":csrf}),
    ));
}
#[handler]
async fn me(req: &mut Request, res: &mut Response) {
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    if let Some(s) = authorize(req, res, false) {
        res.render(Json(
            json!({"id":s.id,"username":s.username,"csrf_token":s.csrf_token}),
        ));
    }
}
#[handler]
async fn logout(req: &mut Request, res: &mut Response) {
    if authorize(req, res, true).is_none() {
        return;
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    if sql_query("DELETE FROM admin_sessions WHERE token_hash=$1")
        .bind::<Text, _>(hash_secret(token(req).unwrap()))
        .execute(&mut conn)
        .is_err()
    {
        return error(res, StatusCode::INTERNAL_SERVER_ERROR, "logout failed");
    }
    cookie(res, "", 0);
    res.status_code(StatusCode::NO_CONTENT);
}

#[derive(Deserialize)]
struct ProductInput {
    name: String,
    category: String,
    price_cents: i64,
    physical: bool,
    brand: String,
    kind: String,
    spec: String,
    description: String,
    tags: Vec<String>,
    stock: i32,
    is_active: bool,
}
impl ProductInput {
    fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.chars().count() <= 200
            && crate::catalog::CATEGORIES
                .iter()
                .any(|(id, _)| *id == self.category)
            && self.price_cents >= 0
            && self.price_cents <= 1_000_000_000
            && self.stock >= 0
            && self.brand.chars().count() <= 100
            && self.kind.chars().count() <= 100
            && self.spec.chars().count() <= 500
            && self.description.chars().count() <= 5000
            && self.tags.len() <= 20
            && self.tags.iter().all(|t| t.chars().count() <= 50)
    }
}
#[handler]
async fn products(req: &mut Request, res: &mut Response) {
    if authorize(req, res, false).is_none() {
        return;
    }
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    let q = req.query::<String>("q").unwrap_or_default();
    let cursor = req.query::<i32>("cursor").unwrap_or(-1);
    if q.len() > 400 || cursor < -1 {
        return error(res, StatusCode::BAD_REQUEST, "invalid query");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let sql=format!("SELECT {} FROM catalog WHERE id>$1 AND ($2='' OR name ILIKE '%'||$2||'%') ORDER BY id LIMIT 101",crate::catalog::COLUMNS);
    match sql_query(sql)
        .bind::<Integer, _>(cursor)
        .bind::<Text, _>(&q)
        .load::<crate::catalog::ProductRow>(&mut conn)
    {
        Ok(mut rows) => {
            let more = rows.len() > 100;
            rows.truncate(100);
            let next = if more {
                rows.last().map(|r| r.id)
            } else {
                None
            };
            res.render(Json(json!({"items":rows,"next_cursor":next})));
        }
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "product query failed",
        ),
    }
}
async fn save(req: &mut Request, res: &mut Response, create: bool) {
    if authorize(req, res, true).is_none() {
        return;
    }
    let body: ProductInput = match req.parse_json().await {
        Ok(v) => v,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if !body.valid() {
        return error(res, StatusCode::BAD_REQUEST, "invalid product fields");
    }
    let id = if create {
        0
    } else {
        match req.param::<i32>("id").filter(|id| *id >= 0) {
            Some(id) => id,
            None => return error(res, StatusCode::BAD_REQUEST, "invalid product id"),
        }
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let sql = if create {
        "INSERT INTO catalog(name,category,price_cents,physical,brand,kind,spec,description,tags,stock,is_active) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING id::bigint AS id"
    } else {
        "UPDATE catalog SET name=$1,category=$2,price_cents=$3,physical=$4,brand=$5,kind=$6,spec=$7,description=$8,tags=$9,stock=$10,is_active=$11 WHERE id=$12 RETURNING id::bigint AS id"
    };
    let query = sql_query(sql)
        .bind::<Text, _>(body.name.trim())
        .bind::<Text, _>(&body.category)
        .bind::<BigInt, _>(body.price_cents)
        .bind::<Bool, _>(body.physical)
        .bind::<Text, _>(&body.brand)
        .bind::<Text, _>(&body.kind)
        .bind::<Text, _>(&body.spec)
        .bind::<Text, _>(&body.description)
        .bind::<Array<Text>, _>(&body.tags)
        .bind::<Integer, _>(body.stock)
        .bind::<Bool, _>(body.is_active);
    let result = if create {
        query.get_result::<Id>(&mut conn)
    } else {
        query.bind::<Integer, _>(id).get_result::<Id>(&mut conn)
    };
    match result {
        Ok(row) => res.render(Json(json!({"id":row.id}))),
        Err(diesel::result::Error::NotFound) => {
            error(res, StatusCode::NOT_FOUND, "product not found")
        }
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "product save failed",
        ),
    }
}
#[handler]
async fn create_product(req: &mut Request, res: &mut Response) {
    save(req, res, true).await;
}
#[handler]
async fn update_product(req: &mut Request, res: &mut Response) {
    save(req, res, false).await;
}

#[handler]
async fn upload_image(req: &mut Request, res: &mut Response) {
    if authorize(req, res, true).is_none() {
        return;
    }
    let Some(id) = req.param::<i32>("id").filter(|id| *id >= 0) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid product id");
    };
    let variant = req.param::<String>("variant").unwrap_or_default();
    if !matches!(variant.as_str(), "thumb" | "card" | "detail") {
        return error(res, StatusCode::BAD_REQUEST, "invalid image variant");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match sql_query("SELECT id::bigint AS id FROM catalog WHERE id=$1")
        .bind::<Integer, _>(id)
        .get_result::<Id>(&mut conn)
        .optional()
    {
        Ok(Some(_)) => (),
        Ok(None) => return error(res, StatusCode::NOT_FOUND, "product not found"),
        Err(_) => {
            return error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "product query failed",
            )
        }
    }
    drop(conn);
    const MAX: usize = 4 * 1024 * 1024;
    if req
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX)
    {
        return error(res, StatusCode::PAYLOAD_TOO_LARGE, "image exceeds 4 MiB");
    }
    let bytes = match req.payload_with_max_size(MAX + 1).await {
        Ok(b) if b.len() <= MAX => b,
        _ => return error(res, StatusCode::PAYLOAD_TOO_LARGE, "image exceeds 4 MiB"),
    };
    let Some((_, w, h)) = crate::avatar::sniff_image(bytes) else {
        return error(res, StatusCode::UNPROCESSABLE_ENTITY, "invalid image");
    };
    if !(16..=4096).contains(&w) || !(16..=4096).contains(&h) {
        return error(
            res,
            StatusCode::UNPROCESSABLE_ENTITY,
            "image dimensions must be 16–4096 pixels",
        );
    }
    let decoded = match image::load_from_memory(bytes) {
        Ok(i) if i.width() == w && i.height() == h => i,
        _ => return error(res, StatusCode::UNPROCESSABLE_ENTITY, "invalid image"),
    };
    let mut png = std::io::Cursor::new(Vec::new());
    if decoded.write_to(&mut png, image::ImageFormat::Png).is_err() {
        return error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "image encoding failed",
        );
    }
    let dir = crate::config()
        .data_dir
        .join("products")
        .join(format!("admin-{id}"));
    if std::fs::create_dir_all(&dir)
        .and_then(|_| {
            crate::avatar::atomic_write(&dir.join(format!("{variant}.png")), png.get_ref())
        })
        .is_err()
    {
        return error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "image storage failed",
        );
    }
    res.render(Json(
        json!({"image_url":format!("/api/v1/media/products/{id}/{variant}")}),
    ));
}

pub(crate) fn routes() -> Router {
    Router::new()
        .push(Router::with_path("admin").get(page))
        .push(Router::with_path("admin/app.js").get(script))
        .push(Router::with_path("admin/app.css").get(style))
        .push(Router::with_path("admin/api/login").post(login))
        .push(Router::with_path("admin/api/logout").post(logout))
        .push(Router::with_path("admin/api/me").get(me))
        .push(
            Router::with_path("admin/api/products")
                .get(products)
                .post(create_product),
        )
        .push(Router::with_path("admin/api/products/{id}").put(update_product))
        .push(Router::with_path("admin/api/products/{id}/images/{variant}").post(upload_image))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_database_admin_upgrade_preserves_data_and_sequence() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let mut conn = PgConnection::establish(&url).unwrap();
        conn.test_transaction::<_, diesel::result::Error, _>(|conn| {
            let schema = format!("admin_upgrade_{}", uuid::Uuid::new_v4().simple());
            conn.batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema}; \
                 CREATE TABLE catalog (id integer PRIMARY KEY, name text NOT NULL); \
                 INSERT INTO catalog VALUES (101, 'preserved product');"
            ))?;
            ensure_schema(conn).unwrap();
            sql_query("INSERT INTO administrators(username,password_hash) VALUES ('existing', 'preserved hash')").execute(conn)?;
            let first: Id = sql_query("INSERT INTO catalog(name) VALUES ('new product') RETURNING id::bigint AS id").get_result(conn)?;
            assert_eq!(first.id, 102);
            ensure_schema(conn).unwrap();
            let next: Id = sql_query("SELECT nextval('catalog_id_seq') AS id").get_result(conn)?;
            assert_eq!(next.id, 103);
            let original: Id = sql_query("SELECT id::bigint AS id FROM catalog WHERE name='preserved product'").get_result(conn)?;
            assert_eq!(original.id, 101);
            let admin: Admin = sql_query("SELECT id,username,password_hash FROM administrators WHERE username='existing'").get_result(conn)?;
            assert_eq!(admin.password_hash, "preserved hash");
            Ok(())
        });
    }

    #[test]
    fn configured_admin_sync_updates_only_changed_password_and_revokes_its_sessions() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let mut conn = PgConnection::establish(&url).unwrap();
        conn.test_transaction::<_, diesel::result::Error, _>(|conn| {
            let schema = format!("admin_sync_{}", uuid::Uuid::new_v4().simple());
            conn.batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema}; \
                 CREATE TABLE catalog (id integer PRIMARY KEY); INSERT INTO catalog VALUES (32);"
            ))?;
            ensure_schema(conn).unwrap();
            sync_configured_admin(conn, "other", "keep").unwrap();
            sync_configured_admin(conn, "admin", "old").unwrap();
            let read = |conn: &mut PgConnection, name: &str| {
                sql_query("SELECT id,username,password_hash FROM administrators WHERE username=$1")
                    .bind::<Text,_>(name).get_result::<Admin>(conn)
            };
            let other_hash = read(conn, "other")?.password_hash;
            let before = read(conn, "admin")?;
            sql_query("INSERT INTO admin_sessions(token_hash,administrator_id,csrf_token) VALUES ('admin-test',$1,'csrf')")
                .bind::<BigInt,_>(before.id).execute(conn)?;
            sync_configured_admin(conn, "admin", "old").unwrap();
            assert_eq!(read(conn,"admin")?.password_hash, before.password_hash);
            let retained: Count = sql_query("SELECT COUNT(*) AS count FROM admin_sessions").get_result(conn)?;
            assert_eq!(retained.count, 1);
            sync_configured_admin(conn, "admin", "admin").unwrap();
            let after = read(conn, "admin")?;
            assert_eq!(after.id, before.id);
            assert!(Argon2::default().verify_password(b"admin", &PasswordHash::new(&after.password_hash).unwrap()).is_ok());
            let revoked: Count = sql_query("SELECT COUNT(*) AS count FROM admin_sessions").get_result(conn)?;
            assert_eq!(revoked.count, 0);
            assert_eq!(read(conn,"other")?.password_hash, other_hash);
            Ok(())
        });
    }

    #[test]
    fn administrator_passwords_use_salted_argon2() {
        let a = password_hash("correct horse battery").unwrap();
        let b = password_hash("correct horse battery").unwrap();
        assert_ne!(a, b);
        let hash = PasswordHash::new(&a).unwrap();
        assert!(Argon2::default()
            .verify_password(b"correct horse battery", &hash)
            .is_ok());
        assert!(Argon2::default().verify_password(b"123456", &hash).is_err());
    }
}
