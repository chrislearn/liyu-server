//! Account details and private shipping addresses. Every query is scoped to the bearer user.
use diesel::prelude::*;
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::schema::{shipping_addresses, user_profiles, users};

#[derive(Deserialize)]
struct ProfilePatch {
    display_name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct ContactBind {
    value: String,
    code: String,
    challenge_id: String,
}

#[derive(Deserialize)]
struct AddressWrite {
    recipient_name: String,
    phone: String,
    address: String,
    #[serde(default)]
    is_default: bool,
}

fn fail(res: &mut Response, status: StatusCode, message: &str) {
    res.status_code(status);
    res.render(Json(json!({"error": message})));
}

fn uid(req: &Request, res: &mut Response) -> Option<i64> {
    let id = crate::user_id(req);
    if id.is_none() {
        fail(res, StatusCode::UNAUTHORIZED, "invalid session");
    }
    id
}

fn db(
    res: &mut Response,
) -> Option<diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<PgConnection>>> {
    match crate::pool().get() {
        Ok(conn) => Some(conn),
        Err(_) => {
            fail(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
            None
        }
    }
}

fn valid_name(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= 50
}

fn valid_phone(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_digit())
}

fn valid_avatar(s: &str) -> bool {
    s.is_empty()
        || (s.len() <= 512
            && s.starts_with("https://")
            && !s.contains(char::is_whitespace)
            && !s.chars().any(|c| matches!(c, '<' | '>' | '"' | '\'')))
}

fn profile_json(conn: &mut PgConnection, owner: i64) -> QueryResult<serde_json::Value> {
    use user_profiles::dsl as p;
    use users::dsl as u;
    let (id, identifier, display_name): (i64, String, String) = u::users
        .find(owner)
        .select((u::id, u::identifier, u::display_name))
        .first(conn)?;
    let details: Option<(Option<String>, Option<String>, Option<String>)> = p::user_profiles
        .find(owner)
        .select((p::phone, p::email, p::avatar_url))
        .first(conn)
        .optional()?;
    let (mut phone, mut email, avatar_url) = details.unwrap_or_default();
    use diesel::sql_types::Jsonb;
    #[derive(QueryableByName)]
    struct ContactRow {
        #[diesel(sql_type=Jsonb)]
        data: serde_json::Value,
    }
    let verified=diesel::sql_query("SELECT jsonb_build_object('phone', (SELECT value FROM contact_identities WHERE user_id=$1 AND kind='phone' ORDER BY verified_at DESC,id DESC LIMIT 1), 'email', (SELECT value FROM contact_identities WHERE user_id=$1 AND kind='email' ORDER BY verified_at DESC,id DESC LIMIT 1), 'phones', COALESCE(jsonb_agg(value ORDER BY verified_at DESC,id DESC) FILTER (WHERE kind='phone'),'[]'::jsonb), 'emails', COALESCE(jsonb_agg(value ORDER BY verified_at DESC,id DESC) FILTER (WHERE kind='email'),'[]'::jsonb)) AS data FROM contact_identities WHERE user_id=$1")
        .bind::<diesel::sql_types::BigInt,_>(owner).get_result::<ContactRow>(conn).optional()?;
    let phone_verified = verified
        .as_ref()
        .is_some_and(|r| r.data["phone"].is_string());
    let email_verified = verified
        .as_ref()
        .is_some_and(|r| r.data["email"].is_string());
    let mut phones = json!([]);
    let mut emails = json!([]);
    if let Some(row) = verified {
        phones = row.data["phones"].clone();
        emails = row.data["emails"].clone();
        if let Some(s) = row.data["phone"].as_str() {
            phone = Some(s.into())
        }
        if let Some(s) = row.data["email"].as_str() {
            email = Some(s.into())
        }
    }
    let avatar_url = avatar_url
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| crate::avatar::default_url(id));
    Ok(
        json!({"id":id,"identifier":identifier,"display_name":display_name,"phone":phone,"email":email,"phones":phones,"emails":emails,"phone_verified":phone_verified,"email_verified":email_verified,"avatar_url":avatar_url}),
    )
}

#[handler]
async fn get_profile(req: &mut Request, res: &mut Response) {
    let Some(owner) = uid(req, res) else { return };
    let Some(mut conn) = db(res) else { return };
    match profile_json(&mut conn, owner) {
        Ok(value) => res.render(Json(value)),
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "profile query failed",
        ),
    }
}

#[handler]
async fn patch_profile(req: &mut Request, res: &mut Response) {
    let Some(owner) = uid(req, res) else { return };
    let body: ProfilePatch = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return fail(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if body.display_name.is_none() && body.avatar_url.is_none() {
        return fail(res, StatusCode::BAD_REQUEST, "no profile changes");
    }
    if body.display_name.as_deref().is_some_and(|n| !valid_name(n)) {
        return fail(res, StatusCode::BAD_REQUEST, "invalid display name");
    }
    if body.avatar_url.as_deref().is_some_and(|a| !valid_avatar(a)) {
        return fail(res, StatusCode::BAD_REQUEST, "invalid avatar URL");
    }
    let Some(mut conn) = db(res) else { return };
    let result = conn.transaction::<(), diesel::result::Error, _>(|conn| {
        if let Some(name) = body.display_name.as_deref() {
            diesel::update(users::table.find(owner))
                .set(users::display_name.eq(name.trim()))
                .execute(conn)?;
        }
        if let Some(avatar) = body.avatar_url.as_deref() {
            diesel::insert_into(user_profiles::table)
                .values((
                    user_profiles::user_id.eq(owner),
                    user_profiles::avatar_url.eq(Some(avatar)),
                ))
                .on_conflict(user_profiles::user_id)
                .do_update()
                .set((
                    user_profiles::avatar_url.eq(Some(avatar)),
                    user_profiles::updated_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
        }
        Ok(())
    });
    if result.is_err() {
        return fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "profile update failed",
        );
    }
    match profile_json(&mut conn, owner) {
        Ok(value) => res.render(Json(value)),
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "profile query failed",
        ),
    }
}

async fn bind_contact(req: &mut Request, res: &mut Response, phone_kind: bool) {
    let Some(owner) = uid(req, res) else { return };
    let body: ContactBind = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return fail(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let kind = if phone_kind { "phone" } else { "email" };
    let Some(value) = crate::contact_delivery::normalize(kind, &body.value) else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid contact value");
    };
    let Some(mut conn) = db(res) else { return };
    let result = conn.transaction::<bool, diesel::result::Error, _>(|conn| {
        if !crate::contact_delivery::verify(
            conn,
            &body.challenge_id,
            kind,
            &value,
            "bind",
            Some(owner),
            &body.code,
        )? {
            return Ok(false);
        }
        crate::contact_delivery::attach(conn, owner, kind, &value)?;
        Ok(true)
    });
    match result {
        Ok(true) => match profile_json(&mut conn, owner) {
            Ok(v) => res.render(Json(v)),
            Err(_) => fail(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "profile query failed",
            ),
        },
        Ok(false) => fail(
            res,
            StatusCode::BAD_REQUEST,
            "verification required or invalid code",
        ),
        Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        )) => fail(res, StatusCode::CONFLICT, "contact already in use"),
        Err(diesel::result::Error::NotFound) => {
            fail(res, StatusCode::CONFLICT, "contact already in use")
        }
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "contact update failed",
        ),
    }
}

#[handler]
async fn put_phone(req: &mut Request, res: &mut Response) {
    bind_contact(req, res, true).await
}
#[handler]
async fn put_email(req: &mut Request, res: &mut Response) {
    bind_contact(req, res, false).await
}

fn address_json(row: (i64, String, String, String, bool)) -> serde_json::Value {
    json!({"id":row.0,"recipient_name":row.1,"phone":row.2,"address":row.3,"is_default":row.4})
}

fn valid_address(body: &AddressWrite) -> bool {
    valid_name(&body.recipient_name)
        && valid_phone(&body.phone)
        && !body.address.trim().is_empty()
        && body.address.chars().count() <= 500
}

#[handler]
async fn list_addresses(req: &mut Request, res: &mut Response) {
    let Some(owner) = uid(req, res) else { return };
    let Some(mut conn) = db(res) else { return };
    use shipping_addresses::dsl as a;
    let rows = a::shipping_addresses
        .filter(a::user_id.eq(owner))
        .order((a::is_default.desc(), a::id.desc()))
        .select((
            a::id,
            a::recipient_name,
            a::phone,
            a::address,
            a::is_default,
        ))
        .load::<(i64, String, String, String, bool)>(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(rows.into_iter().map(address_json).collect::<Vec<_>>())),
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "address query failed",
        ),
    }
}

#[handler]
async fn create_address(req: &mut Request, res: &mut Response) {
    let Some(owner) = uid(req, res) else { return };
    let body: AddressWrite = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return fail(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if !valid_address(&body) {
        return fail(res, StatusCode::BAD_REQUEST, "invalid address");
    }
    let Some(mut conn) = db(res) else { return };
    use shipping_addresses::dsl as a;
    let result = conn.transaction::<i64, diesel::result::Error, _>(|conn| {
        if body.is_default {
            diesel::update(a::shipping_addresses.filter(a::user_id.eq(owner)))
                .set(a::is_default.eq(false))
                .execute(conn)?;
        }
        diesel::insert_into(a::shipping_addresses)
            .values((
                a::user_id.eq(owner),
                a::recipient_name.eq(body.recipient_name.trim()),
                a::phone.eq(&body.phone),
                a::address.eq(body.address.trim()),
                a::is_default.eq(body.is_default),
            ))
            .returning(a::id)
            .get_result(conn)
    });
    match result {
        Ok(id) => {
            res.status_code(StatusCode::CREATED);
            res.render(Json(json!({"id":id})));
        }
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "address create failed",
        ),
    }
}

#[handler]
async fn update_address(req: &mut Request, res: &mut Response) {
    let Some(owner) = uid(req, res) else { return };
    let Some(id) = req.param::<i64>("id") else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid address id");
    };
    let body: AddressWrite = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return fail(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if !valid_address(&body) {
        return fail(res, StatusCode::BAD_REQUEST, "invalid address");
    }
    let Some(mut conn) = db(res) else { return };
    use shipping_addresses::dsl as a;
    let result = conn.transaction::<usize, diesel::result::Error, _>(|conn| {
        let owned = a::shipping_addresses
            .filter(a::id.eq(id))
            .filter(a::user_id.eq(owner))
            .select(a::id)
            .first::<i64>(conn)
            .optional()?;
        if owned.is_none() {
            return Ok(0);
        }
        if body.is_default {
            diesel::update(a::shipping_addresses.filter(a::user_id.eq(owner)))
                .set(a::is_default.eq(false))
                .execute(conn)?;
        }
        diesel::update(
            a::shipping_addresses
                .filter(a::id.eq(id))
                .filter(a::user_id.eq(owner)),
        )
        .set((
            a::recipient_name.eq(body.recipient_name.trim()),
            a::phone.eq(&body.phone),
            a::address.eq(body.address.trim()),
            a::is_default.eq(body.is_default),
            a::updated_at.eq(diesel::dsl::now),
        ))
        .execute(conn)
    });
    match result {
        Ok(0) => fail(res, StatusCode::NOT_FOUND, "address not found"),
        Ok(_) => res.render(Json(json!({"id":id}))),
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "address update failed",
        ),
    }
}

#[handler]
async fn delete_address(req: &mut Request, res: &mut Response) {
    let Some(owner) = uid(req, res) else { return };
    let Some(id) = req.param::<i64>("id") else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid address id");
    };
    let Some(mut conn) = db(res) else { return };
    use shipping_addresses::dsl as a;
    match diesel::delete(
        a::shipping_addresses
            .filter(a::id.eq(id))
            .filter(a::user_id.eq(owner)),
    )
    .execute(&mut conn)
    {
        Ok(0) => fail(res, StatusCode::NOT_FOUND, "address not found"),
        Ok(_) => {
            res.status_code(StatusCode::NO_CONTENT);
        }
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "address delete failed",
        ),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(
            Router::with_path("me/profile")
                .get(get_profile)
                .patch(patch_profile),
        )
        .push(Router::with_path("me/phone").put(put_phone))
        .push(Router::with_path("me/email").put(put_email))
        .push(
            Router::with_path("me/addresses")
                .get(list_addresses)
                .post(create_address),
        )
        .push(
            Router::with_path("me/addresses/{id}")
                .put(update_address)
                .delete(delete_address),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_contacts_and_avatar_references() {
        assert!(valid_phone("13800138000"));
        assert!(!valid_phone("1380013800x"));
        assert!(crate::contact_delivery::normalize("email", "demo@example.com").is_some());
        assert!(crate::contact_delivery::normalize("email", "bad@").is_none());
        assert!(valid_avatar("https://cdn.example.com/avatar.png"));
        assert!(!valid_avatar("http://local/avatar.png"));
    }
}
