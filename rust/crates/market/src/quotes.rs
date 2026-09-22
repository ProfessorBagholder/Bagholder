//! Live-ish prices for what the book holds and what it watches.
//!
//! Every listing is quoted from a feed that is live for its own market. TMX
//! quotes its own exchanges as they trade and stamps a US quote fifteen
//! minutes behind, which is what its licence allows; a US listing is Yahoo's,
//! whose quote is stamped to the second. A coin is Coinbase's, in the
//! position's own currency. A US-listed option is Cboe's delayed chain.

use serde_json::{json, Map, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::http::{get_text, UA};
use crate::parse::{occ_code, option_mark, parse_cboe_ca_quote, parse_coinbase_rec, parse_cboe_options, opt};
use crate::tmx;
use bagholder_model::input::Listing;
use bagholder_model::value::{field_s, get, num};
use bagholder_store::bars::{Ohlcv, SourceBar};

pub const QUOTE_REFRESH_MINUTES: f64 = 1.0;

pub const COINBASE_URL: &str = "https://api.coinbase.com/v2/prices/{}/spot";
pub const CBOE_CA_URL: &str = "https://www-api.cboe.com/ca/equities/securities-1/{}/quote/";
pub const CBOE_OPTIONS_URL: &str = "https://cdn.cboe.com/api/global/delayed_quotes/options/{}.json";
pub const YAHOO_CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";

const YAHOO_HEADERS: [(&str, &str); 2] = [("User-Agent", "Mozilla/5.0"), ("Accept", "application/json")];

/// Yahoo rate-limits bursts: one request at a time, well spaced, and after a
/// 429 nothing is asked of it for ten minutes.
pub const YAHOO_MIN_INTERVAL: Duration = Duration::from_millis(2000);
pub const YAHOO_BACKOFF: Duration = Duration::from_secs(600);

struct YahooGate {
    next_at: Option<Instant>,
    backoff_until: Option<Instant>,
}

static YAHOO: Mutex<YahooGate> = Mutex::new(YahooGate { next_at: None, backoff_until: None });

fn yahoo_get(url: &str) -> Option<String> {
    {
        let mut gate = YAHOO.lock().unwrap();
        let now = Instant::now();
        if let Some(until) = gate.backoff_until {
            if now < until {
                crate::http::note_source("yahoo", false, Some(&crate::http::FetchError::Transport("yahoo: backing off after 429".into())));
                return None;
            }
        }
        if let Some(next) = gate.next_at {
            if next > now {
                let wait = next - now;
                drop(gate);
                std::thread::sleep(wait);
                gate = YAHOO.lock().unwrap();
            }
        }
        gate.next_at = Some(Instant::now() + YAHOO_MIN_INTERVAL);
    }
    match get_text(url, &YAHOO_HEADERS) {
        Ok(t) => Some(t),
        Err(e) => {
            if e.code() == Some(429) {
                YAHOO.lock().unwrap().backoff_until = Some(Instant::now() + YAHOO_BACKOFF);
            }
            None
        }
    }
}

/// A Yahoo request for the chart path: the body, or the failure -- a backoff in force reads as one, and a 404 keeps its
/// code so the symbol can be remembered as one Yahoo does not carry.
pub fn yahoo_get_result(url: &str) -> Result<String, crate::http::FetchError> {
    {
        let mut gate = YAHOO.lock().unwrap();
        let now = Instant::now();
        if let Some(until) = gate.backoff_until {
            if now < until {
                let e = crate::http::FetchError::Transport("yahoo: backing off after 429".into());
                crate::http::note_source("yahoo", false, Some(&e));
                return Err(e);
            }
        }
        if let Some(next) = gate.next_at {
            if next > now {
                let wait = next - now;
                drop(gate);
                std::thread::sleep(wait);
                gate = YAHOO.lock().unwrap();
            }
        }
        gate.next_at = Some(Instant::now() + YAHOO_MIN_INTERVAL);
    }
    match get_text(url, &YAHOO_HEADERS) {
        Ok(t) => Ok(t),
        Err(e) => {
            if e.code() == Some(429) {
                YAHOO.lock().unwrap().backoff_until = Some(Instant::now() + YAHOO_BACKOFF);
            }
            Err(e)
        }
    }
}

/// The same, answering only the HTTP code.
pub fn yahoo_get_public(url: &str) -> Result<String, Option<u16>> {
    yahoo_get_result(url).map_err(|e| e.code())
}

/// An instant as epoch seconds, for callers outside this module.
pub fn instant_secs_public(s: &str) -> Option<f64> {
    instant_secs(s)
}

/// Bars in the exchange's own local day and
/// minute, oldest first; a row with no close is dropped.
///
/// Yahoo's `gmtoffset` is the offset today, not the bar's, so where the
/// exchange names its zone each bar is given its own standard or daylight
/// offset instead.
pub fn parse_yahoo_chart(text: &str) -> Vec<SourceBar> {
    let d: Value = match serde_json::from_str(if text.is_empty() { "{}" } else { text }) { Ok(v) => v, Err(_) => return vec![] };
    let results = match d.get("chart").and_then(|c| c.get("result")).and_then(|r| r.as_array()) {
        Some(r) if !r.is_empty() => r,
        _ => return vec![],
    };
    let r = &results[0];
    let ts = r.get("timestamp").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let q = r
        .get("indicators")
        .and_then(|i| i.get("quote"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let meta = r.get("meta").cloned().unwrap_or_else(|| json!({}));
    let zone = field_s(&meta, "exchangeTimezoneName");
    let fixed = num(get(&meta, "gmtoffset"), 0.0) as i64;

    let col = |k: &str, i: usize| -> Option<f64> {
        q.get(k).and_then(|v| v.as_array()).and_then(|a| a.get(i)).and_then(|x| x.as_f64())
    };

    let mut out: Vec<SourceBar> = Vec::new();
    for (i, t) in ts.iter().enumerate() {
        let t = match t.as_i64() { Some(t) => t, None => continue };
        let close = match col("close", i) { Some(c) if c > 0.0 => c, _ => continue };
        let (day, minute, off) = match crate::clockzone::local_at(&zone, t) {
            Some((d, mi, o)) => (d, mi, o),
            None => {
                let local = t + fixed;
                let days = local.div_euclid(86400);
                let rem = local.rem_euclid(86400);
                let (y, m, dd) = bagholder_model::dates::from_days(days);
                (bagholder_model::dates::fmt(y, m, dd), (rem / 60) as i64, fixed)
            }
        };
        out.push(SourceBar {
            time: t,
            day,
            minute,
            offset: off,
            px: Ohlcv { open: col("open", i), high: col("high", i), low: col("low", i), close, volume: col("volume", i) },
        });
    }
    out.sort_by_key(|b| b.time);
    out
}

/// The chart's meta read as a quote.
pub fn parse_yahoo_quote(text: &str) -> Option<Value> {
    let d: Value = serde_json::from_str(if text.is_empty() { "{}" } else { text }).ok()?;
    let results = d.get("chart")?.get("result")?.as_array()?;
    let meta = results.first()?.get("meta")?;
    if !meta.is_object() || get(meta, "regularMarketPrice").is_none() {
        return None;
    }
    let last = opt(get(meta, "regularMarketPrice"));
    let prev = opt(get(meta, "chartPreviousClose")).or_else(|| opt(get(meta, "previousClose")));
    let change = match (last, prev) {
        (Some(l), Some(p)) if p != 0.0 => Some(l - p),
        _ => None,
    };
    let name = {
        let s = field_s(meta, "shortName");
        if s.is_empty() { field_s(meta, "longName") } else { s }
    };
    Some(json!({
        "price": last,
        "priceChange": change,
        "percentChange": match (change, prev) { (Some(c), Some(p)) if p != 0.0 => Some(c / p * 100.0), _ => None },
        "prevClose": prev,
        "currency": field_s(meta, "currency"),
        "name": name,
        "exchange": field_s(meta, "exchangeName"),
    }))
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// A one-day chart, whose stated previous close is
/// yesterday's -- a longer range states the close before the range.
pub fn fetch_yahoo_quote(code: &str) -> Option<Value> {
    let url = format!("{}{}?range=1d&interval=1d", YAHOO_CHART_URL, percent_encode(code));
    parse_yahoo_quote(&yahoo_get(&url)?)
}

pub fn yahoo_root(symbol: &str) -> String {
    bagholder_model::venues::tmx_symbol(symbol).replace('.', "-")
}

const YAHOO_SUFFIX: [(&str, &str); 6] = [
    ("TSX", ".TO"), ("TSX-V", ".V"), ("TSXV", ".V"), ("CSE", ".CN"), ("CBOE CANADA", ".NE"), ("NEO", ".NE"),
];
const YAHOO_FORMS_CAD: [&str; 4] = [".TO", ".V", ".CN", ".NE"];
const YAHOO_FORMS_USD: [&str; 1] = [""];

/// The venue's own suffix first, then the other venues of its market, so a
/// wrong or missing venue still finds it. The venue names the market before
/// the currency does: a watched listing keeps no currency, and read by
/// currency alone a Nasdaq listing was asked for as a Toronto one, which is
/// another security (`PLTR.TO` is Palantir's Canadian depositary receipt, not
/// the stock) or nothing at all; a TSX listing that trades in US dollars is
/// still a Toronto one. Only a venue the app does not name leaves the currency
/// to decide.
pub fn yahoo_forms(rec: &Listing) -> Vec<String> {
    let root = yahoo_root(&rec.symbol);
    let ccy = match bagholder_model::venues::tmx_form(&rec.exchange, "") {
        Some(":US") => "USD".to_string(),
        Some(_) => "CAD".to_string(),
        None => { let c = rec.currency.clone(); if c.is_empty() { "CAD".to_string() } else { c.trim().to_uppercase() } }
    };
    if root.is_empty() || root.contains(' ') {
        return vec![];
    }
    let base: Vec<&str> = match ccy.as_str() {
        "CAD" => YAHOO_FORMS_CAD.to_vec(),
        "USD" => YAHOO_FORMS_USD.to_vec(),
        _ => return vec![],
    };
    let venue = rec.exchange.clone().trim().to_uppercase();
    let first = YAHOO_SUFFIX.iter().find(|(k, _)| *k == venue).map(|(_, v)| *v);
    let forms: Vec<&str> = match first {
        Some(f) if base.contains(&f) => {
            let mut out = vec![f];
            out.extend(base.iter().filter(|x| **x != f));
            out
        }
        _ => base,
    };
    forms.into_iter().map(|f| format!("{}{}", root, f)).collect()
}

/// The form a listing's quote is filed under, or
/// nothing when TMX does not carry it -- crypto, options, unknown venues.
pub fn tmx_quote_symbol(symbol: &str, exchange: &str, currency: &str) -> Option<String> {
    let s = bagholder_model::venues::tmx_symbol(symbol);
    if s.is_empty() || s.contains(' ') {
        return None;
    }
    bagholder_model::venues::tmx_form(exchange, currency).map(|f| format!("{}{}", s, f))
}

/// Which public source covers this instrument, and the
/// key it is filed under there.
pub fn quote_source(rec: &Listing) -> Option<(String, String)> {
    let kind = { let k = rec.kind.clone(); if k.is_empty() { "Shares".to_string() } else { k } };
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let ccy = { let c = rec.currency.clone(); if c.is_empty() { "CAD".to_string() } else { c.trim().to_uppercase() } };
    if sym.is_empty() {
        return None;
    }
    if kind == "Instrument" {
        let y = rec.yahoo.clone().unwrap_or_default();
        return if y.is_empty() { None } else { Some(("yahoo_quote".into(), y)) };
    }
    if kind == "Crypto" {
        return Some(("coinbase".into(), format!("{}-{}", sym, ccy)));
    }
    if kind == "Options" {
        let code = occ_code(&rec.symbol);
        return if !code.is_empty() && ccy == "USD" { Some(("cboe_options".into(), code)) } else { None };
    }
    if kind != "Shares" {
        return None;
    }
    let venue = rec.exchange.clone().trim().to_uppercase();
    if venue == "CBOE CANADA" || venue == "NEO" {
        return Some(("cboe_ca".into(), sym));
    }
    // a US listing is Yahoo's: TMX stamps one fifteen minutes behind
    if bagholder_model::venues::tmx_form(&rec.exchange, &rec.currency) == Some(":US") {
        let forms = yahoo_forms(rec);
        return forms.first().map(|f| ("yahoo_quote".to_string(), f.clone()));
    }
    tmx_quote_symbol(&rec.symbol, &rec.exchange, &rec.currency)
        .map(|q| ("tmx".to_string(), q))
}

/// The held instruments whose quote is
/// older than the refresh interval, each with its source and key.
// --- when a price can have moved -----------------------------------------------
//
// The exchanges offer no push, so prices are asked for. But a share's price moves
// only while its market trades: a quote read after the close is still the quote at
// midnight, on Saturday, and until the next open, and asking again every minute
// through the night fetches the same number some nine hundred times. So a listing
// is asked again only while its market is open, or when what is held was read
// before the last close (so the close itself is never missed). A coin trades always.
// Both countries' exchanges keep New York hours. A market holiday is not known
// here and is treated as a trading day: a wasted day's reads, never a stale price.

const MARKET_ZONE: &str = "America/New_York";
const OPEN_MINUTE: u32 = 9 * 60 + 30;
/// Twenty minutes past the bell, for the closing print to be published.
const SETTLED_MINUTE: u32 = 16 * 60 + 20;

fn weekday(days_since_epoch: i64) -> i64 {
    (days_since_epoch + 3).rem_euclid(7) // 0 = Monday; 1970-01-01 was a Thursday
}

/// Whether the share markets are trading at `now_unix` (or the close is still settling).
pub fn markets_open(now_unix: f64) -> bool {
    match bagholder_model::clock::civil_in(MARKET_ZONE, now_unix as i64) {
        Some((day, minute)) => weekday(day) < 5 && (OPEN_MINUTE..SETTLED_MINUTE).contains(&minute),
        None => true, // no zone data: ask, rather than show a stale price
    }
}

/// The moment the last session's closing print was settled, before `now_unix`.
fn last_settled(now_unix: f64) -> Option<f64> {
    let (day, minute) = bagholder_model::clock::civil_in(MARKET_ZONE, now_unix as i64)?;
    let mut back = if weekday(day) < 5 && minute >= SETTLED_MINUTE { 0 } else { 1 };
    while weekday(day - back) >= 5 {
        back += 1;
    }
    let local_midnight = now_unix - (minute as f64) * 60.0 - (now_unix % 60.0);
    Some(local_midnight - (back as f64) * 86400.0 + (SETTLED_MINUTE as f64) * 60.0)
}

/// Whether a quote from `source`, last read at `last`, can be different now.
pub fn can_have_moved(source: &str, last: Option<f64>, now_unix: f64) -> bool {
    if source == "coinbase" || markets_open(now_unix) {
        return true;
    }
    match (last, last_settled(now_unix)) {
        (Some(read), Some(settled)) => read < settled,
        _ => true,
    }
}

pub fn quote_symbols_needing_refresh(
    conn: &rusqlite::Connection,
    symbols: &[Listing],
    now_unix: f64,
    max_age_minutes: f64,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let fetched = crate::market_fetched(conn)?;
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for rec in symbols {
        // a watched listing is keyed by symbol and venue
        let sym = {
            let k = rec.quote_key.clone().unwrap_or_default();
            if k.is_empty() { bagholder_model::venues::tmx_symbol(&rec.symbol) } else { k }
        };
        let src = quote_source(rec);
        let (source, key) = match src { Some(s) => s, None => continue };
        if sym.is_empty() || seen.contains(&sym) {
            continue;
        }
        seen.push(sym.clone());
        let last = fetched.get(&sym).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let read = instant_secs(&last);
        let age_ok = read.map_or(false, |then| (now_unix - then) <= max_age_minutes * 60.0);
        if !age_ok && can_have_moved(&source, read, now_unix) {
            out.push((sym, source, key));
        }
    }
    Ok(out)
}

/// Seconds since the epoch for an ISO instant.
fn instant_secs(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let t = s.replace('Z', "+00:00");
    let (d, rest) = t.split_once('T')?;
    let (y, m, day) = bagholder_model::dates::parse_iso(d)?;
    let clock: Vec<&str> = rest.split(|c| c == ':' || c == '+' || c == '-').collect();
    let hh: f64 = clock.first()?.parse().ok()?;
    let mm: f64 = clock.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let ss: f64 = clock.get(2).and_then(|x| x.split('.').next()?.parse().ok()).unwrap_or(0.0);
    Some(bagholder_model::dates::to_days(y, m, day) as f64 * 86400.0 + hh * 3600.0 + mm * 60.0 + ss)
}

pub fn fetch_cboe_ca_quote(sym: &str) -> Option<Value> {
    let url = CBOE_CA_URL.replace("{}", &percent_encode(sym));
    parse_cboe_ca_quote(&get_text(&url, &[("User-Agent", UA), ("Accept", "application/json")]).ok()?)
}

pub fn fetch_cboe_option_chain(root: &str) -> Map<String, Value> {
    let url = CBOE_OPTIONS_URL.replace("{}", &percent_encode(root));
    match get_text(&url, &[("User-Agent", UA), ("Accept", "application/json")]) {
        Ok(t) => parse_cboe_options(&t),
        Err(_) => Map::new(),
    }
}

/// The close of the last completed UTC day on the
/// pair's Coinbase market, in the pair's currency -- the pair's own market
/// when Coinbase has one, else the USD market converted at the day's Bank of
/// Canada rate. Remembered per day.
pub fn coinbase_prev_close(conn: &rusqlite::Connection, pair: &str, today: &str, now_unix: f64) -> Option<f64> {
    let pair = pair.trim().to_uppercase();
    let meta_key = format!("coinbase_prev:{}", pair);
    let v = bagholder_store::tables::get_meta(conn, &meta_key, "").unwrap_or_default();
    if let Some(rest) = v.strip_prefix(&format!("{}@", today)) {
        return bagholder_model::textrules::parse_float(rest);
    }
    let (base, ccy) = match pair.split_once('-') { Some((b, c)) => (b.to_string(), c.to_string()), None => (pair.clone(), String::new()) };
    let products = if ccy == "USD" { vec![pair.clone()] } else { vec![pair.clone(), format!("{}-USD", base)] };
    let mut prev: Option<f64> = None;
    for product in products {
        if crate::history::coinbase_market(conn, &product, today).is_empty() {
            continue;
        }
        let now = now_unix as i64;
        let bars = crate::history::fetch_coinbase_candles(&product, 86400, now - 4 * 86400, now);
        let quoted = product.rsplit('-').next().unwrap_or("").to_string();
        let bars = crate::history::in_position_currency(conn, &bars, &quoted, &ccy);
        let done: Vec<&bagholder_store::bars::TimeBar> = bars
            .iter()
            .filter(|b| {
                let (y, m, d) = bagholder_model::dates::from_days(b.time.div_euclid(86400));
                bagholder_model::dates::fmt(y, m, d).as_str() < today
            })
            .collect();
        if let Some(last) = done.last() {
            prev = Some(last.px.close);
            break;
        }
    }
    if let Some(p) = prev {
        if p != 0.0 {
            let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("{}@{}", today, float_repr(p)));
        }
    }
    prev.filter(|p| *p != 0.0)
}

/// A float as its shortest round-trip text, which is the shortest text that reads back as
/// the same number.
pub fn float_repr(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e16 {
        return format!("{:.1}", x);
    }
    let s = format!("{}", x);
    if s.contains('e') {
        return s;
    }
    // Rust writes 1e-7 as 0.0000001; the text switches to an exponent below 1e-4
    if x.abs() < 1e-4 || x.abs() >= 1e16 {
        let e = format!("{:e}", x);
        let (mant, exp) = e.split_once('e').unwrap();
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!("{}e{}{:02}", mant, if exp < 0 { "-" } else { "+" }, exp.abs());
    }
    s
}

/// The spot price, with the day's change against
/// the previous UTC day's close when Coinbase has a market to take it from.
pub fn fetch_coinbase_spot(conn: &rusqlite::Connection, pair: &str, today: &str, now_unix: f64) -> Option<Value> {
    let url = COINBASE_URL.replace("{}", pair);
    let rec = parse_coinbase_rec(&get_text(&url, &[]).ok()?, pair)?;
    Some(match coinbase_prev_close(conn, pair, today, now_unix) {
        Some(prev) => with_prev_close(rec, prev),
        None => rec,
    })
}

/// A spot price with its day's change against the previous close.
pub fn with_prev_close(mut rec: Value, prev: f64) -> Value {
    let price = rec["price"].as_f64().unwrap_or(0.0);
    rec["prevClose"] = json!(prev);
    rec["priceChange"] = json!(price - prev);
    rec["percentChange"] = json!((price - prev) / prev * 100.0);
    rec
}

/// The root of an OCC code, "" when it is not one.
pub fn occ_root(code: &str) -> String {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| regex::Regex::new(r"^([A-Z][A-Z0-9.]{0,9})\d{6}[CP]\d{8}$").unwrap())
        .captures(code)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// One quote from the named source. A chain is shared
/// across the calls for one option root.
pub fn fetch_for(
    conn: &rusqlite::Connection,
    source: &str,
    key: &str,
    today: &str,
    chains: &mut Map<String, Value>,
) -> Option<Value> {
    match source {
        "tmx" => tmx::fetch_tmx_quote(conn, key, today),
        "cboe_ca" => fetch_cboe_ca_quote(key),
        "coinbase" => {
            let (_, now_unix, _) = crate::clock_now();
            fetch_coinbase_spot(conn, key, today, now_unix)
        }
        "yahoo_quote" => fetch_yahoo_quote(key),
        "cboe_options" => {
            let root = occ_root(key);
            if !chains.contains_key(&root) {
                chains.insert(root.clone(), Value::Object(fetch_cboe_option_chain(&root)));
            }
            let row = chains.get(&root)?.get(key)?.clone();
            option_mark(&row)
        }
        _ => None,
    }
}

pub fn refresh_quotes(
    conn: &rusqlite::Connection,
    symbols: &[Listing],
    today: &str,
    now_unix: f64,
    now_stamp: &str,
) -> rusqlite::Result<usize> {
    let mut done = 0usize;
    let mut chains = Map::new();
    for (sym, source, key) in quote_symbols_needing_refresh(conn, symbols, now_unix, QUOTE_REFRESH_MINUTES)? {
        if let Some(rec) = fetch_for(conn, &source, &key, today, &mut chains) {
            if get(&rec, "price").is_some() {
                let mut with_source = rec.as_object().cloned().unwrap_or_default();
                with_source.insert("source".into(), json!(source));
                crate::market::upsert_quote(conn, &sym, &Value::Object(with_source), &source, now_stamp)?;
                done += 1;
            }
        }
    }
    Ok(done)
}

/// The dividend-paying Canadian listings whose declared
/// distribution record is older than its own stamp allows.
///
/// The record has a separate fetch stamp from the quote: the quote loop keeps
/// prices fresh every few minutes, and that must not make a fund's
/// distribution history look fresh.
pub fn stale_symbols(
    conn: &rusqlite::Connection,
    symbols: &[Listing],
    now_unix: f64,
    stale_hours: f64,
) -> rusqlite::Result<Vec<String>> {
    let fetched = crate::market::distributions_fetched_at(conn)?;
    let mut out = Vec::new();
    for rec in symbols {
        let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
        if sym.is_empty() || !tmx::is_canadian_listing(&rec.exchange, &rec.currency) {
            continue;
        }
        let last = fetched.get(&sym).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let fresh = match instant_secs(&last) {
            Some(then) => (now_unix - then) <= stale_hours * 3600.0,
            None => false,
        };
        if !fresh {
            out.push(sym);
        }
    }
    Ok(out)
}

/// A number read the way the quote parsers do.
pub fn n(v: Option<&Value>) -> f64 {
    num(v, 0.0)
}

/// Yahoo's pace, for a caller that makes its own request: false while a
/// backoff after a 429 stands; otherwise waits its turn and takes it.
pub fn yahoo_turn() -> bool {
    let mut gate = YAHOO.lock().unwrap();
    let now = Instant::now();
    if let Some(until) = gate.backoff_until {
        if now < until {
            return false;
        }
    }
    if let Some(next) = gate.next_at {
        if next > now {
            let wait = next - now;
            drop(gate);
            std::thread::sleep(wait);
            gate = YAHOO.lock().unwrap();
        }
    }
    gate.next_at = Some(Instant::now() + YAHOO_MIN_INTERVAL);
    true
}

/// Yahoo turned a request away with a 429: nothing is asked of it for the
/// backoff.
pub fn yahoo_back_off() {
    YAHOO.lock().unwrap().backoff_until = Some(Instant::now() + YAHOO_BACKOFF);
}

pub const PEEK_SECONDS: u64 = 60;

/// A listing's price and day change for a glance, from
/// the source a watched listing uses, not stored, remembered for a minute.
pub fn peek_quote(conn: &rusqlite::Connection, rec: &Listing, today: &str) -> Option<Value> {
    static PEEK: std::sync::OnceLock<Mutex<std::collections::HashMap<String, (Instant, Value)>>> = std::sync::OnceLock::new();
    let (source, key) = quote_source(rec)?;
    let k = format!("{}@{}", rec.symbol.clone().trim().to_uppercase(), rec.exchange.clone().trim().to_uppercase());
    let cache = PEEK.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some((at, q)) = cache.lock().unwrap().get(&k) {
        if at.elapsed() < Duration::from_secs(PEEK_SECONDS) {
            return Some(q.clone());
        }
    }
    let mut chains = Map::new();
    let q = fetch_for(conn, &source, &key, today, &mut chains)?;
    if get(&q, "price").is_none() {
        return None;
    }
    let out = json!({
        "price": q.get("price").cloned().unwrap_or(Value::Null),
        "priceChange": q.get("priceChange").cloned().unwrap_or(Value::Null),
        "percentChange": q.get("percentChange").cloned().unwrap_or(Value::Null),
    });
    cache.lock().unwrap().insert(k, (Instant::now(), out.clone()));
    Some(out)
}
