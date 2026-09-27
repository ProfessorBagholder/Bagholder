//! Topping up the series the model reads: the Bank of Canada's USD/CAD, the
//! S&P 500's closes, and the two TMX indices.
//!
//! Every one of these appends from a week before the newest stored day, so a
//! revision at the edge of the series is picked up without refetching history.
//! A source that is down leaves what is stored, and its failure is returned.

use rusqlite::Connection;
use bagholder_model::input::Listing;
use serde_json::json;

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

/// A week before the stored day, which is the overlap every top-up takes.
fn from_a_week_before(last: &str, fallback: &str) -> String {
    if last.is_empty() || parse_iso(last).is_none() {
        return fallback.to_string();
    }
    shift_date(last, -7)
}

/// A source's failure in the header's words.
fn failed(source: &str, e: &crate::http::FetchError) -> String {
    format!("{source} {}.", crate::http::describe_failure(e))
}

fn stored(e: rusqlite::Error) -> String {
    format!("The market series could not be stored: {e}")
}

pub fn refresh_fx(conn: &Connection) -> Result<usize, String> {
    let last = fx_last_date(conn, FX_PAIR).map_err(stored)?;
    let start = from_a_week_before(&last, FX_START);
    let url = format!("{}?start_date={}", BOC_URL, start);
    let text = get_text(&url, &[]).map_err(|e| failed("The Bank of Canada", &e))?;
    let rates = parse_boc_json(&text);
    upsert_fx_rates(conn, &rates, FX_PAIR).map_err(stored)
}

/// The S&P 500's closes. FRED serves the trailing
/// ten years in one file; Stooq stands in when it does not answer. Neither
/// answering is the failure.
pub fn refresh_benchmark(conn: &Connection) -> Result<usize, String> {
    let fred = get_text(FRED_URL, &[]).map(|t| parse_fred_csv(&t));
    let mut mapping = match &fred {
        Ok(m) if !m.is_empty() => m.clone(),
        _ => match get_text(STOOQ_URL, &[]).map(|t| parse_stooq_csv(&t)) {
            Ok(m) => m,
            Err(stooq) => {
                let fred = match &fred { Err(e) => failed("FRED", e), Ok(_) => "FRED answered with no closes.".into() };
                return Err(format!("{fred} {}", failed("Stooq", &stooq)));
            }
        },
    };
    if mapping.is_empty() {
        return Ok(0);
    }
    let last = benchmark_last_date(conn, BENCHMARK_SYMBOL).map_err(stored)?;
    if !last.is_empty() {
        let cutoff = from_a_week_before(&last, "");
        mapping.retain(|d, _| *d >= cutoff);
    }
    upsert_benchmark_prices(conn, &mapping, BENCHMARK_SYMBOL).map_err(stored)
}

/// One index's daily closes, appended from a week
/// before the newest stored day.
pub fn refresh_tmx_index(conn: &Connection, key: &str) -> Result<usize, String> {
    let symbol = match TMX_INDICES.iter().find(|(k, _)| *k == key) { Some((_, s)) => *s, None => return Ok(0) };
    let last = benchmark_last_date(conn, key).map_err(stored)?;
    let start = from_a_week_before(&last, TSX_START);
    let payload = json!({
        "operationName": "getTimeSeriesData",
        "variables": {"symbol": symbol, "freq": "day", "interval": 1, "start": start, "end": bagholder_model::clock::today_local()},
        "query": TMX_HISTORY_QUERY,
    });
    let data = post_json(TMX_URL, &payload, &TMX_HEADERS).map_err(|e| failed("TMX Money", &e))?;
    let mut mapping = Series::new();
    for b in parse_tmx_history(&data) {
        if b.px.close != 0.0 {
            mapping.insert(b.date, b.px.close);
        }
    }
    if mapping.is_empty() {
        return Ok(0);
    }
    upsert_benchmark_prices(conn, &mapping, key).map_err(stored)
}

/// The S&P/TSX Composite and the S&P/TSX 60.
pub fn refresh_tsx(conn: &Connection) -> Result<usize, String> {
    let mut done = 0;
    for (key, _) in TMX_INDICES {
        done += refresh_tmx_index(conn, key)?;
    }
    Ok(done)
}

/// Every part of a pass run, each failure said: the parts do not depend on one
/// another, so one failing does not stop the rest.
fn all_of(parts: Vec<Result<(), String>>) -> Result<(), String> {
    let failures: Vec<String> = parts.into_iter().filter_map(Result::err).collect();
    if failures.is_empty() { Ok(()) } else { Err(failures.join(" ")) }
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
/// the payer symbols; nothing when another pass was already running. What
/// failed is returned, the rest done.
pub fn refresh_all(conn: &Connection, symbols: &[Listing]) -> Result<(), String> {
    let _guard = match Refreshing::claim() { Some(g) => g, None => return Ok(()) };
    bagholder_store::tables::set_meta(conn, "market_attempt_at", &crate::now_stamp()).map_err(stored)?;
    all_of(vec![
        refresh_fx(conn).map(drop),
        refresh_benchmark(conn).map(drop),
        refresh_tsx(conn).map(drop),
        refresh_distributions(conn, symbols, false).map(drop),
    ])
}

pub const RECORD_STALE_HOURS: f64 = 20.0;
pub const MARKET_ATTEMPT_HOURS: f64 = 6.0;
pub const MARKET_CHECK_MINUTES: u64 = 60;
/// 16:30 Eastern.
pub const BOC_PUBLISH_MINUTE_ET: i64 = 16 * 60 + 30;

/// Quotes and declared distributions for the
/// dividend payers whose record is stale, or all of them when forced. A payer
/// whose read fails keeps its record and is said; the others are read.
pub fn refresh_distributions(conn: &Connection, symbols: &[Listing], force: bool) -> Result<usize, String> {
    let (today, now_unix, stamp) = crate::clock_now();
    let todo: Vec<String> = if force {
        symbols
            .iter()
            .filter(|r| crate::tmx::is_canadian_listing(&r.exchange, &r.currency))
            .map(|r| bagholder_model::venues::tmx_symbol(&r.symbol))
            .collect()
    } else {
        crate::quotes::stale_symbols(conn, symbols, now_unix, RECORD_STALE_HOURS).map_err(stored)?
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
    let mut failures = Vec::new();
    for sym in todo {
        let exchange = exchanges.get(&sym).cloned().unwrap_or_default();
        let (quote, divs) = match crate::tmx::fetch_tmx(conn, &sym, &exchange, &today) {
            Ok(got) => got,
            Err(e) => {
                failures.push(format!("The distributions of {sym} could not be read: {e}."));
                continue;
            }
        };
        // a Cboe Canada listing's price comes from Cboe's own feed every minute;
        // TMX's delayed quote for it must not replace that, only its record is kept
        let cboe = ["CBOE CANADA", "NEO"].contains(&exchange.trim().to_uppercase().as_str());
        // the three writes are one record: a store that refuses one fails the pass
        if let Some(q) = &quote {
            if !cboe {
                bagholder_store::market::upsert_quote(conn, &sym, &q.quote, "tmx", &stamp).map_err(stored)?;
            }
        }
        if !divs.is_empty() {
            bagholder_store::market::upsert_distributions(conn, &sym, &divs, "tmx").map_err(stored)?;
        }
        if quote.is_some() || !divs.is_empty() {
            bagholder_store::market::mark_distributions_fetched(conn, &sym, &stamp).map_err(stored)?;
            done += 1;
        }
    }
    if failures.is_empty() { Ok(done) } else { Err(failures.join(" ")) }
}

/// Any index the page can show with no closes, or
/// none within four days.
pub fn benchmark_stale(conn: &Connection, today: &str) -> rusqlite::Result<bool> {
    let limit = shift_date(today, -STALE_DAYS);
    for sym in bagholder_store::market::BENCHMARK_SYMBOLS {
        let last = benchmark_last_date(conn, sym)?;
        if last.is_empty() || last < limit {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The rates, a benchmark, or a payer's record.
pub fn is_stale(conn: &Connection, today: &str, symbols: &[Listing]) -> rusqlite::Result<bool> {
    let limit = shift_date(today, -STALE_DAYS);
    let fx = fx_last_date(conn, FX_PAIR)?;
    if fx.is_empty() || fx < limit || benchmark_stale(conn, today)? {
        return Ok(true);
    }
    let (_, now_unix, _) = crate::clock_now();
    Ok(!crate::quotes::stale_symbols(conn, symbols, now_unix, RECORD_STALE_HOURS)?.is_empty())
}

/// The Bank of Canada has published
/// today's rate (16:30 Eastern on a weekday) and the table does not have it.
pub fn fx_day_published_but_missing(conn: &Connection, now_unix: f64) -> rusqlite::Result<bool> {
    let (day, minute, _) = match bagholder_model::clock::local_at("America/Toronto", now_unix as i64) { Some(x) => x, None => return Ok(false) };
    let (y, m, d) = match bagholder_model::dates::parse_iso(&day) { Some(x) => x, None => return Ok(false) };
    let weekday = (bagholder_model::dates::to_days(y, m, d) + 3).rem_euclid(7);
    if weekday > 4 || minute < BOC_PUBLISH_MINUTE_ET {
        return Ok(false);
    }
    Ok(fx_last_date(conn, FX_PAIR)? < day)
}

/// USD/CAD and the benchmarks at most every six
/// hours (sooner once today's rate is out, or a benchmark is stale), and every
/// payer's distribution record past its hours. What failed is returned, the
/// rest done.
pub fn refresh_periodic(conn: &Connection, symbols: &[Listing]) -> Result<(), String> {
    let _guard = match Refreshing::claim() { Some(g) => g, None => return Ok(()) };
    let (today, now_unix, stamp) = crate::clock_now();
    let last = bagholder_store::tables::get_meta(conn, "market_attempt_at", "").map_err(stored)?;
    let old = match crate::quotes::instant_secs_public(&last) {
        Some(then) => now_unix - then > MARKET_ATTEMPT_HOURS * 3600.0,
        None => true,
    };
    let mut parts = Vec::new();
    if old || fx_day_published_but_missing(conn, now_unix).map_err(stored)? || benchmark_stale(conn, &today).map_err(stored)? {
        bagholder_store::tables::set_meta(conn, "market_attempt_at", &stamp).map_err(stored)?;
        parts.push(refresh_fx(conn).map(drop));
        parts.push(refresh_benchmark(conn).map(drop));
        parts.push(refresh_tsx(conn).map(drop));
    }
    parts.push(refresh_distributions(conn, symbols, false).map(drop));
    all_of(parts)
}

pub const STALE_DAYS: i64 = 4;

/// Staleness for the two series that are always needed. A book with
/// no rate within four days cannot convert a recent trade.
pub fn series_stale(conn: &Connection, today: &str) -> rusqlite::Result<bool> {
    let limit = shift_date(today, -STALE_DAYS);
    let fx = fx_last_date(conn, FX_PAIR)?;
    if fx.is_empty() || fx < limit {
        return Ok(true);
    }
    let bench = benchmark_last_date(conn, BENCHMARK_SYMBOL)?;
    Ok(bench.is_empty() || bench < limit)
}
