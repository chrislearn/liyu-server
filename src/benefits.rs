//! Server-owned discounts and gift settlement. Amounts are integer cents.
use crate::{error, management::JsonRow, pool, user_id};
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{BigInt, Integer, Jsonb, Text},
};
use salvo::prelude::*;
use serde_json::{json, Value};

#[derive(QueryableByName)]
struct Coupon {
    #[diesel(sql_type=Jsonb)]
    data: Value,
}

pub(crate) fn coupon_id(req: &Request) -> Result<Option<i64>, ()> {
    match req.headers().get("x-coupon-id") {
        None => Ok(None),
        Some(v) => v
            .to_str()
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|n| *n > 0)
            .map(Some)
            .ok_or(()),
    }
}

pub(crate) fn discount(
    conn: &mut PgConnection,
    uid: i64,
    coupon: Option<i64>,
    items: &[(i32, i64)],
) -> QueryResult<(i64, Vec<i64>)> {
    let mut allocated = vec![0; items.len()];
    let Some(id) = coupon else {
        return Ok((0, allocated));
    };
    let row=sql_query("SELECT to_jsonb(t) AS data FROM user_coupons u JOIN coupon_templates t ON t.id=u.template_id WHERE u.id=$1 AND u.user_id=$2 AND u.status='available' AND t.is_active AND t.starts_at<=now() AND t.expires_at>now() FOR UPDATE OF u,t").bind::<BigInt,_>(id).bind::<BigInt,_>(uid).get_result::<Coupon>(conn)?;
    let rule = &row.data;
    if rule["kind"] == "new_user" {
        let row=sql_query("SELECT jsonb_build_object('paid',EXISTS(SELECT 1 FROM orders WHERE buyer_id=$1 AND status='paid_test')) AS data").bind::<BigInt,_>(uid).get_result::<JsonRow>(conn)?;
        if row.data["paid"] == true {
            return Err(diesel::result::Error::RollbackTransaction);
        }
    }
    let total: i64 = items.iter().map(|(_, p)| p).sum();
    if total < rule["min_spend_cents"].as_i64().unwrap() {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let mut eligible = Vec::new();
    let mut base = 0;
    for (i, (product, price)) in items.iter().enumerate() {
        let category = sql_query(
            "SELECT jsonb_build_object('category',category) AS data FROM catalog WHERE id=$1",
        )
        .bind::<Integer, _>(*product)
        .get_result::<JsonRow>(conn)?;
        if (rule["product_id"].is_null() || rule["product_id"] == *product)
            && (rule["category"] == "" || rule["category"] == category.data["category"])
        {
            eligible.push(i);
            base += price;
        }
    }
    if base == 0 {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let value = rule["value"].as_i64().unwrap();
    let mut amount = if rule["discount_kind"] == "fixed" {
        value.min(base)
    } else {
        ((base as i128 * value as i128) / 10000) as i64
    };
    let cap = rule["max_discount_cents"].as_i64().unwrap();
    if cap > 0 {
        amount = amount.min(cap);
    }
    let mut rest = amount;
    // Allocate the real discount to eligible gift values so recycling cannot cash out face value.
    for i in eligible {
        allocated[i] = rest.min(items[i].1);
        rest -= allocated[i];
    }
    Ok((amount, allocated))
}
pub(crate) fn reserve(
    conn: &mut PgConnection,
    uid: i64,
    id: Option<i64>,
    order: i64,
) -> QueryResult<()> {
    if let Some(id) = id {
        let n=sql_query("UPDATE user_coupons SET status='reserved',order_id=$3 WHERE id=$1 AND user_id=$2 AND status='available'").bind::<BigInt,_>(id).bind::<BigInt,_>(uid).bind::<BigInt,_>(order).execute(conn)?;
        if n != 1 {
            return Err(diesel::result::Error::RollbackTransaction);
        }
    }
    Ok(())
}
pub(crate) fn redeem(conn: &mut PgConnection, order: i64) -> QueryResult<()> {
    let row =
        sql_query("SELECT jsonb_build_object('coupon',coupon_id) AS data FROM orders WHERE id=$1")
            .bind::<BigInt, _>(order)
            .get_result::<JsonRow>(conn)?;
    if row.data["coupon"].is_null() {
        return Ok(());
    }
    let id = row.data["coupon"].as_i64().unwrap();
    sql_query("SELECT to_jsonb(t) AS data FROM user_coupons u JOIN coupon_templates t ON t.id=u.template_id WHERE u.id=$1 AND u.order_id=$2 AND u.status='reserved' AND t.is_active AND t.starts_at<=now() AND t.expires_at>now() AND (t.kind<>'new_user' OR NOT EXISTS(SELECT 1 FROM orders o WHERE o.buyer_id=u.user_id AND o.status='paid_test')) FOR UPDATE OF u,t").bind::<BigInt,_>(id).bind::<BigInt,_>(order).get_result::<Coupon>(conn)?;
    sql_query("UPDATE user_coupons SET status='used',used_at=now() WHERE id=$1")
        .bind::<BigInt, _>(id)
        .execute(conn)?;
    Ok(())
}

pub(crate) fn refund(conn: &mut PgConnection, gift: i64, kind: &str) -> QueryResult<()> {
    sql_query("SELECT set_config('liyu.reason',$1,true)")
        .bind::<Text, _>(format!("gift {kind}"))
        .execute(conn)?;
    let n=sql_query("INSERT INTO wallet_ledger(user_id,amount_cents,kind,gift_id) SELECT sender_id,price_cents,$2,id FROM gifts WHERE id=$1 AND state IN ('sealed','opened') ON CONFLICT(gift_id,kind) DO NOTHING").bind::<BigInt,_>(gift).bind::<Text,_>(kind).execute(conn)?;
    if n == 1 {
        sql_query(
            "UPDATE catalog SET stock=stock+1 WHERE id=(SELECT product_id FROM gifts WHERE id=$1)",
        )
        .bind::<BigInt, _>(gift)
        .execute(conn)?;
    }
    Ok(())
}

fn recovery_quote(conn: &mut PgConnection, uid: i64, id: i64, lock: bool) -> QueryResult<Value> {
    let sql=format!("SELECT jsonb_build_object('id',g.id,'product_id',g.product_id,'paid_cents',g.price_cents,'state',g.state,'mode',p.mode,'value',p.value,'enabled',p.is_active AND (p.expires_at IS NULL OR p.expires_at>now())) AS data FROM gifts g LEFT JOIN recycle_policies p ON p.product_id=g.product_id WHERE g.id=$1 AND g.recipient_id=$2{}",if lock {" FOR UPDATE OF g"} else {""});
    let row = sql_query(sql)
        .bind::<BigInt, _>(id)
        .bind::<BigInt, _>(uid)
        .get_result::<JsonRow>(conn)?;
    let mut v = row.data;
    if v["state"] != "revealed" || v["enabled"] != true {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let paid = v["paid_cents"].as_i64().unwrap();
    let value = v["value"].as_i64().unwrap();
    let amount = if v["mode"] == "fixed" {
        value.min(paid)
    } else {
        ((paid as i128 * value as i128) / 10000) as i64
    };
    v["recovery_cents"] = json!(amount);
    v["fee_cents"] = json!(paid - amount);
    Ok(v)
}
#[handler]
async fn recycle_quote(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Some(id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    match recovery_quote(&mut conn, uid, id, false) {
        Ok(v) => res.render(Json(v)),
        Err(_) => error(
            res,
            StatusCode::CONFLICT,
            "gift not eligible or no active recovery policy",
        ),
    }
}
#[handler]
async fn settle(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Some(id) = req.param::<i64>("id") else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let exchange = req.uri().path().ends_with("/exchange");
    let input: Value = if exchange {
        match req.parse_json().await {
            Ok(v) => v,
            Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
        }
    } else {
        Value::Null
    };
    if exchange
        && !input["product_id"]
            .as_i64()
            .is_some_and(|n| (0..=i32::MAX as i64).contains(&n))
    {
        return error(res, StatusCode::BAD_REQUEST, "invalid replacement product");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Value,diesel::result::Error,_>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock($1)").bind::<BigInt,_>(uid).execute(conn)?;
        let prior=sql_query("SELECT jsonb_build_object('state',state,'replacement',exchanged_item_id) AS data FROM gifts WHERE id=$1 AND recipient_id=$2 FOR UPDATE").bind::<BigInt,_>(id).bind::<BigInt,_>(uid).get_result::<JsonRow>(conn)?;
        if (!exchange && prior.data["state"]=="cashed_out") || (exchange && prior.data["state"]=="exchanged" && prior.data["replacement"]==input["product_id"]) {
            return sql_query("SELECT jsonb_build_object('id',gift_id,'amount_cents',amount_cents,'state',CASE WHEN kind='exchange' THEN 'exchanged' ELSE 'cashed_out' END) AS data FROM wallet_ledger WHERE gift_id=$1 AND kind=$2").bind::<BigInt,_>(id).bind::<Text,_>(if exchange {"exchange"} else {"cash_out"}).get_result::<JsonRow>(conn).map(|r|r.data);
        }
        sql_query("SELECT set_config('liyu.reason',$1,true)").bind::<Text,_>(if exchange {"gift exchange"} else {"gift cash out"}).execute(conn)?;
        let quote=recovery_quote(conn,uid,id,true)?;let credit=quote["recovery_cents"].as_i64().unwrap();
        let replacement=input["product_id"].as_i64().unwrap_or(0) as i32;
        let mut amount=credit;
        if exchange {
            let row=sql_query("SELECT jsonb_build_object('price',price_cents) AS data FROM catalog WHERE id=$1 AND is_active AND stock>0 FOR UPDATE").bind::<Integer,_>(replacement).get_result::<JsonRow>(conn)?;
            amount-=row.data["price"].as_i64().unwrap();
            let balance=sql_query("SELECT jsonb_build_object('balance',COALESCE(sum(amount_cents),0)) AS data FROM wallet_ledger WHERE user_id=$1").bind::<BigInt,_>(uid).get_result::<JsonRow>(conn)?.data["balance"].as_i64().unwrap();
            if balance+amount<0 {return Err(diesel::result::Error::RollbackTransaction);}
            sql_query("UPDATE catalog SET stock=stock-1 WHERE id=$1").bind::<Integer,_>(replacement).execute(conn)?;
        }
        sql_query("INSERT INTO wallet_ledger(user_id,amount_cents,kind,gift_id) VALUES($1,$2,$3,$4)").bind::<BigInt,_>(uid).bind::<BigInt,_>(amount).bind::<Text,_>(if exchange {"exchange"} else {"cash_out"}).bind::<BigInt,_>(id).execute(conn)?;
        sql_query("UPDATE catalog SET stock=stock+1 WHERE id=$1").bind::<Integer,_>(quote["product_id"].as_i64().unwrap() as i32).execute(conn)?;
        if exchange {sql_query("UPDATE gifts SET state='exchanged',exchanged_item_id=$2,settled_at=now() WHERE id=$1").bind::<BigInt,_>(id).bind::<Integer,_>(replacement).execute(conn)?;}
        else {sql_query("UPDATE gifts SET state='cashed_out',settled_at=now() WHERE id=$1").bind::<BigInt,_>(id).execute(conn)?;}
        // A settled wish remains claimed; exchanging must not let a second buyer claim it.
        Ok(json!({"id":id,"amount_cents":amount,"state":if exchange {"exchanged"} else {"cashed_out"}}))
    });
    match result {
        Ok(v) => res.render(Json(v)),
        Err(_) => error(
            res,
            StatusCode::CONFLICT,
            "gift cannot settle, policy unavailable or insufficient balance",
        ),
    }
}
#[handler]
async fn wallet(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let query = if req.uri().path().ends_with("/coupons") {
        "SELECT to_jsonb(r) AS data FROM (SELECT u.id,u.status,t.name,t.kind,t.discount_kind,t.value,t.min_spend_cents,t.max_discount_cents,t.product_id,t.category,t.starts_at,t.expires_at,t.is_active FROM user_coupons u JOIN coupon_templates t ON t.id=u.template_id WHERE u.user_id=$1 ORDER BY u.id DESC LIMIT 100) r"
    } else {
        "SELECT jsonb_build_object('balance_cents',COALESCE(sum(amount_cents),0),'ledger',COALESCE((SELECT jsonb_agg(r) FROM (SELECT id,amount_cents,kind,gift_id,created_at FROM wallet_ledger WHERE user_id=$1 ORDER BY id DESC LIMIT 100) r),'[]'::jsonb)) AS data FROM wallet_ledger WHERE user_id=$1"
    };
    match sql_query(query)
        .bind::<BigInt, _>(uid)
        .load::<JsonRow>(&mut conn)
    {
        Ok(rows) => {
            let data: Value = if req.uri().path().ends_with("/coupons") {
                json!(rows.into_iter().map(|r| r.data).collect::<Vec<_>>())
            } else {
                rows.into_iter().next().unwrap().data
            };
            res.render(Json(data));
        }
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "wallet query failed",
        ),
    }
}
#[handler]
async fn notifications(req: &mut Request, res: &mut Response) {
    let Some(uid) = user_id(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let id = req.param::<i64>("id");
    if let Some(id) = id {
        match sql_query("UPDATE notifications SET read_at=COALESCE(read_at,now()) WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL AND expires_at>now()").bind::<BigInt,_>(id).bind::<BigInt,_>(uid).execute(&mut conn) {Ok(1)=>res.render(Json(json!({"id":id,"read":true}))),_=>error(res,StatusCode::NOT_FOUND,"notification not found")};
        return;
    }
    match sql_query("SELECT to_jsonb(n) AS data FROM notifications n WHERE user_id=$1 AND revoked_at IS NULL AND expires_at>now() ORDER BY id DESC LIMIT 100").bind::<BigInt,_>(uid).load::<JsonRow>(&mut conn) {Ok(rows)=>res.render(Json(json!(rows.into_iter().map(|r|r.data).collect::<Vec<_>>()))),Err(_)=>error(res,StatusCode::INTERNAL_SERVER_ERROR,"notifications query failed")}
}

pub(crate) fn routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/gifts/{id}/recovery-quote").get(recycle_quote))
        .push(Router::with_path("api/v1/gifts/{id}/cash-out").post(settle))
        .push(Router::with_path("api/v1/gifts/{id}/exchange").post(settle))
        .push(Router::with_path("api/v1/wallet").get(wallet))
        .push(Router::with_path("api/v1/me/coupons").get(wallet))
        .push(Router::with_path("api/v1/notifications").get(notifications))
        .push(Router::with_path("api/v1/notifications/{id}/read").post(notifications))
}
