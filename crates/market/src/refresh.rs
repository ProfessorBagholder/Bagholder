//! Topping up the series the model reads: the Bank of Canada's USD/CAD, the
//! S&P 500's closes, and the two TMX indices.
//!
//! Every one of these appends from a week before the newest stored day, so a
//! revision at the edge of the series is picked up without refetching history,
//! and none of them ever raises: a source that is down leaves what is stored.

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::http::{get_text, post_json, TMX_HEADERS};
use crate::parse::{parse_boc_json, parse_fred_csv, parse_stooq_csv, parse_tmx_history, Series};
use bagholder_model::dates::{parse_iso, shift_date};
use bagholder_store::tables::{
    benchmark_last_date, fx_last_date, upsert_benchmark_prices, upsert_fx_rates, BENCHMARK_SYMBOL, FX_PAIR,
};

pub const BOC_URL: &str = "https://www.bankofcanada.ca/valet/observations/FXUSDCAD/json";
pub const FRED_URL: &str = "https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500";
pub const STOOQ_URL: &str = "https://stooq.com/q/d/l/?s=^spx&i=d";
pub const TMX_URL: &str = "https://app-money.tmx.com/graphql";

pub const FX_START: &str = "2016-01-01";
pub const TSX_START: &str = "2016-01-01";

/// `market.TMX_INDICES`: the stored benchmark key -> TMX Money's index symbol.
pub const TMX_INDICES: [(&str, &str); 2] = [("TSX", "^TSX"), ("TSX60", "^TX60")];

const TMX_HISTORY_QUERY: &str = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }";

fn series_to_value(s: &Series) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in s {
        m.insert(k.clone(), json!(v));
    }
    Value::Object(m)
}

/// A week before the stored day, which is the overlap every top-up takes.
fn from_a_week_before(last: &str, fallback: &str) -> String {
    if last.is_empty() || parse_iso(last).is_none() {
        return fallback.to_string();
    }
    shift_date(last, -7)
}

/// `market.refresh_fx`.
pub fn refresh_fx(conn: &Connection) -> usize {
    let last = fx_last_date(conn, FX_PAIR).unwrap_or_default();
    let start = from_a_week_before(&last, FX_START);
    let url = format!("{}?start_date={}", BOC_URL, start);
    let text = match get_text(&url, &[]) { Ok(t) => t, Err(_) => return 0 };
    let rates = parse_boc_json(&text);
    upsert_fx_rates(conn, Some(&series_to_value(&rates)), FX_PAIR).unwrap_or(0)
}

/// `market.refresh_benchmark`: the S&P 500's closes. FRED serves the trailing
/// ten years in one file; Stooq stands in when it does not answer.
pub fn refresh_benchmark(conn: &Connection) -> usize {
    let mut mapping = get_text(FRED_URL, &[]).map(|t| parse_fred_csv(&t)).unwrap_or_default();
    if mapping.is_empty() {
        mapping = get_text(STOOQ_URL, &[]).map(|t| parse_stooq_csv(&t)).unwrap_or_default();
    }
    if mapping.is_empty() {
        return 0;
    }
    let last = benchmark_last_date(conn, BENCHMARK_SYMBOL).unwrap_or_default();
    if !last.is_empty() {
        let cutoff = from_a_week_before(&last, "");
        mapping.retain(|d, _| *d >= cutoff);
    }
    upsert_benchmark_prices(conn, Some(&series_to_value(&mapping)), BENCHMARK_SYMBOL).unwrap_or(0)
}

/// `market.refresh_tmx_index`: one index's daily closes, appended from a week
/// before the newest stored day.
pub fn refresh_tmx_index(conn: &Connection, key: &str) -> usize {
    let symbol = match TMX_INDICES.iter().find(|(k, _)| *k == key) { Some((_, s)) => *s, None => return 0 };
    let last = benchmark_last_date(conn, key).unwrap_or_default();
    let start = from_a_week_before(&last, TSX_START);
    let payload = json!({
        "operationName": "getTimeSeriesData",
        "variables": {"symbol": symbol, "freq": "day", "interval": 1, "start": start, "end": bagholder_model::clock::today_local()},
        "query": TMX_HISTORY_QUERY,
    });
    let data = match post_json(TMX_URL, &payload, &TMX_HEADERS) { Ok(d) => d, Err(_) => return 0 };
    let mut mapping = Series::new();
    for b in parse_tmx_history(&data) {
        if let Some(close) = b.get("close").and_then(|c| c.as_f64()) {
            if close != 0.0 {
                mapping.insert(bagholder_model::value::field_s(&b, "date"), close);
            }
        }
    }
    if mapping.is_empty() {
        return 0;
    }
    upsert_benchmark_prices(conn, Some(&series_to_value(&mapping)), key).unwrap_or(0)
}

/// `market.refresh_tsx`: the S&P/TSX Composite and the S&P/TSX 60.
pub fn refresh_tsx(conn: &Connection) -> usize {
    TMX_INDICES.iter().map(|(key, _)| refresh_tmx_index(conn, key)).sum()
}

/// `market.refresh_all`, less the declared distributions, which need the payer
/// symbols the model supplies.
pub fn refresh_all(conn: &Connection) -> Value {
    let _ = bagholder_store::tables::set_meta(conn, "market_attempt_at", &crate::now_stamp());
    json!({
        "fx": refresh_fx(conn),
        "benchmark": refresh_benchmark(conn) + refresh_tsx(conn),
        "skipped": false,
    })
}

/// `market.STALE_DAYS`.
pub const STALE_DAYS: i64 = 4;

/// `market.is_stale`, for the two series that are always needed. A book with
/// no rate within four days cannot convert a recent trade.
pub fn series_stale(conn: &Connection, today: &str) -> bool {
    let limit = shift_date(today, -STALE_DAYS);
    let fx = fx_last_date(conn, FX_PAIR).unwrap_or_default();
    if fx.is_empty() || fx < limit {
        return true;
    }
    let bench = benchmark_last_date(conn, BENCHMARK_SYMBOL).unwrap_or_default();
    bench.is_empty() || bench < limit
}
