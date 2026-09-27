//! Operations domain: whitelisted reports and transactional, audited commands.
use crate::{admin, error, pool};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_query,
    sql_types::{BigInt, Jsonb, Text},
};
use salvo::prelude::*;
use serde_json::{json, Value};

#[derive(QueryableByName)]
pub(crate) struct JsonRow {
    #[diesel(sql_type = Jsonb)]
    pub data: Value,
}

pub(crate) fn ensure_schema(conn: &mut PgConnection) -> Result<(), String> {
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        sql_query("SELECT pg_advisory_xact_lock(731129927)").execute(conn)?;
        let row: JsonRow = sql_query("SELECT jsonb_build_object('installed',to_regclass('management_schema_version') IS NOT NULL) AS data").get_result(conn)?;
        if row.data["installed"] != true {
            let section = include_str!("../migrations/20260926000000_init/up.sql").split_once("-- 13. management domains").unwrap().1;
            conn.batch_execute(&format!("-- 13. management domains{section}"))?;
        } else {
            let version=sql_query("SELECT jsonb_build_object('version',version) AS data FROM management_schema_version").get_result::<JsonRow>(conn)?;
            if version.data["version"]!=1 {return Err(diesel::result::Error::RollbackTransaction);}
        }
        Ok(())
    }).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)] // mirrors the complete persisted audit record
pub(crate) fn audit(
    conn: &mut PgConnection,
    actor: i64,
    action: &str,
    entity: &str,
    id: i64,
    reason: &str,
    before: Value,
    after: Value,
) -> QueryResult<()> {
    sql_query("INSERT INTO admin_audit(administrator_id,action,entity,entity_id,reason,before_data,after_data) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind::<BigInt,_>(actor).bind::<Text,_>(action).bind::<Text,_>(entity).bind::<BigInt,_>(id)
        .bind::<Text,_>(reason).bind::<Jsonb,_>(before).bind::<Jsonb,_>(after).execute(conn)?;
    Ok(())
}

fn report_sql(module: &str) -> Option<&'static str> {
    Some(match module {
        "users" => "SELECT u.id,u.identifier,u.display_name,u.is_active,u.created_at,p.phone,p.email,(SELECT count(*) FROM sessions s WHERE s.user_id=u.id AND expires_at>now()) AS sessions,(SELECT COALESCE(sum(amount_cents),0) FROM wallet_ledger w WHERE w.user_id=u.id) AS balance_cents FROM users u LEFT JOIN user_profiles p ON p.user_id=u.id",
        "low-stock" => "SELECT id,name,category,price_cents,stock,is_active FROM catalog WHERE is_active AND stock<=10",
        "inventory" | "prices" => "SELECT id,name,category,price_cents,stock,is_active FROM catalog",
        "history" => "SELECT * FROM catalog_history",
        "coupons" => "SELECT t.*,(SELECT count(*) FROM user_coupons u WHERE u.template_id=t.id) AS issued_count FROM coupon_templates t",
        "issued-coupons" => "SELECT u.*,t.name,t.expires_at,t.discount_kind,t.value FROM user_coupons u JOIN coupon_templates t ON t.id=u.template_id",
        "recycling" => "SELECT c.id,c.name,c.price_cents,p.is_active,p.mode,p.value,p.expires_at FROM catalog c LEFT JOIN recycle_policies p ON p.product_id=c.id",
        "wallet" => "SELECT w.*,u.display_name FROM wallet_ledger w JOIN users u ON u.id=w.user_id",
        "orders" => "SELECT o.*,(SELECT jsonb_agg(jsonb_build_object('product_id',i.product_id,'recipient_id',i.recipient_id,'price_cents',i.price_cents,'gift_id',i.gift_id)) FROM order_items i WHERE i.order_id=o.id) AS items FROM orders o",
        "gifts" => "SELECT id,sender_id,recipient_id,product_id,exchanged_item_id,price_cents,state,unlock_kind,attempts,available_at,expires_at,created_at,settled_at FROM gifts",
        "shipments" => "SELECT s.gift_id AS id,s.carrier,s.tracking_number,s.recipient_name,s.recipient_phone,s.recipient_address,s.delivered_at,s.recipient_confirmed_at,(SELECT jsonb_agg(jsonb_build_object('at',event_at,'description',description) ORDER BY event_at) FROM shipment_events e WHERE e.gift_id=s.gift_id) AS events FROM shipments s",
        "friends" => "SELECT id,user_low_id,user_high_id,status,created_at FROM friendships",
        "wishlists" => "SELECT w.*,(SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id=w.id) AS item_count,(SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id=w.id AND claimed_gift_id IS NOT NULL) AS claimed_count,(SELECT jsonb_agg(jsonb_build_object('id',i.id,'product_id',i.product_id,'kind',i.kind,'claimed_gift_id',i.claimed_gift_id)) FROM wishlist_items i WHERE i.wishlist_id=w.id) AS items FROM wishlists w",
        "contracts" => "SELECT g.id,g.sender_id,g.recipient_id,g.contract_text,g.state AS gift_state,CASE WHEN g.state IN ('cashed_out','exchanged','withdrawn','expired') THEN 'voided' ELSE COALESCE(c.status,'pending') END AS status,c.updated_at FROM gifts g LEFT JOIN gift_contracts c ON c.gift_id=g.id WHERE g.contract_text<>''",
        "notifications" => "SELECT * FROM notifications",
        "audit" => "SELECT a.*,u.username FROM admin_audit a LEFT JOIN administrators u ON u.id=a.administrator_id",
        _ => return None,
    })
}

#[handler]
async fn report(req: &mut Request, res: &mut Response) {
    if admin::authorize(req, res, false).is_none() {
        return;
    }
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    let module = req.param::<String>("module").unwrap_or_default();
    let Some(sql) = report_sql(&module) else {
        return error(res, StatusCode::NOT_FOUND, "unknown report");
    };
    let q = req.query::<String>("q").unwrap_or_default();
    let cursor = req.query::<i64>("cursor").unwrap_or(-1);
    if q.len() > 400 || cursor < -1 {
        return error(res, StatusCode::BAD_REQUEST, "invalid query");
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let query=format!("SELECT to_jsonb(r) AS data FROM ({sql}) r WHERE r.id>$1 AND ($2='' OR to_jsonb(r)::text ILIKE '%'||$2||'%') ORDER BY r.id LIMIT 51");
    match sql_query(query)
        .bind::<BigInt, _>(cursor)
        .bind::<Text, _>(q)
        .load::<JsonRow>(&mut conn)
    {
        Ok(mut rows) => {
            let more = rows.len() > 50;
            rows.truncate(50);
            let next = if more {
                rows.last().map(|r| r.data["id"].clone())
            } else {
                None
            };
            res.render(Json(json!({"items":rows.into_iter().map(|r|r.data).collect::<Vec<_>>(),"next_cursor":next})));
        }
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "report query failed",
        ),
    }
}
#[handler]
async fn overview(req: &mut Request, res: &mut Response) {
    if admin::authorize(req, res, false).is_none() {
        return;
    }
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let row=sql_query("SELECT jsonb_build_object('users',(SELECT count(*) FROM users),'active_products',(SELECT count(*) FROM catalog WHERE is_active),'low_stock',(SELECT count(*) FROM catalog WHERE is_active AND stock<=10),'pending_orders',(SELECT count(*) FROM orders WHERE status='pending'),'paid_cents',(SELECT COALESCE(sum(total_cents),0) FROM orders WHERE status='paid_test'),'wallet_cents',(SELECT COALESCE(sum(amount_cents),0) FROM wallet_ledger)) AS data").get_result::<JsonRow>(&mut conn);
    match row {
        Ok(r) => res.render(Json(r.data)),
        Err(_) => error(res, StatusCode::INTERNAL_SERVER_ERROR, "overview failed"),
    }
}

fn integer(v: &Value, k: &str, min: i64, max: i64) -> bool {
    v[k].as_i64().is_some_and(|n| (min..=max).contains(&n))
}
fn text(v: &Value, k: &str, min: usize, max: usize) -> bool {
    v[k].as_str()
        .is_some_and(|s| (min..=max).contains(&s.trim().chars().count()))
}
fn choice(v: &Value, k: &str, choices: &[&str]) -> bool {
    v[k].as_str().is_some_and(|s| choices.contains(&s))
}
fn boolean(v: &Value, k: &str) -> bool {
    v[k].is_boolean()
}

// SQL is selected by the server, never accepted from the request.
fn command(action: &str, v: &Value) -> Option<(&'static str, &'static str, &'static str)> {
    let money = 1_000_000_000;
    Some(match action {
        "user-status" if boolean(v,"is_active") => ("users","SELECT to_jsonb(u)-'password_hash'-'state' AS data FROM users u WHERE id=$2 FOR UPDATE", "UPDATE users SET is_active=($1->>'is_active')::boolean WHERE id=$2 RETURNING to_jsonb(users)-'password_hash'-'state' AS data"),
        "user-sessions" => ("users", "SELECT to_jsonb(u)-'password_hash'-'state' AS data FROM users u WHERE id=$2 FOR UPDATE", "WITH removed AS (DELETE FROM sessions WHERE user_id=$2 AND $1 IS NOT NULL RETURNING user_id) SELECT jsonb_build_object('id',$2,'revoked',count(*)) AS data FROM removed"),
        "stock" if integer(v,"delta",-2_000_000_000,2_000_000_000) => ("catalog","SELECT to_jsonb(c) AS data FROM catalog c WHERE id=$2 FOR UPDATE", "UPDATE catalog SET stock=stock+($1->>'delta')::integer WHERE id=$2 AND stock::bigint+($1->>'delta')::bigint BETWEEN 0 AND 2147483647 RETURNING to_jsonb(catalog) AS data"),
        "price" if integer(v,"price_cents",0,money) => ("catalog","SELECT to_jsonb(c) AS data FROM catalog c WHERE id=$2 FOR UPDATE", "UPDATE catalog SET price_cents=($1->>'price_cents')::bigint WHERE id=$2 RETURNING to_jsonb(catalog) AS data"),
        "listing" if boolean(v,"is_active") => ("catalog","SELECT to_jsonb(c) AS data FROM catalog c WHERE id=$2 FOR UPDATE", "UPDATE catalog SET is_active=($1->>'is_active')::boolean WHERE id=$2 RETURNING to_jsonb(catalog) AS data"),
        "coupon" if text(v,"name",1,100) && choice(v,"kind",&["promotion","new_user","compensation"]) && choice(v,"discount_kind",&["fixed","percentage"]) && integer(v,"value",1,money) && (v["discount_kind"]!="percentage" || integer(v,"value",1,10000)) && integer(v,"min_spend_cents",0,money) && integer(v,"max_discount_cents",0,money) && integer(v,"issue_limit",1,1000000) && integer(v,"per_user_limit",1,10000) && text(v,"starts_at",1,40) && text(v,"expires_at",1,40) && boolean(v,"is_active") && choice(v,"category",&["","coffee","movie","trendy","blind","sweet","digital","home","baby"]) && (v["product_id"].is_null() || integer(v,"product_id",0,2147483647)) => ("coupon_templates","SELECT to_jsonb(t) AS data FROM coupon_templates t WHERE id=$2 FOR UPDATE", "INSERT INTO coupon_templates(name,kind,discount_kind,value,min_spend_cents,max_discount_cents,product_id,category,starts_at,expires_at,issue_limit,per_user_limit,is_active) SELECT $1->>'name',$1->>'kind',$1->>'discount_kind',($1->>'value')::bigint,($1->>'min_spend_cents')::bigint,($1->>'max_discount_cents')::bigint,($1->>'product_id')::integer,$1->>'category',($1->>'starts_at')::timestamptz,($1->>'expires_at')::timestamptz,($1->>'issue_limit')::integer,($1->>'per_user_limit')::integer,($1->>'is_active')::boolean WHERE $2=0 RETURNING to_jsonb(coupon_templates) AS data"),
        "coupon-status" if boolean(v,"is_active") => ("coupon_templates","SELECT to_jsonb(t) AS data FROM coupon_templates t WHERE id=$2 FOR UPDATE","UPDATE coupon_templates SET is_active=($1->>'is_active')::boolean WHERE id=$2 RETURNING to_jsonb(coupon_templates) AS data"),
        "coupon-revoke" => ("user_coupons","SELECT to_jsonb(t) AS data FROM user_coupons t WHERE id=$2 FOR UPDATE","UPDATE user_coupons SET status='revoked' WHERE id=$2 AND status='available' AND $1 IS NOT NULL RETURNING to_jsonb(user_coupons) AS data"),
        "recycle" if boolean(v,"is_active") && choice(v,"mode",&["fixed","percentage"]) && integer(v,"value",0,money) && (v["mode"]!="percentage" || integer(v,"value",0,10000)) && (v["expires_at"].is_null() || text(v,"expires_at",1,40)) => ("recycle_policies","SELECT to_jsonb(c) AS data FROM catalog c WHERE id=$2 FOR UPDATE","INSERT INTO recycle_policies(product_id,is_active,mode,value,expires_at) VALUES($2,($1->>'is_active')::boolean,$1->>'mode',($1->>'value')::bigint,($1->>'expires_at')::timestamptz) ON CONFLICT(product_id) DO UPDATE SET is_active=excluded.is_active,mode=excluded.mode,value=excluded.value,expires_at=excluded.expires_at RETURNING to_jsonb(recycle_policies) AS data"),
        "cancel-order" => ("orders","SELECT to_jsonb(o) AS data FROM orders o WHERE id=$2 FOR UPDATE","UPDATE orders SET status='cancelled' WHERE id=$2 AND status='pending' AND $1 IS NOT NULL RETURNING to_jsonb(orders) AS data"),
        "close-wishlist" => ("wishlists","SELECT to_jsonb(w) AS data FROM wishlists w WHERE id=$2 FOR UPDATE","UPDATE wishlists SET closed_at=COALESCE(closed_at,now()) WHERE id=$2 AND $1 IS NOT NULL RETURNING to_jsonb(wishlists) AS data"),
        "remove-friend" => ("friendships","SELECT to_jsonb(f) AS data FROM friendships f WHERE id=$2 FOR UPDATE","DELETE FROM friendships WHERE id=$2 AND $1 IS NOT NULL RETURNING to_jsonb(friendships) AS data"),
        "contract" if choice(v,"status",&["fulfilled","waived"]) => ("gift_contracts","SELECT jsonb_build_object('id',g.id,'status',COALESCE(c.status,'pending')) AS data FROM gifts g LEFT JOIN gift_contracts c ON c.gift_id=g.id WHERE g.id=$2 AND g.contract_text<>'' AND g.state='accepted' FOR UPDATE OF g","INSERT INTO gift_contracts(gift_id,status) VALUES($2,$1->>'status') ON CONFLICT(gift_id) DO UPDATE SET status=excluded.status,updated_at=now() WHERE gift_contracts.status='pending' RETURNING to_jsonb(gift_contracts) AS data"),
        "notify" if text(v,"title",1,100) && text(v,"body",0,2000) && integer(v,"user_id",1,i64::MAX) && text(v,"expires_at",1,40) => ("notifications","SELECT to_jsonb(n) AS data FROM notifications n WHERE id=$2 FOR UPDATE","INSERT INTO notifications(user_id,title,body,expires_at) SELECT ($1->>'user_id')::bigint,$1->>'title',$1->>'body',($1->>'expires_at')::timestamptz WHERE $2=0 AND ($1->>'expires_at')::timestamptz>now() RETURNING to_jsonb(notifications) AS data"),
        "revoke-notification" => ("notifications","SELECT to_jsonb(n) AS data FROM notifications n WHERE id=$2 FOR UPDATE","UPDATE notifications SET revoked_at=COALESCE(revoked_at,now()) WHERE id=$2 AND $1 IS NOT NULL RETURNING to_jsonb(notifications) AS data"),
        _=>return None,
    })
}

#[handler]
async fn manage(req: &mut Request, res: &mut Response) {
    let Some(actor) = admin::authorize(req, res, true) else {
        return;
    };
    let action = req.param::<String>("action").unwrap_or_default();
    let Some(id) = req.param::<i64>("id").filter(|id| *id >= 0) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid id");
    };
    let input: Value = match req.parse_json().await {
        Ok(v) => v,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if !text(&input, "reason", 1, 500) {
        return error(
            res,
            StatusCode::BAD_REQUEST,
            "reason required (1-500 characters)",
        );
    }
    let Ok(mut conn) = pool().get() else {
        return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    };
    let result=conn.transaction::<Value,diesel::result::Error,_>(|conn| {
        sql_query("SELECT set_config('liyu.administrator',$1,true),set_config('liyu.reason',$2,true)").bind::<Text,_>(actor.id.to_string()).bind::<Text,_>(input["reason"].as_str().unwrap()).execute(conn)?;
        if action=="coupon-issue" {return issue(conn,actor.id,id,&input);}
        if action=="shipment" {return shipment(conn,actor.id,id,&input);}
        let (entity,before_sql,sql)=command(&action,&input).ok_or(diesel::result::Error::RollbackTransaction)?;
        let before=sql_query(before_sql).bind::<Jsonb,_>(&input).bind::<BigInt,_>(id).get_result::<JsonRow>(conn).optional()?;
        if id!=0 && before.is_none() {return Err(diesel::result::Error::NotFound);}
        let row=sql_query(sql).bind::<Jsonb,_>(&input).bind::<BigInt,_>(id).get_result::<JsonRow>(conn)?;
        if action=="user-status" && input["is_active"]==false {sql_query("DELETE FROM sessions WHERE user_id=$1").bind::<BigInt,_>(id).execute(conn)?;}
        if action=="cancel-order" {sql_query("UPDATE user_coupons SET status='available',order_id=NULL WHERE order_id=$1 AND status='reserved'").bind::<BigInt,_>(id).execute(conn)?;}
        let entity_id=row.data["id"].as_i64().unwrap_or(id);
        audit(conn,actor.id,&action,entity,entity_id,input["reason"].as_str().unwrap(),before.map_or(Value::Null,|r|r.data),row.data.clone())?;
        Ok(row.data)
    });
    match result {
        Ok(v) => res.render(Json(v)),
        Err(diesel::result::Error::NotFound) => error(
            res,
            StatusCode::CONFLICT,
            "record missing or state prevents this operation",
        ),
        Err(_) => error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid fields, quota, date or conflicting state",
        ),
    }
}

fn issue(conn: &mut PgConnection, actor: i64, id: i64, v: &Value) -> QueryResult<Value> {
    if !integer(v, "user_id", 1, i64::MAX) {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let before=sql_query("SELECT to_jsonb(t) AS data FROM coupon_templates t WHERE id=$1 AND is_active AND expires_at>now() FOR UPDATE").bind::<BigInt,_>(id).get_result::<JsonRow>(conn)?;
    let row=sql_query("INSERT INTO user_coupons(template_id,user_id) SELECT $1,$2 WHERE EXISTS(SELECT 1 FROM users WHERE id=$2 AND is_active) AND (SELECT count(*) FROM user_coupons WHERE template_id=$1)<$3 AND (SELECT count(*) FROM user_coupons WHERE template_id=$1 AND user_id=$2)<$4 AND ($5<>'new_user' OR NOT EXISTS(SELECT 1 FROM orders WHERE buyer_id=$2 AND status='paid_test')) RETURNING to_jsonb(user_coupons) AS data")
        .bind::<BigInt,_>(id).bind::<BigInt,_>(v["user_id"].as_i64().unwrap()).bind::<BigInt,_>(before.data["issue_limit"].as_i64().unwrap()).bind::<BigInt,_>(before.data["per_user_limit"].as_i64().unwrap()).bind::<Text,_>(before.data["kind"].as_str().unwrap()).get_result::<JsonRow>(conn)?;
    audit(
        conn,
        actor,
        "coupon-issue",
        "user_coupons",
        row.data["id"].as_i64().unwrap(),
        v["reason"].as_str().unwrap(),
        Value::Null,
        row.data.clone(),
    )?;
    Ok(row.data)
}
fn shipment(conn: &mut PgConnection, actor: i64, id: i64, v: &Value) -> QueryResult<Value> {
    if !text(v, "carrier", 1, 100)
        || !text(v, "tracking_number", 1, 100)
        || !text(v, "recipient_name", 1, 100)
        || !text(v, "recipient_phone", 1, 100)
        || !text(v, "recipient_address", 1, 500)
        || !text(v, "description", 1, 500)
        || !boolean(v, "delivered")
    {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    sql_query("SELECT id FROM gifts WHERE id=$1 AND state IN ('accepted','exchanged') AND EXISTS(SELECT 1 FROM catalog WHERE id=COALESCE(gifts.exchanged_item_id,gifts.product_id) AND physical) FOR UPDATE").bind::<BigInt,_>(id).execute(conn)?;
    // get_result is required: execute(SELECT) alone does not prove that a gift matched.
    let gift=sql_query("SELECT jsonb_build_object('id',id) AS data FROM gifts WHERE id=$1 AND state IN ('accepted','exchanged') AND EXISTS(SELECT 1 FROM catalog WHERE id=COALESCE(gifts.exchanged_item_id,gifts.product_id) AND physical)").bind::<BigInt,_>(id).get_result::<JsonRow>(conn)?;
    let before =
        sql_query("SELECT to_jsonb(s) AS data FROM shipments s WHERE gift_id=$1 FOR UPDATE")
            .bind::<BigInt, _>(id)
            .get_result::<JsonRow>(conn)
            .optional()?;
    if before
        .as_ref()
        .is_some_and(|r| !r.data["delivered_at"].is_null())
    {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let row=sql_query("INSERT INTO shipments(gift_id,carrier,tracking_number,recipient_name,recipient_phone,recipient_address,delivered_at) VALUES($2,$1->>'carrier',$1->>'tracking_number',$1->>'recipient_name',$1->>'recipient_phone',$1->>'recipient_address',CASE WHEN ($1->>'delivered')::boolean THEN now() END) ON CONFLICT(gift_id) DO UPDATE SET carrier=excluded.carrier,tracking_number=excluded.tracking_number,recipient_name=excluded.recipient_name,recipient_phone=excluded.recipient_phone,recipient_address=excluded.recipient_address,delivered_at=excluded.delivered_at RETURNING to_jsonb(shipments) AS data").bind::<Jsonb,_>(v).bind::<BigInt,_>(id).get_result::<JsonRow>(conn)?;
    sql_query("INSERT INTO shipment_events(gift_id,event_at,description) VALUES($1,now(),$2)")
        .bind::<BigInt, _>(id)
        .bind::<Text, _>(v["description"].as_str().unwrap())
        .execute(conn)?;
    audit(
        conn,
        actor,
        "shipment",
        "shipments",
        id,
        v["reason"].as_str().unwrap(),
        before.map_or(gift.data, |r| r.data),
        row.data.clone(),
    )?;
    Ok(row.data)
}

pub(crate) fn routes() -> Router {
    Router::new()
        .push(Router::with_path("admin/api/overview").get(overview))
        .push(Router::with_path("admin/api/reports/{module}").get(report))
        .push(Router::with_path("admin/api/manage/{action}/{id}").post(manage))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_upgrade_preserves_existing_rows_and_is_idempotent() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let mut conn = PgConnection::establish(&url).unwrap();
        conn.test_transaction::<_,diesel::result::Error,_>(|conn| {
            let name=format!("management_upgrade_{}",uuid::Uuid::new_v4().simple());
            conn.batch_execute(&format!("CREATE SCHEMA {name}; SET LOCAL search_path TO {name}"))?;
            let old=include_str!("../migrations/20260926000000_init/up.sql").split("-- 13. management domains").next().unwrap();
            conn.batch_execute(old)?;
            ensure_schema(conn).unwrap();
            conn.batch_execute("UPDATE catalog SET stock=stock-1,price_cents=price_cents+1 WHERE id=0")?;
            ensure_schema(conn).unwrap();
            let row=sql_query("SELECT jsonb_build_object('users',(SELECT count(*) FROM users),'products',(SELECT count(*) FROM catalog),'history',(SELECT count(*) FROM catalog_history),'stock',(SELECT stock FROM catalog WHERE id=0)) AS data").get_result::<JsonRow>(conn)?;
            assert_eq!(row.data,json!({"users":3,"products":33,"history":1,"stock":99}));
            Ok(())
        });
    }
}
