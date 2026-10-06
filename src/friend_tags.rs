//! Owner-private, many-to-many labels for confirmed friends.

use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{error, pool, user_id};

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct TagRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
struct MemberRow {
    #[diesel(sql_type = BigInt)]
    tag_id: i64,
    #[diesel(sql_type = BigInt)]
    friend_id: i64,
    #[diesel(sql_type = Text)]
    display_name: String,
}

#[derive(Deserialize)]
struct TagInput {
    name: String,
}

#[derive(Deserialize)]
struct TagIdsInput {
    tag_ids: Vec<i64>,
}

#[derive(Deserialize)]
struct FriendIdsInput {
    friend_ids: Vec<i64>,
}

fn owner(req: &Request, res: &mut Response) -> Option<i64> {
    let uid = user_id(req);
    if uid.is_none() {
        error(res, StatusCode::UNAUTHORIZED, "invalid session");
    }
    uid
}

fn id(req: &Request, res: &mut Response, key: &str) -> Option<i64> {
    let value = req.param::<i64>(key).filter(|value| *value > 0);
    if value.is_none() {
        error(res, StatusCode::BAD_REQUEST, "invalid id");
    }
    value
}

fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 32 || name.chars().any(char::is_control) {
        None
    } else {
        Some(name.to_string())
    }
}

fn owned_tag(conn: &mut PgConnection, uid: i64, tag_id: i64) -> QueryResult<bool> {
    diesel::sql_query("SELECT count(*) AS count FROM friend_tags WHERE id=$1 AND owner_id=$2")
        .bind::<BigInt, _>(tag_id)
        .bind::<BigInt, _>(uid)
        .get_result::<CountRow>(conn)
        .map(|row| row.count == 1)
}

fn confirmed_friend(conn: &mut PgConnection, uid: i64, friend_id: i64) -> QueryResult<bool> {
    diesel::sql_query(
        "SELECT count(*) AS count FROM friendships WHERE status='accepted' \
         AND user_low_id=LEAST($1,$2) AND user_high_id=GREATEST($1,$2)",
    )
    .bind::<BigInt, _>(uid)
    .bind::<BigInt, _>(friend_id)
    .get_result::<CountRow>(conn)
    .map(|row| row.count == 1)
}

#[handler]
async fn list(req: &mut Request, res: &mut Response) {
    let Some(uid) = owner(req, res) else { return };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let tags = diesel::sql_query(
        "SELECT id,name FROM friend_tags WHERE owner_id=$1 ORDER BY lower(name),id",
    )
    .bind::<BigInt, _>(uid)
    .load::<TagRow>(&mut conn);
    let members = diesel::sql_query(
        "SELECT m.tag_id,m.friend_id,COALESCE(NULLIF(d.nickname,''),u.display_name) AS display_name \
         FROM friend_tag_members m JOIN friend_tags t ON t.id=m.tag_id \
         JOIN users u ON u.id=m.friend_id \
         LEFT JOIN friend_details d ON d.owner_id=t.owner_id AND d.friend_id=m.friend_id \
         WHERE t.owner_id=$1 AND EXISTS (SELECT 1 FROM friendships f WHERE f.status='accepted' \
         AND f.user_low_id=LEAST(t.owner_id,m.friend_id) \
         AND f.user_high_id=GREATEST(t.owner_id,m.friend_id)) \
         ORDER BY display_name,m.friend_id",
    )
    .bind::<BigInt, _>(uid)
    .load::<MemberRow>(&mut conn);
    match (tags, members) {
        (Ok(tags), Ok(members)) => {
            let result = tags
                .into_iter()
                .map(|tag| {
                    let belonging: Vec<&MemberRow> = members.iter().filter(|m| m.tag_id == tag.id).collect();
                    json!({"id":tag.id,"name":tag.name,"member_count":belonging.len(),
                        "member_ids":belonging.iter().map(|m| m.friend_id).collect::<Vec<_>>(),
                        "member_names":belonging.iter().take(5).map(|m| m.display_name.as_str()).collect::<Vec<_>>()})
                })
                .collect::<Vec<Value>>();
            res.render(Json(result));
        }
        _ => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "friend tags query failed",
        ),
    }
}

#[handler]
async fn create(req: &mut Request, res: &mut Response) {
    let Some(uid) = owner(req, res) else { return };
    let Ok(input) = req.parse_json::<TagInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid tag");
    };
    let Some(name) = clean_name(&input.name) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid tag name");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row = diesel::sql_query(
        "INSERT INTO friend_tags(owner_id,name) VALUES ($1,$2) RETURNING id,name",
    )
    .bind::<BigInt, _>(uid)
    .bind::<Text, _>(name)
    .get_result::<TagRow>(&mut conn);
    match row {
        Ok(row) => res.render(Json(
            json!({"id":row.id,"name":row.name,"member_count":0,"member_ids":[]}),
        )),
        Err(_) => error(res, StatusCode::CONFLICT, "tag already exists"),
    }
}

#[handler]
async fn rename(req: &mut Request, res: &mut Response) {
    let Some(uid) = owner(req, res) else { return };
    let Some(tag_id) = id(req, res, "id") else {
        return;
    };
    let Ok(input) = req.parse_json::<TagInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid tag");
    };
    let Some(name) = clean_name(&input.name) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid tag name");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row = diesel::sql_query(
        "UPDATE friend_tags SET name=$3 WHERE id=$1 AND owner_id=$2 RETURNING id,name",
    )
    .bind::<BigInt, _>(tag_id)
    .bind::<BigInt, _>(uid)
    .bind::<Text, _>(name)
    .get_result::<TagRow>(&mut conn)
    .optional();
    match row {
        Ok(Some(row)) => res.render(Json(json!({"id":row.id,"name":row.name}))),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "tag not found"),
        Err(_) => error(res, StatusCode::CONFLICT, "tag already exists"),
    }
}

#[handler]
async fn remove(req: &mut Request, res: &mut Response) {
    let Some(uid) = owner(req, res) else { return };
    let Some(tag_id) = id(req, res, "id") else {
        return;
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = diesel::sql_query("DELETE FROM friend_tags WHERE id=$1 AND owner_id=$2")
        .bind::<BigInt, _>(tag_id)
        .bind::<BigInt, _>(uid)
        .execute(&mut conn);
    match result {
        Ok(1) => res.render(Json(json!({"deleted":true}))),
        Ok(_) => error(res, StatusCode::NOT_FOUND, "tag not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "tag deletion failed",
        ),
    }
}

#[handler]
async fn set_friend_tags(req: &mut Request, res: &mut Response) {
    let Some(uid) = owner(req, res) else { return };
    let Some(friend_id) = id(req, res, "friend_id") else {
        return;
    };
    let Ok(input) = req.parse_json::<TagIdsInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid tag selection");
    };
    let ids: BTreeSet<i64> = input.tag_ids.into_iter().collect();
    if ids.len() > 100 || ids.iter().any(|id| *id <= 0) {
        return error(res, StatusCode::BAD_REQUEST, "invalid tag selection");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    if confirmed_friend(&mut conn, uid, friend_id).ok() != Some(true)
        || ids
            .iter()
            .any(|tag_id| owned_tag(&mut conn, uid, *tag_id).ok() != Some(true))
    {
        return error(res, StatusCode::BAD_REQUEST, "friend or tag not found");
    }
    let result = conn.transaction::<_, diesel::result::Error, _>(|conn| {
        diesel::sql_query("DELETE FROM friend_tag_members m USING friend_tags t WHERE m.tag_id=t.id AND t.owner_id=$1 AND m.friend_id=$2")
            .bind::<BigInt, _>(uid)
            .bind::<BigInt, _>(friend_id)
            .execute(conn)?;
        for tag_id in &ids {
            diesel::sql_query("INSERT INTO friend_tag_members(tag_id,friend_id) VALUES ($1,$2)")
                .bind::<BigInt, _>(*tag_id)
                .bind::<BigInt, _>(friend_id)
                .execute(conn)?;
        }
        Ok(())
    });
    match result {
        Ok(()) => res.render(Json(json!({"friend_id":friend_id,"tag_ids":ids}))),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "friend tags update failed",
        ),
    }
}

#[handler]
async fn set_tag_members(req: &mut Request, res: &mut Response) {
    let Some(uid) = owner(req, res) else { return };
    let Some(tag_id) = id(req, res, "id") else {
        return;
    };
    let Ok(input) = req.parse_json::<FriendIdsInput>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid friend selection");
    };
    let ids: BTreeSet<i64> = input.friend_ids.into_iter().collect();
    if ids.len() > 1000 || ids.iter().any(|id| *id <= 0) {
        return error(res, StatusCode::BAD_REQUEST, "invalid friend selection");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    if owned_tag(&mut conn, uid, tag_id).ok() != Some(true)
        || ids
            .iter()
            .any(|friend_id| confirmed_friend(&mut conn, uid, *friend_id).ok() != Some(true))
    {
        return error(res, StatusCode::BAD_REQUEST, "tag or friend not found");
    }
    let result = conn.transaction::<_, diesel::result::Error, _>(|conn| {
        diesel::sql_query("DELETE FROM friend_tag_members WHERE tag_id=$1")
            .bind::<BigInt, _>(tag_id)
            .execute(conn)?;
        for friend_id in &ids {
            diesel::sql_query("INSERT INTO friend_tag_members(tag_id,friend_id) VALUES ($1,$2)")
                .bind::<BigInt, _>(tag_id)
                .bind::<BigInt, _>(*friend_id)
                .execute(conn)?;
        }
        Ok(())
    });
    match result {
        Ok(()) => res.render(Json(json!({"tag_id":tag_id,"friend_ids":ids}))),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "tag members update failed",
        ),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(
            Router::with_path("api/v1/friend-tags")
                .get(list)
                .post(create),
        )
        .push(
            Router::with_path("api/v1/friend-tags/{id}")
                .put(rename)
                .delete(remove),
        )
        .push(Router::with_path("api/v1/friend-tags/{id}/members").put(set_tag_members))
        .push(Router::with_path("api/v1/friends/{friend_id}/tags").put(set_friend_tags))
}
