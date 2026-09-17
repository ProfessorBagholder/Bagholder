//! The exposure readers and symbol search, on fixtures or live.
use serde_json::{json, Value};
use std::io::Read;

fn st(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::{exposure as e, search as s};
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();

    if mode == "live" {
        let db = std::path::PathBuf::from(st(&doc["db"]));
        let conn = rusqlite::Connection::open(&db).unwrap();
        bagholder_store::relabel::ensure(&conn).unwrap();
        let (today, _, _) = bagholder_market::clock_now();
        let ctx = e::Ctx { conn: &conn, db: db.clone(), today };
        let recs: Vec<Value> = arr("securities").iter().map(|sec| e::refresh_security(&ctx, sec)).collect();
        let shares: Vec<Value> = arr("shares").iter().map(|r| e::share_exposure(&ctx, &st(&r[0]), &st(&r[1]), &st(&r[2]))).collect();
        let searches: Vec<Value> = arr("searches").iter().map(|t| s::symbol_search(&db, &st(t))).collect();
        println!("{}", serde_json::to_string(&json!({"securities": recs, "shares": shares, "searches": searches})).unwrap());
        return;
    }
    let tables: Vec<Value> = arr("html").iter().map(|h| json!(bagholder_market::htmltables::html_tables(&st(h)))).collect();
    let harvest: Vec<Value> = arr("html").iter().map(|h| {
        let (hold, r) = e::parse_harvest_tables(&bagholder_market::htmltables::html_tables(&st(h)));
        json!([hold, r])
    }).collect();
    let ninepoint: Vec<Value> = arr("html").iter().map(|h| {
        let (a, b, c) = e::parse_ninepoint_page(&st(h));
        json!([a, b, c])
    }).collect();
    let evolve: Vec<Value> = arr("html").iter().map(|h| {
        let (sec, hold) = e::parse_evolve_page(&st(h));
        json!([sec, hold])
    }).collect();
    let ishares: Vec<Value> = arr("csv").iter().map(|t| {
        let (h, a) = e::parse_ishares_csv(&st(t));
        json!([h, a])
    }).collect();
    let yahoo: Vec<Value> = arr("yahoo").iter().map(|d| {
        let (sec, hold) = e::parse_yahoo_summary(d);
        json!([sec, hold])
    }).collect();
    let countries: Vec<Value> = arr("countries").iter().map(|c| json!([e::norm_country(&st(c)), e::venue_country(&st(c))])).collect();
    let nasdaq: Vec<Value> = arr("nasdaq").iter().map(|d| json!(s::parse_nasdaq_search(d))).collect();
    let tsx: Vec<Value> = arr("tsx").iter().map(|d| json!(s::parse_tsx_search(d, "TSX"))).collect();
    let ranks: Vec<Value> = arr("ranks").iter().map(|r| json!(s::rank_search(&st(&r[0]), r[1].as_array().cloned().unwrap_or_default()))).collect();
    println!("{}", serde_json::to_string(&json!({
        "tables": tables, "harvest": harvest, "ninepoint": ninepoint, "evolve": evolve, "ishares": ishares, "yahoo": yahoo,
        "countries": countries, "nasdaq": nasdaq, "tsx": tsx, "ranks": ranks,
    })).unwrap());
}
