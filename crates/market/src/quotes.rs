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
use bagholder_model::value::{field_s, get, num};

/// `market.QUOTE_REFRESH_MINUTES`.
pub const QUOTE_REFRESH_MINUTES: f64 = 1.0;

pub const COINBASE_URL: &str = "https://api.coinbase.com/v2/prices/{}/spot";
pub const CBOE_CA_URL: &str = "https://www-api.cboe.com/ca/equities/securities-1/{}/quote/";
pub const CBOE_OPTIONS_URL: &str = "https://cdn.cboe.com/api/global/delayed_quotes/options/{}.json";
pub const YAHOO_CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";

/// `market.YAHOO_HEADERS`.
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

/// `market._yahoo_get`.
fn yahoo_get(url: &str) -> Option<String> {
    {
        let mut gate = YAHOO.lock().unwrap();
        let now = Instant::now();
        if let Some(until) = gate.backoff_until {
            if now < until {
                crate::http::note_source("yahoo", false, None);
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

/// `market.parse_yahoo_quote`: the chart's meta read as a quote.
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

fn percent_encode(s: &str) -> String {
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

/// `market.fetch_yahoo_quote`: a one-day chart, whose stated previous close is
/// yesterday's -- a longer range states the close before the range.
pub fn fetch_yahoo_quote(code: &str) -> Option<Value> {
    let url = format!("{}{}?range=1d&interval=1d", YAHOO_CHART_URL, percent_encode(code));
    parse_yahoo_quote(&yahoo_get(&url)?)
}

/// `market.yahoo_root`.
pub fn yahoo_root(symbol: &str) -> String {
    bagholder_model::venues::tmx_symbol(symbol).replace('.', "-")
}

/// `market.YAHOO_SUFFIX` / `YAHOO_FORMS`.
const YAHOO_SUFFIX: [(&str, &str); 6] = [
    ("TSX", ".TO"), ("TSX-V", ".V"), ("TSXV", ".V"), ("CSE", ".CN"), ("CBOE CANADA", ".NE"), ("NEO", ".NE"),
];
const YAHOO_FORMS_CAD: [&str; 4] = [".TO", ".V", ".CN", ".NE"];
const YAHOO_FORMS_USD: [&str; 1] = [""];

/// `market.yahoo_forms`: the venue's own suffix first, then the other venues
/// of the listing's currency, so a wrong or missing venue still finds it.
pub fn yahoo_forms(rec: &Value) -> Vec<String> {
    let root = yahoo_root(&field_s(rec, "symbol"));
    let ccy = { let c = field_s(rec, "currency"); if c.is_empty() { "CAD".to_string() } else { c.trim().to_uppercase() } };
    if root.is_empty() || root.contains(' ') {
        return vec![];
    }
    let base: Vec<&str> = match ccy.as_str() {
        "CAD" => YAHOO_FORMS_CAD.to_vec(),
        "USD" => YAHOO_FORMS_USD.to_vec(),
        _ => return vec![],
    };
    let venue = field_s(rec, "exchange").trim().to_uppercase();
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

/// `market.tmx_quote_symbol`: the form a listing's quote is filed under, or
/// nothing when TMX does not carry it -- crypto, options, unknown venues.
pub fn tmx_quote_symbol(symbol: &str, exchange: &str, currency: &str) -> Option<String> {
    let s = bagholder_model::venues::tmx_symbol(symbol);
    if s.is_empty() || s.contains(' ') {
        return None;
    }
    bagholder_model::venues::tmx_form(exchange, currency).map(|f| format!("{}{}", s, f))
}

/// `market.quote_source`: which public source covers this instrument, and the
/// key it is filed under there.
pub fn quote_source(rec: &Value) -> Option<(String, String)> {
    let kind = { let k = field_s(rec, "kind"); if k.is_empty() { "Shares".to_string() } else { k } };
    let sym = bagholder_model::venues::tmx_symbol(&field_s(rec, "symbol"));
    let ccy = { let c = field_s(rec, "currency"); if c.is_empty() { "CAD".to_string() } else { c.trim().to_uppercase() } };
    if sym.is_empty() {
        return None;
    }
    if kind == "Instrument" {
        let y = field_s(rec, "yahoo");
        return if y.is_empty() { None } else { Some(("yahoo_quote".into(), y)) };
    }
    if kind == "Crypto" {
        return Some(("coinbase".into(), format!("{}-{}", sym, ccy)));
    }
    if kind == "Options" {
        let code = occ_code(&field_s(rec, "symbol"));
        return if !code.is_empty() && ccy == "USD" { Some(("cboe_options".into(), code)) } else { None };
    }
    if kind != "Shares" {
        return None;
    }
    let venue = field_s(rec, "exchange").trim().to_uppercase();
    if venue == "CBOE CANADA" || venue == "NEO" {
        return Some(("cboe_ca".into(), sym));
    }
    // a US listing is Yahoo's: TMX stamps one fifteen minutes behind
    if bagholder_model::venues::tmx_form(&field_s(rec, "exchange"), &field_s(rec, "currency")) == Some(":US") {
        let forms = yahoo_forms(rec);
        return forms.first().map(|f| ("yahoo_quote".to_string(), f.clone()));
    }
    tmx_quote_symbol(&field_s(rec, "symbol"), &field_s(rec, "exchange"), &field_s(rec, "currency"))
        .map(|q| ("tmx".to_string(), q))
}

/// `market.quote_symbols_needing_refresh`: the held instruments whose quote is
/// older than the refresh interval, each with its source and key.
pub fn quote_symbols_needing_refresh(
    conn: &rusqlite::Connection,
    symbols: &[Value],
    now_unix: f64,
    max_age_minutes: f64,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let fetched = crate::market_fetched(conn)?;
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for rec in symbols {
        // a watched listing is keyed by symbol and venue
        let sym = {
            let k = field_s(rec, "quoteKey");
            if k.is_empty() { bagholder_model::venues::tmx_symbol(&field_s(rec, "symbol")) } else { k }
        };
        let src = quote_source(rec);
        let (source, key) = match src { Some(s) => s, None => continue };
        if sym.is_empty() || seen.contains(&sym) {
            continue;
        }
        seen.push(sym.clone());
        let last = fetched.get(&sym).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let age_ok = match instant_secs(&last) {
            Some(then) => (now_unix - then) <= max_age_minutes * 60.0,
            None => false,
        };
        if !age_ok {
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

/// `market.fetch_cboe_ca_quote`.
pub fn fetch_cboe_ca_quote(sym: &str) -> Option<Value> {
    let url = CBOE_CA_URL.replace("{}", &percent_encode(sym));
    parse_cboe_ca_quote(&get_text(&url, &[("User-Agent", UA), ("Accept", "application/json")]).ok()?)
}

/// `market.fetch_cboe_option_chain`.
pub fn fetch_cboe_option_chain(root: &str) -> Map<String, Value> {
    let url = CBOE_OPTIONS_URL.replace("{}", &percent_encode(root));
    match get_text(&url, &[("User-Agent", UA), ("Accept", "application/json")]) {
        Ok(t) => parse_cboe_options(&t),
        Err(_) => Map::new(),
    }
}

/// `market.fetch_coinbase_spot`, without the previous close: that needs the
/// candle history, which the chart path carries.
pub fn fetch_coinbase_spot(pair: &str) -> Option<Value> {
    let url = COINBASE_URL.replace("{}", &percent_encode(pair));
    parse_coinbase_rec(&get_text(&url, &[("User-Agent", UA), ("Accept", "application/json")]).ok()?, pair)
}

/// `market.fetch_for`: one quote from the named source. A chain is shared
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
        "coinbase" => fetch_coinbase_spot(key),
        "yahoo_quote" => fetch_yahoo_quote(key),
        "cboe_options" => {
            // the OCC code's root is everything before the six-digit date
            let root: String = key.chars().take_while(|c| !c.is_ascii_digit()).collect();
            if root.is_empty() {
                return None;
            }
            if !chains.contains_key(&root) {
                chains.insert(root.clone(), Value::Object(fetch_cboe_option_chain(&root)));
            }
            let row = chains.get(&root)?.get(key)?.clone();
            option_mark(&row)
        }
        _ => None,
    }
}

/// `market.refresh_quotes`.
pub fn refresh_quotes(
    conn: &rusqlite::Connection,
    symbols: &[Value],
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

/// `market.stale_symbols`: the dividend-paying Canadian listings whose declared
/// distribution record is older than its own stamp allows.
///
/// The record has a separate fetch stamp from the quote: the quote loop keeps
/// prices fresh every few minutes, and that must not make a fund's
/// distribution history look fresh.
pub fn stale_symbols(
    conn: &rusqlite::Connection,
    symbols: &[Value],
    now_unix: f64,
    stale_hours: f64,
) -> rusqlite::Result<Vec<String>> {
    let fetched = crate::market::distributions_fetched_at(conn)?;
    let mut out = Vec::new();
    for rec in symbols {
        let sym = bagholder_model::venues::tmx_symbol(&field_s(rec, "symbol"));
        if sym.is_empty() || !tmx::is_canadian_listing(&field_s(rec, "exchange"), &field_s(rec, "currency")) {
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
