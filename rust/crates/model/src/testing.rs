//! For tests, here and in the crates that read the model: rows as the JSON the
//! page is sent (what most tests assert on), and a holding sketched from a few
//! fields (what a test of one function needs of it).

use serde::Serialize;
use serde_json::Value;

use crate::activity::Kind;
use crate::value::{field_num, field_s};
use crate::wire::{Mark, Position};

/// Rows as the page is sent them.
pub fn sent<T: Serialize>(rows: &[T]) -> Vec<Value> {
    rows.iter().map(|r| serde_json::to_value(r).expect("a row of the model is plain data")).collect()
}

/// A holding with the fields given and nothing else known.
pub fn holding(fields: Value) -> Position {
    let text = |k: &str| field_s(&fields, k);
    Position {
        id: text("id"),
        symbol: text("symbol"),
        underlying: text("underlying"),
        name: text("name"),
        exchange: text("exchange"),
        kind: Kind::parse(&text("kind")).unwrap_or(Kind::Shares),
        account: text("account"),
        account_id: text("accountId"),
        currency: text("currency"),
        security_id: text("securityId"),
        short: fields.get("short").and_then(Value::as_bool).unwrap_or(false),
        qty: field_num(&fields, "qty"),
        mult: 1,
        avg: field_num(&fields, "avg"),
        cost: field_num(&fields, "cost"),
        fees: 0.0,
        last: field_num(&fields, "last"),
        price_source: Mark::Fill,
        price_change: None,
        percent_change: fields.get("percentChange").and_then(Value::as_f64),
        day_change: fields.get("dayChange").and_then(Value::as_f64),
        mv: field_num(&fields, "mv"),
        unreal: field_num(&fields, "unreal"),
        unreal_pct: None,
        held: 0,
        opened: text("opened"),
        ws_qty: None,
        rt: None,
        lots: vec![],
        fills: None,
        grade: String::new(),
        thesis: String::new(),
        tags: vec![],
        alloc: 0.0,
    }
}
