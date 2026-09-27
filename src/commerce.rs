//! Server-priced test checkout. Cart and order views are buyer-only.
use crate::{error, pool, user_id};
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{BigInt, Bool, Integer, Jsonb, Nullable, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

#[derive(QueryableByName)]
struct CartRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    product_id: i32,
    #[diesel(sql_type = Nullable<BigInt>)]
    recipient_id: Option<i64>,
    #[diesel(sql_type = Nullable<Jsonb>)]
    recipient_contact: Option<serde_json::Value>,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    wish_item_id: Option<i64>,
}

#[derive(QueryableByName)]
struct OrderRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    total_cents: i64,
    #[diesel(sql_type = Text)]
    status: String,
}

#[derive(QueryableByName)]
struct OrderItemRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    product_id: i32,
    #[diesel(sql_type = Nullable<BigInt>)]
    recipient_id: Option<i64>,
    #[diesel(sql_type = Nullable<Jsonb>)]
    recipient_contact: Option<serde_json::Value>,
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    gift_id: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    wish_item_id: Option<i64>,
}

#[derive(Deserialize)]
struct CartInput {
    product_id: i32,
    recipient_id: Option<i64>,
    recipient: Option<crate::contact_delivery::Recipient>,
    wish_item_id: Option<i64>,
}

#[derive(QueryableByName)]
struct WishClaimRow {
    #[diesel(sql_type = BigInt)]
    owner_id: i64,
    #[diesel(sql_type = Nullable<Integer>)]
    exact_product_id: Option<i32>,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    selected_kind: String,
    #[diesel(sql_type = Bool)]
    claimed: bool,
    #[diesel(sql_type = Bool)]
    is_open: bool,
    #[diesel(sql_type = Bool)]
    allowed: bool,
}

fn claim_matches(r: &WishClaimRow, sender: i64, recipient: i64, product: i32) -> bool {
    r.owner_id == recipient
        && r.owner_id != sender
        && r.is_open
        && r.allowed
        && !r.claimed
        && r.exact_product_id
            .map_or(r.kind == r.selected_kind, |exact| exact == product)
}

fn valid_wish_claim(
    conn: &mut PgConnection,
    sender: i64,
    recipient: i64,
    product: i32,
    wish_item: i64,
    lock: bool,
) -> QueryResult<bool> {
    let lock_clause = if lock { " FOR UPDATE OF wi,w" } else { "" };
    let sql = format!("SELECT w.owner_id,wi.product_id AS exact_product_id,wi.kind, \
        c.kind AS selected_kind,(wi.claimed_gift_id IS NOT NULL) AS claimed, \
        (w.closed_at IS NULL AND w.event_on>=CURRENT_DATE-7) AS is_open, \
        (EXISTS (SELECT 1 FROM friendships f WHERE f.status='accepted' \
            AND f.user_low_id=LEAST(w.owner_id,$2) AND f.user_high_id=GREATEST(w.owner_id,$2)) \
         AND (NOT EXISTS (SELECT 1 FROM wishlist_audience a WHERE a.wishlist_id=w.id) \
            OR EXISTS (SELECT 1 FROM wishlist_audience a WHERE a.wishlist_id=w.id AND a.user_id=$2))) AS allowed \
        FROM wishlist_items wi JOIN wishlists w ON w.id=wi.wishlist_id \
        JOIN catalog c ON c.id=$3 WHERE wi.id=$1{lock_clause}");
    let row = sql_query(sql)
        .bind::<BigInt, _>(wish_item)
        .bind::<BigInt, _>(sender)
        .bind::<Integer, _>(product)
        .get_result::<WishClaimRow>(conn)
        .optional()?;
    Ok(row.is_some_and(|r| claim_matches(&r, sender, recipient, product)))
}

fn cart(conn: &mut PgConnection, uid: i64) -> QueryResult<Vec<CartRow>> {
    sql_query("SELECT ci.id, ci.product_id, ci.recipient_id, ci.recipient_contact, c.name, c.price_cents,ci.wish_item_id FROM cart_items ci JOIN catalog c ON c.id = ci.product_id WHERE ci.user_id = $1 ORDER BY ci.id")
        .bind::<BigInt, _>(uid).load(conn)
}

fn cart_json(rows: &[CartRow]) -> serde_json::Value {
    json!({"items": rows.iter().map(|r| json!({
        "id":r.id, "product_id":r.product_id, "recipient_id":if r.recipient_contact.is_some(){None}else{r.recipient_id}, "recipient":r.recipient_contact,
        "wish_item_id":r.wish_item_id,
        "name":r.name, "price_cents":r.price_cents,
        "image_url":format!("/api/v1/media/products/{}/thumb",r.product_id)
    })).collect::<Vec<_>>(),
    "total_cents": rows.iter().map(|r| r.price_cents).sum::<i64>()})
}

#[handler]
async fn get_cart(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match cart(&mut conn, uid) {
        Ok(rows) => res.render(Json(cart_json(&rows))),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "cart query failed"),
    }
}

#[handler]
async fn add_item(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let input: CartInput = match req.parse_json().await {
        Ok(x) => x,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    if input.recipient.is_some() && input.recipient_id.is_some() {
        return error(res, StatusCode::BAD_REQUEST, "choose one recipient input");
    }
    let contact = if let Some(r) = input.recipient {
        let Some(value) = crate::contact_delivery::normalize(&r.kind, &r.value) else {
            return error(res, StatusCode::BAD_REQUEST, "invalid recipient contact");
        };
        if r.label.chars().count() > 100 {
            return error(res, StatusCode::BAD_REQUEST, "recipient label too long");
        }
        Some(json!({"kind":r.kind,"value":value,"label":r.label.trim()}))
    } else {
        None
    };
    // Contact resolution remains internal. Public cart views never disclose matching IDs.
    let recipient_id = if let Some(contact) = &contact {
        match crate::contact_delivery::resolve(&mut conn, contact) {
            Ok(id) => id,
            Err(_) => {
                return error(
                    res,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "recipient lookup failed",
                )
            }
        }
    } else if let Some(id) = input.recipient_id {
        let found = sql_query("SELECT id FROM users WHERE id=$1 AND is_active")
            .bind::<BigInt, _>(id)
            .get_result::<IdRow>(&mut conn);
        if found.is_err() {
            return error(res, StatusCode::NOT_FOUND, "recipient unavailable");
        }
        Some(id)
    } else {
        return error(res, StatusCode::BAD_REQUEST, "recipient contact required");
    };
    if recipient_id == Some(uid) {
        return error(res, StatusCode::BAD_REQUEST, "cannot send to yourself");
    }
    if let Some(wish_item) = input.wish_item_id {
        if wish_item <= 0
            || !matches!(
                valid_wish_claim(
                    &mut conn,
                    uid,
                    recipient_id.unwrap_or(0),
                    input.product_id,
                    wish_item,
                    false
                ),
                Ok(true)
            )
        {
            return error(res, StatusCode::CONFLICT, "wish item unavailable");
        }
    }
    let available =
        sql_query("SELECT id::bigint AS id FROM catalog WHERE id=$1 AND is_active AND stock>0")
            .bind::<Integer, _>(input.product_id)
            .get_result::<IdRow>(&mut conn)
            .optional();
    if !matches!(available, Ok(Some(_))) {
        return error(res, StatusCode::CONFLICT, "product unavailable");
    }
    let inserted: QueryResult<IdRow> = sql_query("INSERT INTO cart_items (user_id,product_id,recipient_id,wish_item_id,recipient_contact) VALUES ($1,$2,$3,$4,$5) RETURNING id")
        .bind::<BigInt, _>(uid).bind::<Integer, _>(input.product_id)
        .bind::<Nullable<BigInt>, _>(recipient_id).bind::<Nullable<BigInt>,_>(input.wish_item_id).bind::<Nullable<Jsonb>,_>(contact).get_result(&mut conn);
    match inserted {
        Ok(row) => res.render(Json(json!({"id":row.id}))),
        Err(_) => error(res, StatusCode::BAD_REQUEST, "invalid product"),
    }
}

#[handler]
async fn delete_item(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Some(item_id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid item id");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let removed = sql_query("DELETE FROM cart_items WHERE id = $1 AND user_id = $2")
        .bind::<BigInt, _>(item_id)
        .bind::<BigInt, _>(uid)
        .execute(&mut conn);
    match removed {
        Ok(1) => res.render(Json(json!({"deleted":true}))),
        Ok(_) => error(res, StatusCode::NOT_FOUND, "item not found"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "cart delete failed"),
    }
}

#[handler]
async fn clear_cart(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match sql_query("DELETE FROM cart_items WHERE user_id = $1")
        .bind::<BigInt, _>(uid)
        .execute(&mut conn)
    {
        Ok(_) => res.render(Json(json!({"items":[]}))),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "cart clear failed"),
    }
}

#[handler]
async fn quote(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let coupon = match crate::benefits::coupon_id(req) {
        Ok(v) => v,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid coupon id"),
    };
    match cart(&mut conn, uid) {
        Ok(rows) if !rows.is_empty() => {
            let items = rows
                .iter()
                .map(|r| (r.product_id, r.price_cents))
                .collect::<Vec<_>>();
            match conn.transaction::<_, diesel::result::Error, _>(|conn| {
                crate::benefits::discount(conn, uid, coupon, &items)
            }) {
                Ok((discount, _)) => {
                    let mut data = cart_json(&rows);
                    data["subtotal_cents"] = data["total_cents"].clone();
                    data["discount_cents"] = json!(discount);
                    data["total_cents"] =
                        json!(items.iter().map(|(_, p)| p).sum::<i64>() - discount);
                    res.render(Json(data));
                }
                Err(_) => error(res, StatusCode::CONFLICT, "coupon unavailable"),
            }
        }
        Ok(_) => error(res, StatusCode::BAD_REQUEST, "cart is empty"),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "quote failed"),
    }
}

#[handler]
async fn create_order(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let coupon = match crate::benefits::coupon_id(req) {
        Ok(v) => v,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid coupon id"),
    };
    let key = req
        .headers()
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    if key.is_empty() || key.len() > 100 {
        return error(res, StatusCode::BAD_REQUEST, "Idempotency-Key required");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<OrderRow, diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock($1)").bind::<BigInt,_>(uid).execute(conn)?;
        let prior: QueryResult<OrderRow> = sql_query("SELECT id,total_cents,status FROM orders WHERE buyer_id=$1 AND idempotency_key=$2")
            .bind::<BigInt,_>(uid).bind::<Text,_>(key).get_result(conn);
        if let Ok(order) = prior { return Ok(order); }
        let rows = cart(conn, uid)?;
        if rows.is_empty() { return Err(diesel::result::Error::NotFound); }
        for row in &rows {
            let available = sql_query("SELECT id::bigint AS id FROM catalog WHERE id=$1 AND is_active AND stock>0")
                .bind::<Integer,_>(row.product_id).get_result::<IdRow>(conn).optional()?;
            if available.is_none() { return Err(diesel::result::Error::RollbackTransaction); }
            if let Some(wish_item) = row.wish_item_id {
                if !valid_wish_claim(conn,uid,row.recipient_id.unwrap_or(0),row.product_id,wish_item,false)? {
                    return Err(diesel::result::Error::RollbackTransaction);
                }
            }
        }
        let total = rows.iter().try_fold(0_i64, |sum, row| sum.checked_add(row.price_cents))
            .ok_or(diesel::result::Error::RollbackTransaction)?;
        let (discount,allocated)=crate::benefits::discount(conn,uid,coupon,&rows.iter().map(|r|(r.product_id,r.price_cents)).collect::<Vec<_>>())?;
        let order: OrderRow = sql_query("INSERT INTO orders (buyer_id,total_cents,idempotency_key,subtotal_cents,discount_cents,coupon_id) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id,total_cents,status")
            .bind::<BigInt,_>(uid).bind::<BigInt,_>(total-discount).bind::<Text,_>(key).bind::<BigInt,_>(total).bind::<BigInt,_>(discount).bind::<Nullable<BigInt>,_>(coupon).get_result(conn)?;
        crate::benefits::reserve(conn,uid,coupon,order.id)?;
        for (index,row) in rows.into_iter().enumerate() {
            sql_query("INSERT INTO order_items (order_id,product_id,recipient_id,price_cents,wish_item_id,recipient_contact) VALUES ($1,$2,$3,$4,$5,$6)")
                .bind::<BigInt,_>(order.id).bind::<Integer,_>(row.product_id)
                .bind::<Nullable<BigInt>,_>(row.recipient_id).bind::<BigInt,_>(row.price_cents-allocated[index])
                .bind::<Nullable<BigInt>,_>(row.wish_item_id).bind::<Nullable<Jsonb>,_>(row.recipient_contact).execute(conn)?;
        }
        sql_query("DELETE FROM cart_items WHERE user_id=$1").bind::<BigInt,_>(uid).execute(conn)?;
        Ok(order)
    });
    match result {
        Ok(order) => res.render(Json(
            json!({"id":order.id,"total_cents":order.total_cents,"status":order.status}),
        )),
        Err(diesel::result::Error::NotFound) => {
            error(res, StatusCode::BAD_REQUEST, "cart is empty")
        }
        Err(diesel::result::Error::RollbackTransaction) => {
            error(res, StatusCode::CONFLICT, "product or wish unavailable")
        }
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "order create failed",
        ),
    }
}

#[handler]
async fn list_orders(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let rows: QueryResult<Vec<OrderRow>> = sql_query(
        "SELECT id,total_cents,status FROM orders WHERE buyer_id=$1 ORDER BY id DESC LIMIT 50",
    )
    .bind::<BigInt, _>(uid)
    .load(&mut conn);
    match rows {
        Ok(rows) => res.render(Json(
            rows.into_iter()
                .map(|r| json!({"id":r.id,"total_cents":r.total_cents,"status":r.status}))
                .collect::<Vec<_>>(),
        )),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "orders query failed",
        ),
    }
}

#[handler]
async fn get_order(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Some(oid) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid order id");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let order: QueryResult<OrderRow> =
        sql_query("SELECT id,total_cents,status FROM orders WHERE id=$1 AND buyer_id=$2")
            .bind::<BigInt, _>(oid)
            .bind::<BigInt, _>(uid)
            .get_result(&mut conn);
    let Ok(order) = order else {
        return error(res, StatusCode::NOT_FOUND, "order not found");
    };
    let items: QueryResult<Vec<OrderItemRow>> = sql_query("SELECT id,product_id,recipient_id,recipient_contact,price_cents,gift_id,wish_item_id FROM order_items WHERE order_id=$1 ORDER BY id")
        .bind::<BigInt,_>(oid).load(&mut conn);
    match items {
        Ok(items) => res.render(Json(json!({"id":order.id,"total_cents":order.total_cents,"status":order.status,
            "items":items.into_iter().map(|i| json!({"id":i.id,"product_id":i.product_id,"recipient_id":if i.recipient_contact.is_some(){None}else{i.recipient_id},"recipient":i.recipient_contact,"price_cents":i.price_cents,"gift_id":i.gift_id,"wish_item_id":i.wish_item_id})).collect::<Vec<_>>() }))),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "order items query failed"),
    }
}

#[handler]
async fn pay_test(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Some(oid) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid order id");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<OrderRow, diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock($1)").bind::<BigInt,_>(uid).execute(conn)?;
        let order: OrderRow = sql_query("SELECT id,total_cents,status FROM orders WHERE id=$1 AND buyer_id=$2 FOR UPDATE")
            .bind::<BigInt,_>(oid).bind::<BigInt,_>(uid).get_result(conn)?;
        if order.status == "paid_test" { return Ok(order); }
        if order.status != "pending" { return Err(diesel::result::Error::RollbackTransaction); }
        crate::benefits::redeem(conn,oid)?;
        sql_query("SELECT set_config('liyu.reason','test payment',true)").execute(conn)?;
        let items: Vec<OrderItemRow> = sql_query("SELECT id,product_id,recipient_id,recipient_contact,price_cents,gift_id,wish_item_id FROM order_items WHERE order_id=$1 ORDER BY id")
            .bind::<BigInt,_>(oid).load(conn)?;
        // Lock/update products in stable ID order; repeated items reserve their total quantity.
        let mut quantities = std::collections::BTreeMap::<i32, i32>::new();
        for item in &items { *quantities.entry(item.product_id).or_default() += 1; }
        for (product, quantity) in quantities {
            let updated = sql_query("UPDATE catalog SET stock=stock-$2 WHERE id=$1 AND is_active AND stock>=$2")
                .bind::<Integer,_>(product).bind::<Integer,_>(quantity).execute(conn)?;
            if updated != 1 { return Err(diesel::result::Error::RollbackTransaction); }
        }
        for item in items {
            let recipient_id=if let Some(contact)=&item.recipient_contact {
                crate::contact_delivery::resolve(conn,contact)?
            } else { item.recipient_id };
            if recipient_id==Some(uid) { return Err(diesel::result::Error::RollbackTransaction); }
            if let Some(wish_item) = item.wish_item_id {
                if !valid_wish_claim(conn,uid,item.recipient_id.unwrap_or(0),item.product_id,wish_item,true)? {
                    return Err(diesel::result::Error::RollbackTransaction);
                }
            }
            let gift: IdRow = sql_query("INSERT INTO gifts (sender_id,recipient_id,product_id,price_cents,state,recipient_contact) VALUES ($1,$2,$3,$4,'sealed',$5) RETURNING id")
                .bind::<BigInt,_>(uid).bind::<Nullable<BigInt>,_>(recipient_id)
                .bind::<Integer,_>(item.product_id).bind::<BigInt,_>(item.price_cents).bind::<Nullable<Jsonb>,_>(&item.recipient_contact).get_result(conn)?;
            crate::contact_delivery::gift_delivery(conn,gift.id,recipient_id,item.recipient_contact.as_ref())?;
            sql_query("UPDATE order_items SET gift_id=$1 WHERE id=$2")
                .bind::<BigInt,_>(gift.id).bind::<BigInt,_>(item.id).execute(conn)?;
            if let Some(wish_item) = item.wish_item_id {
                let claimed = sql_query("UPDATE wishlist_items SET claimed_gift_id=$2,claimed_by_user_id=$3,claimed_at=now() WHERE id=$1 AND claimed_gift_id IS NULL")
                    .bind::<BigInt,_>(wish_item).bind::<BigInt,_>(gift.id)
                    .bind::<BigInt,_>(uid).execute(conn)?;
                if claimed != 1 { return Err(diesel::result::Error::RollbackTransaction); }
            }
        }
        sql_query("UPDATE orders SET status='paid_test',paid_at=now() WHERE id=$1")
            .bind::<BigInt,_>(oid).execute(conn)?;
        Ok(OrderRow { status:"paid_test".into(), ..order })
    });
    match result {
        Ok(order) => res.render(Json(
            json!({"id":order.id,"status":order.status,"total_cents":order.total_cents}),
        )),
        Err(diesel::result::Error::NotFound) => {
            error(res, StatusCode::NOT_FOUND, "order not found")
        }
        Err(_) => error(res, StatusCode::CONFLICT, "order cannot be paid"),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(
            Router::with_path("api/v1/cart")
                .get(get_cart)
                .delete(clear_cart),
        )
        .push(Router::with_path("api/v1/cart/items").post(add_item))
        .push(Router::with_path("api/v1/cart/items/{id}").delete(delete_item))
        .push(Router::with_path("api/v1/orders/quote").post(quote))
        .push(
            Router::with_path("api/v1/orders")
                .get(list_orders)
                .post(create_order),
        )
        .push(Router::with_path("api/v1/orders/{id}").get(get_order))
        .push(Router::with_path("api/v1/orders/{id}/pay-test").post(pay_test))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wish_claim_requires_owner_friend_visibility_and_matching_product() {
        let mut row = WishClaimRow {
            owner_id: 2,
            exact_product_id: Some(17),
            kind: "耳机".into(),
            selected_kind: "耳机".into(),
            claimed: false,
            is_open: true,
            allowed: true,
        };
        assert!(claim_matches(&row, 1, 2, 17));
        assert!(!claim_matches(&row, 1, 3, 17));
        assert!(!claim_matches(&row, 2, 2, 17));
        assert!(!claim_matches(&row, 1, 2, 16));
        row.claimed = true;
        assert!(!claim_matches(&row, 1, 2, 17));
        row.claimed = false;
        row.allowed = false;
        assert!(!claim_matches(&row, 1, 2, 17));
        row.allowed = true;
        row.exact_product_id = None;
        assert!(claim_matches(&row, 1, 2, 17));
        row.selected_kind = "电视".into();
        assert!(!claim_matches(&row, 1, 2, 17));
    }
}
