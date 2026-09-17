//! Sector and country exposure of the book's holdings, for the Portfolio tab.
//!
//! A share is classified by its listing's own record: TMX Money's quote
//! carries a sector and an industry for Canadian and US listings (Nasdaq's
//! summary is the fallback for a US one), and the country is the listing
//! venue's. A fund is looked through: each issuer adapter has one job, the
//! fund's holdings with tickers and weights (or the fund's own stated
//! breakdowns), and the holdings are classified here by the same records as a
//! share, a holding that is itself a fund being looked through in turn.
//! Nothing here asks Wealthsimple for anything. What no source covers is
//! reported as unclassified, never guessed.

use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::http::FetchError;
pub use bagholder_model::exposure::{is_fund, issuer_of, norm_sector};
use bagholder_model::pytext::py_strip;

pub const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";
/// Between two requests to the same issuer.
pub const PACE: Duration = Duration::from_millis(600);
/// A fund of funds of funds is as deep as the look-through goes.
pub const MAX_DEPTH: usize = 3;
/// A breakdown older than this is fetched again.
pub const FRESH_DAYS: i64 = 7;
pub const SHARE_KEY: &str = "share:";
pub const FUND_KEY: &str = "fund:";

/// What one pass of classification works in: the database, and where it is,
/// for the name search that opens its own reader.
pub struct Ctx<'a> {
    pub conn: &'a Connection,
    pub db: std::path::PathBuf,
    pub today: String,
}


/// Stand-ins for the network sources, per thread, so the look-through can be
/// exercised on inline data (the Python tests patch the same functions).
pub mod hooks {
    use serde_json::Value;
    use std::cell::RefCell;
    pub type Classify = Box<dyn Fn(&str, &str, &str) -> Value>;
    pub type Resolve = Box<dyn Fn(&str) -> Option<Value>>;
    /// (family, symbol, name, exchange) -> breakdown
    pub type Adapter = Box<dyn Fn(&str, &str, &str, &str) -> Option<Value>>;
    /// (symbol, name, exchange) -> breakdown, or Err for a source that failed
    pub type Fallback = Box<dyn Fn(&str, &str, &str) -> Result<Option<Value>, String>>;
    pub type TmxRecord = Box<dyn Fn(&str) -> Option<Value>>;
    thread_local! {
        pub static CLASSIFY: RefCell<Option<Classify>> = RefCell::new(None);
        pub static RESOLVE: RefCell<Option<Resolve>> = RefCell::new(None);
        pub static ADAPTER: RefCell<Option<Adapter>> = RefCell::new(None);
        pub static FALLBACK: RefCell<Option<Fallback>> = RefCell::new(None);
        pub static TMX_RECORD: RefCell<Option<TmxRecord>> = RefCell::new(None);
    }
    pub fn clear() {
        CLASSIFY.with(|h| *h.borrow_mut() = None);
        RESOLVE.with(|h| *h.borrow_mut() = None);
        ADAPTER.with(|h| *h.borrow_mut() = None);
        FALLBACK.with(|h| *h.borrow_mut() = None);
        TMX_RECORD.with(|h| *h.borrow_mut() = None);
    }
}

fn s(v: Option<&Value>) -> String {
    bagholder_model::value::s(v.filter(|x| !x.is_null()))
}

/// `exposure._num`: thousands and percent signs dropped, parentheses negative.
pub fn num(v: Option<&Value>, default: f64) -> f64 {
    let raw = match v {
        None | Some(Value::Null) => return default,
        Some(Value::Number(n)) => return n.as_f64().unwrap_or(default),
        Some(other) => bagholder_model::value::s(Some(other)),
    };
    let mut t = py_strip(&raw).replace(',', "").replace('%', "");
    if t.starts_with('(') && t.ends_with(')') && t.len() >= 2 {
        t = format!("-{}", &t[1..t.len() - 1]);
    }
    bagholder_model::pytext::py_float(&t).unwrap_or(default)
}

fn num_str(t: &str, default: f64) -> f64 {
    num(Some(&json!(t)), default)
}

fn pace(host: &str) {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(HashMap::new()));
    let wait = last.lock().unwrap().get(host).map(|t| PACE.saturating_sub(t.elapsed())).unwrap_or(Duration::ZERO);
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
    last.lock().unwrap().insert(host.to_string(), Instant::now());
}

fn host_of(url: &str) -> String {
    url.split('/').nth(2).unwrap_or("").to_string()
}

fn base_headers<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut h: Vec<(&str, &str)> = vec![
        ("User-Agent", UA),
        ("Accept", "text/html,application/json;q=0.9,*/*;q=0.8"),
        ("Accept-Language", "en-CA,en;q=0.9"),
    ];
    for (k, v) in extra {
        match h.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
            Some(slot) => *slot = (k, v),
            None => h.push((k, v)),
        }
    }
    h
}

fn get(url: &str, extra: &[(&str, &str)]) -> Result<String, FetchError> {
    pace(&host_of(url));
    crate::http::get_text(url, &base_headers(extra))
}

fn post(url: &str, payload: &Value, extra: &[(&str, &str)]) -> Result<Value, FetchError> {
    pace(&host_of(url));
    crate::http::post_json(url, payload, &base_headers(extra))
}

// --- names ----------------------------------------------------------------------

const COUNTRY_ALIAS: [(&str, &str); 34] = [
    ("united states", "United States"), ("united states of america", "United States"), ("usa", "United States"), ("us", "United States"), ("u.s.", "United States"), ("u.s.a.", "United States"),
    ("canada", "Canada"), ("ca", "Canada"), ("can", "Canada"),
    ("united kingdom", "United Kingdom"), ("uk", "United Kingdom"), ("gb", "United Kingdom"), ("great britain", "United Kingdom"), ("britain", "United Kingdom"),
    ("korea", "South Korea"), ("korea, republic of", "South Korea"), ("republic of korea", "South Korea"), ("south korea", "South Korea"),
    ("taiwan, province of china", "Taiwan"), ("taiwan", "Taiwan"), ("hong kong sar", "Hong Kong"), ("hong kong", "Hong Kong"),
    ("russian federation", "Russia"), ("viet nam", "Vietnam"), ("czech republic", "Czechia"),
    ("broad", ""), ("global", ""), ("other", ""), ("-", ""), ("n/a", ""), ("cash", ""), ("", ""), ("", ""), ("", ""),
];

/// The venue a listing trades on says which country it is listed in.
const VENUE_COUNTRY: [(&str, &str); 21] = [
    ("TSX", "Canada"), ("TSX-V", "Canada"), ("TSXV", "Canada"), ("CSE", "Canada"), ("CBOE CANADA", "Canada"), ("NEO", "Canada"), ("ALPHA EXCHANGE", "Canada"),
    ("TORONTO STOCK EXCHANGE", "Canada"), ("TSX VENTURE EXCHANGE", "Canada"), ("CANADIAN SECURITIES EXCHANGE", "Canada"),
    ("NYSE", "United States"), ("NASDAQ", "United States"), ("NYSE ARCA", "United States"), ("NYSE AMERICAN", "United States"), ("BATS", "United States"), ("AMEX", "United States"), ("ARCA", "United States"),
    ("NASDAQ GLOBAL SELECT", "United States"), ("NASDAQ GLOBAL MARKET", "United States"), ("NASDAQ CAPITAL MARKET", "United States"), ("NEW YORK STOCK EXCHANGE", "United States"),
];

/// Bloomberg's market codes, as issuers write tickers ("MSFT US EQUITY").
const BLOOMBERG_COUNTRY: [(&str, &str); 36] = [
    ("US", "United States"), ("UN", "United States"), ("UW", "United States"), ("UQ", "United States"), ("UA", "United States"), ("CN", "Canada"), ("CT", "Canada"), ("CV", "Canada"),
    ("LN", "United Kingdom"), ("JP", "Japan"), ("JT", "Japan"), ("GR", "Germany"), ("GY", "Germany"), ("FP", "France"), ("AU", "Australia"), ("AT", "Australia"), ("HK", "Hong Kong"),
    ("SW", "Switzerland"), ("SE", "Switzerland"), ("NA", "Netherlands"), ("SM", "Spain"), ("IM", "Italy"), ("KS", "South Korea"), ("TT", "Taiwan"), ("IN", "India"), ("IS", "India"),
    ("BZ", "Brazil"), ("SS", "Sweden"), ("DC", "Denmark"), ("NO", "Norway"), ("FH", "Finland"), ("BB", "Belgium"), ("ID", "Ireland"), ("SP", "Singapore"), ("MM", "Mexico"), ("CH", "China"),
];

fn bloomberg(code: &str) -> String {
    let c = code.to_uppercase();
    if c == "C1" {
        return "China".into();
    }
    BLOOMBERG_COUNTRY.iter().find(|(k, _)| *k == c).map(|(_, v)| v.to_string()).unwrap_or_default()
}

/// `exposure.norm_country`.
pub fn norm_country(name: &str) -> String {
    let key = py_strip(name).to_lowercase();
    if key.is_empty() {
        return String::new();
    }
    match COUNTRY_ALIAS.iter().take(31).find(|(k, _)| *k == key) {
        Some((_, v)) => v.to_string(),
        None => py_strip(name).to_string(),
    }
}

/// `exposure.venue_country`.
pub fn venue_country(exchange: &str) -> String {
    let key = py_strip(exchange).to_uppercase();
    VENUE_COUNTRY.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string()).unwrap_or_default()
}

// --- a share ----------------------------------------------------------------------

pub const TMX_SECTOR_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name sector industry exchangeName } }";
pub const NASDAQ_SUMMARY_URL: &str = "https://api.nasdaq.com/api/quote/{}/summary?assetclass=stocks";

fn tmx_record(key: &str) -> Option<Value> {
    if let Some(r) = hooks::TMX_RECORD.with(|h| h.borrow().as_ref().map(|f| f(key))) {
        return r;
    }
    if key.is_empty() {
        return None;
    }
    pace("app-money.tmx.com");
    let d = crate::http::post_json(
        crate::tmx::TMX_URL,
        &json!({"operationName": "getQuoteBySymbol", "variables": {"symbol": key, "locale": "en"}, "query": TMX_SECTOR_QUERY}),
        &crate::http::TMX_HEADERS,
    )
    .ok()?;
    let q = d.get("data").and_then(|x| x.get("getQuoteBySymbol")).cloned().unwrap_or(Value::Null);
    let has = |k: &str| !s(q.get(k)).is_empty() && q.get(k).map(truthy).unwrap_or(false);
    if has("sector") || has("industry") || has("name") { Some(q) } else { None }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(t) => !t.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

fn nasdaq_summary(symbol: &str) -> (String, String) {
    let raw = match get(&NASDAQ_SUMMARY_URL.replace("{}", symbol), &[("Accept", "application/json, text/plain, */*")]) { Ok(t) => t, Err(_) => return (String::new(), String::new()) };
    let d: Value = match serde_json::from_str(&raw) { Ok(v) => v, Err(_) => return (String::new(), String::new()) };
    let sd = d.get("data").and_then(|x| x.get("summaryData")).cloned().unwrap_or(Value::Null);
    let val = |k: &str| s(sd.get(k).and_then(|x| x.get("value")));
    (val("Sector"), val("Industry"))
}

/// `exposure.classify_share`: {sector, industry, country, source} for one
/// listing, from TMX's record, Nasdaq's for a US listing TMX has no sector for;
/// the country is the venue's.
pub fn classify_share(ctx: &Ctx, symbol: &str, exchange: &str, currency: &str) -> Value {
    if let Some(v) = hooks::CLASSIFY.with(|h| h.borrow().as_ref().map(|f| f(symbol, exchange, currency))) {
        return v;
    }
    let sym = bagholder_model::venues::tmx_symbol(symbol);
    let country = venue_country(exchange);
    let mut sector = String::new();
    let mut industry = String::new();
    let mut out_country = country.clone();
    let mut source = String::new();
    let mut key = crate::quotes::tmx_quote_symbol(symbol, exchange, currency);
    if key.is_none() && !sym.is_empty() && !sym.contains(' ') {
        key = Some(format!("{}{}", sym, if currency.to_uppercase() == "USD" { ":US" } else { "" }));
    }
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        let (mut rec, _) = crate::tmx::tmx_lookup(ctx.conn, &k, &ctx.today, |form| tmx_record(form));
        static CDR: OnceLock<Regex> = OnceLock::new();
        if let Some(r) = rec.as_ref() {
            if country.is_empty() && CDR.get_or_init(|| Regex::new(r"\bCDR\b").unwrap()).is_match(&s(r.get("name"))) && !k.ends_with(":US") {
                // a bare ticker answered with the Canadian depositary receipt of a
                // US company; the company itself is the US listing
                if let Some(us) = tmx_record(&format!("{}:US", crate::tmx::tmx_bare(&k))) {
                    rec = Some(us);
                }
            }
        }
        if let Some(r) = rec {
            sector = norm_sector(&s(r.get("sector")));
            industry = py_strip(&s(r.get("industry"))).to_string();
            out_country = if country.is_empty() { venue_country(&s(r.get("exchangeName"))) } else { country.clone() };
            source = "TMX Money".into();
        }
    }
    if sector.is_empty() && (country == "United States" || currency.to_uppercase() == "USD") && !sym.is_empty() && !sym.contains(' ') {
        let (nsec, nind) = nasdaq_summary(&sym);
        if !nsec.is_empty() {
            sector = norm_sector(&nsec);
            if industry.is_empty() {
                industry = nind;
            }
            if out_country.is_empty() {
                out_country = "United States".into();
            }
            if source.is_empty() {
                source = "Nasdaq".into();
            }
        }
    }
    json!({"sector": sector, "industry": industry, "country": out_country, "source": source})
}

// --- the issuers -------------------------------------------------------------------

type Breakdown = Value;

fn header_index(header: &[String], names: &[&str]) -> i64 {
    let low: Vec<String> = header.iter().map(|h| h.to_lowercase()).collect();
    for n in names {
        if let Some(i) = low.iter().position(|h| h.contains(n)) {
            return i as i64;
        }
    }
    -1
}

pub const VANGUARD_GQL: &str = "https://www.vanguard.ca/gpx/graphql";
const VANGUARD_HEADERS: [(&str, &str); 5] = [
    ("Content-Type", "application/json"), ("X-Consumer-ID", "ca0"), ("apollographql-client-name", "gpx"),
    ("Origin", "https://www.vanguard.ca"), ("Referer", "https://www.vanguard.ca/en/product"),
];
/// Every Canadian Vanguard portfolio id the product list names (2026-09).
const VANGUARD_PORT_IDS: [&str; 41] = ["1811", "1817", "1936", "9561", "9554", "9559", "9560", "9569", "9570", "9558", "9555", "9550", "9549", "9742", "9556", "9548", "9828", "9835", "9795", "9563", "9562",
    "9566", "9564", "9551", "9567", "9870", "9841", "9552", "9553", "9565", "9568", "9691", "9577", "9578", "9579", "9692", "9557", "9864", "9865", "9867", "9896"];

fn vanguard_port_id(symbol: &str) -> Result<String, FetchError> {
    static MAP: OnceLock<Mutex<Map<String, Value>>> = OnceLock::new();
    let map = MAP.get_or_init(|| Mutex::new(Map::new()));
    if map.lock().unwrap().is_empty() {
        let q = json!({"operationName": "FundFinderFunds", "variables": {"portIds": VANGUARD_PORT_IDS.to_vec()},
                       "query": "query FundFinderFunds($portIds: [String!]!) { funds(portIds: $portIds) { portId profile { fundFullName listings { identifiers(altIds: [\"Ticker - Canada\", \"Ticker\"]) { altId altIdValue } } } } }"});
        let d = post(VANGUARD_GQL, &q, &VANGUARD_HEADERS)?;
        let mut m = map.lock().unwrap();
        for f in d.get("data").and_then(|x| x.get("funds")).and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            let p = f.get("profile").cloned().unwrap_or(json!({}));
            for l in p.get("listings").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                for i in l.get("identifiers").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                    let v = s(i.get("altIdValue"));
                    if !v.is_empty() {
                        let pid = { let a = s(f.get("portId")); if a.is_empty() { s(p.get("portId")) } else { a } };
                        m.entry(v.to_uppercase()).or_insert(json!(pid));
                    }
                }
            }
        }
    }
    Ok(map.lock().unwrap().get(&bagholder_model::venues::tmx_symbol(symbol)).and_then(|v| v.as_str()).unwrap_or("").to_string())
}

fn add(m: &mut Map<String, Value>, n: &str, w: f64) {
    let cur = m.get(n).and_then(|v| v.as_f64()).unwrap_or(0.0);
    m.insert(n.to_string(), json!(cur + w));
}

fn vanguard_ca(symbol: &str) -> Result<Option<Breakdown>, FetchError> {
    let pid = vanguard_port_id(symbol)?;
    if pid.is_empty() {
        return Ok(None);
    }
    let sec = post(VANGUARD_GQL, &json!({"operationName": "getSectorDiversification", "variables": {"portIds": [pid]},
        "query": "query getSectorDiversification($portIds: [String!]!) { funds(portIds: $portIds) { sectorDiversification { sectorName fundPercent date } } }"}), &VANGUARD_HEADERS)?;
    let mkt = post(VANGUARD_GQL, &json!({"operationName": "MarketAllocationGqlQuery", "variables": {"portIds": [pid]},
        "query": "query MarketAllocationGqlQuery($portIds: [String!]!) { funds(portIds: $portIds) { marketAllocation { countryName fundMktPercent date } } }"}), &VANGUARD_HEADERS)?;
    let first = |d: &Value, k: &str| -> Vec<Value> {
        d.get("data").and_then(|x| x.get("funds")).and_then(|x| x.as_array()).and_then(|a| a.first()).and_then(|f| f.get(k)).and_then(|x| x.as_array()).cloned().unwrap_or_default()
    };
    let srows = first(&sec, "sectorDiversification");
    let crows = first(&mkt, "marketAllocation");
    let mut sectors = Map::new();
    let mut countries = Map::new();
    for r in &srows {
        let (n, w) = (norm_sector(&s(r.get("sectorName"))), num(r.get("fundPercent"), 0.0));
        if !n.is_empty() && w > 0.0 {
            add(&mut sectors, &n, w);
        }
    }
    for r in &crows {
        let (n, w) = (norm_country(&s(r.get("countryName"))), num(r.get("fundMktPercent"), 0.0));
        if !n.is_empty() && w > 0.0 {
            add(&mut countries, &n, w);
        }
    }
    if sectors.is_empty() && countries.is_empty() {
        return Ok(None);
    }
    let rows = if !srows.is_empty() { &srows } else { &crows };
    let as_of = rows.first().map(|r| s(r.get("date"))).unwrap_or_default();
    Ok(Some(json!({"sectors": sectors, "countries": countries, "holdings": [], "source": "Vanguard Canada", "asOf": as_of})))
}

pub const ISHARES_SCREENER: &str = "https://www.blackrock.com/ca/investors/en/product-screener/product-screener-v3.1.jsn?dcrPath=/templatedata/config/product-screener-v3/data/en/ca-one/product-screener-backend-config&siteEntryPassthrough=true";
pub const ISHARES_HOLDINGS: &str = "https://www.blackrock.com{}/1464253357814.ajax?fileType=csv&fileName=holdings&dataType=fund";

fn ishares_page(symbol: &str) -> Result<String, FetchError> {
    static MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let map = MAP.get_or_init(|| Mutex::new(HashMap::new()));
    if map.lock().unwrap().is_empty() {
        let raw = get(ISHARES_SCREENER, &[("Accept", "application/json, text/plain, */*")])?;
        let d: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).map_err(|e| FetchError::Transport(e.to_string()))?;
        let mut m = map.lock().unwrap();
        if let Value::Object(o) = d {
            for rec in o.values() {
                let t = s(rec.get("localExchangeTicker"));
                let u = s(rec.get("productPageUrl"));
                if rec.is_object() && truthy(rec.get("localExchangeTicker").unwrap_or(&Value::Null)) && truthy(rec.get("productPageUrl").unwrap_or(&Value::Null)) {
                    m.insert(t.to_uppercase(), u);
                }
            }
        }
    }
    Ok(map.lock().unwrap().get(&bagholder_model::venues::tmx_symbol(symbol)).cloned().unwrap_or_default())
}

/// `exposure.parse_ishares_csv`: the holdings CSV, a few preamble lines then a
/// header row starting with Ticker. (holdings, as of).
pub fn parse_ishares_csv(text: &str) -> (Vec<Value>, String) {
    let body = text.trim_start_matches('\u{feff}');
    let lines = bagholder_model::pytext::splitlines(body);
    let mut as_of = String::new();
    let mut start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("Fund Holdings as of") {
            if let Ok(rows) = bagholder_model::pytext::csv_rows(line) {
                if let Some(parts) = rows.first() {
                    as_of = parts.get(1).cloned().unwrap_or_default();
                }
            }
        }
        if line.starts_with("Ticker,") || line.starts_with("\"Ticker\"") {
            start = Some(i);
            break;
        }
    }
    let start = match start { Some(s) => s, None => return (vec![], as_of) };
    // csv.reader over the remaining lines, each its own line
    let joined = lines[start..].join("\n");
    let rows = match bagholder_model::pytext::csv_rows(&joined) { Ok(r) => r, Err(_) => return (vec![], as_of) };
    let rows: Vec<Vec<String>> = rows;
    if rows.is_empty() {
        return (vec![], as_of);
    }
    let header: Vec<String> = rows[0].iter().map(|h| py_strip(h).to_string()).collect();
    let (it, iname, isec, icls, iw, iloc, iex, iccy) = (
        header_index(&header, &["ticker"]), header_index(&header, &["name"]), header_index(&header, &["sector"]), header_index(&header, &["asset class"]),
        header_index(&header, &["weight"]), header_index(&header, &["location"]), header_index(&header, &["exchange"]), header_index(&header, &["currency"]),
    );
    let cell = |r: &Vec<String>, i: i64| -> Option<String> {
        if i < 0 {
            return None;
        }
        // Python's r[-1] where an index is -1 reads the last cell; the guards above keep that out
        r.get(i as usize).cloned()
    };
    let mut out = Vec::new();
    for r in rows.iter().skip(1) {
        let need = it.max(iw);
        if (r.len() as i64) <= need || py_strip(&py_index(r, it)).is_empty() {
            continue;
        }
        let cls = if icls >= 0 && (icls as usize) < r.len() { py_strip(&r[icls as usize]).to_lowercase() } else { String::new() };
        let w = if iw >= 0 { num_str(&py_index(r, iw), 0.0) } else { 0.0 };
        if w <= 0.0 || ["cash", "money market", "futures", "derivatives", "forwards", "fx"].contains(&cls.as_str()) {
            continue;
        }
        let nm = if iname >= 0 { py_strip(&py_index(r, iname)).to_string() } else { String::new() };
        let _ = cell;
        out.push(json!({
            "ticker": py_strip(&py_index(r, it)),
            "name": nm,
            "weight": w,
            "sector": if isec >= 0 { norm_sector(&py_index(r, isec)) } else { String::new() },
            "country": if iloc >= 0 { norm_country(&py_index(r, iloc)) } else { String::new() },
            "exchange": if iex >= 0 { py_strip(&py_index(r, iex)).to_string() } else { String::new() },
            "currency": if iccy >= 0 { py_strip(&py_index(r, iccy)).to_string() } else { String::new() },
            "fund": nm.to_uppercase().contains("ISHARES") || is_fund(&nm),
        }));
    }
    (out, as_of)
}

/// `r[i]` as Python indexes a list: a negative index from the end; out of range
/// is the IndexError Python raises, which no caller here survives, so the
/// reader returns "" where the Python guard would have skipped the row first.
fn py_index(r: &[String], i: i64) -> String {
    let n = r.len() as i64;
    let j = if i < 0 { n + i } else { i };
    if j < 0 || j >= n { String::new() } else { r[j as usize].clone() }
}

fn ishares_ca(symbol: &str) -> Result<Option<Breakdown>, FetchError> {
    let page = ishares_page(symbol)?;
    if page.is_empty() {
        return Ok(None);
    }
    let (mut holdings, as_of) = parse_ishares_csv(&get(&ISHARES_HOLDINGS.replace("{}", &page), &[("Accept", "text/csv,*/*")])?);
    if holdings.is_empty() {
        return Ok(None);
    }
    for h in holdings.iter_mut() {
        if h["fund"].as_bool().unwrap_or(false) {
            // a fund's row says Other or the top holding's sector; the fund is
            // looked through instead
            h["sector"] = json!("");
        }
    }
    Ok(Some(json!({"sectors": {}, "countries": {}, "holdings": holdings, "source": "iShares Canada", "asOf": as_of})))
}

pub const HARVEST_PAGE: &str = "https://harvestportfolios.com/etf/{}/";

/// `exposure.parse_harvest_tables`: holdings from a Harvest page's tables,
/// whichever shape the fund's page uses.
pub fn parse_harvest_tables(tables: &[Vec<Vec<String>>]) -> (Vec<Value>, String) {
    static CASH: OnceLock<Regex> = OnceLock::new();
    static HOLDINGS: OnceLock<Regex> = OnceLock::new();
    let cash = CASH.get_or_init(|| Regex::new(r"(?i)written options|cash and other|cash & other").unwrap());
    let holdings_head = HOLDINGS.get_or_init(|| Regex::new(r"(?i)^holdings?\b").unwrap());
    let mut holdings: Vec<Value> = Vec::new();
    let mut reference = String::new();
    for t in tables {
        if t.is_empty() {
            continue;
        }
        let header = &t[0];
        for row in t {
            if row.len() >= 2 && py_strip(&row[0]).to_lowercase().starts_with("reference asset") {
                reference = py_strip(&row[1]).to_string();
            }
        }
        let (it, iw) = (header_index(header, &["ticker"]), header_index(header, &["weight"]));
        let (iname, isec, ictry) = (header_index(header, &["name"]), header_index(header, &["sector"]), header_index(header, &["country"]));
        if it >= 0 && iw >= 0 && iname >= 0 {
            for r in t.iter().skip(1) {
                if (r.len() as i64) <= it.max(iw).max(iname) {
                    continue;
                }
                let w = num_str(&r[iw as usize], 0.0);
                let (nm, tk) = (py_strip(&r[iname as usize]).to_string(), py_strip(&r[it as usize]).to_string());
                if w <= 0.0 || nm.is_empty() || cash.is_match(&nm) {
                    continue;
                }
                let parts: Vec<&str> = tk.split(bagholder_model::pychars::is_space).filter(|x| !x.is_empty()).collect();
                let (sym, code) = if parts.len() >= 2 { (parts[0].to_string(), parts[1].to_string()) } else { (tk.clone(), String::new()) };
                let sector = if isec >= 0 && (isec as usize) < r.len() { norm_sector(&r[isec as usize]) } else { String::new() };
                let mut country = if ictry >= 0 && (ictry as usize) < r.len() { norm_country(&r[ictry as usize]) } else { String::new() };
                if country.is_empty() {
                    country = bloomberg(&code);
                }
                holdings.push(json!({"ticker": sym, "name": nm, "weight": w, "sector": sector, "country": country, "exchange": "", "currency": "", "fund": is_fund(&nm)}));
            }
            continue;
        }
        if !header.is_empty() && holdings_head.is_match(py_strip(&header[0])) {
            for r in t.iter().skip(1) {
                if r.len() < 2 {
                    continue;
                }
                let (nm, w) = (py_strip(&r[0]).to_string(), num_str(&r[1], 0.0));
                if w <= 0.0 || nm.is_empty() || cash.is_match(&nm) {
                    continue;
                }
                holdings.push(json!({"ticker": "", "name": nm, "weight": w, "sector": "", "country": "", "exchange": "", "currency": "", "fund": is_fund(&nm)}));
            }
        }
    }
    (holdings, reference)
}

fn harvest(symbol: &str) -> Result<Option<Breakdown>, FetchError> {
    let html = get(&HARVEST_PAGE.replace("{}", &bagholder_model::venues::tmx_symbol(symbol).to_lowercase()), &[])?;
    let (mut holdings, reference) = parse_harvest_tables(&crate::htmltables::html_tables(&html));
    static TICKER: OnceLock<Regex> = OnceLock::new();
    let ticker = TICKER.get_or_init(|| Regex::new(r"^[A-Z0-9][A-Z0-9.:-]{0,9}$").unwrap());
    if !reference.is_empty() && ticker.is_match(&reference) && !holdings.iter().any(|h| !s(h.get("ticker")).is_empty()) {
        // a single-stock fund names its reference asset as a ticker: that is
        // the whole exposure
        holdings = vec![json!({"ticker": reference, "name": reference, "weight": 100.0, "sector": "", "country": "", "exchange": "", "currency": "", "fund": false})];
    }
    if holdings.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({"sectors": {}, "countries": {}, "holdings": holdings, "source": "Harvest ETFs", "asOf": ""})))
}

pub const NINEPOINT_LIST: &str = "https://www.ninepoint.com/landing-pages/ninepoint-highshares-etfs/";
pub const NINEPOINT_BASE: &str = "https://www.ninepoint.com";

/// `exposure.parse_ninepoint_page`: (ticker, underlying ticker, underlying
/// exchange) from a Ninepoint fund page.
pub fn parse_ninepoint_page(html: &str) -> (String, String, String) {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    static TICKER: OnceLock<Regex> = OnceLock::new();
    static UNDER: OnceLock<Regex> = OnceLock::new();
    let text = TAGS.get_or_init(|| Regex::new(r"<[^>]+>").unwrap()).replace_all(html, " ");
    let text = WS.get_or_init(|| Regex::new(r"\s+").unwrap()).replace_all(&text, " ");
    let m = TICKER.get_or_init(|| Regex::new(r"Ticker\s*\*?\*?\s*([A-Z0-9.]{1,8}):([A-Z]{2,6})\b").unwrap()).captures(&text);
    let u = UNDER.get_or_init(|| Regex::new(r"Underlying Stock\s*\*?\*?\s*(.*?)\(([A-Z0-9.]{1,8}):([A-Z]{2,6})\)").unwrap()).captures(&text);
    (
        m.as_ref().map(|c| c[1].to_string()).unwrap_or_default(),
        u.as_ref().map(|c| c[2].to_string()).unwrap_or_default(),
        u.as_ref().map(|c| c[3].to_string()).unwrap_or_default(),
    )
}

fn ninepoint(symbol: &str) -> Result<Option<Breakdown>, FetchError> {
    static PAGES: OnceLock<Mutex<Map<String, Value>>> = OnceLock::new();
    let pages = PAGES.get_or_init(|| Mutex::new(Map::new()));
    let sym = bagholder_model::venues::tmx_symbol(symbol);
    if !pages.lock().unwrap().contains_key(&sym) {
        let html = get(NINEPOINT_LIST, &[])?;
        static SLUG: OnceLock<Regex> = OnceLock::new();
        let mut slugs: Vec<String> = SLUG
            .get_or_init(|| Regex::new(r#"href="(?:https://www\.ninepoint\.com)?(/funds/[a-z0-9-]+/)""#).unwrap())
            .captures_iter(&html)
            .map(|c| c[1].to_string())
            .collect();
        slugs.sort();
        slugs.dedup();
        for slug in slugs {
            if pages.lock().unwrap().contains_key(&sym) {
                break;
            }
            if pages.lock().unwrap().values().any(|v| v.as_str() == Some(slug.as_str())) {
                continue;
            }
            let page = match get(&format!("{}{}", NINEPOINT_BASE, slug), &[]) { Ok(p) => p, Err(_) => continue };
            let (t, _, _) = parse_ninepoint_page(&page);
            if !t.is_empty() {
                pages.lock().unwrap().insert(t, json!(slug));
            }
        }
    }
    let slug = match pages.lock().unwrap().get(&sym).and_then(|v| v.as_str()) { Some(s) => s.to_string(), None => return Ok(None) };
    let (_, under, ex) = parse_ninepoint_page(&get(&format!("{}{}", NINEPOINT_BASE, slug), &[])?);
    if under.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({"sectors": {}, "countries": {}, "holdings": [{"ticker": under, "name": under, "weight": 100.0, "sector": "", "country": "", "exchange": ex, "currency": "", "fund": false}],
        "source": "Ninepoint", "asOf": ""})))
}

pub const EVOLVE_PAGE: &str = "https://evolveetfs.com/product/{}/";

/// `exposure.parse_evolve_page`: (sectors, holdings) from the page's embedded
/// `portfolioBreakdownData` and `holdingsData`.
pub fn parse_evolve_page(html: &str) -> (Map<String, Value>, Vec<Value>) {
    static BREAKDOWN: OnceLock<Regex> = OnceLock::new();
    static HOLDINGS: OnceLock<Regex> = OnceLock::new();
    let mut sectors = Map::new();
    let mut holdings = Vec::new();
    if let Some(m) = BREAKDOWN.get_or_init(|| Regex::new(r"(?s)var portfolioBreakdownData\s*=\s*(\{.*?\});\s*\n").unwrap()).captures(html) {
        if let Ok(v) = serde_json::from_str::<Value>(&m[1]) {
            for r in v.get("data").and_then(|d| d.get("sector")).and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                let (n, w) = (norm_sector(&s(r.get("name"))), num(r.get("weight"), 0.0));
                if !n.is_empty() && w > 0.0 {
                    add(&mut sectors, &n, w);
                }
            }
        }
    }
    if let Some(m) = HOLDINGS.get_or_init(|| Regex::new(r"(?s)var holdingsData\s*=\s*(\{.*?\});\s*\n").unwrap()).captures(html) {
        let rows = serde_json::from_str::<Value>(&m[1]).ok().and_then(|v| v.get("data").and_then(|d| d.as_array()).cloned()).unwrap_or_default();
        for r in rows {
            let tk = py_strip(&s(r.get("ticker"))).to_string();
            let parts: Vec<&str> = tk.split(bagholder_model::pychars::is_space).filter(|x| !x.is_empty()).collect();
            let (sym, code) = if parts.len() >= 2 { (parts[0].to_string(), parts[1].to_string()) } else { (tk.clone(), String::new()) };
            let w = num(r.get("weight_percent"), 0.0);
            let nm = py_strip(&s(r.get("security_name"))).to_string();
            if w <= 0.0 || sym.is_empty() {
                continue;
            }
            let mut ctry = norm_country(&s(r.get("country")));
            if !ctry.is_empty() && ctry.chars().count() <= 5 && ctry.to_uppercase() == ctry {
                // a fund-of-funds page writes the sub-fund's ticker here, not a country
                ctry = String::new();
            }
            if ctry.is_empty() {
                ctry = bloomberg(&code);
            }
            holdings.push(json!({"ticker": sym, "name": nm, "weight": w, "sector": norm_sector(&s(r.get("gics_sector"))), "country": ctry, "exchange": "", "currency": "", "fund": is_fund(&nm)}));
        }
    }
    (sectors, holdings)
}

fn evolve(symbol: &str) -> Result<Option<Breakdown>, FetchError> {
    let (sectors, holdings) = parse_evolve_page(&get(&EVOLVE_PAGE.replace("{}", &bagholder_model::venues::tmx_symbol(symbol).to_lowercase()), &[])?);
    if sectors.is_empty() && holdings.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({"sectors": sectors, "countries": {}, "holdings": holdings, "source": "Evolve ETFs", "asOf": ""})))
}

pub const YAHOO_CRUMB: &str = "https://query2.finance.yahoo.com/v1/test/getcrumb";
pub const YAHOO_SUMMARY: &str = "https://query2.finance.yahoo.com/v10/finance/quoteSummary/{}?modules=topHoldings&crumb={}";
const YAHOO_SUFFIX: [(&str, &str); 6] = [("TSX", ".TO"), ("TSX-V", ".V"), ("TSXV", ".V"), ("CSE", ".CN"), ("CBOE CANADA", ".NE"), ("NEO", ".NE")];

fn yahoo_session() -> Result<(String, String), FetchError> {
    static SESSION: OnceLock<Mutex<(String, String)>> = OnceLock::new();
    let sess = SESSION.get_or_init(|| Mutex::new((String::new(), String::new())));
    {
        let s = sess.lock().unwrap();
        if !s.1.is_empty() {
            return Ok(s.clone());
        }
    }
    pace("fc.yahoo.com");
    let cookies: Vec<String> = match crate::client::request_any("GET", "https://fc.yahoo.com", &[("User-Agent", UA)], None, Duration::from_secs(crate::http::TIMEOUT_SEC)) {
        Ok(r) => r.headers.iter().filter(|(k, _)| k == "set-cookie").map(|(_, v)| v.split(';').next().unwrap_or("").to_string()).collect(),
        Err(_) => vec![],
    };
    let cookie = cookies.join("; ");
    let crumb = py_strip(&get(YAHOO_CRUMB, &[("Cookie", &cookie)])?).to_string();
    *sess.lock().unwrap() = (cookie.clone(), crumb.clone());
    Ok((cookie, crumb))
}

/// `exposure.yahoo_symbol`.
pub fn yahoo_symbol(symbol: &str, exchange: &str) -> String {
    let ex = py_strip(exchange).to_uppercase();
    format!("{}{}", bagholder_model::venues::tmx_symbol(symbol), YAHOO_SUFFIX.iter().find(|(k, _)| *k == ex).map(|(_, v)| *v).unwrap_or(""))
}

/// `round(x, 4)`.
fn round4(x: f64) -> f64 {
    format!("{:.4}", x).parse().unwrap_or(x)
}

/// `exposure.parse_yahoo_summary`: (sectors, holdings) from Yahoo's
/// topHoldings module.
pub fn parse_yahoo_summary(data: &Value) -> (Map<String, Value>, Vec<Value>) {
    let res = data.get("quoteSummary").and_then(|q| q.get("result")).and_then(|r| r.as_array()).and_then(|a| a.first()).cloned().unwrap_or(json!({}));
    let th = res.get("topHoldings").cloned().unwrap_or(json!({}));
    let mut sectors = Map::new();
    for entry in th.get("sectorWeightings").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
        if let Value::Object(e) = entry {
            for (k, v) in e {
                let w = match &v { Value::Object(m) => num(m.get("raw"), 0.0), other => num(Some(other), 0.0) };
                let n = norm_sector(&k.replace('_', " "));
                if !n.is_empty() && w > 0.0 {
                    let cur = sectors.get(&n).and_then(|x| x.as_f64()).unwrap_or(0.0);
                    sectors.insert(n, json!(round4(cur + w * 100.0)));
                }
            }
        }
    }
    let mut holdings = Vec::new();
    for h in th.get("holdings").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
        let sym = py_strip(&s(h.get("symbol"))).to_string();
        let w = match h.get("holdingPercent") { Some(Value::Object(m)) => num(m.get("raw"), 0.0), other => num(other, 0.0) };
        if !sym.is_empty() && w > 0.0 {
            let mut ex = "";
            for (suf, venue) in [(".TO", "TSX"), (".V", "TSX-V"), (".CN", "CSE"), (".NE", "CBOE CANADA")] {
                if sym.to_uppercase().ends_with(suf) {
                    ex = venue;
                }
            }
            let name = s(h.get("holdingName"));
            holdings.push(json!({"ticker": sym, "name": name, "weight": round4(w * 100.0), "sector": "", "country": "", "exchange": ex,
                                 "currency": if ex.is_empty() { "" } else { "CAD" }, "fund": is_fund(&name)}));
        }
    }
    (sectors, holdings)
}

fn yahoo_fund(symbol: &str, exchange: &str) -> Result<Option<Breakdown>, FetchError> {
    let (cookie, crumb) = yahoo_session()?;
    let raw = get(&YAHOO_SUMMARY.replacen("{}", &yahoo_symbol(symbol, exchange), 1).replacen("{}", &crumb, 1), &[("Cookie", &cookie), ("Accept", "application/json")])?;
    let d: Value = serde_json::from_str(&raw).map_err(|e| FetchError::Transport(e.to_string()))?;
    let (sectors, holdings) = parse_yahoo_summary(&d);
    if sectors.is_empty() && holdings.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({"sectors": sectors, "countries": {}, "holdings": holdings, "source": "Yahoo Finance", "asOf": ""})))
}

// --- the look-through ------------------------------------------------------------

/// `exposure.resolve_name`: a holding named without a ticker, the directories'
/// first match on the name.
pub fn resolve_name(ctx: &Ctx, name: &str) -> Option<Value> {
    if let Some(v) = hooks::RESOLVE.with(|h| h.borrow().as_ref().map(|f| f(name))) {
        return v;
    }
    static SUFFIX: OnceLock<Regex> = OnceLock::new();
    static JUNK: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let clean = SUFFIX
        .get_or_init(|| Regex::new(r"(?i)\b(inc|corp|corporation|ltd|limited|plc|co|class [a-z]|common shares?|common stock|the)\b\.?").unwrap())
        .replace_all(name, " ");
    let clean = JUNK.get_or_init(|| Regex::new(r"[^A-Za-z0-9 &.-]").unwrap()).replace_all(&clean, " ");
    let clean = WS.get_or_init(|| Regex::new(r"\s+").unwrap()).replace_all(&clean, " ");
    let clean = py_strip(&clean).to_string();
    if clean.is_empty() {
        return None;
    }
    let r = crate::search::symbol_search(&ctx.db, &clean.chars().take(40).collect::<String>());
    r.get("matches").and_then(|m| m.as_array()).and_then(|a| a.first()).cloned()
}

fn cache_get(ctx: &Ctx, key: &str) -> Option<Value> {
    let rec = bagholder_store::feeds::exposure_record(ctx.conn, key).ok()??;
    let fetched = s(rec.get("fetchedAt"));
    let (d, _) = fetched.split_once('T')?;
    if fetched.len() != 20 || !fetched.ends_with('Z') {
        return None;
    }
    let (y, m, dd) = bagholder_model::dates::parse_iso(d)?;
    let then = bagholder_model::dates::to_days(y, m, dd) * 86400 + {
        let t = &fetched[11..19];
        let p: Vec<i64> = t.split(':').filter_map(|x| x.parse().ok()).collect();
        if p.len() != 3 {
            return None;
        }
        p[0] * 3600 + p[1] * 60 + p[2]
    };
    let (_, now, _) = crate::clock_now();
    let age_days = ((now as i64) - then).div_euclid(86400);
    if age_days < FRESH_DAYS { Some(rec) } else { None }
}

fn store_exposure(ctx: &Ctx, key: &str, rec: &Value) {
    let _ = bagholder_store::feeds::replace_exposure(ctx.conn, key, rec, &crate::now_stamp());
}

/// `exposure.share_exposure`: a classified share as an exposure record, cached
/// by ticker and venue form.
pub fn share_exposure(ctx: &Ctx, symbol: &str, exchange: &str, currency: &str) -> Value {
    let key = format!("{}{}:{}", SHARE_KEY, bagholder_model::venues::tmx_symbol(symbol), bagholder_model::venues::tmx_form(exchange, currency).unwrap_or(""));
    if let Some(hit) = cache_get(ctx, &key) {
        return hit;
    }
    let c = classify_share(ctx, symbol, exchange, currency);
    let (sec, ctry) = (s(c.get("sector")), s(c.get("country")));
    let mut sectors = Map::new();
    if !sec.is_empty() {
        sectors.insert(sec.clone(), json!(1.0));
    }
    let mut countries = Map::new();
    if !ctry.is_empty() {
        countries.insert(ctry.clone(), json!(1.0));
    }
    let rec = json!({"sectors": sectors, "countries": countries, "coverage": if !sec.is_empty() || !ctry.is_empty() { 1.0 } else { 0.0 },
                     "source": c["source"], "asOf": "", "industry": c["industry"]});
    store_exposure(ctx, &key, &rec);
    rec
}

fn first_key(v: Option<&Value>) -> String {
    v.and_then(|x| x.as_object()).and_then(|m| m.keys().next().cloned()).unwrap_or_default()
}

/// `exposure.lookthrough`: holdings into {sectors, countries, coverage} --
/// weights over the positive rows, each row classified as given, by its
/// ticker, by its name, or by looking a fund through.
pub fn lookthrough(ctx: &Ctx, holdings: &[Value], depth: usize, seen: &mut Vec<String>) -> Value {
    let rows: Vec<&Value> = holdings.iter().filter(|h| num(h.get("weight"), 0.0) > 0.0).collect();
    let total: f64 = rows.iter().map(|h| num(h.get("weight"), 0.0)).sum();
    if total <= 0.0 {
        return json!({"sectors": {}, "countries": {}, "coverage": 0.0});
    }
    let mut sectors = Map::new();
    let mut countries = Map::new();
    let mut covered = 0.0;
    for h in rows {
        let w = num(h.get("weight"), 0.0) / total;
        let (mut sec, mut ctry) = (s(h.get("sector")), s(h.get("country")));
        let (mut tk, mut ex, mut ccy) = (s(h.get("ticker")), s(h.get("exchange")), s(h.get("currency")));
        if !sec.is_empty() && !ctry.is_empty() {
            // the issuer states both: nothing to look up
            add(&mut sectors, &sec, w);
            add(&mut countries, &ctry, w);
            covered += w;
            continue;
        }
        let name = s(h.get("name"));
        if tk.is_empty() && !name.is_empty() {
            if let Some(m) = resolve_name(ctx, &name) {
                tk = s(m.get("symbol"));
                ex = s(m.get("exchange"));
                ccy = s(m.get("currency"));
            }
        }
        let mut sub: Option<Value> = None;
        if h.get("fund").map(truthy).unwrap_or(false) && depth < MAX_DEPTH && (!tk.is_empty() || !name.is_empty()) {
            sub = fund_exposure(ctx, &tk, &name, &ex, depth + 1, seen);
        }
        if let Some(sb) = sub.as_ref() {
            let has = |k: &str| sb.get(k).map(truthy).unwrap_or(false);
            if has("sectors") || has("countries") {
                for (n, f) in sb.get("sectors").and_then(|x| x.as_object()).cloned().unwrap_or_default() {
                    add(&mut sectors, &n, w * f.as_f64().unwrap_or(0.0));
                }
                for (n, f) in sb.get("countries").and_then(|x| x.as_object()).cloned().unwrap_or_default() {
                    add(&mut countries, &n, w * f.as_f64().unwrap_or(0.0));
                }
                covered += w * sb.get("coverage").and_then(|x| x.as_f64()).unwrap_or(0.0);
                continue;
            }
        }
        if !tk.is_empty() {
            if ex.is_empty() && ctry == "United States" && ccy.is_empty() {
                ccy = "USD".into();
            }
            let c = share_exposure(ctx, &tk, &ex, &ccy);
            if sec.is_empty() {
                sec = first_key(c.get("sectors"));
            }
            if ctry.is_empty() {
                ctry = first_key(c.get("countries"));
            }
        }
        if !sec.is_empty() {
            add(&mut sectors, &sec, w);
        }
        if !ctry.is_empty() {
            add(&mut countries, &ctry, w);
        }
        if !sec.is_empty() || !ctry.is_empty() {
            covered += w;
        }
    }
    json!({"sectors": sectors, "countries": countries, "coverage": covered.min(1.0)})
}

/// `exposure.fund_exposure`: a fund's {sectors, countries, coverage, source,
/// asOf}, through its issuer's adapter, cached by ticker; None when no source
/// covers its family or the source answered nothing.
pub fn fund_exposure(ctx: &Ctx, symbol: &str, name: &str, exchange: &str, depth: usize, seen: &mut Vec<String>) -> Option<Value> {
    let key = format!("{}{}", FUND_KEY, bagholder_model::venues::tmx_symbol(if symbol.is_empty() { name } else { symbol }));
    if seen.contains(&key) {
        return None;
    }
    seen.push(key.clone());
    if let Some(hit) = cache_get(ctx, &key) {
        return Some(hit);
    }
    let family = issuer_of(name);
    let adapter: Option<fn(&str) -> Result<Option<Breakdown>, FetchError>> = match family {
        "vanguard" => Some(vanguard_ca),
        "ishares" => Some(ishares_ca),
        "harvest" => Some(harvest),
        "ninepoint" => Some(ninepoint),
        "evolve" => Some(evolve),
        _ => None,
    };
    let mut data: Option<Value> = None;
    let hooked = hooks::ADAPTER.with(|h| h.borrow().as_ref().map(|f| f(family, symbol, name, exchange)));
    if let Some(d) = hooked {
        data = d;
    } else if let Some(f) = adapter {
        match f(symbol) {
            Ok(d) => data = d,
            Err(e) => crate::http::note_source(family, false, Some(&e)),
        }
    }
    if data.is_none() {
        // a family with no adapter, or one whose page answered nothing
        match hooks::FALLBACK.with(|h| h.borrow().as_ref().map(|f| f(symbol, name, exchange))) {
            Some(Ok(d)) => data = d,
            Some(Err(_)) => {}
            None => match yahoo_fund(symbol, exchange) {
                Ok(d) => data = d,
                Err(e) => crate::http::note_source("yahoo", false, Some(&e)),
            },
        }
    }
    let data = data?;
    let scale = |k: &str| -> Map<String, Value> {
        data.get(k).and_then(|x| x.as_object()).map(|m| m.iter().map(|(n, w)| (n.clone(), json!(w.as_f64().unwrap_or(0.0) / 100.0))).collect()).unwrap_or_default()
    };
    let mut sectors = scale("sectors");
    let mut countries = scale("countries");
    let mut coverage: f64 = if !sectors.is_empty() || !countries.is_empty() { 1.0 } else { 0.0 };
    let holdings = data.get("holdings").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    if !holdings.is_empty() && (sectors.is_empty() || countries.is_empty()) {
        let agg = lookthrough(ctx, &holdings, depth, seen);
        if sectors.is_empty() {
            sectors = agg["sectors"].as_object().cloned().unwrap_or_default();
        }
        if countries.is_empty() {
            countries = agg["countries"].as_object().cloned().unwrap_or_default();
        }
        let agg_cov = agg["coverage"].as_f64().unwrap_or(0.0);
        coverage = if !sectors.is_empty() && !countries.is_empty() { coverage.max(agg_cov) } else { agg_cov };
    }
    let tot = |m: &Map<String, Value>| m.values().map(|v| v.as_f64().unwrap_or(0.0)).sum::<f64>();
    let (tot_s, tot_c) = (tot(&sectors), tot(&countries));
    if tot_s > 1.0001 {
        sectors = sectors.into_iter().map(|(n, w)| (n, json!(w.as_f64().unwrap_or(0.0) / tot_s))).collect();
    }
    if tot_c > 1.0001 {
        countries = countries.into_iter().map(|(n, w)| (n, json!(w.as_f64().unwrap_or(0.0) / tot_c))).collect();
    }
    let source = { let v = s(data.get("source")); if v.is_empty() { family.to_string() } else { v } };
    let rec = json!({"sectors": sectors, "countries": countries, "coverage": coverage, "source": source, "asOf": s(data.get("asOf"))});
    store_exposure(ctx, &key, &rec);
    Some(rec)
}

/// `exposure.refresh_security`: the exposure record for one of the book's
/// securities, stored under its id -- a fund looked through, a share
/// classified.
pub fn refresh_security(ctx: &Ctx, sec: &Value) -> Value {
    let sid = s(sec.get("id"));
    let (symbol, name, exchange, currency) = (s(sec.get("symbol")), s(sec.get("name")), s(sec.get("primaryExchange")), s(sec.get("currency")));
    let rec = if is_fund(&name) {
        // a fund no source covers is unclassified: its venue says nothing about what it holds
        let mut seen = Vec::new();
        fund_exposure(ctx, &symbol, &name, &exchange, 0, &mut seen)
            .unwrap_or_else(|| json!({"sectors": {}, "countries": {}, "coverage": 0.0, "source": "", "asOf": ""}))
    } else {
        share_exposure(ctx, &symbol, &exchange, &currency)
    };
    store_exposure(ctx, &sid, &rec);
    rec
}

/// `exposure.stale`: the ids whose record is missing or older than a week.
pub fn stale(ctx: &Ctx, ids: &[String]) -> Vec<String> {
    ids.iter().filter(|sid| cache_get(ctx, sid).is_none()).cloned().collect()
}
