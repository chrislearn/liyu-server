mod admin;
mod avatar;
mod benefits;
mod catalog;
mod commerce;
mod config;
mod demo_logistics;
mod fulfillment;
mod gifting;
mod management;
mod profile;
mod schema;
mod wishlist;

use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::sql_types::{BigInt, Text};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub(crate) type DbPool = Pool<ConnectionManager<PgConnection>>;
static CONFIG: OnceLock<config::Config> = OnceLock::new();

pub(crate) fn config() -> &'static config::Config {
    CONFIG.get().expect("configuration initialized")
}

static DB: OnceLock<DbPool> = OnceLock::new();
const MIGRATIONS: EmbeddedMigrations = embed_migrations!();
const TEST_CODE: &str = "123456";
const SESSION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(QueryableByName)]
pub(crate) struct SessionOwner {
    #[diesel(sql_type = BigInt)]
    user_id: i64,
}

#[derive(Deserialize)]
struct Credentials {
    identifier: String,
    password: String,
    code: Option<String>,
    display_name: Option<String>,
}

pub(crate) fn pool() -> &'static DbPool {
    DB.get().expect("database initialized")
}

pub(crate) fn error(res: &mut Response, status: StatusCode, message: &str) {
    res.status_code(status);
    res.render(Json(json!({"error": message})));
}

fn bearer(req: &Request) -> Option<&str> {
    req.headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn hash_secret(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn new_session_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn session_owner(
    conn: &mut PgConnection,
    secret_hash: &str,
) -> Result<Option<i64>, diesel::result::Error> {
    diesel::sql_query("SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > now() AND EXISTS (SELECT 1 FROM users u WHERE u.id=sessions.user_id AND u.is_active)")
        .bind::<Text, _>(secret_hash)
        .get_result::<SessionOwner>(conn)
        .optional()
        .map(|row| row.map(|owner| owner.user_id))
}

fn revoke_session(
    conn: &mut PgConnection,
    secret_hash: &str,
) -> Result<bool, diesel::result::Error> {
    let deleted = diesel::sql_query(
        "DELETE FROM sessions WHERE token_hash = $1 AND expires_at > now() RETURNING user_id",
    )
    .bind::<Text, _>(secret_hash)
    .get_result::<SessionOwner>(conn)
    .optional()?;
    Ok(deleted.is_some())
}

fn rotate_session(
    conn: &mut PgConnection,
    old_hash: &str,
    new_hash: &str,
) -> Result<bool, diesel::result::Error> {
    conn.transaction::<bool, diesel::result::Error, _>(|conn| {
        // DELETE's row lock allows exactly one concurrent refresh to win.
        let removed = diesel::sql_query(
            "DELETE FROM sessions WHERE token_hash = $1 AND expires_at > now() RETURNING user_id",
        )
        .bind::<Text, _>(old_hash)
        .get_result::<SessionOwner>(conn)
        .optional()?;
        let Some(owner) = removed else {
            return Ok(false);
        };
        diesel::sql_query("INSERT INTO sessions (token_hash, user_id) VALUES ($1, $2)")
            .bind::<Text, _>(new_hash)
            .bind::<BigInt, _>(owner.user_id)
            .execute(conn)?;
        Ok(true)
    })
}

pub(crate) fn user_id(req: &Request) -> Option<i64> {
    let hash = hash_secret(bearer(req)?);
    let mut conn = pool().get().ok()?;
    session_owner(&mut conn, &hash).ok().flatten()
}

#[handler]
async fn health(res: &mut Response) {
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    if diesel::sql_query("SELECT 1").execute(&mut conn).is_err() {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    }
    res.render(Json(json!({"status": "ok"})));
}

async fn authenticate(req: &mut Request, res: &mut Response, is_register: bool) {
    use schema::users::dsl::*;
    let body: Credentials = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let identity = body.identifier.trim().to_lowercase();
    if identity.is_empty() || identity.len() > 254 {
        return error(res, StatusCode::BAD_REQUEST, "invalid identifier");
    }
    if is_register && body.code.as_deref() != Some(TEST_CODE) {
        return error(res, StatusCode::BAD_REQUEST, "invalid test code");
    }
    if body.password != TEST_CODE {
        return error(res, StatusCode::UNAUTHORIZED, "invalid password");
    }
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    if is_register {
        let name = body
            .display_name
            .as_deref()
            .unwrap_or(identity.as_str())
            .trim();
        if name.is_empty() || name.chars().count() > 50 {
            return error(res, StatusCode::BAD_REQUEST, "invalid display name");
        }
        let inserted = diesel::insert_into(users)
            .values((
                identifier.eq(&identity),
                display_name.eq(name),
                password_hash.eq(hash_secret(&body.password)),
            ))
            .on_conflict(identifier)
            .do_nothing()
            .execute(&mut conn);
        if inserted != Ok(1) {
            return error(
                res,
                StatusCode::CONFLICT,
                "account already exists or database error",
            );
        }
    }
    let found: Result<(i64, String, String), _> = users
        .filter(identifier.eq(&identity))
        .select((id, display_name, password_hash))
        .first(&mut conn);
    let (uid, name, stored_hash) = match found {
        Ok(found) => found,
        Err(_) => return error(res, StatusCode::UNAUTHORIZED, "account not found"),
    };
    let active = diesel::sql_query("SELECT id AS user_id FROM users WHERE id=$1 AND is_active")
        .bind::<BigInt, _>(uid)
        .get_result::<SessionOwner>(&mut conn)
        .is_ok();
    if !active || stored_hash != hash_secret(&body.password) {
        return error(res, StatusCode::UNAUTHORIZED, "invalid password");
    }
    let token = new_session_token();
    use schema::sessions::dsl::{sessions, token_hash as session_hash, user_id as session_user};
    if diesel::insert_into(sessions)
        .values((session_hash.eq(hash_secret(&token)), session_user.eq(uid)))
        .execute(&mut conn)
        .is_err()
    {
        return error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot create session",
        );
    }
    res.render(Json(json!({
        "token": token,
        "expires_in_seconds": SESSION_TTL_SECONDS,
        "user": {"id": uid, "identifier": identity, "display_name": name}
    })));
}

#[handler]
async fn register(req: &mut Request, res: &mut Response) {
    authenticate(req, res, true).await;
}

#[handler]
async fn login(req: &mut Request, res: &mut Response) {
    authenticate(req, res, false).await;
}

#[handler]
async fn logout(req: &mut Request, res: &mut Response) {
    let Some(token) = bearer(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    match revoke_session(&mut conn, &hash_secret(token)) {
        Ok(true) => {
            res.status_code(StatusCode::NO_CONTENT);
        }
        Ok(false) => error(res, StatusCode::UNAUTHORIZED, "invalid session"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "session logout failed",
        ),
    }
}

#[handler]
async fn refresh(req: &mut Request, res: &mut Response) {
    let Some(token) = bearer(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let new_token = new_session_token();
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    match rotate_session(&mut conn, &hash_secret(token), &hash_secret(&new_token)) {
        Ok(true) => res.render(Json(json!({
            "token": new_token, "expires_in_seconds": SESSION_TTL_SECONDS
        }))),
        Ok(false) => error(res, StatusCode::UNAUTHORIZED, "invalid session"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "session refresh failed",
        ),
    }
}

#[handler]
async fn me(req: &mut Request, res: &mut Response) {
    use schema::users::dsl::*;
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    match users
        .find(uid)
        .select((id, identifier, display_name))
        .first::<(i64, String, String)>(&mut conn)
    {
        Ok((user_id, user_identifier, user_name)) => res.render(Json(
            json!({"id": user_id, "identifier": user_identifier, "display_name": user_name}),
        )),
        Err(_) => error(res, StatusCode::NOT_FOUND, "user not found"),
    }
}

#[handler]
async fn legacy_state_disabled(res: &mut Response) {
    error(
        res,
        StatusCode::GONE,
        "legacy state sync is permanently disabled",
    );
}

#[tokio::main]
async fn main() {
    if let Err(err) = dotenvy::dotenv() {
        if !err.not_found() {
            panic!("load .env: {err}");
        }
    }
    let settings = config::Config::load().expect("invalid server configuration");
    settings.prepare_storage().expect("prepare data storage");
    CONFIG
        .set(settings)
        .unwrap_or_else(|_| panic!("configuration already initialized"));
    let url = std::env::var("DATABASE_URL")
        .expect("set DATABASE_URL in .env (copy .env.example) or the environment");
    let manager = ConnectionManager::<PgConnection>::new(url);
    let db = Pool::builder()
        .max_size(config().pool_max_size)
        .build(manager)
        .expect("connect to PostgreSQL");
    {
        let mut conn = db.get().expect("get database connection");
        conn.run_pending_migrations(MIGRATIONS)
            .expect("run migrations");
        admin::ensure_schema(&mut conn).expect("upgrade administrator schema");
        management::ensure_schema(&mut conn).expect("upgrade management schema");
        admin::bootstrap(&mut conn).expect("initialize administrator");
    }
    DB.set(db)
        .unwrap_or_else(|_| panic!("database already initialized"));
    let router = Router::new()
        .push(Router::with_path("health").get(health))
        .push(admin::routes())
        .push(management::routes())
        .push(benefits::routes())
        .push(Router::with_path("api/v1/auth/register").post(register))
        .push(Router::with_path("api/v1/auth/login").post(login))
        .push(Router::with_path("api/v1/auth/logout").post(logout))
        .push(Router::with_path("api/v1/auth/refresh").post(refresh))
        .push(Router::with_path("api/v1").push(profile::routes()))
        .push(Router::with_path("api/v1").push(avatar::routes()))
        .push(fulfillment::routes())
        .push(commerce::routes())
        .push(catalog::routes())
        .push(wishlist::routes())
        .push(gifting::routes())
        .push(demo_logistics::routes())
        .push(Router::with_path("api/v1/me").get(me))
        .push(
            Router::with_path("api/v1/state")
                .get(legacy_state_disabled)
                .put(legacy_state_disabled),
        );
    let addr = std::env::var("LIYU_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    println!("LiYu test API listening on {addr}");
    let acceptor = TcpListener::new(addr).bind().await;
    Server::new(acceptor).serve(router).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_tokens_are_hashed() {
        assert_eq!(
            hash_secret("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn database_session_rotation_and_expiry() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return; // Run with a migrated test database to exercise the DB path.
        };
        let mut conn = PgConnection::establish(&url).expect("connect test database");
        let old = hash_secret(&new_session_token());
        let new = hash_secret(&new_session_token());
        let expired = hash_secret(&new_session_token());
        let rolled_back = conn.transaction::<(), diesel::result::Error, _>(|conn| {
            diesel::sql_query(
                "INSERT INTO sessions (token_hash, user_id) \
                 SELECT $1, id FROM users WHERE identifier = 'demo@liyu.test'",
            )
            .bind::<Text, _>(&old)
            .execute(conn)?;
            diesel::sql_query(
                "INSERT INTO sessions (token_hash, user_id, expires_at) \
                 SELECT $1, id, now() - INTERVAL '1 minute' \
                 FROM users WHERE identifier = 'demo@liyu.test'",
            )
            .bind::<Text, _>(&expired)
            .execute(conn)?;
            assert!(session_owner(conn, &old)?.is_some());
            assert_eq!(session_owner(conn, &expired)?, None);
            assert!(!rotate_session(
                conn,
                &expired,
                &hash_secret(&new_session_token())
            )?);
            assert!(rotate_session(conn, &old, &new)?);
            assert_eq!(session_owner(conn, &old)?, None);
            assert!(session_owner(conn, &new)?.is_some());
            assert!(revoke_session(conn, &new)?);
            assert_eq!(session_owner(conn, &new)?, None);
            Err(diesel::result::Error::RollbackTransaction)
        });
        assert!(matches!(
            rolled_back,
            Err(diesel::result::Error::RollbackTransaction)
        ));
    }
}
