//! The market universes the heatmap can show, from two public sources: the
//! S&P/TSX 60 from TMX Money (its constituents with their index weights, each
//! quoted for the day's change and its sector), and the US market from
//! Nasdaq's screener (every US listing with its price, day change, market cap,
//! sector and country, in one answer), from which the hundred largest US
//! companies and the hundred largest foreign companies listed in the US are
//! taken.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::http::{describe_failure, get_text, post_json};
use crate::news::{nasdaq_headers, pace, tmx_headers};
use bagholder_model::input::UniverseRow;
use bagholder_model::lenient;

pub const CANADA_INDEX: &str = "^TX60";
pub const TOP: usize = 100;

pub const TMX_CONSTITUENTS_QUERY: &str = "query getIndexConstituents($symbol: String!) { constituents: getIndexConstituents(symbol: $symbol) { symbol quotedMarketValue longName shortName weight exShortName exchange exLongName } }";
pub const TMX_TILE_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name price percentChange sector } }";
pub const SCREENER_URL: &str =
    "https://api.nasdaq.com/api/screener/stocks?tableonly=true&limit=25&offset=0&download=true";

pub const KEYS: [&str; 3] = ["ca", "us", "intl"];

/// A screener field is written for a page -- `$1.23`,
/// `-0.45%`, `1,234` -- and `N/A` where there is no figure.
fn figure(v: &Value) -> Option<f64> {
    let raw = match v {
        Value::Null => return None,
        Value::String(s) => s.clone(),
        Value::Number(x) => return x.as_f64(),
        Value::Bool(b) => (if *b { "True" } else { "False" }).to_string(),
        other => other.to_string(),
    };
    let s: String = raw.replace('$', "").replace('%', "").replace(',', "").trim().to_string();
    if s.is_empty() || s == "N/A" || s == "NA" || s == "None" {
        return None;
    }
    s.parse::<f64>().ok()
}

fn maybe_figure<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(figure(&Value::deserialize(d)?))
}

/// No figure and a zero figure are the same thing here, and a negative zero is
/// a zero.
fn figure_or_zero<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(or_zero(figure(&Value::deserialize(d)?).unwrap_or(0.0)))
}

fn or_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// One of Nasdaq's screener rows as it answers.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ScreenerSource {
    #[serde(deserialize_with = "lenient::text")]
    symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    name: String,
    #[serde(deserialize_with = "maybe_figure")]
    lastsale: Option<f64>,
    #[serde(deserialize_with = "maybe_figure")]
    pctchange: Option<f64>,
    #[serde(rename = "marketCap", deserialize_with = "figure_or_zero")]
    market_cap: f64,
    #[serde(deserialize_with = "lenient::text")]
    sector: String,
    #[serde(deserialize_with = "lenient::text")]
    country: String,
}

/// A US listing as the screener gives it, its sector folded into the app's.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenerRow {
    pub symbol: String,
    pub name: String,
    pub last: Option<f64>,
    pub percent_change: Option<f64>,
    pub cap: f64,
    pub sector: String,
    pub country: String,
}

/// One of an index's constituents as TMX answers.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ConstituentSource {
    #[serde(deserialize_with = "lenient::text")]
    symbol: String,
    #[serde(deserialize_with = "figure_or_zero")]
    quoted_market_value: f64,
    #[serde(deserialize_with = "lenient::text")]
    long_name: String,
    #[serde(deserialize_with = "lenient::text")]
    short_name: String,
    #[serde(deserialize_with = "figure_or_zero")]
    weight: f64,
    #[serde(deserialize_with = "lenient::text")]
    exchange: String,
}

/// One of the index's constituents: its weight in the index and its quoted
/// market value.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Constituent {
    pub symbol: String,
    pub name: String,
    pub weight: f64,
    pub cap: f64,
    pub exchange: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TileQuoteSource {
    #[serde(deserialize_with = "lenient::text")]
    name: String,
    #[serde(deserialize_with = "maybe_figure")]
    percent_change: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    sector: String,
}

/// What a constituent's own quote adds to its tile.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TileQuote {
    pub percent_change: Option<f64>,
    pub sector: String,
    pub name: String,
}

pub fn sector_of(name: &str) -> String {
    let s = bagholder_model::exposure::norm_sector(name);
    if s.is_empty() { "Not classified".into() } else { s }
}

/// Nasdaq's screener rows, each with a symbol.
pub fn parse_screener(data: &Value) -> Vec<ScreenerRow> {
    let rows = data.get("data").and_then(|d| d.get("rows")).unwrap_or(&Value::Null);
    lenient::rows::<ScreenerSource>(rows)
        .into_iter()
        .filter(|r| !r.symbol.is_empty())
        .map(|r| ScreenerRow {
            symbol: r.symbol.trim().to_string(),
            name: r.name.trim().to_string(),
            last: r.lastsale,
            percent_change: r.pctchange,
            cap: r.market_cap,
            sector: sector_of(&r.sector),
            country: r.country.trim().to_string(),
        })
        .collect()
}

/// The largest by market cap, tiles sized by market cap. The sort is
/// stable, so equal caps keep the screener's own order.
fn largest(rows: &[ScreenerRow], take: usize, keep: impl Fn(&str) -> bool) -> Vec<UniverseRow> {
    let mut picked: Vec<&ScreenerRow> = rows.iter().filter(|r| keep(&r.country) && r.cap > 0.0).collect();
    picked.sort_by(|a, b| b.cap.partial_cmp(&a.cap).unwrap_or(std::cmp::Ordering::Equal));
    picked
        .into_iter()
        .take(take)
        .map(|r| UniverseRow {
            symbol: r.symbol.clone(),
            name: r.name.clone(),
            value: r.cap,
            percent_change: r.percent_change,
            sector: r.sector.clone(),
            country: r.country.clone(),
        })
        .collect()
}

pub fn us_rows(rows: &[ScreenerRow], take: usize) -> Vec<UniverseRow> {
    largest(rows, take, |c| c == "United States")
}

/// The largest companies listed in the US from outside
/// the US and Canada.
pub fn intl_rows(rows: &[ScreenerRow], take: usize) -> Vec<UniverseRow> {
    largest(rows, take, |c| c != "United States" && c != "Canada" && !c.is_empty())
}

pub fn parse_constituents(data: &Value) -> Vec<Constituent> {
    let rows = data.get("data").and_then(|d| d.get("constituents")).unwrap_or(&Value::Null);
    lenient::rows::<ConstituentSource>(rows)
        .into_iter()
        .filter(|c| !c.symbol.is_empty())
        .map(|c| Constituent {
            symbol: c.symbol.trim().to_string(),
            name: if c.long_name.is_empty() { c.short_name } else { c.long_name }.trim().to_string(),
            weight: c.weight,
            cap: c.quoted_market_value,
            exchange: c.exchange.trim().to_string(),
        })
        .collect()
}

pub fn parse_tile_quote(data: &Value) -> Option<TileQuote> {
    let q = data.get("data")?.get("getQuoteBySymbol")?;
    if q.as_object()?.is_empty() {
        return None;
    }
    let q = TileQuoteSource::deserialize(q).ok()?;
    Some(TileQuote { percent_change: q.percent_change, sector: sector_of(&q.sector), name: q.name.trim().to_string() })
}

/// A constituent's tile: sized by its index weight, or by its quoted market
/// value where the index publishes no weight for it; its day's change and
/// sector from its own quote where that answered.
pub fn canada_tile(c: &Constituent, q: Option<&TileQuote>) -> UniverseRow {
    UniverseRow {
        symbol: c.symbol.clone(),
        name: c.name.clone(),
        value: or_zero(if c.weight == 0.0 { c.cap } else { c.weight }),
        percent_change: q.and_then(|q| q.percent_change),
        sector: q.map(|q| q.sector.clone()).unwrap_or_else(|| "Not classified".into()),
        country: "Canada".into(),
    }
}

/// Where the universes come from: Nasdaq's screener carries the US and
/// International in one answer, TMX Money the S&P/TSX 60. Each is read on its
/// own, when a universe it carries is wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Screener,
    Tmx,
}

impl Source {
    /// The source that carries the universe `key`; `None` for a key that is not a
    /// market universe.
    pub fn of(key: &str) -> Option<Source> {
        match key {
            "us" | "intl" => Some(Source::Screener),
            "ca" => Some(Source::Tmx),
            _ => None,
        }
    }

    /// The universes one read of this source replaces.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Source::Screener => &["us", "intl"],
            Source::Tmx => &["ca"],
        }
    }

    /// The source's name, as a failure names it.
    pub fn label(self) -> &'static str {
        match self {
            Source::Screener => "Nasdaq's screener",
            Source::Tmx => "TMX Money",
        }
    }
}

/// Every US listing from the screener, or what failed: an answer with no
/// listings in it is a failure, never an empty market.
pub fn fetch_screener() -> Result<Vec<ScreenerRow>, String> {
    let label = Source::Screener.label();
    pace("api.nasdaq.com", 0.6);
    let text = get_text(SCREENER_URL, &nasdaq_headers()).map_err(|e| format!("{label} {}.", describe_failure(&e)))?;
    let data: Value = serde_json::from_str(&text).map_err(|_| format!("{label} answered with something other than its listings."))?;
    let rows = parse_screener(&data);
    if rows.is_empty() {
        return Err(format!("{label} answered with no listings."));
    }
    Ok(rows)
}

/// The S&P/TSX 60, its constituents by index weight,
/// each quoted for the day's change and its sector, or what failed.
pub fn fetch_canada() -> Result<Vec<UniverseRow>, String> {
    let label = Source::Tmx.label();
    pace("app-money.tmx.com", 0.6);
    let payload = json!({
        "operationName": "getIndexConstituents",
        "variables": {"symbol": CANADA_INDEX},
        "query": TMX_CONSTITUENTS_QUERY,
    });
    let data = post_json(crate::tmx::TMX_URL, &payload, &tmx_headers()).map_err(|e| format!("{label} {}.", describe_failure(&e)))?;
    let constituents = parse_constituents(&data);
    if constituents.is_empty() {
        return Err(format!("{label} answered with no constituents of the S&P/TSX 60."));
    }
    let mut out = Vec::new();
    for c in constituents {
        pace("app-money.tmx.com", 0.6);
        let q = post_json(
            crate::tmx::TMX_URL,
            &json!({
                "operationName": "getQuoteBySymbol",
                "variables": {"symbol": c.symbol, "locale": "en"},
                "query": TMX_TILE_QUERY,
            }),
            &tmx_headers(),
        )
        .ok()
        .and_then(|d| parse_tile_quote(&d));
        out.push(canada_tile(&c, q.as_ref()));
    }
    Ok(out)
}

/// Read one source: the rows of each universe it carries, by key, or what failed.
pub fn read(source: Source) -> Result<Vec<(&'static str, Vec<UniverseRow>)>, String> {
    match source {
        Source::Screener => {
            let rows = fetch_screener()?;
            Ok(vec![("us", us_rows(&rows, TOP)), ("intl", intl_rows(&rows, TOP))])
        }
        Source::Tmx => Ok(vec![("ca", fetch_canada()?)]),
    }
}
