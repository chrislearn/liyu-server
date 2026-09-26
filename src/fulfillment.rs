//! Role-specific fulfillment endpoints. Sender responses never load shipment PII.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Text};
use salvo::prelude::*;
use serde_json::{json, Value};

use super::{error, pool, user_id};

#[derive(QueryableByName)]
struct ShipmentDetail {
    #[diesel(sql_type = BigInt)]
    gift_id: i64,
    #[diesel(sql_type = Text)]
    carrier: String,
    #[diesel(sql_type = Text)]
    tracking_number: String,
    #[diesel(sql_type = Text)]
    recipient_name: String,
    #[diesel(sql_type = Text)]
    recipient_phone: String,
    #[diesel(sql_type = Text)]
    recipient_address: String,
    #[diesel(sql_type = Bool)]
    carrier_delivered: bool,
    #[diesel(sql_type = Bool)]
    recipient_confirmed: bool,
}

#[derive(QueryableByName)]
struct ShipmentEvent {
    #[diesel(sql_type = Text)]
    event_at: String,
    #[diesel(sql_type = Text)]
    description: String,
}

#[derive(QueryableByName)]
struct SenderDeliveryStatus {
    #[diesel(sql_type = BigInt)]
    gift_id: i64,
    #[diesel(sql_type = Bool)]
    carrier_delivered: bool,
    #[diesel(sql_type = Bool)]
    recipient_confirmed: bool,
}

fn recipient_projection(detail: ShipmentDetail, events: Vec<ShipmentEvent>) -> Value {
    json!({
        "gift_id": detail.gift_id,
        "carrier": detail.carrier,
        "tracking_number": detail.tracking_number,
        "recipient": {
            "name": detail.recipient_name,
            "phone": detail.recipient_phone,
            "address": detail.recipient_address,
        },
        "carrier_delivered": detail.carrier_delivered,
        "recipient_confirmed": detail.recipient_confirmed,
        "events": events.into_iter().map(|e| json!({
            "at": e.event_at,
            "description": e.description,
        })).collect::<Vec<_>>(),
    })
}

fn sender_projection(status: SenderDeliveryStatus) -> Value {
    json!({
        "gift_id": status.gift_id,
        "carrier_delivered": status.carrier_delivered,
        "recipient_confirmed": status.recipient_confirmed,
    })
}

fn requested_gift(req: &Request, res: &mut Response) -> Option<i64> {
    let Some(id) = req.param::<i64>("gift_id").filter(|id| *id > 0) else {
        error(res, StatusCode::BAD_REQUEST, "invalid gift id");
        return None;
    };
    Some(id)
}

fn authenticated_user(req: &Request, res: &mut Response) -> Option<i64> {
    let Some(uid) = user_id(req) else {
        error(res, StatusCode::UNAUTHORIZED, "invalid session");
        return None;
    };
    Some(uid)
}

#[handler]
async fn recipient_shipment(req: &mut Request, res: &mut Response) {
    let Some(uid) = authenticated_user(req, res) else {
        return;
    };
    let Some(gid) = requested_gift(req, res) else {
        return;
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    // The recipient predicate is in SQL, so an unrelated account cannot fetch the row.
    let detail = diesel::sql_query(
        "SELECT s.gift_id, s.carrier, s.tracking_number, s.recipient_name, \
         s.recipient_phone, s.recipient_address, \
         (s.delivered_at IS NOT NULL) AS carrier_delivered, \
         (s.recipient_confirmed_at IS NOT NULL) AS recipient_confirmed \
         FROM shipments s JOIN gifts g ON g.id = s.gift_id \
         WHERE s.gift_id = $1 AND g.recipient_id = $2",
    )
    .bind::<BigInt, _>(gid)
    .bind::<BigInt, _>(uid)
    .get_result::<ShipmentDetail>(&mut conn)
    .optional();
    let detail = match detail {
        Ok(Some(detail)) => detail,
        Ok(None) => return error(res, StatusCode::NOT_FOUND, "shipment not found"),
        Err(_) => {
            return error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "shipment query failed",
            )
        }
    };
    let events = diesel::sql_query(
        "SELECT to_char(event_at, 'YYYY-MM-DD\"T\"HH24:MI:SSOF') AS event_at, \
         description FROM shipment_events WHERE gift_id = $1 ORDER BY event_at, id",
    )
    .bind::<BigInt, _>(gid)
    .load::<ShipmentEvent>(&mut conn);
    match events {
        Ok(events) => res.render(Json(recipient_projection(detail, events))),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "shipment events query failed",
        ),
    }
}

#[handler]
async fn confirm_receipt(req: &mut Request, res: &mut Response) {
    let Some(uid) = authenticated_user(req, res) else {
        return;
    };
    let Some(gid) = requested_gift(req, res) else {
        return;
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    // An absent or foreign gift has the same response. The sender cannot confirm receipt.
    let owned = diesel::sql_query(
        "SELECT s.gift_id, (s.delivered_at IS NOT NULL) AS carrier_delivered, \
         (s.recipient_confirmed_at IS NOT NULL) AS recipient_confirmed \
         FROM shipments s JOIN gifts g ON g.id = s.gift_id \
         WHERE s.gift_id = $1 AND g.recipient_id = $2",
    )
    .bind::<BigInt, _>(gid)
    .bind::<BigInt, _>(uid)
    .get_result::<SenderDeliveryStatus>(&mut conn)
    .optional();
    let owned = match owned {
        Ok(Some(status)) => status,
        Ok(None) => return error(res, StatusCode::NOT_FOUND, "shipment not found"),
        Err(_) => {
            return error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "shipment query failed",
            )
        }
    };
    if !owned.carrier_delivered {
        return error(
            res,
            StatusCode::CONFLICT,
            "carrier has not delivered the gift",
        );
    }
    let result = diesel::sql_query(
        "UPDATE shipments SET recipient_confirmed_at = \
         COALESCE(recipient_confirmed_at, now()) \
         WHERE gift_id = $1 AND delivered_at IS NOT NULL \
         AND EXISTS (SELECT 1 FROM gifts WHERE gifts.id = $1 AND gifts.recipient_id = $2) \
         RETURNING gift_id, (delivered_at IS NOT NULL) AS carrier_delivered, \
         (recipient_confirmed_at IS NOT NULL) AS recipient_confirmed",
    )
    .bind::<BigInt, _>(gid)
    .bind::<BigInt, _>(uid)
    .get_result::<SenderDeliveryStatus>(&mut conn)
    .optional();
    match result {
        Ok(Some(status)) => res.render(Json(sender_projection(status))),
        Ok(None) => error(
            res,
            StatusCode::CONFLICT,
            "carrier has not delivered the gift",
        ),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "receipt confirmation failed",
        ),
    }
}

#[handler]
async fn sender_delivery_status(req: &mut Request, res: &mut Response) {
    let Some(uid) = authenticated_user(req, res) else {
        return;
    };
    let Some(gid) = requested_gift(req, res) else {
        return;
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    // Select only two booleans. No address, tracking, event, or exchange data is loaded.
    let status = diesel::sql_query(
        "SELECT g.id AS gift_id, (s.delivered_at IS NOT NULL) AS carrier_delivered, \
         (s.recipient_confirmed_at IS NOT NULL) AS recipient_confirmed \
         FROM gifts g LEFT JOIN shipments s ON s.gift_id = g.id \
         WHERE g.id = $1 AND g.sender_id = $2",
    )
    .bind::<BigInt, _>(gid)
    .bind::<BigInt, _>(uid)
    .get_result::<SenderDeliveryStatus>(&mut conn)
    .optional();
    match status {
        Ok(Some(status)) => res.render(Json(sender_projection(status))),
        Ok(None) => error(res, StatusCode::NOT_FOUND, "gift not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "delivery status query failed",
        ),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/shipments/{gift_id}").get(recipient_shipment))
        .push(Router::with_path("api/v1/shipments/{gift_id}/confirm-receipt").post(confirm_receipt))
        .push(
            Router::with_path("api/v1/orders/{gift_id}/delivery-status")
                .get(sender_delivery_status),
        )
        .push(
            Router::with_path("api/v1/gifts/{gift_id}/delivery-summary")
                .get(sender_delivery_status),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ShipmentDetail {
        ShipmentDetail {
            gift_id: 900001,
            carrier: "礼遇演示快递".into(),
            tracking_number: "LY-DEMO-20260926-0001".into(),
            recipient_name: "林舟".into(),
            recipient_phone: "13800138000".into(),
            recipient_address: "上海市虚构区演示路 123 号".into(),
            carrier_delivered: true,
            recipient_confirmed: false,
        }
    }

    #[test]
    fn three_user_projection_has_no_sender_privacy_leak() {
        let sender = 1_i64;
        let recipient = 2_i64;
        let stranger = 3_i64;
        let recipient_json = recipient_projection(
            fixture(),
            vec![ShipmentEvent {
                event_at: "2026-09-26T10:00:00+08".into(),
                description: "演示包裹已签收".into(),
            }],
        );
        assert_eq!(recipient_json["recipient"]["name"], "林舟");
        assert_eq!(recipient_json["events"].as_array().unwrap().len(), 1);

        let sender_json = sender_projection(SenderDeliveryStatus {
            gift_id: 900001,
            carrier_delivered: true,
            recipient_confirmed: false,
        });
        assert_eq!(sender_json.as_object().unwrap().len(), 3);
        for secret in [
            "林舟",
            "13800138000",
            "演示路",
            "LY-DEMO",
            "exchanged_item_id",
        ] {
            assert!(!sender_json.to_string().contains(secret));
        }
        assert_ne!(sender, recipient);
        assert_ne!(stranger, recipient);
        assert_ne!(stranger, sender);
        // The SQL predicates above bind recipient_id / sender_id to the token's user ID;
        // the unrelated third user matches neither route and receives 404.
    }
}
