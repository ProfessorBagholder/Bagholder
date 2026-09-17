//! The intraday chain and the remaining market readers, on fixtures or live.
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
    use bagholder_market::history as h;
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let today = st(&doc["today"]);

    if mode == "live" {
        let conn = rusqlite::Connection::open(st(&doc["db"])).unwrap();
        bagholder_store::relabel::ensure(&conn).unwrap();
        let (_, now_unix, stamp) = bagholder_market::clock_now();
        let bars: Vec<Value> = arr("intraday").iter().map(|c| {
            let got = h::ensure_bars(&conn, &c["rec"], &st(&c["tf"]), &st(&c["start"]), &st(&c["end"]), &today, now_unix, &stamp).unwrap();
            json!({"bars": got, "offered": h::offered_timeframes(&conn, &c["rec"], &st(&c["start"]), &today, now_unix),
                   "ready": h::intraday_ready(&conn, &c["rec"], &st(&c["tf"]), &st(&c["start"]), &today, now_unix),
                   "reason": h::chart_reason(&c["rec"], &st(&c["tf"]))})
        }).collect();
        let dist = bagholder_market::refresh::refresh_distributions(&conn, &arr("payers"), true);
        let prev: Vec<Value> = arr("pairs").iter().map(|p| json!(bagholder_market::quotes::coinbase_prev_close(&conn, &st(p), &today, now_unix))).collect();
        let peeks: Vec<Value> = arr("peeks").iter().map(|r| json!(bagholder_market::quotes::peek_quote(&conn, r, &today))).collect();
        let archived = h::archive_intraday(&conn, &arr("archive"), &today, now_unix, &stamp, 12);
        println!("{}", serde_json::to_string(&json!({"intraday": bars, "distributions": dist, "prev": prev, "peeks": peeks, "archived": archived})).unwrap());
        return;
    }

    let stamps: Vec<Value> = arr("stamps").iter().map(|s| json!(h::minute_stamp(&st(s)).map(|(a, b, c, d)| json!([a, b, c, d])))).collect();
    let minutes: Vec<Value> = arr("tmx").iter().map(|d| json!(h::parse_tmx_minutes(d))).collect();
    let sessions: Vec<Value> = arr("sessions").iter().map(|c| {
        let bars = c["bars"].as_array().cloned().unwrap_or_default();
        json!(h::aggregate_session(&bars, c["bucket"].as_i64().unwrap_or(60)))
    }).collect();
    let hourly: Vec<Value> = arr("hourly").iter().map(|c| {
        let bars = c["bars"].as_array().cloned().unwrap_or_default();
        json!(h::aggregate_hourly(&bars, c["seconds"].as_i64().unwrap_or(14400)))
    }).collect();
    let reach: Vec<Value> = arr("recs").iter().map(|r| json!({
        "reach": h::intraday_reach(r, &today),
        "available": h::available_timeframes(r, &st(&r["start"]), &today),
    })).collect();
    let roots: Vec<Value> = arr("occ").iter().map(|c| json!(bagholder_market::quotes::occ_root(&st(c)))).collect();
    let reprs: Vec<Value> = arr("floats").iter().map(|f| json!(bagholder_market::quotes::py_repr_float(f.as_f64().unwrap_or(0.0)))).collect();
    println!("{}", serde_json::to_string(&json!({
        "stamps": stamps, "tmx": minutes, "sessions": sessions, "hourly": hourly, "recs": reach, "occ": roots, "floats": reprs,
    })).unwrap());
}
