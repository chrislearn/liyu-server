//! Server-priced test checkout. Cart and order views are buyer-only.
use crate::{error, pool, user_id};
use diesel::{prelude::*, sql_query, sql_types::{BigInt, Integer, Nullable, Text}};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;

#[derive(QueryableByName)]
struct IdRow { #[diesel(sql_type = BigInt)] id: i64 }

#[derive(QueryableByName)]
struct CartRow {
    #[diesel(sql_type = BigInt)] id: i64,
    #[diesel(sql_type = Integer)] product_id: i32,
    #[diesel(sql_type = BigInt)] recipient_id: i64,
    #[diesel(sql_type = Text)] name: String,
    #[diesel(sql_type = BigInt)] price_cents: i64,
}

#[derive(QueryableByName)]
struct OrderRow {
    #[diesel(sql_type = BigInt)] id: i64,
    #[diesel(sql_type = BigInt)] total_cents: i64,
    #[diesel(sql_type = Text)] status: String,
}

#[derive(QueryableByName)]
struct OrderItemRow {
    #[diesel(sql_type = BigInt)] id: i64,
    #[diesel(sql_type = Integer)] product_id: i32,
    #[diesel(sql_type = BigInt)] recipient_id: i64,
    #[diesel(sql_type = BigInt)] price_cents: i64,
    #[diesel(sql_type = Nullable<BigInt>)] gift_id: Option<i64>,
}

#[derive(Deserialize)]
struct CartInput { product_id: i32, recipient_id: i64 }

fn cart(conn: &mut PgConnection, uid: i64) -> QueryResult<Vec<CartRow>> {
    sql_query("SELECT ci.id, ci.product_id, ci.recipient_id, c.name, c.price_cents FROM cart_items ci JOIN catalog c ON c.id = ci.product_id WHERE ci.user_id = $1 ORDER BY ci.id")
        .bind::<BigInt, _>(uid).load(conn)
}

fn cart_json(rows: &[CartRow]) -> serde_json::Value {
    json!({"items": rows.iter().map(|r| json!({
        "id":r.id, "product_id":r.product_id, "recipient_id":r.recipient_id,
        "name":r.name, "price_cents":r.price_cents,
        "image_url":format!("/api/v1/media/products/{}/thumb",r.product_id)
    })).collect::<Vec<_>>(),
    "total_cents": rows.iter().map(|r| r.price_cents).sum::<i64>()})
}

#[handler]
async fn get_cart(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    match cart(&mut conn, uid) {
        Ok(rows) => res.render(Json(cart_json(&rows))),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "cart query failed"),
    }
}

#[handler]
async fn add_item(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let input: CartInput = match req.parse_json().await {
        Ok(x) => x,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if input.recipient_id == uid { return error(res, StatusCode::BAD_REQUEST, "cannot send to yourself") }
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let recipient: QueryResult<IdRow> = sql_query("SELECT id FROM users WHERE id = $1")
        .bind::<BigInt, _>(input.recipient_id).get_result(&mut conn);
    if recipient.is_err() { return error(res, StatusCode::NOT_FOUND, "recipient not found") }
    let friend: QueryResult<IdRow> = sql_query("SELECT id FROM friendships WHERE user_low_id=LEAST($1,$2) AND user_high_id=GREATEST($1,$2) AND status='accepted'")
        .bind::<BigInt, _>(uid).bind::<BigInt, _>(input.recipient_id).get_result(&mut conn);
    if friend.is_err() { return error(res, StatusCode::FORBIDDEN, "recipient is not a confirmed friend") }
    let inserted: QueryResult<IdRow> = sql_query("INSERT INTO cart_items (user_id,product_id,recipient_id) VALUES ($1,$2,$3) RETURNING id")
        .bind::<BigInt, _>(uid).bind::<Integer, _>(input.product_id)
        .bind::<BigInt, _>(input.recipient_id).get_result(&mut conn);
    match inserted {
        Ok(row) => res.render(Json(json!({"id":row.id}))),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "invalid product"),
    }
}

#[handler]
async fn delete_item(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Some(item_id) = req.param::<i64>("id") else { return error(res, StatusCode::BAD_REQUEST, "invalid item id") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let removed = sql_query("DELETE FROM cart_items WHERE id = $1 AND user_id = $2")
        .bind::<BigInt, _>(item_id).bind::<BigInt, _>(uid).execute(&mut conn);
    match removed {
        Ok(1) => res.render(Json(json!({"deleted":true}))),
        Ok(_) => error(res, StatusCode::NOT_FOUND, "item not found"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "cart delete failed"),
    }
}

#[handler]
async fn clear_cart(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    match sql_query("DELETE FROM cart_items WHERE user_id = $1").bind::<BigInt, _>(uid).execute(&mut conn) {
        Ok(_) => res.render(Json(json!({"items":[]}))),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "cart clear failed"),
    }
}

#[handler]
async fn quote(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    match cart(&mut conn, uid) {
        Ok(rows) if !rows.is_empty() => res.render(Json(cart_json(&rows))),
        Ok(_) => error(res, StatusCode::BAD_REQUEST, "cart is empty"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "quote failed"),
    }
}

#[handler]
async fn create_order(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let key = req.headers().get("Idempotency-Key").and_then(|v| v.to_str().ok()).unwrap_or("").trim();
    if key.is_empty() || key.len() > 100 { return error(res, StatusCode::BAD_REQUEST, "Idempotency-Key required") }
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let result = conn.transaction::<OrderRow, diesel::result::Error, _>(|conn| {
        let prior: QueryResult<OrderRow> = sql_query("SELECT id,total_cents,status FROM orders WHERE buyer_id=$1 AND idempotency_key=$2")
            .bind::<BigInt,_>(uid).bind::<Text,_>(key).get_result(conn);
        if let Ok(order) = prior { return Ok(order); }
        let rows = cart(conn, uid)?;
        if rows.is_empty() { return Err(diesel::result::Error::NotFound); }
        let total = rows.iter().try_fold(0_i64, |sum, row| sum.checked_add(row.price_cents))
            .ok_or(diesel::result::Error::RollbackTransaction)?;
        let order: OrderRow = sql_query("INSERT INTO orders (buyer_id,total_cents,idempotency_key) VALUES ($1,$2,$3) RETURNING id,total_cents,status")
            .bind::<BigInt,_>(uid).bind::<BigInt,_>(total).bind::<Text,_>(key).get_result(conn)?;
        for row in rows {
            sql_query("INSERT INTO order_items (order_id,product_id,recipient_id,price_cents) VALUES ($1,$2,$3,$4)")
                .bind::<BigInt,_>(order.id).bind::<Integer,_>(row.product_id)
                .bind::<BigInt,_>(row.recipient_id).bind::<BigInt,_>(row.price_cents).execute(conn)?;
        }
        sql_query("DELETE FROM cart_items WHERE user_id=$1").bind::<BigInt,_>(uid).execute(conn)?;
        Ok(order)
    });
    match result {
        Ok(order) => res.render(Json(json!({"id":order.id,"total_cents":order.total_cents,"status":order.status}))),
        Err(diesel::result::Error::NotFound) => error(res, StatusCode::BAD_REQUEST, "cart is empty"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "order create failed"),
    }
}

#[handler]
async fn list_orders(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let rows: QueryResult<Vec<OrderRow>> = sql_query("SELECT id,total_cents,status FROM orders WHERE buyer_id=$1 ORDER BY id DESC LIMIT 50")
        .bind::<BigInt,_>(uid).load(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(rows.into_iter().map(|r| json!({"id":r.id,"total_cents":r.total_cents,"status":r.status})).collect::<Vec<_>>())),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "orders query failed"),
    }
}

#[handler]
async fn get_order(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Some(oid) = req.param::<i64>("id") else { return error(res, StatusCode::BAD_REQUEST, "invalid order id") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let order: QueryResult<OrderRow> = sql_query("SELECT id,total_cents,status FROM orders WHERE id=$1 AND buyer_id=$2")
        .bind::<BigInt,_>(oid).bind::<BigInt,_>(uid).get_result(&mut conn);
    let Ok(order) = order else { return error(res, StatusCode::NOT_FOUND, "order not found") };
    let items: QueryResult<Vec<OrderItemRow>> = sql_query("SELECT id,product_id,recipient_id,price_cents,gift_id FROM order_items WHERE order_id=$1 ORDER BY id")
        .bind::<BigInt,_>(oid).load(&mut conn);
    match items {
        Ok(items) => res.render(Json(json!({"id":order.id,"total_cents":order.total_cents,"status":order.status,
            "items":items.into_iter().map(|i| json!({"id":i.id,"product_id":i.product_id,"recipient_id":i.recipient_id,"price_cents":i.price_cents,"gift_id":i.gift_id})).collect::<Vec<_>>() }))),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "order items query failed"),
    }
}

#[handler]
async fn pay_test(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else { return error(res, StatusCode::UNAUTHORIZED, "invalid session") };
    let Some(oid) = req.param::<i64>("id") else { return error(res, StatusCode::BAD_REQUEST, "invalid order id") };
    let Ok(mut conn) = pool().get() else { return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let result = conn.transaction::<OrderRow, diesel::result::Error, _>(|conn| {
        let order: OrderRow = sql_query("SELECT id,total_cents,status FROM orders WHERE id=$1 AND buyer_id=$2 FOR UPDATE")
            .bind::<BigInt,_>(oid).bind::<BigInt,_>(uid).get_result(conn)?;
        if order.status == "paid_test" { return Ok(order); }
        if order.status != "pending" { return Err(diesel::result::Error::RollbackTransaction); }
        let items: Vec<OrderItemRow> = sql_query("SELECT id,product_id,recipient_id,price_cents,gift_id FROM order_items WHERE order_id=$1")
            .bind::<BigInt,_>(oid).load(conn)?;
        for item in items {
            let gift: IdRow = sql_query("INSERT INTO gifts (sender_id,recipient_id,product_id,state) VALUES ($1,$2,$3,'sealed') RETURNING id")
                .bind::<BigInt,_>(uid).bind::<BigInt,_>(item.recipient_id)
                .bind::<Integer,_>(item.product_id).get_result(conn)?;
            sql_query("UPDATE order_items SET gift_id=$1 WHERE id=$2")
                .bind::<BigInt,_>(gift.id).bind::<BigInt,_>(item.id).execute(conn)?;
        }
        sql_query("UPDATE orders SET status='paid_test',paid_at=now() WHERE id=$1")
            .bind::<BigInt,_>(oid).execute(conn)?;
        Ok(OrderRow { status:"paid_test".into(), ..order })
    });
    match result {
        Ok(order) => res.render(Json(json!({"id":order.id,"status":order.status,"total_cents":order.total_cents}))),
        Err(diesel::result::Error::NotFound) => error(res, StatusCode::NOT_FOUND, "order not found"),
        Err(_) => error(res, StatusCode::CONFLICT, "order cannot be paid"),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/cart").get(get_cart).delete(clear_cart))
        .push(Router::with_path("api/v1/cart/items").post(add_item))
        .push(Router::with_path("api/v1/cart/items/{id}").delete(delete_item))
        .push(Router::with_path("api/v1/orders/quote").post(quote))
        .push(Router::with_path("api/v1/orders").get(list_orders).post(create_order))
        .push(Router::with_path("api/v1/orders/{id}").get(get_order))
        .push(Router::with_path("api/v1/orders/{id}/pay-test").post(pay_test))
}
