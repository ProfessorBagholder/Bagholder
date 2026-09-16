//! Reads activity rows as JSON on stdin, writes the normalized rows and the
//! symbol readings as JSON on stdout, so the Python model and this one can be
//! compared on the same input.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let rows: Vec<Value> = serde_json::from_str(&buf).unwrap();
    let out: Vec<Value> = rows
        .iter()
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
                "dirBuy": bagholder_model::normalize::opening_direction(a, "BUY"),
                "dirSell": bagholder_model::normalize::opening_direction(a, "SELL"),
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}
