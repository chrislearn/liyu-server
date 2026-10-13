//! Optional descending-ID pages, bounded below the native host's 60 KB response limit.
use salvo::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub(crate) struct Page {
    before: i64,
    limit: Option<usize>,
}
impl Page {
    pub fn read(req: &Request) -> Result<Self, &'static str> {
        let limit = req.query::<String>("limit");
        let before = req.query::<String>("before");
        if limit.is_none() && before.is_some() {
            return Err("pagination requires limit");
        }
        let limit = limit
            .map(|n| n.parse::<usize>().ok().filter(|n| (1..=20).contains(n)))
            .map(|n| n.ok_or("limit must be 1–20"))
            .transpose()?;
        let before = before
            .map(|n| n.parse::<i64>().ok().filter(|n| *n >= 0))
            .map(|n| n.ok_or("invalid before cursor"))
            .transpose()?
            .unwrap_or(0);
        Ok(Self { before, limit })
    }
    pub fn sql(&self, column: &str, legacy_order: &str) -> String {
        match self.limit {
            None => legacy_order.into(),
            Some(limit) => format!(
                "AND ({}=0 OR {column}<{}) ORDER BY {column} DESC LIMIT {}",
                self.before,
                self.before,
                limit + 1
            ),
        }
    }
    pub fn render(&self, res: &mut Response, rows: impl Serialize, id_field: &str) {
        let values = serde_json::to_value(rows).expect("query rows serialize");
        if self.limit.is_none() {
            res.render(Json(values));
            return;
        }
        match self.pack(
            values.as_array().expect("query rows are an array"),
            id_field,
        ) {
            Ok(page) => res.render(Json(page)),
            Err(message) => crate::error(res, StatusCode::PAYLOAD_TOO_LARGE, message),
        }
    }
    pub fn bounded(rows: &[Value], limit: usize, id_field: &str) -> Result<Value, &'static str> {
        Self {
            before: 0,
            limit: Some(limit),
        }
        .pack(rows, id_field)
    }
    fn pack(&self, rows: &[Value], id_field: &str) -> Result<Value, &'static str> {
        let mut items = Vec::new();
        let mut bytes = 0;
        for row in rows.iter().take(self.limit.unwrap_or(20)) {
            let length = serde_json::to_vec(row).expect("JSON row").len() + 1;
            if bytes + length > 48_000 {
                break;
            }
            bytes += length;
            items.push(row.clone());
        }
        if !rows.is_empty() && items.is_empty() {
            return Err("one list item exceeds host limit; use a shorter summary");
        }
        let next = if items.len() < rows.len() {
            items
                .last()
                .and_then(|row| row[id_field].as_i64())
                .map(|id| id.to_string())
        } else {
            None
        };
        Ok(json!({"items": items, "next_cursor": next}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_bound_keeps_a_cursor_instead_of_dropping_large_rows() {
        let rows: Vec<_> = (1..=21)
            .rev()
            .map(|id| json!({"id":id,"note":"字".repeat(2000)}))
            .collect();
        let page = Page {
            before: 0,
            limit: Some(20),
        }
        .pack(&rows, "id")
        .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 60_000);
        assert_eq!(
            page["next_cursor"],
            page["items"].as_array().unwrap().last().unwrap()["id"]
                .as_i64()
                .unwrap()
                .to_string()
        );
        assert!(Page {
            before: 0,
            limit: Some(1)
        }
        .pack(&[json!({"id":1,"note":"字".repeat(20000)})], "id")
        .is_err());
    }
}
