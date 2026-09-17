//! The market universes the heatmap can show, from two public sources: the
//! S&P/TSX 60 from TMX Money (its constituents with their index weights, each
//! quoted for the day's change and its sector), and the US market from
//! Nasdaq's screener (every US listing with its price, day change, market cap,
//! sector and country, in one answer), from which the hundred largest US
//! companies and the hundred largest foreign companies listed in the US are
//! taken.

use serde_json::{json, Value};

use crate::http::{get_text, post_json};
use crate::news::{nasdaq_headers, pace, tmx_headers};
use bagholder_model::value::{field_s, get};

pub const CANADA_INDEX: &str = "^TX60";
pub const TOP: usize = 100;

pub const TMX_CONSTITUENTS_QUERY: &str = "query getIndexConstituents($symbol: String!) { constituents: getIndexConstituents(symbol: $symbol) { symbol quotedMarketValue longName shortName weight exShortName exchange exLongName } }";
pub const TMX_TILE_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name price percentChange sector } }";
pub const SCREENER_URL: &str =
    "https://api.nasdaq.com/api/screener/stocks?tableonly=true&limit=25&offset=0&download=true";

pub const KEYS: [&str; 3] = ["ca", "us", "intl"];

/// A screener field is written for a page -- `$1.23`,
/// `-0.45%`, `1,234` -- and `N/A` where there is no figure.
fn n(v: Option<&Value>) -> Option<f64> {
    let raw = match v {
        None | Some(Value::Null) => return None,
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(x)) => return x.as_f64(),
        Some(Value::Bool(b)) => (if *b { "True" } else { "False" }).to_string(),
        Some(other) => other.to_string(),
    };
    let s: String = raw.replace('$', "").replace('%', "").replace(',', "").trim().to_string();
    if s.is_empty() || s == "N/A" || s == "NA" || s == "None" {
        return None;
    }
    s.parse::<f64>().ok()
}

fn n_or(v: Option<&Value>, default: f64) -> f64 {
    n(v).unwrap_or(default)
}

/// No figure and a zero figure are the same thing here,
/// and a negative zero is a zero.
fn or_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// A number written back into JSON: absent where there is
/// none.
fn maybe(v: Option<f64>) -> Value {
    match v {
        Some(x) => json!(x),
        None => Value::Null,
    }
}

pub fn sector_of(name: &str) -> String {
    let s = bagholder_model::exposure::norm_sector(name);
    if s.is_empty() { "Not classified".into() } else { s }
}

/// Nasdaq's screener rows into
/// {symbol, name, last, percentChange, cap, sector, country}.
pub fn parse_screener(data: &Value) -> Vec<Value> {
    let rows = data
        .get("data")
        .and_then(|d| d.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let symbol = field_s(&r, "symbol");
        if !r.is_object() || symbol.is_empty() {
            continue;
        }
        // `cap or 0.0`: a blank market cap and a zero one are the same thing here
        let cap = or_zero(n_or(get(&r, "marketCap"), 0.0));
        out.push(json!({
            "symbol": symbol.trim(),
            "name": field_s(&r, "name").trim(),
            "last": maybe(n(get(&r, "lastsale"))),
            "percentChange": maybe(n(get(&r, "pctchange"))),
            "cap": cap,
            "sector": sector_of(&field_s(&r, "sector")),
            "country": field_s(&r, "country").trim(),
        }));
    }
    out
}

fn tile(r: &Value) -> Value {
    json!({
        "symbol": field_s(r, "symbol"),
        "name": field_s(r, "name"),
        "value": get(r, "cap").cloned().unwrap_or(Value::Null),
        "percentChange": get(r, "percentChange").cloned().unwrap_or(Value::Null),
        "sector": field_s(r, "sector"),
        "country": field_s(r, "country"),
    })
}

/// The largest by market cap, tiles sized by market cap. The sort is
/// stable, so equal caps keep the screener's own order.
fn largest(rows: &[Value], take: usize, keep: impl Fn(&str) -> bool) -> Vec<Value> {
    let mut picked: Vec<&Value> = rows
        .iter()
        .filter(|r| keep(&field_s(r, "country")) && n_or(get(r, "cap"), 0.0) > 0.0)
        .collect();
    picked.sort_by(|a, b| {
        n_or(get(b, "cap"), 0.0).partial_cmp(&n_or(get(a, "cap"), 0.0)).unwrap_or(std::cmp::Ordering::Equal)
    });
    picked.into_iter().take(take).map(tile).collect()
}

pub fn us_rows(rows: &[Value], take: usize) -> Vec<Value> {
    largest(rows, take, |c| c == "United States")
}

/// The largest companies listed in the US from outside
/// the US and Canada.
pub fn intl_rows(rows: &[Value], take: usize) -> Vec<Value> {
    largest(rows, take, |c| c != "United States" && c != "Canada" && !c.is_empty())
}

pub fn parse_constituents(data: &Value) -> Vec<Value> {
    let rows = data
        .get("data")
        .and_then(|d| d.get("constituents"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for c in rows {
        let symbol = field_s(&c, "symbol");
        if !c.is_object() || symbol.is_empty() {
            continue;
        }
        let long = field_s(&c, "longName");
        let name = if long.is_empty() { field_s(&c, "shortName") } else { long };
        out.push(json!({
            "symbol": symbol.trim(),
            "name": name.trim(),
            "weight": or_zero(n_or(get(&c, "weight"), 0.0)),
            "cap": or_zero(n_or(get(&c, "quotedMarketValue"), 0.0)),
            "exchange": field_s(&c, "exchange").trim(),
        }));
    }
    out
}

pub fn parse_tile_quote(data: &Value) -> Option<Value> {
    let q = data.get("data")?.get("getQuoteBySymbol")?;
    if !q.is_object() || q.as_object()?.is_empty() {
        return None;
    }
    Some(json!({
        "percentChange": maybe(n(get(q, "percentChange"))),
        "sector": sector_of(&field_s(q, "sector")),
        "name": field_s(q, "name").trim(),
    }))
}

pub fn fetch_screener() -> Option<Vec<Value>> {
    pace("api.nasdaq.com");
    let text = get_text(SCREENER_URL, &nasdaq_headers()).ok()?;
    let data: Value = serde_json::from_str(&text).ok()?;
    Some(parse_screener(&data))
}

/// The S&P/TSX 60, its constituents by index weight,
/// each quoted for the day's change and its sector.
pub fn fetch_canada() -> Option<Vec<Value>> {
    pace("app-money.tmx.com");
    let payload = json!({
        "operationName": "getIndexConstituents",
        "variables": {"symbol": CANADA_INDEX},
        "query": TMX_CONSTITUENTS_QUERY,
    });
    let data = post_json(crate::tmx::TMX_URL, &payload, &tmx_headers()).ok()?;
    let cons = parse_constituents(&data);
    let mut out = Vec::new();
    for c in cons {
        let symbol = field_s(&c, "symbol");
        pace("app-money.tmx.com");
        let q = post_json(
            crate::tmx::TMX_URL,
            &json!({
                "operationName": "getQuoteBySymbol",
                "variables": {"symbol": symbol, "locale": "en"},
                "query": TMX_TILE_QUERY,
            }),
            &tmx_headers(),
        )
        .ok()
        .and_then(|d| parse_tile_quote(&d));
        // the index weight sizes the tile; its quoted market value where the
        // index publishes no weight for it
        let weight = n_or(get(&c, "weight"), 0.0);
        let value = if weight == 0.0 { n_or(get(&c, "cap"), 0.0) } else { weight };
        let value = or_zero(value);
        out.push(json!({
            "symbol": symbol,
            "name": field_s(&c, "name"),
            "value": value,
            "percentChange": q.as_ref().map(|q| get(q, "percentChange").cloned().unwrap_or(Value::Null)).unwrap_or(Value::Null),
            "sector": q.as_ref().map(|q| field_s(q, "sector")).unwrap_or_else(|| "Not classified".into()),
            "country": "Canada",
        }));
    }
    Some(out)
}

/// Read every universe; each answer replaces its rows.
/// Returns the keys that answered.
pub fn refresh(conn: &rusqlite::Connection, now: &str) -> Vec<String> {
    let mut done: Vec<String> = Vec::new();
    if let Some(rows) = fetch_screener() {
        let us = us_rows(&rows, TOP);
        let intl = intl_rows(&rows, TOP);
        if bagholder_store::feeds::replace_universe(conn, "us", &us, now).is_ok()
            && bagholder_store::feeds::replace_universe(conn, "intl", &intl, now).is_ok()
        {
            done.push("us".into());
            done.push("intl".into());
        }
    }
    if let Some(ca) = fetch_canada() {
        if !ca.is_empty() && bagholder_store::feeds::replace_universe(conn, "ca", &ca, now).is_ok() {
            done.push("ca".into());
        }
    }
    done
}
