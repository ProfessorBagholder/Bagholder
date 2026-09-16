//! The Rust store driven from the command line, so it can be compared with the
//! Python one on the same database and the same rows.
//!
//!     storetool ensure <db>      -- schema, migrations and relabelling
//!     storetool rows   <db>      -- every activity, as the model reads it
//!     storetool insert <db>      -- rows on stdin, inserted; the stored rows out
//!     storetool local  <db>      -- the same, through insert_local
//!     storetool keys             -- rows on stdin, their match keys out
//!
//! `insert` and `local` number any id they have to make `gen-0`, `gen-1`, ...
//! rather than drawing a UUID, so a comparison does not turn on randomness.

use serde_json::{json, Value};
use std::cell::Cell;
use std::io::Read;

use bagholder_store::activities as act;

fn stdin_json() -> Value {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    serde_json::from_str(&buf).unwrap()
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let path = std::env::args().nth(2).unwrap_or_default();

    if mode == "keys" {
        let rows: Vec<Value> = serde_json::from_value(stdin_json()).unwrap();
        let out: Vec<Value> = rows
            .iter()
            .map(|a| {
                let (d, ac, sym, q, px, cash) = act::field_match_key(a, true);
                let (lsym, side, lq, lpx, ld, lac) = act::link_match_key(a, true);
                let (nd, nac, nsym, nq, npx, ncash) = act::field_match_key(a, false);
                json!({
                    "fieldKey": [d, ac, sym, q, px, cash],
                    "fieldKeyNoAccount": [nd, nac, nsym, nq, npx, ncash],
                    "linkKey": [lsym, side, lq, lpx, ld, lac],
                    "tradeSide": act::trade_side(a),
                    "homemade": act::looks_like_homemade_id(&bagholder_model::value::field_s(a, "id")),
                    "realAccount": act::is_real_account(&{
                        let v = bagholder_model::value::field_s(a, "accountId");
                        if v.is_empty() { bagholder_model::value::field_s(a, "account_id") } else { v }
                    }),
                    "canonical": act::canonical_from_row(a, &{
                        let s = bagholder_model::value::field_s(a, "source");
                        if s.is_empty() { "wealthsimple".to_string() } else { s }
                    }),
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let conn = rusqlite::Connection::open(&path).unwrap();
    match mode.as_str() {
        "ensure" => bagholder_store::relabel::ensure(&conn).unwrap(),
        "rows" => {
            let rows = act::all_activities(&conn).unwrap();
            println!("{}", serde_json::to_string(&rows).unwrap());
        }
        "insert" | "local" => {
            let rows: Vec<Value> = serde_json::from_value(stdin_json()).unwrap();
            let n = Cell::new(0usize);
            let gen = || {
                let i = n.get();
                n.set(i + 1);
                format!("gen-{}", i)
            };
            let mut out = Vec::new();
            for a in &rows {
                let stored = if mode == "local" {
                    act::insert_local(&conn, a, &gen).unwrap()
                } else {
                    act::insert_activity(&conn, a, None, None, &gen).unwrap()
                };
                out.push(stored);
            }
            println!("{}", serde_json::to_string(&out).unwrap());
        }
        other => panic!("unknown mode {other}"),
    }
}
