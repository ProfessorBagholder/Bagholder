//! Reads JSON on stdin and writes what the Rust model makes of it, so the
//! Python model and this one can be compared on the same input.
//!
//!     difftool rows   -- activity rows in, the normalized rows and symbol readings out
//!     difftool fifo   -- activity rows in, match_fifo's result out
//!     difftool when   -- ISO instants in, the local date and time out
//!     difftool book   -- {snapshot, today, fx} in, build_book plus apply_fx out
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "rows".into());
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let input: Value = serde_json::from_str(&buf).unwrap();

    let out = match mode.as_str() {
        "when" => {
            let rows: Vec<String> = serde_json::from_value(input).unwrap();
            Value::Array(rows.iter().map(|r| {
                let (d, t) = bagholder_model::clock::when_parts(r);
                json!([d, t])
            }).collect())
        }
        "fifo" => {
            let rows: Vec<Value> = serde_json::from_value(input).unwrap();
            let m = bagholder_model::fifo::match_fifo(&rows);
            json!({"closed": m.closed, "open": m.open, "unmatched": m.unmatched})
        }
        "book" => {
            let snapshot = input.get("snapshot").cloned().unwrap_or(Value::Null);
            let today = input.get("today").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let fx: bagholder_model::fx::Fx = input
                .get("fx")
                .and_then(|v| v.as_object())
                .map(|m| m.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f))).collect())
                .unwrap_or_default();
            let mut b = bagholder_model::book::build_book(&snapshot, &today);
            bagholder_model::fx::apply_fx(&mut b.fifo.closed, &fx);
            let saved: Vec<Value> = snapshot.get("tradeGroups").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let journal = input.get("journal").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let trades = bagholder_model::trades::build_trades(&b.fifo.closed, &saved, &b.acts_by_id, &b.securities, &journal);
            let last_prices = bagholder_model::trades::last_fill_prices(&b.activities);
            json!({
                "trades": trades,
                "lastPrices": last_prices,
                "activities": b.activities,
                "closed": b.fifo.closed,
                "open": b.fifo.open,
                "unmatched": b.fifo.unmatched,
                "rawCount": b.raw_count,
                "cashCurrencies": b.securities.cash_currencies(),
                "knownExchanges": b.securities.known_exchanges(),
            })
        }
        _ => {
            let rows: Vec<Value> = serde_json::from_value(input).unwrap();
            Value::Array(
                rows.iter()
                    .map(|a| {
                        let sym = bagholder_model::value::field_s(a, "symbol");
                        json!({
                            "normalized": bagholder_model::normalize::normalize_activity(a),
                            "kind": bagholder_model::normalize::kind_of(a),
                            "isOption": bagholder_model::symbols::is_option_symbol(&sym),
                            "underlying": bagholder_model::symbols::underlying_symbol(&sym),
                            "right": bagholder_model::symbols::option_right(&sym),
                            "mult": bagholder_model::symbols::option_multiplier(&sym),
                            "closeOnly": bagholder_model::normalize::is_close_only(a),
                            "open": bagholder_model::normalize::is_intentional_open(a),
                            "fifoAccount": bagholder_model::normalize::fifo_account(a),
                            "bookKey": bagholder_model::normalize::book_key(a),
                            "expiry": bagholder_model::dates::option_expiry(&sym),
                            "dirBuy": bagholder_model::normalize::opening_direction(a, "BUY"),
                            "dirSell": bagholder_model::normalize::opening_direction(a, "SELL"),
                        })
                    })
                    .collect(),
            )
        }
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}
