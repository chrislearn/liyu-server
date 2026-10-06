//! Server-priced direct gift checkout. Order views are buyer-only.
use crate::{error, pool, user_id};
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{BigInt, Bool, Integer, Jsonb, Nullable, Text},
};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

#[derive(QueryableByName)]
struct ProductRow {
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
}

struct Prepared {
    product_id: i32,
    recipient_id: Option<i64>,
    recipient_contact: Option<serde_json::Value>,
    wish_item_id: Option<i64>,
    price_cents: i64,
    binding: Option<crate::contact_delivery::Binding>,
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
    #[diesel(sql_type = Nullable<BigInt>)]
    recipient_bound_user_id: Option<i64>,
    #[diesel(sql_type = Bool)]
    recipient_change_confirmed: bool,
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    gift_id: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    wish_item_id: Option<i64>,
}

#[derive(Deserialize)]
struct CheckoutInput {
    product_id: i32,
    expires_hours: Option<i32>,
    recipient_id: Option<i64>,
    recipient: Option<crate::contact_delivery::Recipient>,
    wish_item_id: Option<i64>,
    #[serde(default)]
    confirm_recipient_change: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CheckoutRequest {
    Batch {
        items: Vec<CheckoutInput>,
        expires_hours: Option<i32>,
    },
    Single(CheckoutInput),
}

impl CheckoutRequest {
    fn parts(self) -> Option<(Vec<CheckoutInput>, i32)> {
        let (items, expires_hours) = match self {
            Self::Batch {
                items,
                expires_hours,
            } => (items, expires_hours.unwrap_or(24)),
            Self::Single(item) => {
                let expires_hours = item.expires_hours.unwrap_or(24);
                (vec![item], expires_hours)
            }
        };
        if items.is_empty() || items.len() > 100 || !(1..=720).contains(&expires_hours) {
            return None;
        }
        let mut recipients = HashSet::new();
        let mut wish_items = HashSet::new();
        for item in &items {
            if let Some(id) = item.recipient_id {
                if !recipients.insert(id) {
                    return None;
                }
            }
            if let Some(id) = item.wish_item_id {
                if !wish_items.insert(id) {
                    return None;
                }
            }
        }
        Some((items, expires_hours))
    }
}

#[derive(QueryableByName)]
struct ExpiryHoursRow {
    #[diesel(sql_type = Integer)]
    gift_expires_hours: i32,
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
        (w.published_at IS NOT NULL AND w.closed_at IS NULL AND w.expires_at>now() AND w.event_on>=CURRENT_DATE-7) AS is_open, \
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

fn prepare(
    conn: &mut PgConnection,
    uid: i64,
    input: &CheckoutInput,
) -> QueryResult<Option<Prepared>> {
    let product = sql_query(
        "SELECT price_cents FROM catalog WHERE id=$1 AND is_active AND stock>0 FOR SHARE",
    )
    .bind::<Integer, _>(input.product_id)
    .get_result::<ProductRow>(conn)
    .optional()?;
    let Some(product) = product else {
        return Ok(None);
    };
    if input.recipient.is_some() == input.recipient_id.is_some() {
        return Ok(None);
    };
    let (recipient_id, contact, binding) = if let Some(r) = &input.recipient {
        let Some(value) = crate::contact_delivery::normalize(&r.kind, &r.value) else {
            return Ok(None);
        };
        if r.label.chars().count() > 100 {
            return Ok(None);
        }
        let contact = json!({"kind":r.kind,"value":value,"label":r.label.trim()});
        let binding = crate::contact_delivery::binding(conn, uid, &contact)?;
        (binding.current, Some(contact), Some(binding))
    } else {
        let id = input.recipient_id.unwrap();
        let found = sql_query("SELECT id FROM users WHERE id=$1 AND is_active")
            .bind::<BigInt, _>(id)
            .get_result::<IdRow>(conn)
            .optional()?;
        if found.is_none() {
            return Ok(None);
        }
        (Some(id), None, None)
    };
    if recipient_id == Some(uid) {
        return Ok(None);
    }
    if let Some(wish) = input.wish_item_id {
        if !valid_wish_claim(
            conn,
            uid,
            recipient_id.unwrap_or(0),
            input.product_id,
            wish,
            false,
        )? {
            return Ok(None);
        }
    }
    Ok(Some(Prepared {
        product_id: input.product_id,
        recipient_id,
        recipient_contact: contact,
        wish_item_id: input.wish_item_id,
        price_cents: product.price_cents,
        binding,
    }))
}

#[handler]
async fn quote(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(request) = req.parse_json::<CheckoutRequest>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid checkout");
    };
    let Some((inputs, expires_hours)) = request.parts() else {
        return error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid checkout items or expiry",
        );
    };
    let coupon = match crate::benefits::coupon_id(req) {
        Ok(v) => v,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid coupon id"),
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn
        .transaction::<Option<(i64, i64, Option<serde_json::Value>)>, diesel::result::Error, _>(
            |conn| {
                let mut prepared = Vec::with_capacity(inputs.len());
                for input in &inputs {
                    let Some(item) = prepare(conn, uid, input)? else {
                        return Ok(None);
                    };
                    prepared.push(item);
                }
                let Some(subtotal) = prepared
                    .iter()
                    .try_fold(0_i64, |sum, item| sum.checked_add(item.price_cents))
                else {
                    return Ok(None);
                };
                let prices: Vec<_> = prepared
                    .iter()
                    .map(|item| (item.product_id, item.price_cents))
                    .collect();
                let (discount, _) = crate::benefits::discount(conn, uid, coupon, &prices)?;
                let warning = prepared
                    .iter()
                    .filter_map(|item| item.binding.as_ref().filter(|b| b.changed()))
                    .map(|binding| binding.warning())
                    .next();
                Ok(Some((subtotal, discount, warning)))
            },
        );
    match result {
        Ok(Some((subtotal,discount,warning)))=>res.render(Json(json!({"product_id":if inputs.len()==1 {Some(inputs[0].product_id)} else {None},"item_count":inputs.len(),"subtotal_cents":subtotal,"discount_cents":discount,"total_cents":subtotal-discount,"expires_hours":expires_hours,"recipient_warning":warning}))),
        Ok(None)=>error(res,StatusCode::CONFLICT,"product, recipient or wish unavailable"),
        Err(_)=>error(res,StatusCode::CONFLICT,"coupon unavailable"),
    }
}

#[handler]
async fn create_order(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(request) = req.parse_json::<CheckoutRequest>().await else {
        return error(res, StatusCode::BAD_REQUEST, "invalid checkout");
    };
    let Some((inputs, expires_hours)) = request.parts() else {
        return error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid checkout items or expiry",
        );
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
    let result=conn.transaction::<Result<Option<OrderRow>,serde_json::Value>,diesel::result::Error,_>(|conn|{
        sql_query("SELECT pg_advisory_xact_lock($1)").bind::<BigInt,_>(uid).execute(conn)?;
        if let Some(prior)=sql_query("SELECT id,total_cents,status FROM orders WHERE buyer_id=$1 AND idempotency_key=$2")
            .bind::<BigInt,_>(uid).bind::<Text,_>(key).get_result::<OrderRow>(conn).optional()? {return Ok(Ok(Some(prior)))}
        let mut prepared=Vec::with_capacity(inputs.len());
        for input in &inputs {
            let Some(item)=prepare(conn,uid,input)? else {return Ok(Ok(None))};
            if let Some(binding)=&item.binding {
                if binding.changed() && !input.confirm_recipient_change { return Ok(Err(binding.warning())); }
                if !binding.changed() {
                    crate::contact_delivery::save_binding(conn,uid,item.recipient_contact.as_ref().unwrap(),binding,false)?;
                }
            }
            prepared.push(item);
        }
        let Some(subtotal)=prepared.iter().try_fold(0_i64,|sum,item|sum.checked_add(item.price_cents)) else {return Ok(Ok(None))};
        let prices:Vec<_>=prepared.iter().map(|item|(item.product_id,item.price_cents)).collect();
        let (discount,allocated)=crate::benefits::discount(conn,uid,coupon,&prices)?;
        let order=sql_query("INSERT INTO orders (buyer_id,total_cents,idempotency_key,subtotal_cents,discount_cents,coupon_id,gift_expires_hours) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id,total_cents,status")
            .bind::<BigInt,_>(uid).bind::<BigInt,_>(subtotal-discount).bind::<Text,_>(key).bind::<BigInt,_>(subtotal).bind::<BigInt,_>(discount).bind::<Nullable<BigInt>,_>(coupon).bind::<Integer,_>(expires_hours).get_result::<OrderRow>(conn)?;
        crate::benefits::reserve(conn,uid,coupon,order.id)?;
        for (index,item) in prepared.iter().enumerate() {
            sql_query("INSERT INTO order_items (order_id,product_id,recipient_id,price_cents,wish_item_id,recipient_contact,recipient_bound_user_id,recipient_change_confirmed) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind::<BigInt,_>(order.id).bind::<Integer,_>(item.product_id).bind::<Nullable<BigInt>,_>(item.recipient_id).bind::<BigInt,_>(item.price_cents-allocated[index]).bind::<Nullable<BigInt>,_>(item.wish_item_id).bind::<Nullable<Jsonb>,_>(&item.recipient_contact).bind::<Nullable<BigInt>,_>(item.binding.as_ref().and_then(|b|b.current)).bind::<Bool,_>(item.binding.as_ref().is_some_and(|b|b.changed() && inputs[index].confirm_recipient_change)).execute(conn)?;
        }
        Ok(Ok(Some(order)))
    });
    match result {
        Ok(Ok(Some(order))) => res.render(Json(
            json!({"id":order.id,"total_cents":order.total_cents,"status":order.status}),
        )),
        Ok(Err(warning)) => {
            res.status_code(StatusCode::CONFLICT);
            res.render(Json(warning));
        }
        Ok(Ok(None)) => error(
            res,
            StatusCode::CONFLICT,
            "product, recipient or wish unavailable",
        ),
        Err(_) => error(res, StatusCode::CONFLICT, "coupon or order unavailable"),
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
    let items: QueryResult<Vec<OrderItemRow>> = sql_query("SELECT id,product_id,recipient_id,recipient_contact,recipient_bound_user_id,recipient_change_confirmed,price_cents,gift_id,wish_item_id FROM order_items WHERE order_id=$1 ORDER BY id")
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
    let confirmed = req
        .headers()
        .get("X-Confirm-Recipient-Change")
        .and_then(|v| v.to_str().ok())
        == Some("true");
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result = conn.transaction::<Result<OrderRow,serde_json::Value>, diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock($1)").bind::<BigInt,_>(uid).execute(conn)?;
        let order: OrderRow = sql_query("SELECT id,total_cents,status FROM orders WHERE id=$1 AND buyer_id=$2 FOR UPDATE")
            .bind::<BigInt,_>(oid).bind::<BigInt,_>(uid).get_result(conn)?;
        if order.status == "paid_test" { return Ok(Ok(order)); }
        if order.status != "pending" { return Err(diesel::result::Error::RollbackTransaction); }
        let expiry: ExpiryHoursRow = sql_query("SELECT gift_expires_hours FROM orders WHERE id=$1")
            .bind::<BigInt,_>(oid).get_result(conn)?;
        let items: Vec<OrderItemRow> = sql_query("SELECT id,product_id,recipient_id,recipient_contact,recipient_bound_user_id,recipient_change_confirmed,price_cents,gift_id,wish_item_id FROM order_items WHERE order_id=$1 ORDER BY id")
            .bind::<BigInt,_>(oid).load(conn)?;
        for item in &items {
            if let Some(contact)=&item.recipient_contact {
                let binding=crate::contact_delivery::binding(conn,uid,contact)?;
                let snapshot_changed=item.recipient_bound_user_id.is_some_and(|old|Some(old)!=binding.current);
                let prior_confirmation=item.recipient_change_confirmed && item.recipient_bound_user_id==binding.current;
                if (binding.changed() || snapshot_changed) && !confirmed && !prior_confirmation {
                    return Ok(Err(binding.warning()));
                }
                crate::contact_delivery::save_binding(conn,uid,contact,&binding,confirmed || prior_confirmation)?;
            }
        }
        crate::benefits::redeem(conn,oid)?;
        sql_query("SELECT set_config('liyu.reason','test payment',true)").execute(conn)?;
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
            let gift: IdRow = sql_query("INSERT INTO gifts (sender_id,recipient_id,product_id,price_cents,state,recipient_contact,expires_at) VALUES ($1,$2,$3,$4,'sealed',$5,now()+$6*interval '1 hour') RETURNING id")
                .bind::<BigInt,_>(uid).bind::<Nullable<BigInt>,_>(recipient_id)
                .bind::<Integer,_>(item.product_id).bind::<BigInt,_>(item.price_cents).bind::<Nullable<Jsonb>,_>(&item.recipient_contact).bind::<Integer,_>(expiry.gift_expires_hours).get_result(conn)?;
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
        Ok(Ok(OrderRow { status:"paid_test".into(), ..order }))
    });
    match result {
        Ok(Ok(order)) => res.render(Json(
            json!({"id":order.id,"status":order.status,"total_cents":order.total_cents}),
        )),
        Ok(Err(warning)) => {
            res.status_code(StatusCode::CONFLICT);
            res.render(Json(warning));
        }
        Err(diesel::result::Error::NotFound) => {
            error(res, StatusCode::NOT_FOUND, "order not found")
        }
        Err(_) => error(res, StatusCode::CONFLICT, "order cannot be paid"),
    }
}

pub fn routes() -> Router {
    Router::new()
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
    fn checkout_accepts_distinct_batch_recipients_and_legacy_single_item() {
        let batch: CheckoutRequest = serde_json::from_value(json!({"items":[
            {"product_id":1,"recipient_id":10},
            {"product_id":1,"recipient_id":11}
        ]}))
        .unwrap();
        assert_eq!(batch.parts().unwrap().0.len(), 2);
        let single: CheckoutRequest = serde_json::from_value(json!({
            "product_id":1,"recipient_id":10
        }))
        .unwrap();
        assert_eq!(single.parts().unwrap().1, 24);
        let custom_single: CheckoutRequest = serde_json::from_value(json!({
            "product_id":1,"recipient_id":10,"expires_hours":48
        }))
        .unwrap();
        assert_eq!(custom_single.parts().unwrap().1, 48);
        let duplicate: CheckoutRequest = serde_json::from_value(json!({"items":[
            {"product_id":1,"recipient_id":10},
            {"product_id":1,"recipient_id":10}
        ]}))
        .unwrap();
        assert!(duplicate.parts().is_none());
        let empty: CheckoutRequest = serde_json::from_value(json!({"items":[]})).unwrap();
        assert!(empty.parts().is_none());
        for hours in [0, 721] {
            let request: CheckoutRequest = serde_json::from_value(
                json!({"items":[{"product_id":1,"recipient_id":10}],"expires_hours":hours}),
            )
            .unwrap();
            assert!(request.parts().is_none());
        }
        let custom: CheckoutRequest = serde_json::from_value(
            json!({"items":[{"product_id":1,"recipient_id":10}],"expires_hours":36}),
        )
        .unwrap();
        assert_eq!(custom.parts().unwrap().1, 36);
    }

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
