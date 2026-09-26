//! Public, read-only product catalog. IDs match `apps/liyu/src/data.rs::CATALOG`.
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Integer, Text};
use salvo::prelude::*;
use serde::Serialize;
use serde_json::json;

use crate::pool;

#[derive(QueryableByName)]
struct ProductRow {
    #[diesel(sql_type = Integer)]
    id: i32,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    category: String,
    #[diesel(sql_type = BigInt)]
    price_cents: i64,
    #[diesel(sql_type = Bool)]
    physical: bool,
    #[diesel(sql_type = Text)]
    brand: String,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    spec: String,
    #[diesel(sql_type = Text)]
    description: String,
    #[diesel(sql_type = Array<Text>)]
    tags: Vec<String>,
    #[diesel(sql_type = Integer)]
    stock: i32,
    #[diesel(sql_type = Bool)]
    is_active: bool,
}

#[derive(Serialize)]
struct Product {
    id: i32,
    name: String,
    category: String,
    price_cents: i64,
    physical: bool,
    brand: String,
    kind: String,
    spec: String,
    description: String,
    tags: Vec<String>,
    stock: i32,
    available: bool,
    image_thumb_url: String,
    image_card_url: String,
    image_detail_url: String,
}

impl From<ProductRow> for Product {
    fn from(r: ProductRow) -> Self {
        let base = format!("/api/v1/media/products/{}/", r.id);
        Self {
            id: r.id,
            name: r.name,
            category: r.category,
            price_cents: r.price_cents,
            physical: r.physical,
            brand: r.brand,
            kind: r.kind,
            spec: r.spec,
            description: r.description,
            tags: r.tags,
            stock: r.stock,
            available: r.is_active && r.stock > 0,
            image_thumb_url: format!("{base}thumb"),
            image_card_url: format!("{base}card"),
            image_detail_url: format!("{base}detail"),
        }
    }
}

const COLUMNS: &str = "id, name, category, price_cents, physical, brand, kind, spec, description, tags, stock, is_active";
const CATEGORIES: [(&str, &str); 8] = [
    ("coffee", "咖啡茶饮"),
    ("movie", "电影演出"),
    ("trendy", "潮流小物"),
    ("blind", "盲盒"),
    ("sweet", "甜点鲜花"),
    ("digital", "数码家电"),
    ("home", "家居生活"),
    ("baby", "母婴亲子"),
];

fn fail(res: &mut Response, status: StatusCode, code: &str) {
    res.status_code(status);
    res.render(Json(json!({"error": code})));
}

#[handler]
async fn categories(res: &mut Response) {
    res.render(Json(
        CATEGORIES
            .iter()
            .map(|(id, name)| json!({"id": id, "name": name}))
            .collect::<Vec<_>>(),
    ));
}

#[handler]
async fn list(req: &mut Request, res: &mut Response) {
    let category = req.query::<String>("category").unwrap_or_default();
    if !category.is_empty() && !CATEGORIES.iter().any(|(id, _)| *id == category) {
        return fail(res, StatusCode::BAD_REQUEST, "invalid_category");
    }
    let q = req.query::<String>("q").unwrap_or_default();
    if q.chars().count() > 100 {
        return fail(res, StatusCode::BAD_REQUEST, "query_too_long");
    }
    let cursor = match req.query::<String>("cursor").map(|s| s.parse::<i32>()) {
        None => -1,
        Some(Ok(c)) if c >= -1 => c,
        _ => return fail(res, StatusCode::BAD_REQUEST, "invalid_cursor"),
    };
    let limit = match req.query::<String>("limit").map(|s| s.parse::<i64>()) {
        None => 20,
        Some(Ok(n)) if (1..=50).contains(&n) => n,
        _ => return fail(res, StatusCode::BAD_REQUEST, "invalid_limit"),
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return fail(res, StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
    };
    let sql = format!("SELECT {COLUMNS} FROM catalog WHERE is_active AND id > $1 AND ($2 = '' OR category = $2) AND ($3 = '' OR name ILIKE '%' || $3 || '%' OR brand ILIKE '%' || $3 || '%' OR kind ILIKE '%' || $3 || '%' OR description ILIKE '%' || $3 || '%') ORDER BY id LIMIT $4");
    let rows = diesel::sql_query(sql)
        .bind::<Integer, _>(cursor)
        .bind::<Text, _>(category)
        .bind::<Text, _>(q.trim())
        .bind::<BigInt, _>(limit + 1)
        .load::<ProductRow>(&mut conn);
    match rows {
        Ok(mut rows) => {
            let has_more = rows.len() as i64 > limit;
            rows.truncate(limit as usize);
            let next_cursor = if has_more {
                rows.last().map(|p| p.id.to_string())
            } else {
                None
            };
            res.render(Json(json!({"items": rows.into_iter().map(Product::from).collect::<Vec<_>>(), "next_cursor": next_cursor})));
        }
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "catalog_query_failed",
        ),
    }
}

#[handler]
async fn detail(req: &mut Request, res: &mut Response) {
    let Some(product_id) = req.param::<i32>("id") else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid_product_id");
    };
    let mut conn = match pool().get() {
        Ok(conn) => conn,
        Err(_) => return fail(res, StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
    };
    let sql = format!("SELECT {COLUMNS} FROM catalog WHERE id = $1 AND is_active");
    match diesel::sql_query(sql)
        .bind::<Integer, _>(product_id)
        .get_result::<ProductRow>(&mut conn)
    {
        Ok(row) => res.render(Json(Product::from(row))),
        Err(diesel::result::Error::NotFound) => {
            fail(res, StatusCode::NOT_FOUND, "product_not_found")
        }
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "catalog_query_failed",
        ),
    }
}

#[handler]
async fn image(req: &mut Request, res: &mut Response) {
    let Some(id) = req.param::<i32>("id") else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid_product_id");
    };
    if !(0..33).contains(&id) {
        return fail(res, StatusCode::NOT_FOUND, "image_not_found");
    }
    let Some(variant) = req.param::<String>("variant") else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid_variant");
    };
    if !matches!(variant.as_str(), "thumb" | "card" | "detail") {
        return fail(res, StatusCode::NOT_FOUND, "image_not_found");
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/products");
    let detail_path = dir.join(format!("p{id:02}-detail.png"));
    let path = if variant == "detail" && detail_path.is_file() {
        detail_path
    } else {
        dir.join(format!("p{id:02}.png"))
    };
    match std::fs::read(path) {
        Ok(data) => {
            res.headers_mut()
                .insert("content-type", "image/png".parse().unwrap());
            res.headers_mut()
                .insert("cache-control", "public, max-age=86400".parse().unwrap());
            let _ = res.write_body(data);
        }
        Err(_) => fail(res, StatusCode::NOT_FOUND, "image_not_found"),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(Router::with_path("api/v1/catalog/categories").get(categories))
        .push(Router::with_path("api/v1/catalog").get(list))
        .push(Router::with_path("api/v1/catalog/{id}").get(detail))
        .push(Router::with_path("api/v1/media/products/{id}/{variant}").get(image))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_seeded_product_has_baseline_image() {
        for id in 0..33 {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/products")
                .join(format!("p{id:02}.png"));
            assert!(p.exists(), "missing {}", p.display());
        }
    }

    #[test]
    fn every_category_has_a_lifestyle_detail_image() {
        let examples = [0, 4, 5, 8, 11, 12, 22, 27];
        assert_eq!(examples.len(), 8);
        for id in examples {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/products")
                .join(format!("p{id:02}-detail.png"));
            assert!(p.exists(), "missing {}", p.display());
        }
    }
}
