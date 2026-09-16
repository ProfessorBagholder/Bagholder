//! Reads JSON on stdin and writes what the Rust model makes of it, so the
//! Python model and this one can be compared on the same input.
//!
//!     difftool rows   -- activity rows in, the normalized rows and symbol readings out
//!     difftool fifo   -- activity rows in, match_fifo's result out
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "rows".into());
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let rows: Vec<Value> = serde_json::from_str(&buf).unwrap();

    let out = match mode.as_str() {
        "fifo" => {
            let m = bagholder_model::fifo::match_fifo(&rows);
            json!({"closed": m.closed, "open": m.open, "unmatched": m.unmatched})
        }
        _ => Value::Array(
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
        ),
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}
