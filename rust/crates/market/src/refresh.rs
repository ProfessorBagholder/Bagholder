//! Topping up the series the model reads: the Bank of Canada's USD/CAD, the
//! S&P 500's closes, and the two TMX indices.
//!
//! Every one of these appends from a week before the newest stored day, so a
//! revision at the edge of the series is picked up without refetching history,
//! and none of them ever raises: a source that is down leaves what is stored.

use rusqlite::Connection;
use bagholder_model::input::Listing;
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

/// The stored benchmark key -> TMX Money's index symbol.
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

pub fn refresh_fx(conn: &Connection) -> usize {
    let last = fx_last_date(conn, FX_PAIR).unwrap_or_default();
    let start = from_a_week_before(&last, FX_START);
    let url = format!("{}?start_date={}", BOC_URL, start);
    let text = match get_text(&url, &[]) { Ok(t) => t, Err(_) => return 0 };
    let rates = parse_boc_json(&text);
    upsert_fx_rates(conn, Some(&series_to_value(&rates)), FX_PAIR).unwrap_or(0)
}

/// The S&P 500's closes. FRED serves the trailing
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

/// One index's daily closes, appended from a week
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

/// The S&P/TSX Composite and the S&P/TSX 60.
pub fn refresh_tsx(conn: &Connection) -> usize {
    TMX_INDICES.iter().map(|(key, _)| refresh_tmx_index(conn, key)).sum()
}

static REFRESHING: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

struct Refreshing;

impl Refreshing {
    fn claim() -> Option<Refreshing> {
        let mut r = REFRESHING.lock().unwrap();
        if *r {
            return None;
        }
        *r = true;
        Some(Refreshing)
    }
}

impl Drop for Refreshing {
    fn drop(&mut self) {
        *REFRESHING.lock().unwrap() = false;
    }
}

/// FX, the benchmarks and the declared distributions for
/// the payer symbols. Never fails; the row counts written.
pub fn refresh_all(conn: &Connection, symbols: &[Listing]) -> Value {
    let _guard = match Refreshing::claim() { Some(g) => g, None => return json!({"fx": 0, "benchmark": 0, "skipped": true}) };
    let _ = bagholder_store::tables::set_meta(conn, "market_attempt_at", &crate::now_stamp());
    json!({
        "fx": refresh_fx(conn),
        "benchmark": refresh_benchmark(conn) + refresh_tsx(conn),
        "distributions": refresh_distributions(conn, symbols, false),
        "skipped": false,
    })
}

pub const RECORD_STALE_HOURS: f64 = 20.0;
pub const MARKET_ATTEMPT_HOURS: f64 = 6.0;
pub const MARKET_CHECK_MINUTES: u64 = 60;
/// 16:30 Eastern.
pub const BOC_PUBLISH_MINUTE_ET: i64 = 16 * 60 + 30;

/// Quotes and declared distributions for the
/// dividend payers whose record is stale, or all of them when forced.
pub fn refresh_distributions(conn: &Connection, symbols: &[Listing], force: bool) -> usize {
    let (today, now_unix, stamp) = crate::clock_now();
    let todo: Vec<String> = if force {
        symbols
            .iter()
            .filter(|r| crate::tmx::is_canadian_listing(&r.exchange, &r.currency))
            .map(|r| bagholder_model::venues::tmx_symbol(&r.symbol))
            .collect()
    } else {
        crate::quotes::stale_symbols(conn, symbols, now_unix, RECORD_STALE_HOURS).unwrap_or_default()
    };
    // the last record naming a symbol decides its exchange, as the dict does
    let mut exchanges: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for r in symbols {
        exchanges.insert(
            bagholder_model::venues::tmx_symbol(&r.symbol),
            r.exchange.clone(),
        );
    }
    let mut done = 0;
    for sym in todo {
        let exchange = exchanges.get(&sym).cloned().unwrap_or_default();
        let (quote, divs) = crate::tmx::fetch_tmx(conn, &sym, &exchange, &today);
        // a Cboe Canada listing's price comes from Cboe's own feed every minute;
        // TMX's delayed quote for it must not replace that, only its record is kept
        let cboe = ["CBOE CANADA", "NEO"].contains(&exchange.trim().to_uppercase().as_str());
        if let Some(q) = &quote {
            if !cboe {
                let _ = bagholder_store::market::upsert_quote(conn, &sym, q, "tmx", &stamp);
            }
        }
        if !divs.is_empty() {
            let _ = bagholder_store::market::upsert_distributions(conn, &sym, &divs, "tmx");
        }
        if quote.is_some() || !divs.is_empty() {
            let _ = bagholder_store::market::mark_distributions_fetched(conn, &sym, &stamp);
            done += 1;
        }
    }
    done
}

/// Any index the page can show with no closes, or
/// none within four days.
pub fn benchmark_stale(conn: &Connection, today: &str) -> bool {
    let limit = shift_date(today, -STALE_DAYS);
    bagholder_store::market::BENCHMARK_SYMBOLS.iter().any(|sym| {
        let last = benchmark_last_date(conn, sym).unwrap_or_default();
        last.is_empty() || last < limit
    })
}

/// The rates, a benchmark, or a payer's record.
pub fn is_stale(conn: &Connection, today: &str, symbols: &[Listing]) -> bool {
    let limit = shift_date(today, -STALE_DAYS);
    let fx = fx_last_date(conn, FX_PAIR).unwrap_or_default();
    if fx.is_empty() || fx < limit || benchmark_stale(conn, today) {
        return true;
    }
    let (_, now_unix, _) = crate::clock_now();
    !crate::quotes::stale_symbols(conn, symbols, now_unix, RECORD_STALE_HOURS).unwrap_or_default().is_empty()
}

/// The Bank of Canada has published
/// today's rate (16:30 Eastern on a weekday) and the table does not have it.
pub fn fx_day_published_but_missing(conn: &Connection, now_unix: f64) -> bool {
    let (day, minute, _) = match crate::clockzone::local_at("America/Toronto", now_unix as i64) { Some(x) => x, None => return false };
    let (y, m, d) = match bagholder_model::dates::parse_iso(&day) { Some(x) => x, None => return false };
    let weekday = (bagholder_model::dates::to_days(y, m, d) + 3).rem_euclid(7);
    if weekday > 4 || minute < BOC_PUBLISH_MINUTE_ET {
        return false;
    }
    fx_last_date(conn, FX_PAIR).unwrap_or_default() < day
}

/// USD/CAD and the benchmarks at most every six
/// hours (sooner once today's rate is out, or a benchmark is stale), and every
/// payer's distribution record past its hours.
pub fn refresh_periodic(conn: &Connection, symbols: &[Listing]) -> Value {
    let _guard = match Refreshing::claim() { Some(g) => g, None => return json!({"fx": 0, "benchmark": 0, "distributions": 0, "skipped": true}) };
    let (today, now_unix, stamp) = crate::clock_now();
    let mut fx = 0;
    let mut bench = 0;
    let last = bagholder_store::tables::get_meta(conn, "market_attempt_at", "").unwrap_or_default();
    let old = match crate::quotes::instant_secs_public(&last) {
        Some(then) => now_unix - then > MARKET_ATTEMPT_HOURS * 3600.0,
        None => true,
    };
    if old || fx_day_published_but_missing(conn, now_unix) || benchmark_stale(conn, &today) {
        let _ = bagholder_store::tables::set_meta(conn, "market_attempt_at", &stamp);
        fx = refresh_fx(conn);
        bench = refresh_benchmark(conn) + refresh_tsx(conn);
    }
    let dist = refresh_distributions(conn, symbols, false);
    json!({"fx": fx, "benchmark": bench, "distributions": dist, "skipped": false})
}

pub const STALE_DAYS: i64 = 4;

/// Staleness for the two series that are always needed. A book with
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
