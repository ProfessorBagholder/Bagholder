//! The Rust store driven from the command line, so it can be compared with the
//! Python one on the same database and the same rows.
//!
//!     storetool ensure <db>      -- schema, migrations and relabelling
//!     storetool rows   <db>      -- every activity, as the model reads it
//!     storetool insert <db>      -- rows on stdin, inserted; the stored rows out
//!     storetool local  <db>      -- the same, through insert_local
//!     storetool keys             -- rows on stdin, their match keys out
//!     storetool tables <db>      -- accounts/balances/margin/nav/fx/journal in and out
//!     storetool snapshot <db>    -- everything the model is built from
//!     storetool market <db>      -- fx, benchmarks, distributions and quotes
//!     storetool journal <db>     -- the v2 journal
//!     storetool merge  <db>      -- {ws, local} rows merged in; the counts and the table out
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
        // {accounts, balances, margin, nav, navUpsert, fx, benchmark, groups, notes, now}
        // written in that order, then read back.
        "tables" => {
            let doc = stdin_json();
            let a = |k: &str| -> Vec<Value> {
                doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default()
            };
            use bagholder_store::tables as t;
            let now = doc.get("now").and_then(|v| v.as_str()).unwrap_or("").to_string();
            t::replace_accounts(&conn, &a("accounts")).unwrap();
            t::replace_balances(&conn, &a("balances")).unwrap();
            t::replace_margin(&conn, &a("margin"), &now).unwrap();
            t::replace_nav(&conn, &a("nav")).unwrap();
            t::upsert_nav(&conn, &a("navUpsert")).unwrap();
            let fx_n = t::upsert_fx_rates(&conn, doc.get("fx"), t::FX_PAIR).unwrap();
            let bench_n = t::upsert_benchmark_prices(&conn, doc.get("benchmark"), t::BENCHMARK_SYMBOL).unwrap();
            let groups = t::save_trade_groups(&conn, doc.get("groups")).unwrap();
            let notes = t::save_trade_notes(&conn, doc.get("notes")).unwrap();
            println!("{}", serde_json::to_string(&json!({
                "fxWritten": fx_n,
                "benchWritten": bench_n,
                "savedGroups": groups,
                "savedNotes": notes,
                "accounts": t::accounts(&conn).unwrap(),
                "balances": t::balances(&conn).unwrap(),
                "margin": t::margin(&conn).unwrap(),
                "navAll": t::nav_history(&conn, "").unwrap(),
                "navLastDates": t::nav_last_dates(&conn).unwrap(),
                "fx": t::fx_rates(&conn, t::FX_PAIR).unwrap(),
                "fxLast": t::fx_last_date(&conn, t::FX_PAIR).unwrap(),
                "bench": t::benchmark_prices(&conn, t::BENCHMARK_SYMBOL).unwrap(),
                "benchLast": t::benchmark_last_date(&conn, t::BENCHMARK_SYMBOL).unwrap(),
                "benchDays": t::benchmark_days(&conn, t::BENCHMARK_SYMBOL, "2024-01-01", "2026-12-31").unwrap(),
                "groups": t::trade_groups(&conn).unwrap(),
                "notes": t::trade_notes(&conn).unwrap(),
            })).unwrap());
        }
        // {ws: [...], local: [...]} applied in that order
        "merge" => {
            let doc = stdin_json();
            let a = |k: &str| -> Vec<Value> { doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default() };
            // the tool is a fresh process per step, so the caller says where
            // the id sequence has got to
            let n = Cell::new(doc.get("idStart").and_then(|v| v.as_u64()).unwrap_or(0) as usize);
            let gen = || { let i = n.get(); n.set(i + 1); format!("gen-{}", i) };
            let applied = bagholder_store::merge::apply_wealthsimple_mapped(&conn, &a("ws"), &gen).unwrap();
            let merged = bagholder_store::merge::merge_local_rows(&conn, &a("local"), &gen).unwrap();
            println!("{}", serde_json::to_string(&json!({
                "applied": applied,
                "merged": merged,
                "rows": act::all_activities(&conn).unwrap(),
            })).unwrap());
        }
        // {orders: [...], orderPatches: [[id, patch]], brackets: [...],
        // bracketPatches: [[id, patch]], now} written then read back
        "orders" => {
            let doc = stdin_json();
            let now = doc.get("now").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let a = |k: &str| -> Vec<Value> { doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default() };
            use bagholder_store::orders as o;
            for row in a("orders") { o::insert_order(&conn, &row, &now).unwrap(); }
            for pair in a("orderPatches") {
                let id = pair.get(0).and_then(|v| v.as_str()).unwrap_or("");
                o::update_order(&conn, id, pair.get(1).unwrap_or(&Value::Null), &now).unwrap();
            }
            for b in a("brackets") { o::insert_bracket(&conn, &b, &now).unwrap(); }
            for pair in a("bracketPatches") {
                let id = pair.get(0).and_then(|v| v.as_str()).unwrap_or("");
                o::update_bracket(&conn, id, pair.get(1).unwrap_or(&Value::Null), &now).unwrap();
            }
            let booked: Vec<Value> = a("booked").iter().map(|p| {
                let id = p.get(0).and_then(|v| v.as_str()).unwrap_or("");
                let qty = p.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                json!(o::mark_order_fill_booked(&conn, id, qty, &now).unwrap())
            }).collect();
            let statuses: Vec<String> = a("statuses").iter().filter_map(|s| s.as_str().map(|x| x.to_string())).collect();
            println!("{}", serde_json::to_string(&json!({
                "orders": o::list_orders(&conn, 200).unwrap(),
                "brackets": o::list_brackets(&conn, &[]).unwrap(),
                "byStatus": o::list_brackets(&conn, &statuses).unwrap(),
                "booked": booked,
                "forOrder": o::bracket_for_order(&conn, doc.get("forOrder").and_then(|v| v.as_str()).unwrap_or("")).unwrap(),
                "symbolFor": o::symbol_for_security(&conn, doc.get("symbolFor").and_then(|v| v.as_str()).unwrap_or("")).unwrap(),
            })).unwrap());
        }
        "snapshot" => {
            bagholder_store::relabel::ensure(&conn).unwrap();
            println!("{}", serde_json::to_string(&bagholder_store::snapshot::snapshot(&conn, true).unwrap()).unwrap());
        }
        "market" => {
            println!("{}", serde_json::to_string(&bagholder_store::market::market_data(&conn).unwrap()).unwrap());
        }
        "journal" => {
            println!("{}", serde_json::to_string(&bagholder_store::snapshot::journal(&conn).unwrap()).unwrap());
        }
        other => panic!("unknown mode {other}"),
    }
}
