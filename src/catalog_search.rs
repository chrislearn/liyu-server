//! Narrow, public read-only tools for gift selection. No model-supplied SQL or URLs.
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Jsonb, Text};
use salvo::oapi::{extract::QueryParam, ToSchema};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Delivery {
    Any,
    Physical,
    Electronic,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchConstraints {
    /// Per-item price ceiling in cents. Zero explicitly means no ceiling.
    pub max_price_cents: i64,
    /// Exact catalog kind values; empty permits any kind. Different from broad categories.
    pub allowed_kinds: Vec<String>,
    /// Exact catalog kind values to exclude; never overridden by q or pagination.
    pub excluded_kinds: Vec<String>,
    pub delivery: Delivery,
}
impl SearchConstraints {
    fn validate(&self) -> Result<(), &'static str> {
        if !(0..=99_999_999).contains(&self.max_price_cents) {
            return Err("invalid_budget");
        }
        for values in [&self.allowed_kinds, &self.excluded_kinds] {
            if values.len() > 12
                || values
                    .iter()
                    .any(|s| s.is_empty() || s.chars().count() > 40)
            {
                return Err("invalid_kinds");
            }
            let unique: std::collections::HashSet<_> = values.iter().collect();
            if unique.len() != values.len() {
                return Err("duplicate_kind");
            }
        }
        // Contradictory filters fail explicitly; never silently broaden to all kinds.
        if self
            .allowed_kinds
            .iter()
            .any(|k| self.excluded_kinds.contains(k))
        {
            return Err("contradictory_kinds");
        }
        Ok(())
    }
}
#[derive(Serialize, ToSchema)]
struct SearchPage {
    items: Vec<crate::catalog::Product>,
    next_cursor: Option<String>,
    /// Total for locked hard constraints across the entire current catalog, ignoring soft q and cursor.
    eligible_total: i64,
    /// Total for locked hard constraints plus this optional soft query, ignoring cursor.
    total_matches: i64,
    /// Content revision; discard accumulated pages if it changes between requests.
    catalog_revision: String,
    applied: SearchConstraints,
}
#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}
#[derive(QueryableByName)]
struct Kind {
    #[diesel(sql_type = Text)]
    kind: String,
}
#[derive(QueryableByName)]
struct Snapshot {
    #[diesel(sql_type = Jsonb)]
    data: serde_json::Value,
}
fn bad(res: &mut Response, status: StatusCode, error: &str) {
    res.status_code(status);
    res.render(Json(json!({"error":error})));
}

/// Query available, in-stock products under immutable AI-interpreted constraints.
///
/// `filter` is JSON conforming to SearchConstraints in components.schemas. The caller
/// must lock it after interpretation within each search scope and reuse it byte-for-byte in every round.
/// q is optional literal text, not SQL or a wildcard; it may change without changing
/// hard constraints. An empty result only proves no eligible product when
/// eligible_total is zero. Cursor/soft-query emptiness is not a no-match proof.
#[endpoint(responses((status_code=200, body=SearchPage), (status_code=400, description="Invalid or contradictory constraints"), (status_code=503, description="Database unavailable")))]
async fn search(
    filter: QueryParam<String, true>,
    q: QueryParam<String, false>,
    cursor: QueryParam<String, false>,
    limit: QueryParam<String, false>,
    res: &mut Response,
) {
    let raw = filter.into_inner();
    if raw.len() > 1600 {
        return bad(res, StatusCode::BAD_REQUEST, "filter_too_long");
    }
    let constraints: SearchConstraints = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return bad(res, StatusCode::BAD_REQUEST, "invalid_filter"),
    };
    if let Err(e) = constraints.validate() {
        return bad(res, StatusCode::BAD_REQUEST, e);
    }
    let q = q.into_inner().unwrap_or_default().trim().to_string();
    if q.chars().count() > 100 {
        return bad(res, StatusCode::BAD_REQUEST, "query_too_long");
    }
    let cursor = match cursor.into_inner().map(|v| v.parse::<i32>()) {
        None => -1,
        Some(Ok(v)) if v >= -1 => v,
        _ => return bad(res, StatusCode::BAD_REQUEST, "invalid_cursor"),
    };
    let limit = match limit.into_inner().map(|v| v.parse::<i64>()) {
        None => 12,
        Some(Ok(v)) if (1..=20).contains(&v) => v,
        _ => return bad(res, StatusCode::BAD_REQUEST, "invalid_limit"),
    };
    let mut conn = match crate::pool().get() {
        Ok(c) => c,
        Err(_) => return bad(res, StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
    };
    let delivery = match constraints.delivery {
        Delivery::Any => "any",
        Delivery::Physical => "physical",
        Delivery::Electronic => "electronic",
    };
    let escaped = q
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = if q.is_empty() {
        String::new()
    } else {
        format!("%{escaped}%")
    };
    let hard="is_active AND stock>0 AND ($1=0 OR price_cents<=$1) AND (cardinality($2)=0 OR kind=ANY($2)) AND NOT(kind=ANY($3)) AND ($4='any' OR ($4='physical' AND physical) OR ($4='electronic' AND NOT physical))";
    let soft="($5='' OR name ILIKE $5 OR brand ILIKE $5 OR kind ILIKE $5 OR spec ILIKE $5 OR description ILIKE $5)";
    let result=conn.build_transaction().repeatable_read().read_only().run::<_,diesel::result::Error,_>(|conn|{
        let kinds=diesel::sql_query("SELECT DISTINCT kind FROM catalog WHERE is_active ORDER BY kind").load::<Kind>(conn)?;
        if constraints.allowed_kinds.iter().chain(&constraints.excluded_kinds).any(|k|!kinds.iter().any(|v|v.kind==*k)){return Ok(None);}
        let eligible=diesel::sql_query(format!("SELECT count(*) AS n FROM catalog WHERE {hard}"))
            .bind::<BigInt,_>(constraints.max_price_cents).bind::<Array<Text>,_>(&constraints.allowed_kinds).bind::<Array<Text>,_>(&constraints.excluded_kinds).bind::<Text,_>(delivery).get_result::<Count>(conn)?.n;
        let total=diesel::sql_query(format!("SELECT count(*) AS n FROM catalog WHERE {hard} AND {soft}"))
            .bind::<BigInt,_>(constraints.max_price_cents).bind::<Array<Text>,_>(&constraints.allowed_kinds).bind::<Array<Text>,_>(&constraints.excluded_kinds).bind::<Text,_>(delivery).bind::<Text,_>(&pattern).get_result::<Count>(conn)?.n;
        let mut rows=diesel::sql_query(format!("SELECT {} FROM catalog WHERE {hard} AND {soft} AND id>$6 ORDER BY id LIMIT $7",crate::catalog::COLUMNS))
            .bind::<BigInt,_>(constraints.max_price_cents).bind::<Array<Text>,_>(&constraints.allowed_kinds).bind::<Array<Text>,_>(&constraints.excluded_kinds).bind::<Text,_>(delivery).bind::<Text,_>(&pattern).bind::<diesel::sql_types::Integer,_>(cursor).bind::<BigInt,_>(limit+1).load::<crate::catalog::ProductRow>(conn)?;
        let snapshot=diesel::sql_query("SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY id),'[]'::jsonb) AS data FROM catalog c").get_result::<Snapshot>(conn)?;
        let has_more=rows.len() as i64>limit;rows.truncate(limit as usize);
        let next_cursor=if has_more{rows.last().map(|p|p.id.to_string())}else{None};
        Ok(Some(SearchPage{items:rows.into_iter().map(crate::catalog::Product::from).collect(),next_cursor,eligible_total:eligible,total_matches:total,catalog_revision:Sha256::digest(snapshot.data.to_string().as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect(),applied:constraints.clone()}))
    });
    match result {
        Ok(Some(page)) => res.render(Json(page)),
        Ok(None) => bad(res, StatusCode::BAD_REQUEST, "unknown_kind"),
        Err(_) => bad(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "catalog_query_failed",
        ),
    }
}

/// Read the exact kind vocabulary and the sole allowed AI query operation.
#[endpoint]
async fn options(res: &mut Response) {
    let mut conn = match crate::pool().get() {
        Ok(c) => c,
        Err(_) => return bad(res, StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
    };
    let kinds =
        match diesel::sql_query("SELECT DISTINCT kind FROM catalog WHERE is_active ORDER BY kind")
            .load::<Kind>(&mut conn)
        {
            Ok(v) => v.into_iter().map(|v| v.kind).collect::<Vec<_>>(),
            Err(_) => {
                return bad(
                    res,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "catalog_query_failed",
                )
            }
        };
    let doc = serde_json::to_value(document()).expect("OpenAPI serializable");
    res.render(Json(json!({"version":1,"kinds":kinds,"max_rounds":4,"openapi_path":"/api/v1/catalog/openapi","tool":{"name":"catalog_search","method":"GET","path":"/api/v1/catalog/search","operation":doc["paths"]["/api/v1/catalog/search"]["get"],"constraints_schema":doc["components"]["schemas"]["SearchConstraints"],"delivery_schema":doc["components"]["schemas"]["Delivery"]}})));
}
fn tool_routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/catalog/search").get(search))
        .push(Router::with_path("api/v1/catalog/search-options").get(options))
}
fn document() -> OpenApi {
    use salvo::oapi::naming::{assign_name, NameRule};
    assign_name::<SearchConstraints>(NameRule::Force("SearchConstraints"));
    assign_name::<Delivery>(NameRule::Force("Delivery"));
    OpenApi::new(
        "LIYU read-only gift catalog tools",
        env!("CARGO_PKG_VERSION"),
    )
    .merge_router(&tool_routes())
}
pub fn routes() -> Router {
    tool_routes().push(document().into_router("api/v1/catalog/openapi"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn filter() -> SearchConstraints {
        SearchConstraints {
            max_price_cents: 3000,
            allowed_kinds: vec!["咖啡".into()],
            excluded_kinds: vec!["奶茶".into()],
            delivery: Delivery::Any,
        }
    }
    #[test]
    fn strict_constraint_contract() {
        let mut f = filter();
        assert!(f.validate().is_ok());
        f.max_price_cents = -1;
        assert!(f.validate().is_err());
        f.max_price_cents = 3000;
        f.excluded_kinds.push("咖啡".into());
        assert!(f.validate().is_err());
        assert!(serde_json::from_value::<SearchConstraints>(json!({"max_price_cents":3000,"allowed_kinds":[],"excluded_kinds":[],"delivery":"any","ignore_stock":true})).is_err());
    }
    #[test]
    fn openapi_has_only_read_tools() {
        let d = serde_json::to_value(document()).unwrap();
        assert!(d["paths"]["/api/v1/catalog/search"]["get"].is_object());
        assert!(d["components"]["schemas"]["SearchConstraints"].is_object());
        assert!(d["components"]["schemas"]["Delivery"].is_object());
        assert_eq!(d["paths"].as_object().unwrap().len(), 2);
    }
}
