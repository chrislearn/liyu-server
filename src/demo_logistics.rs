//! Local-only demonstration shipment progression. Never mount on a public deployment.
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Text};
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::{error, pool};

#[derive(Deserialize)]
struct AdvanceRequest {
    stage: String,
}

#[derive(QueryableByName)]
struct CurrentStage {
    #[diesel(sql_type = Text)]
    stage: String,
}

#[derive(QueryableByName)]
struct LockedShipment {
    #[diesel(sql_type = Bool)]
    delivered: bool,
}

enum AdvanceOutcome {
    Changed(bool),
    Unchanged(bool),
    Conflict,
    NotFound,
}

const STAGES: [&str; 4] = ["collected", "transit", "out_for_delivery", "delivered"];

fn stage_index(value: &str) -> Option<usize> {
    STAGES.iter().position(|stage| *stage == value)
}

// Compare bytes without short-circuiting. The configured key must be long and random.
fn same_key(configured: &str, supplied: &str) -> bool {
    let mut difference = configured.len() ^ supplied.len();
    for (a, b) in configured.bytes().zip(supplied.bytes()) {
        difference |= (a ^ b) as usize;
    }
    difference == 0
}

fn permitted(mode: &str, configured: &str, supplied: Option<&str>, loopback: bool) -> bool {
    mode == "true"
        && loopback
        && configured.len() >= 32
        && supplied.is_some_and(|key| same_key(configured, key))
}

#[handler]
async fn advance(req: &mut Request, res: &mut Response) {
    let mode = std::env::var("LIYU_DEMO_MODE").unwrap_or_default();
    let key = std::env::var("LIYU_DEMO_ADMIN_KEY").unwrap_or_default();
    let supplied = req
        .headers()
        .get("x-liyu-demo-key")
        .and_then(|value| value.to_str().ok());
    let loopback = req
        .remote_addr()
        .as_ipv4()
        .is_some_and(|addr| addr.ip().is_loopback())
        || req
            .remote_addr()
            .as_ipv6()
            .is_some_and(|addr| addr.ip().is_loopback());
    if !permitted(&mode, &key, supplied, loopback) {
        return error(res, StatusCode::FORBIDDEN, "demo logistics unavailable");
    }
    let Some(gift_id) = req.param::<i64>("gift_id").filter(|id| *id > 0) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid gift id");
    };
    let body: AdvanceRequest = match req.parse_json().await {
        Ok(body) => body,
        Err(_) => return error(res, StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    let Some(wanted) = stage_index(&body.stage) else {
        return error(res, StatusCode::BAD_REQUEST, "invalid stage");
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return error(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    };
    let outcome = conn.transaction::<AdvanceOutcome, diesel::result::Error, _>(|conn| {
        // Lock the shipment so concurrent progression cannot skip or reorder events.
        let locked = diesel::sql_query(
            "SELECT (delivered_at IS NOT NULL) AS delivered \
             FROM shipments WHERE gift_id = $1 FOR UPDATE",
        )
        .bind::<BigInt, _>(gift_id)
        .get_result::<LockedShipment>(conn)
        .optional()?;
        let Some(locked) = locked else {
            return Ok(AdvanceOutcome::NotFound);
        };
        // A new statement after acquiring the row lock sees the preceding
        // transaction's committed event, even when two admins click together.
        let current = diesel::sql_query(
            "SELECT COALESCE((SELECT stage FROM shipment_events \
             WHERE gift_id = $1 AND stage IS NOT NULL \
             ORDER BY CASE stage WHEN 'collected' THEN 1 WHEN 'transit' THEN 2 \
             WHEN 'out_for_delivery' THEN 3 WHEN 'delivered' THEN 4 ELSE 0 END DESC \
             LIMIT 1), '') AS stage",
        )
        .bind::<BigInt, _>(gift_id)
        .get_result::<CurrentStage>(conn)?;
        let previous = stage_index(&current.stage);
        if previous == Some(wanted) {
            return Ok(AdvanceOutcome::Unchanged(locked.delivered));
        }
        if previous.map_or(wanted != 0, |index| wanted != index + 1) || locked.delivered {
            return Ok(AdvanceOutcome::Conflict);
        }
        let description = match wanted {
            0 => "演示包裹已揽收",
            1 => "演示包裹运输中",
            2 => "演示包裹正在派送",
            3 => "演示包裹已签收",
            _ => unreachable!(),
        };
        diesel::sql_query(
            "INSERT INTO shipment_events (gift_id, stage, description, event_at) \
             VALUES ($1, $2, $3, GREATEST(now(), COALESCE( \
             (SELECT MAX(event_at) + INTERVAL '1 millisecond' \
              FROM shipment_events WHERE gift_id = $1), now())))",
        )
        .bind::<BigInt, _>(gift_id)
        .bind::<Text, _>(&body.stage)
        .bind::<Text, _>(description)
        .execute(conn)?;
        if wanted == 3 {
            diesel::sql_query(
                "UPDATE shipments SET delivered_at = (SELECT event_at FROM shipment_events \
                 WHERE gift_id = $1 AND stage = 'delivered') WHERE gift_id = $1",
            )
            .bind::<BigInt, _>(gift_id)
            .execute(conn)?;
        }
        Ok(AdvanceOutcome::Changed(wanted == 3))
    });
    match outcome {
        Ok(AdvanceOutcome::Changed(delivered)) => res.render(Json(json!({
            "gift_id": gift_id, "stage": body.stage, "carrier_delivered": delivered,
            "changed": true
        }))),
        Ok(AdvanceOutcome::Unchanged(delivered)) => res.render(Json(json!({
            "gift_id": gift_id, "stage": body.stage, "carrier_delivered": delivered,
            "changed": false
        }))),
        Ok(AdvanceOutcome::Conflict) => {
            error(res, StatusCode::CONFLICT, "invalid stage transition")
        }
        Ok(AdvanceOutcome::NotFound) => error(res, StatusCode::NOT_FOUND, "shipment not found"),
        Err(_) => error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "demo shipment update failed",
        ),
    }
}

pub fn routes() -> Router {
    Router::new().push(Router::with_path("api/v1/demo/shipments/{gift_id}/advance").post(advance))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(QueryableByName)]
    struct FixtureGift {
        #[diesel(sql_type = Text)]
        state: String,
        #[diesel(sql_type = Text)]
        recipient_identifier: String,
    }
    #[test]
    fn demo_gate_requires_every_guard() {
        let key = "0123456789abcdef0123456789abcdef";
        assert!(permitted("true", key, Some(key), true));
        assert!(!permitted("false", key, Some(key), true));
        assert!(!permitted("true", key, Some(key), false));
        assert!(!permitted("true", "short", Some("short"), true));
        assert!(!permitted("true", key, Some("wrong"), true));
        assert!(!permitted("true", key, None, true));
    }
    #[test]
    fn stage_order_is_explicit() {
        assert_eq!(
            STAGES,
            ["collected", "transit", "out_for_delivery", "delivered"]
        );
        assert_eq!(stage_index("signed"), None);
    }

    #[test]
    fn migrated_in_transit_fixture_has_correct_owner_and_state() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return; // The SQL migration itself asserts this invariant on every database.
        };
        let mut conn = diesel::PgConnection::establish(&url).expect("connect test database");
        let gift = diesel::sql_query(
            "SELECT g.state, u.identifier AS recipient_identifier \
             FROM gifts g JOIN users u ON u.id = g.recipient_id WHERE g.id = 900003",
        )
        .get_result::<FixtureGift>(&mut conn)
        .expect("in-transit fixture gift");
        assert_eq!(gift.state, "accepted");
        assert_eq!(gift.recipient_identifier, "demo@liyu.test");
    }
}
