//! Runs a market top-up against a database and reports what it wrote, so the
//! two implementations can be compared on the same starting point.
use serde_json::json;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let path = std::env::args().nth(2).expect("db path");
    let conn = rusqlite::Connection::open(&path).unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    let out = match mode.as_str() {
        "fx" => json!({"fx": bagholder_market::refresh::refresh_fx(&conn)}),
        "benchmark" => json!({"benchmark": bagholder_market::refresh::refresh_benchmark(&conn)}),
        "tsx" => json!({"tsx": bagholder_market::refresh::refresh_tsx(&conn)}),
        "all" => bagholder_market::refresh::refresh_all(&conn, &[]),
        "health" => json!(bagholder_market::http::source_health()),
        other => panic!("unknown mode {other}"),
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}
