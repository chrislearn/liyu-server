mod schema;
mod profile;
mod fulfillment;
mod commerce;
mod catalog;
mod wishlist;

use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub(crate) type DbPool = Pool<ConnectionManager<PgConnection>>;
static DB: OnceLock<DbPool> = OnceLock::new();
const MIGRATIONS: EmbeddedMigrations = embed_migrations!();
const TEST_CODE: &str = "123456";

#[derive(Deserialize)]
struct Credentials {
    identifier: String,
    password: String,
    code: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct StateWrite {
    revision: i64,
    state: Value,
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

pub(crate) fn user_id(req: &Request) -> Option<i64> {
    use schema::sessions::dsl::*;
    let hash = hash_secret(bearer(req)?);
    let mut conn = pool().get().ok()?;
    sessions
        .filter(token_hash.eq(hash))
        .select(user_id)
        .first(&mut conn)
        .ok()
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
    if stored_hash != hash_secret(&body.password) {
        return error(res, StatusCode::UNAUTHORIZED, "invalid password");
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
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
    res.render(Json(
        json!({"token": token, "user": {"id": uid, "identifier": identity, "display_name": name}}),
    ));
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
async fn get_state(req: &mut Request, res: &mut Response) {
    if std::env::var("LIYU_ENABLE_LEGACY_STATE").as_deref() != Ok("1") {
        return error(res, StatusCode::GONE, "legacy state sync is disabled");
    }
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
        .select((state, state_revision))
        .first::<(Option<Value>, i64)>(&mut conn)
    {
        Ok((value, revision)) => res.render(Json(json!({"revision": revision, "state": value}))),
        Err(_) => error(res, StatusCode::NOT_FOUND, "user not found"),
    }
}

#[handler]
async fn put_state(req: &mut Request, res: &mut Response) {
    if std::env::var("LIYU_ENABLE_LEGACY_STATE").as_deref() != Ok("1") {
        return error(res, StatusCode::GONE, "legacy state sync is disabled");
    }
    use schema::users::dsl::*;
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let body: StateWrite = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if body.state.get("version").and_then(Value::as_u64) != Some(4)
        || !body.state.get("gifts").is_some_and(Value::is_array)
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid LiYu state version");
    }
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    let updated = diesel::update(
        users
            .filter(id.eq(uid))
            .filter(state_revision.eq(body.revision)),
    )
    .set((
        state.eq(Some(body.state)),
        state_revision.eq(body.revision + 1),
    ))
    .execute(&mut conn);
    match updated {
        Ok(1) => res.render(Json(json!({"revision": body.revision + 1}))),
        Ok(_) => error(
            res,
            StatusCode::CONFLICT,
            "state revision changed; reload first",
        ),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "state write failed"),
    }
}

#[tokio::main]
async fn main() {
    let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL");
    let manager = ConnectionManager::<PgConnection>::new(url);
    let db = Pool::builder()
        .max_size(8)
        .build(manager)
        .expect("connect to PostgreSQL");
    {
        let mut conn = db.get().expect("get database connection");
        conn.run_pending_migrations(MIGRATIONS)
            .expect("run migrations");
    }
    DB.set(db)
        .unwrap_or_else(|_| panic!("database already initialized"));
    let router = Router::new()
        .push(Router::with_path("health").get(health))
        .push(Router::with_path("api/v1/auth/register").post(register))
        .push(Router::with_path("api/v1/auth/login").post(login))
        .push(Router::with_path("api/v1").push(profile::routes()))
        .push(fulfillment::routes())
        .push(commerce::routes())
        .push(catalog::routes())
        .push(wishlist::routes())
        .push(Router::with_path("api/v1/me").get(me))
        .push(
            Router::with_path("api/v1/state")
                .get(get_state)
                .put(put_state),
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
}
