//! What the book paid out: `model.build_cashflow`.
//!
//! Dividends, interest, withholding tax and interest charges, each in its own
//! currency with the CAD value converted on the day it was paid.

use serde_json::{json, Value};

use crate::clock::when_parts;
use crate::fx::{to_cad, Fx};
use crate::securities::Securities;
use crate::value::{compact, field_num, field_s, norm_account_name, EPS};

pub fn build_cashflow(activities: &[Value], securities: &Securities, fx: &Fx) -> Vec<Value> {
    let mut rows: Vec<Value> = Vec::new();
    for a in activities {
        let cat = field_s(a, "category");
        let raw = compact(&field_s(a, "rawType"));
        let at = compact(&field_s(a, "activityType"));
        let cash = field_num(a, "netCashAmount");

        let kind = if cat == "dividend" {
            "Dividend"
        } else if cat == "interest" {
            "Interest"
        } else if raw == "WITHHOLDINGTAX" || at == "WITHHOLDINGTAX" {
            "Withholding tax"
        } else if raw == "INTERESTCHARGE" || at == "INTERESTCHARGE" {
            "Interest charge"
        } else {
            continue;
        };
        if cash.abs() < EPS {
            continue;
        }

        let occurred = { let o = field_s(a, "occurredAt"); if o.is_empty() { field_s(a, "transactionDate") } else { o } };
        let (day, clock) = when_parts(&occurred);
        let raw_symbol = field_s(a, "symbol").trim().to_string();
        let symbol = if !raw_symbol.is_empty() {
            raw_symbol
        } else if kind == "Interest" || kind == "Interest charge" {
            "Cash".to_string()
        } else {
            String::new()
        };

        let name_field = field_s(a, "name");
        let fallback = if name_field != symbol { name_field } else { String::new() };
        let account = {
            let n = norm_account_name(&field_s(a, "accountType"));
            if n.is_empty() { field_s(a, "accountId") } else { n }
        };
        let date = { let d = field_s(a, "transactionDate"); if d.is_empty() { day } else { d } };
        let currency = { let c = field_s(a, "currency"); if c.is_empty() { "CAD".to_string() } else { c } };

        // Python's `_num(...) or None`: a zero reads as nothing to show.
        let qty = field_num(a, "quantity");
        let per = field_num(a, "unitPrice");

        rows.push(json!({
            "id": field_s(a, "id"),
            "date": date,
            "time": clock,
            "symbol": if symbol.is_empty() { "—".to_string() } else { symbol.clone() },
            "name": securities.name(&field_s(a, "securityId"), &fallback),
            "kind": kind,
            "account": account,
            "accountId": field_s(a, "accountId"),
            "qty": if qty == 0.0 { Value::Null } else { json!(qty) },
            "per": if per == 0.0 { Value::Null } else { json!(per) },
            "amount": cash,
            "currency": currency,
            "amountCad": to_cad(fx, cash, &field_s(a, "currency"), &field_s(a, "transactionDate")),
        }));
    }
    rows.sort_by(|a, b| {
        (field_s(b, "date"), field_s(b, "id")).cmp(&(field_s(a, "date"), field_s(a, "id")))
    });
    rows
}
