//! Short selling for one listing, from the regulator that publishes it: FINRA
//! for a US listing, CIRO for a Canadian one.
//!
//! Two measures are kept apart, because they answer different questions and
//! are never added together or folded into one number:
//!
//! - the short position: the shares still sold short, as the dealers reported
//!   them, which both regulators publish twice a month, on the 15th and the
//!   last day of each month;
//! - the short volume: the part of a period's trading that was sold short,
//!   which the US publishes for every trading day and Canada for each
//!   half-month.
//!
//! Neither is live, and nothing published anywhere is. Nothing here is
//! estimated or filled in -- every figure is the regulator's own. Days to
//! cover is the one derived number, and it is derived the same way on both
//! markets.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::http::{get_text, post_json, FetchError, TIMEOUT_SEC};
use bagholder_model::dates;
use bagholder_model::lenient;
use bagholder_model::textrules::{csv_records, parse_float, splitlines};
use bagholder_model::value::s as vs;
use bagholder_store::feeds::{ShortMarket, ShortPoint, Shorts, VolumeSpan};

pub const US_POSITION_URL: &str = "https://api.finra.org/data/group/otcMarket/name/consolidatedShortInterest";
pub const US_VOLUME_URL: &str = "https://cdn.finra.org/equity/regsho/daily/CNMSshvol{}.txt";
pub const CA_POSITION_URL: &str = "https://www.ciro.ca/sites/default/files/epubs/CSPR/{}_CSPR_Report.xls";
pub const CA_VOLUME_URL: &str = "https://www.ciro.ca/sites/default/files/epubs/SSALE/{}-{}_ShortSaleTradingSummaryReport.csv";
pub const CA_CBOE_URL: &str = "https://www-api.cboe.com/ca/equities/listing-directory-data/";
/// What that venue calls the listings whose units in issue are their float.
pub const CBOE_FUNDS: [&str; 2] = ["etf", "cef"];
/// What the Canadian files call each venue, against what the app calls it.
pub const CA_VENUES: [(&str, &[&str]); 4] = [
    ("TSX", &["TSX"]),
    ("TSXV", &["TSX-V", "TSXV"]),
    ("CSE", &["CSE"]),
    ("AQL", &["CBOE CANADA", "NEO"]),
];
/// And how the app writes each of them, for a listing the app knew no venue
/// for.
pub const CA_VENUE_NAMES: [(&str, &str); 4] = [("TSX", "TSX"), ("TSXV", "TSX-V"), ("CSE", "CSE"), ("AQL", "Cboe Canada")];
/// How often a whole-market file is looked for again.
pub const FILE_HOURS: u64 = 6;
/// How many report dates back to try before giving up.
pub const TRIES: usize = 6;
/// Reports behind the run shown with the position.
pub const SERIES: usize = 8;
/// How often a float that answered is looked up again.
pub const FLOAT_HOURS: u64 = 12;
/// A lookup that answered with nothing is tried again far sooner: a float that
/// did not arrive is usually the source being slow, not the figure being
/// absent.
pub const FLOAT_MISS_MIN: u64 = 20;
pub const YAHOO_QUOTE_URL: &str = "https://finance.yahoo.com/quote/{}/";
pub const YAHOO_CRUMB_URL: &str = "https://query1.finance.yahoo.com/v1/test/getcrumb";
pub const YAHOO_STATS_URL: &str = "https://query1.finance.yahoo.com/v10/finance/quoteSummary/{}?modules=defaultKeyStatistics&crumb={}";
pub const TMX_UNITS_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol shareOutStanding } }";
const TMX_HISTORY_QUERY: &str = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }";

fn headers() -> [(&'static str, &'static str); 2] {
    [("User-Agent", crate::http::UA), ("Accept", "*/*")]
}

// --- numbers ---------------------------------------------------------------------

/// A field as a number, thousands separators dropped and whitespace trimmed;
/// None where it is not a number.
pub fn num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Bool(_)) => None,
        Some(Value::String(t)) => parse_float(&t.replace(',', "")),
        Some(other) => parse_float(&vs(Some(other)).replace(',', "")),
    }
}

fn truthy(v: Option<f64>) -> bool {
    matches!(v, Some(x) if x != 0.0)
}

/// A non-finite figure is no figure.
fn finite(x: f64) -> Option<f64> {
    if x.is_finite() { Some(x) } else { None }
}

/// `round(x, 1)`: the nearest tenth of the exact binary value, ties to even,
/// which is what Rust's formatting does too.
fn round1(x: f64) -> f64 {
    format!("{:.1}", x).parse().unwrap_or(x)
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.get(k)
}

fn field(v: &Value, k: &str) -> String {
    vs(v.get(k).filter(|x| !x.is_null()))
}

/// A field read leniently from a FINRA answer row, commas and all -- the same
/// rule `num` above applies to every other regulator's numbers.
fn lenient_num<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(num(Some(&v)))
}

// --- which regulator, if any, publishes for a listing -------------------------

/// The regulator that publishes for an instrument, or `None` for one no one
/// reports short selling on -- a coin, an index, a futures or currency
/// contract, an option.
pub fn market_of(symbol: &str, exchange: &str, currency: &str) -> Option<ShortMarket> {
    let sym = symbol.trim().to_uppercase();
    let ex = exchange.trim().to_uppercase();
    if sym.is_empty()
        || bagholder_model::symbols::is_option_symbol(&sym)
        || ex == "CRYPTO"
        || bagholder_model::instruments::find(&sym, &ex).is_some()
    {
        return None;
    }
    match bagholder_model::venues::tmx_form(&ex, currency) {
        None => None,
        Some(":US") => Some(ShortMarket::Us),
        Some(_) => Some(ShortMarket::Ca),
    }
}

// --- when each report is for ---------------------------------------------------

type Day = (i64, u32, u32);

fn day_of(iso: &str) -> Day {
    dates::parse_iso(iso).unwrap_or((1970, 1, 1))
}

fn iso(d: Day) -> String {
    dates::fmt(d.0, d.1, d.2)
}

fn last_of(year: i64, month: u32) -> Day {
    (year, month, dates::days_in_month(year, month))
}

fn prev_month(year: i64, month: u32) -> (i64, u32) {
    if month == 1 { (year - 1, 12) } else { (year, month - 1) }
}

/// The reporting dates of the twice-monthly position
/// reports, newest first.
pub fn position_dates(today: &str, back: usize) -> Vec<String> {
    let t = day_of(today);
    let (mut year, mut month) = (t.0, t.1);
    let mut out = Vec::new();
    while out.len() < back {
        for d in [last_of(year, month), (year, month, 15)] {
            if d <= t && out.len() < back {
                out.push(iso(d));
            }
        }
        (year, month) = prev_month(year, month);
    }
    out
}

/// The half-month periods the Canadian volume report
/// covers, newest first.
pub fn volume_periods(today: &str, back: usize) -> Vec<(String, String)> {
    let t = day_of(today);
    let (mut year, mut month) = (t.0, t.1);
    let mut out = Vec::new();
    while out.len() < back {
        for (start, end) in [((year, month, 16), last_of(year, month)), ((year, month, 1), (year, month, 15))] {
            if end <= t && out.len() < back {
                out.push((iso(start), iso(end)));
            }
        }
        (year, month) = prev_month(year, month);
    }
    out
}

/// The days the US volume file could be for, newest
/// first -- weekdays. A holiday has no file and is skipped by the asking.
pub fn trading_days(today: &str, back: usize) -> Vec<String> {
    let t = day_of(today);
    let mut n = dates::to_days(t.0, t.1, t.2);
    let mut out = Vec::new();
    while out.len() < back {
        // 1970-01-01 was a Thursday
        let weekday = (n + 3).rem_euclid(7);
        if weekday < 5 {
            let (y, m, d) = dates::from_days(n);
            out.push(dates::fmt(y, m, d));
        }
        n -= 1;
    }
    out
}

fn compact(iso: &str) -> String {
    iso.replace('-', "")
}

// --- the whole-market files, read once and kept --------------------------------

/// One whole-market file, read once and kept for `FILE_HOURS`: every listing
/// looked up in it shares the one read.
#[derive(Clone)]
pub struct Held<T> {
    pub key: String,
    pub rows: HashMap<String, T>,
    at: Instant,
}

/// A cache of named whole-market files, all of the one row type.
pub struct Files<T> {
    store: OnceLock<Mutex<HashMap<String, Held<T>>>>,
}

impl<T> Files<T> {
    pub const fn new() -> Self {
        Files { store: OnceLock::new() }
    }

    fn cache(&self) -> &Mutex<HashMap<String, Held<T>>> {
        self.store.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Forget every file this cache holds.
    pub fn clear(&self) {
        self.cache().lock().unwrap().clear();
    }

    /// Make a held file look `secs` older, as a test setting its `at` back
    /// does. A name this cache does not hold is left alone.
    pub fn age(&self, name: &str, secs: u64) {
        if let Some(h) = self.cache().lock().unwrap().get_mut(name) {
            h.at = h.at.checked_sub(Duration::from_secs(secs)).unwrap_or(h.at);
        }
    }
}

impl<T: Clone> Files<T> {
    /// Put a file in place as though it had just been read.
    pub fn warm(&self, name: &str, key: &str, rows: HashMap<String, T>) {
        self.cache().lock().unwrap().insert(name.to_string(), Held { key: key.to_string(), rows, at: Instant::now() });
    }

    /// The rows of a named file already held, whatever its age -- a file kept
    /// forever once read, as one dated Canadian report is.
    fn cached(&self, name: &str) -> Option<HashMap<String, T>> {
        self.cache().lock().unwrap().get(name).map(|h| h.rows.clone())
    }

    /// A file every listing is looked up in, fetched at most once
    /// every FILE_HOURS. A fetch that fails keeps what was already read rather
    /// than emptying it.
    pub fn table<F>(&self, name: &str, build: F) -> Held<T>
    where
        F: FnOnce() -> Option<(String, HashMap<String, T>)>,
    {
        if let Some(held) = self.cache().lock().unwrap().get(name) {
            if held.at.elapsed() < Duration::from_secs(FILE_HOURS * 3600) {
                return held.clone();
            }
        }
        let built = build();
        let mut f = self.cache().lock().unwrap();
        let held = match built {
            None => {
                let mut h = f.get(name).cloned().unwrap_or(Held { key: String::new(), rows: HashMap::new(), at: Instant::now() });
                h.at = Instant::now();
                h
            }
            Some((key, rows)) => Held { key, rows, at: Instant::now() },
        };
        f.insert(name.to_string(), held.clone());
        held
    }
}

/// FINRA's daily short volume file, one symbol.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsVolumeRow {
    pub short_volume: f64,
    pub total_volume: f64,
}

/// CIRO's position report row: issue name, venue, shares short, net change.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaPositionRow {
    pub venue: String,
    pub shares: f64,
    pub change: Option<f64>,
    pub name: String,
}

/// CIRO's short sale summary row: the short part of a period's trading.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaVolumeRow {
    pub venue: String,
    pub short_volume: f64,
    pub volume_pct: Option<f64>,
    pub total_volume: Option<f64>,
}

static US_VOLUME_FILES: Files<UsVolumeRow> = Files::new();
static CA_POSITION_FILES: Files<CaPositionRow> = Files::new();
static CA_VOLUME_FILES: Files<CaVolumeRow> = Files::new();
static CBOE_FILES: Files<f64> = Files::new();

/// Forget every whole-market file.
pub fn clear_files() {
    US_VOLUME_FILES.clear();
    CA_POSITION_FILES.clear();
    CA_VOLUME_FILES.clear();
    CBOE_FILES.clear();
}

/// Make a held file look `secs` older, as a test setting its `at` back does.
pub fn age_file(name: &str, secs: u64) {
    US_VOLUME_FILES.age(name, secs);
    CA_POSITION_FILES.age(name, secs);
    CA_VOLUME_FILES.age(name, secs);
    CBOE_FILES.age(name, secs);
}

/// Put a Canadian volume file in place as though it had just been read (the
/// only whole-market file a test warms directly).
pub fn warm_ca_volume(name: &str, key: &str, rows: HashMap<String, CaVolumeRow>) {
    CA_VOLUME_FILES.warm(name, key, rows);
}

/// FINRA's daily file, one pipe-separated line per
/// symbol, a trailer at the end.
pub fn parse_us_volume(text: &str) -> HashMap<String, UsVolumeRow> {
    let mut rows = HashMap::new();
    for line in splitlines(text).into_iter().skip(1) {
        let parts: Vec<&str> = line.trim().split('|').collect();
        if parts.len() < 5 {
            continue;
        }
        let sym = parts[1].trim().to_uppercase();
        let short = parse_float(&parts[2].replace(',', ""));
        let total = parse_float(&parts[4].replace(',', ""));
        if !sym.is_empty() && short.is_some() && truthy(total) {
            rows.insert(sym, UsVolumeRow { short_volume: short.unwrap(), total_volume: total.unwrap() });
        }
    }
    rows
}

fn us_volume_file(today: &str) -> Option<(String, HashMap<String, UsVolumeRow>)> {
    us_volume_file_with(today, |url| get_text(url, &headers()).map_err(|e| e.to_string()))
}

/// `us_volume_file` with the fetch handed in.
pub fn us_volume_file_with<G: FnMut(&str) -> Result<String, String>>(today: &str, mut get: G) -> Option<(String, HashMap<String, UsVolumeRow>)> {
    for d in trading_days(today, TRIES) {
        let url = US_VOLUME_URL.replace("{}", &compact(&d));
        let rows = match get(&url) {
            Ok(t) => parse_us_volume(&t),
            Err(_) => continue,
        };
        if !rows.is_empty() {
            return Some((d, rows));
        }
    }
    None
}

/// CIRO's position report -- issue name, symbol,
/// venue, shares short, net change.
pub fn parse_ca_positions(grid: &[Vec<Value>]) -> HashMap<String, CaPositionRow> {
    let mut rows = HashMap::new();
    for r in grid {
        if r.len() < 5 {
            continue;
        }
        let sym = vs(Some(&r[1])).trim().to_uppercase();
        let shares = num(Some(&r[3]));
        if sym.is_empty() || shares.is_none() {
            continue;
        }
        rows.insert(sym, CaPositionRow {
            venue: vs(Some(&r[2])).trim().to_uppercase(),
            shares: shares.unwrap(),
            change: num(Some(&r[4])),
            name: vs(Some(&r[0])).trim().to_string(),
        });
    }
    rows
}

fn get_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    bagholder_net::client::request("GET", url, &headers(), None, Duration::from_secs(TIMEOUT_SEC)).map(|r| r.body)
}

fn ca_position_file(today: &str) -> Option<(String, HashMap<String, CaPositionRow>)> {
    ca_position_file_with(today, |url| get_bytes(url).map_err(|e| e.to_string()).and_then(|raw| crate::xls::table(&raw)))
}

/// `ca_position_file` with the fetch and the spreadsheet reading handed in.
pub fn ca_position_file_with<G: FnMut(&str) -> Result<Vec<Vec<Value>>, String>>(today: &str, mut grid_of: G) -> Option<(String, HashMap<String, CaPositionRow>)> {
    for d in position_dates(today, TRIES) {
        let url = CA_POSITION_URL.replace("{}", &compact(&d));
        let rows = grid_of(&url).map(|grid| parse_ca_positions(&grid));
        match rows {
            Err(_) => continue,
            Ok(rows) if !rows.is_empty() => return Some((d, rows)),
            Ok(_) => {}
        }
    }
    None
}

/// A Canadian listing's own volume over the report's
/// period, from TMX's daily series under the venue's own form.
pub fn ca_traded(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, span: &str, today: &str) -> Option<f64> {
    let (start, end) = match span.split_once('/') { Some((a, b)) => (a.to_string(), b.to_string()), None => (span.to_string(), String::new()) };
    if end.is_empty() {
        return None;
    }
    let code = crate::quotes::tmx_quote_symbol(symbol, exchange, currency)?;
    let ask = |form: &str| -> Result<Option<Vec<bagholder_store::bars::DayBar>>, FetchError> {
        let data = post_json(crate::tmx::TMX_URL, &json!({
            "operationName": "getTimeSeriesData",
            "variables": {"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end},
            "query": TMX_HISTORY_QUERY,
        }), &crate::http::TMX_HEADERS)?;
        let bars = crate::parse::parse_tmx_history(&data);
        Ok(if bars.is_empty() { None } else { Some(bars) })
    };
    let bars = match crate::tmx::tmx_lookup_try(conn, &code, today, ask) {
        Ok((b, _)) => b.unwrap_or_default(),
        Err(e) => {
            eprintln!("bagholder shorts: {} traded volume failed: {}", symbol, e);
            return None;
        }
    };
    let traded: Vec<f64> = bars.iter().filter_map(|b| b.px.volume.filter(|v| *v != 0.0)).collect();
    if traded.is_empty() { None } else { Some(traded.iter().sum()) }
}

/// CIRO's short sale summary, the short part of a
/// period's trading, per listing.
pub fn parse_ca_volume(text: &str) -> Result<HashMap<String, CaVolumeRow>, String> {
    let mut rows = HashMap::new();
    for r in csv_records(text.trim_start_matches('\u{feff}'))? {
        let r = Value::Object(r);
        let sym = field(&r, "Security").trim().to_uppercase();
        let short = num(get(&r, "Short Traded Volume"));
        let pct = num(get(&r, "% Total Traded Volume"));
        if sym.is_empty() || short.is_none() {
            continue;
        }
        let total = if truthy(pct) { finite(short.unwrap() / pct.unwrap() * 100.0) } else { None };
        rows.insert(sym, CaVolumeRow {
            venue: field(&r, "Listing Market").trim().to_uppercase(),
            short_volume: short.unwrap(),
            volume_pct: pct,
            total_volume: total,
        });
    }
    Ok(rows)
}

fn ca_volume_file(today: &str) -> Option<(String, HashMap<String, CaVolumeRow>)> {
    ca_volume_file_with(today, |url| get_text(url, &headers()).map_err(|e| e.to_string()))
}

/// `ca_volume_file` with the fetch handed in.
pub fn ca_volume_file_with<G: FnMut(&str) -> Result<String, String>>(today: &str, mut get: G) -> Option<(String, HashMap<String, CaVolumeRow>)> {
    for (start, end) in volume_periods(today, TRIES) {
        let url = CA_VOLUME_URL.replacen("{}", &compact(&start), 1).replacen("{}", &compact(&end), 1);
        let rows = match get(&url) {
            Ok(t) => match parse_ca_volume(&t) { Ok(r) => r, Err(_) => continue },
            Err(_) => continue,
        };
        if !rows.is_empty() {
            return Some((format!("{}/{}", start, end), rows));
        }
    }
    None
}

/// One dated Canadian report, kept for the session
/// so a run of them is read once.
fn ca_positions_on(day: &str) -> HashMap<String, CaPositionRow> {
    let name = format!("ca_position:{}", day);
    if let Some(rows) = CA_POSITION_FILES.cached(&name) {
        return rows;
    }
    let rows = get_bytes(&CA_POSITION_URL.replace("{}", &compact(day)))
        .map_err(|e| e.to_string())
        .and_then(|raw| crate::xls::table(&raw))
        .map(|g| parse_ca_positions(&g))
        .unwrap_or_default();
    CA_POSITION_FILES.warm(&name, day, rows.clone());
    rows
}

/// The listing's position across the last reports, oldest
/// first. Canada publishes one file per reporting date, so each is read on its
/// own and kept.
pub fn ca_series(symbol: &str, exchange: &str, asof: &str, today: &str, back: usize) -> Vec<ShortPoint> {
    ca_series_with(symbol, exchange, asof, today, back, ca_positions_on)
}

/// `ca_series` with each dated report's rows handed in.
pub fn ca_series_with<R: FnMut(&str) -> HashMap<String, CaPositionRow>>(symbol: &str, exchange: &str, asof: &str, today: &str, back: usize, mut ca_positions_on: R) -> Vec<ShortPoint> {
    let sym = symbol.trim().to_uppercase();
    let mut out: Vec<ShortPoint> = Vec::new();
    for day in position_dates(today, back) {
        if !asof.is_empty() && day.as_str() > asof {
            continue;
        }
        let rows = ca_positions_on(&day);
        if let Some(row) = rows.get(&sym) {
            if venue_fits(&row.venue, exchange) {
                out.push(ShortPoint { date: day, shares: row.shares });
            }
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date));
    out
}

// --- the float -----------------------------------------------------------------

struct Yahoo {
    session: bagholder_net::browser::Session,
    crumb: String,
}

fn yahoo() -> &'static Mutex<Option<Yahoo>> {
    static Y: OnceLock<Mutex<Option<Yahoo>>> = OnceLock::new();
    Y.get_or_init(|| Mutex::new(None))
}

/// A session that can read Yahoo's statistics. Its
/// own TLS handshake is the gate, so it goes through the browser helper;
/// without it the float is simply unknown, as it is for a listing Yahoo does
/// not carry.
fn yahoo_open(slot: &mut Option<Yahoo>) -> bool {
    if slot.is_some() {
        return true;
    }
    let mut session = match bagholder_net::browser::Session::new() { Some(s) => s, None => return false };
    let timeout = Duration::from_secs(TIMEOUT_SEC);
    if let Err(e) = session.get(&YAHOO_QUOTE_URL.replace("{}", "AAPL"), timeout) {
        eprintln!("bagholder shorts: yahoo would not open: {}", e);
        return false;
    }
    let crumb = match session.get(YAHOO_CRUMB_URL, timeout) {
        Ok(a) => a.text().trim().to_string(),
        Err(e) => {
            eprintln!("bagholder shorts: yahoo would not open: {}", e);
            return false;
        }
    };
    if crumb.is_empty() || crumb.chars().count() > 32 {
        return false;
    }
    *slot = Some(Yahoo { session, crumb });
    true
}

/// The units a fund listed on Cboe Canada has in issue,
/// from that venue's own directory -- its market capitalisation divided by its
/// last price, which gives back the count the exchange put in, whole for every
/// listing it carries.
fn cboe_units(symbol: &str) -> Option<f64> {
    cboe_units_with(symbol, || get_text(CA_CBOE_URL, &headers()).ok())
}

/// `cboe_units` with the directory's fetch handed in; the directory is kept as
/// a whole-market file.
pub fn cboe_units_with<G: FnOnce() -> Option<String>>(symbol: &str, get: G) -> Option<f64> {
    let held = CBOE_FILES.table("cboe_listings", || Some(("cboe".to_string(), parse_cboe_directory(&get()?)?)));
    held.rows.get(&symbol.trim().to_uppercase()).copied()
}

/// The fund counts in Cboe Canada's listing directory.
pub fn parse_cboe_directory(text: &str) -> Option<HashMap<String, f64>> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let mut rows = HashMap::new();
    for r in doc.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default() {
        if !CBOE_FUNDS.contains(&field(&r, "security").trim().to_lowercase().as_str()) {
            continue;
        }
        let (cap, last) = (num(r.get("marketcap")), num(r.get("last")));
        if !truthy(cap) || !truthy(last) {
            continue;
        }
        let count = cap.unwrap() / last.unwrap();
        // anything else is not the exchange's own count
        if (count - count.round_ties_even()).abs() < 1e-6 {
            rows.insert(field(&r, "symbol").trim().to_uppercase(), count.round_ties_even());
        }
    }
    Some(rows)
}

/// The units an exchange-traded fund has in issue, from
/// the market's own source. For a fund this is the float, not a stand-in.
fn fund_units(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, today: &str) -> Option<f64> {
    fund_units_with(symbol, exchange, currency, |code| tmx_units(conn, code, today), cboe_units)
}

/// `fund_units` with TMX's count and the venue's directory handed in.
pub fn fund_units_with<T, C>(symbol: &str, exchange: &str, currency: &str, tmx: T, cboe: C) -> Option<f64>
where
    T: FnOnce(&str) -> Option<f64>,
    C: FnOnce(&str) -> Option<f64>,
{
    let sym = symbol.trim().to_uppercase();
    if market_of(&sym, exchange, currency) == Some(ShortMarket::Us) {
        // the US count comes from Yahoo with the float
        return None;
    }
    let count = crate::quotes::tmx_quote_symbol(&sym, exchange, currency).and_then(|code| tmx(&code)).filter(|c| *c != 0.0);
    if truthy(count) {
        return count;
    }
    // the venue is asked for its own listings only: a count taken from the
    // wrong venue would be the wrong fund's
    let ex = exchange.trim().to_uppercase();
    if !CA_VENUES[3].1.contains(&ex.as_str()) {
        return None;
    }
    cboe(&sym)
}

fn tmx_units(conn: &rusqlite::Connection, code: &str, today: &str) -> Option<f64> {
    let mut count: Option<f64> = None;
    let sym = code;
    {
        let ask = |form: &str| -> Result<Option<Value>, FetchError> {
            let answered = post_json(crate::tmx::TMX_URL, &json!({
                "operationName": "getQuoteBySymbol",
                "variables": {"symbol": form, "locale": "en"},
                "query": TMX_UNITS_QUERY,
            }), &crate::http::TMX_HEADERS)?;
            let q = answered.get("data").and_then(|d| d.get("getQuoteBySymbol")).cloned().unwrap_or(Value::Null);
            Ok(match &q { Value::Object(m) if !m.is_empty() => Some(q), _ => None })
        };
        match crate::tmx::tmx_lookup_try(conn, code, today, ask) {
            Ok((q, _)) => count = q.and_then(|q| num(q.get("shareOutStanding"))).filter(|c| *c != 0.0),
            Err(e) => eprintln!("bagholder shorts: {} units failed: {}", sym, e),
        }
    }
    count
}

struct Float {
    value: Option<f64>,
    at: Instant,
}

fn floats() -> &'static Mutex<HashMap<String, Float>> {
    static F: OnceLock<Mutex<HashMap<String, Float>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What a short position is measured against, the
/// shares actually available to trade. For a company that is the free float
/// Yahoo publishes, never swapped for the shares in issue; a fund's units in
/// issue are its float, and stand in where no float is published for one.
pub fn float_shares(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, name: &str, today: &str) -> Option<f64> {
    let mut slot = yahoo().lock().unwrap();
    let opened = yahoo_open(&mut slot);
    let ask = |form: &str| -> Option<(u16, String)> {
        let y = slot.as_mut()?;
        let url = YAHOO_STATS_URL.replacen("{}", form, 1).replacen("{}", &y.crumb, 1);
        match y.session.get(&url, Duration::from_secs(TIMEOUT_SEC)) {
            Ok(a) => Some((a.status, a.text())),
            Err(e) => {
                eprintln!("bagholder shorts: {} float from yahoo failed: {}", form, e);
                None
            }
        }
    };
    float_shares_with(symbol, exchange, currency, name, opened, ask, crate::quotes::yahoo_may_ask, crate::quotes::yahoo_back_off, |sym, ccy| {
        fund_units(conn, sym, exchange, ccy, today)
    })
}

/// Forget every float.
pub fn clear_floats() {
    floats().lock().unwrap().clear();
}

/// Make a kept float look `secs` older.
pub fn age_float(key: &str, secs: u64) {
    if let Some(h) = floats().lock().unwrap().get_mut(key) {
        h.at = h.at.checked_sub(Duration::from_secs(secs)).unwrap_or(h.at);
    }
}

/// `float_shares` with Yahoo and the fund count handed in: `ask` answers a
/// symbol form with a status and a body, `session` says whether there is a
/// client to ask with at all.
#[allow(clippy::too_many_arguments)]
pub fn float_shares_with<A, T, B, U>(symbol: &str, exchange: &str, currency: &str, name: &str, session: bool, mut ask: A, mut turn: T, mut back_off: B, units: U) -> Option<f64>
where
    A: FnMut(&str) -> Option<(u16, String)>,
    T: FnMut() -> bool,
    B: FnMut(),
    U: FnOnce(&str, &str) -> Option<f64>,
{
    let sym = symbol.trim().to_uppercase();
    let key = format!("{}|{}", sym, exchange.trim().to_uppercase());
    if let Some(held) = floats().lock().unwrap().get(&key) {
        let keep = if truthy(held.value) { FLOAT_HOURS * 3600 } else { FLOAT_MISS_MIN * 60 };
        if held.at.elapsed() < Duration::from_secs(keep) {
            return held.value;
        }
    }
    let fund = bagholder_model::exposure::is_fund(name);
    // the symbol forms follow the market the venue already settled on, not the
    // currency the row happens to carry
    let where_ = market_of(&sym, exchange, currency);
    let ccy = if currency.trim().is_empty() { if where_ == Some(ShortMarket::Us) { "USD".to_string() } else { "CAD".to_string() } } else { currency.trim().to_string() };
    let mut count: Option<f64> = None;
    if session {
        let mut forms = crate::quotes::yahoo_forms(&bagholder_model::input::Listing::new(sym.clone(), exchange, ccy.clone(), ""));
        if forms.is_empty() {
            forms = vec![bagholder_model::venues::tmx_symbol(&sym)];
        }
        for form in forms {
            if !turn() {
                continue;
            }
            let (status, text) = match ask(&form) { Some(a) => a, None => continue };
            if status == 429 {
                back_off();
                continue;
            }
            if status != 200 {
                continue;
            }
            let doc: Value = match serde_json::from_str(&text) {
                Ok(Value::Object(m)) => Value::Object(m),
                _ => {
                    eprintln!("bagholder shorts: {} float from yahoo failed: not a statistics answer", form);
                    continue;
                }
            };
            let result = doc.get("quoteSummary").filter(|v| truthy_json(v)).and_then(|q| q.get("result")).filter(|v| truthy_json(v));
            let first = match result.and_then(|r| r.as_array()).and_then(|a| a.first()) {
                Some(v) if truthy_json(v) => v.clone(),
                _ => json!({}),
            };
            let stats = first.get("defaultKeyStatistics").filter(|v| truthy_json(v)).cloned().unwrap_or_else(|| json!({}));
            let pick = |f: &str| -> Option<f64> {
                match stats.get(f) {
                    Some(Value::Object(m)) => num(m.get("raw")),
                    other => num(other),
                }
            };
            let floated = pick("floatShares");
            count = if truthy(floated) { floated } else if fund { pick("sharesOutstanding") } else { None };
            if truthy(count) {
                break;
            }
        }
    }
    if !truthy(count) && fund {
        count = units(&sym, &ccy);
    }
    let count = count.filter(|c| *c != 0.0);
    floats().lock().unwrap().insert(key, Float { value: count, at: Instant::now() });
    count
}

/// Whether a JSON value counts as present: not null, false, zero, or an empty
/// string, array or object.
fn truthy_json(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// Whether a row's venue is the listing's.
pub fn venue_fits(code: &str, exchange: &str) -> bool {
    let ex = exchange.trim().to_uppercase();
    if ex.is_empty() {
        return true;
    }
    let code = code.trim().to_uppercase();
    CA_VENUES.iter().find(|(k, _)| *k == code).map(|(_, v)| v.contains(&ex.as_str())).unwrap_or(false)
}

// --- one listing ---------------------------------------------------------------

/// One reporting date's position, and the run behind it: what `finish` builds
/// a listing's short selling from, before its float and derived figures are
/// added.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub as_of: String,
    pub shares: Option<f64>,
    pub previous: Option<f64>,
    pub previous_of: String,
    pub change: Option<f64>,
    pub average_volume: Option<f64>,
    /// The reports behind this one, oldest first; `None` where they were not
    /// read.
    pub series: Option<Vec<ShortPoint>>,
    pub venue: String,
    pub issuer: String,
}

/// The short part of a period's trading, as one regulator publishes it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortVolume {
    pub volume_of: String,
    pub volume_span: VolumeSpan,
    pub short_volume: Option<f64>,
    pub total_volume: Option<f64>,
    pub volume_pct: Option<f64>,
}

/// One of FINRA's settlement rows, read leniently.
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct UsPositionSource {
    #[serde(rename = "settlementDate", deserialize_with = "lenient::text")]
    settlement_date: String,
    #[serde(rename = "currentShortPositionQuantity", deserialize_with = "lenient_num")]
    current: Option<f64>,
    #[serde(rename = "previousShortPositionQuantity", deserialize_with = "lenient_num")]
    previous: Option<f64>,
    #[serde(rename = "changePreviousNumber", deserialize_with = "lenient_num")]
    change: Option<f64>,
    #[serde(rename = "averageDailyVolumeQuantity", deserialize_with = "lenient_num")]
    average_volume: Option<f64>,
}

/// FINRA's answer into the newest settlement and
/// the run of them. Split from the request so the two can be compared on the
/// same answer.
pub fn parse_us_position(answered: &Value) -> Position {
    let mut rows: Vec<UsPositionSource> = lenient::rows(answered);
    rows.retain(|r| !r.settlement_date.is_empty());
    if rows.is_empty() {
        return Position::default();
    }
    rows.sort_by(|a, b| b.settlement_date.cmp(&a.settlement_date));
    let date10 = |s: &str| s.chars().take(10).collect::<String>();
    let mut ascending = rows.clone();
    ascending.sort_by(|a, b| a.settlement_date.cmp(&b.settlement_date));
    // every settlement FINRA answered with, oldest first
    let series: Vec<ShortPoint> = ascending.iter().filter(|x| x.current.is_some()).map(|x| ShortPoint { date: date10(&x.settlement_date), shares: x.current.unwrap() }).collect();
    let r = &rows[0];
    Position {
        as_of: date10(&r.settlement_date),
        shares: r.current,
        previous: r.previous,
        previous_of: if rows.len() > 1 { date10(&rows[1].settlement_date) } else { String::new() },
        change: r.change,
        average_volume: r.average_volume,
        series: Some(series),
        venue: String::new(),
        issuer: String::new(),
    }
}

/// The newest settlement FINRA has for a US listing.
pub fn us_position(symbol: &str, today: &str) -> Position {
    let body = json!({
        "limit": 20,
        "compareFilters": [{"fieldName": "symbolCode", "fieldValue": symbol.trim().to_uppercase(), "compareType": "EQUAL"}],
        "dateRangeFilters": [{"fieldName": "settlementDate", "startDate": dates::shift_date(today, -150), "endDate": today}],
    });
    match post_json(US_POSITION_URL, &body, &[("Accept", "application/json")]) {
        Ok(answered) => parse_us_position(&answered),
        Err(e) => {
            eprintln!("bagholder shorts: {} position from finra failed: {}", symbol, e);
            Position::default()
        }
    }
}

/// The last trading day's short volume for a US listing.
pub fn us_volume(symbol: &str, today: &str) -> Option<ShortVolume> {
    let held = US_VOLUME_FILES.table("us_volume", || us_volume_file(today));
    us_volume_from(&held.key, &held.rows, symbol)
}

/// `us_volume` with the file's reading handed in.
pub fn us_volume_with<G: FnOnce() -> Option<(String, HashMap<String, UsVolumeRow>)>>(symbol: &str, read: G) -> Option<ShortVolume> {
    let held = US_VOLUME_FILES.table("us_volume", read);
    us_volume_from(&held.key, &held.rows, symbol)
}

/// `ca_position` with the file's reading handed in.
pub fn ca_position_with<G: FnOnce() -> Option<(String, HashMap<String, CaPositionRow>)>>(symbol: &str, exchange: &str, today: &str, read: G) -> Position {
    let held = CA_POSITION_FILES.table("ca_position", read);
    ca_position_from(&held.key, &held.rows, symbol, exchange, today)
}

/// `ca_volume` with the file's reading and the exchange's traded volume
/// (given the report's period) handed in.
pub fn ca_volume_with<G, F>(symbol: &str, exchange: &str, read: G, traded: F) -> Option<ShortVolume>
where
    G: FnOnce() -> Option<(String, HashMap<String, CaVolumeRow>)>,
    F: FnOnce(&str) -> Option<f64>,
{
    let held = CA_VOLUME_FILES.table("ca_volume", read);
    if held.key.is_empty() {
        return None;
    }
    let row = held.rows.get(&symbol.trim().to_uppercase());
    ca_volume_from(&held.key, row, exchange, || traded(&held.key))
}

pub fn us_volume_from(key: &str, rows: &HashMap<String, UsVolumeRow>, symbol: &str) -> Option<ShortVolume> {
    let row = rows.get(&symbol.trim().to_uppercase())?;
    let pct = if row.total_volume != 0.0 { Some(row.short_volume / row.total_volume * 100.0) } else { None };
    Some(ShortVolume {
        volume_of: key.to_string(),
        volume_span: VolumeSpan::Day,
        short_volume: finite(row.short_volume),
        total_volume: finite(row.total_volume),
        volume_pct: pct.and_then(finite),
    })
}

pub fn ca_position(symbol: &str, exchange: &str, today: &str) -> Position {
    let held = CA_POSITION_FILES.table("ca_position", || ca_position_file(today));
    ca_position_from(&held.key, &held.rows, symbol, exchange, today)
}

pub fn ca_position_from(key: &str, rows: &HashMap<String, CaPositionRow>, symbol: &str, exchange: &str, today: &str) -> Position {
    let row = match rows.get(&symbol.trim().to_uppercase()) { Some(r) => r, None => return Position::default() };
    if !venue_fits(&row.venue, exchange) {
        return Position::default();
    }
    // the report before this one is the previous reporting date, which is what
    // the change is against
    let earlier: Vec<String> = position_dates(today, TRIES).into_iter().filter(|d| d.as_str() < key).collect();
    Position {
        as_of: key.to_string(),
        shares: finite(row.shares),
        change: row.change,
        previous: row.change.and_then(|c| finite(row.shares - c)),
        previous_of: earlier.first().cloned().unwrap_or_default(),
        average_volume: None,
        series: None,
        // the report names the venue and the issuer
        venue: row.venue.trim().to_uppercase(),
        issuer: row.name.trim().to_string(),
    }
}

/// The short part of a Canadian listing's trading over the
/// report's period. A listing the report does not carry was not sold short in
/// it, so its short volume is none of its trading rather than unknown, and
/// what it did trade comes from the exchange.
pub fn ca_volume(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, today: &str) -> Option<ShortVolume> {
    let held = CA_VOLUME_FILES.table("ca_volume", || ca_volume_file(today));
    if held.key.is_empty() {
        return None;
    }
    let row = held.rows.get(&symbol.trim().to_uppercase());
    ca_volume_from(&held.key, row, exchange, || ca_traded(conn, symbol, exchange, currency, &held.key, today))
}

pub fn ca_volume_from<F: FnOnce() -> Option<f64>>(key: &str, row: Option<&CaVolumeRow>, exchange: &str, traded: F) -> Option<ShortVolume> {
    match row {
        Some(r) if !venue_fits(&r.venue, exchange) => None,
        None => match traded() {
            Some(t) if t != 0.0 => Some(ShortVolume { volume_of: key.to_string(), volume_span: VolumeSpan::Period, short_volume: Some(0.0), total_volume: finite(t), volume_pct: Some(0.0) }),
            _ => None,
        },
        Some(r) => Some(ShortVolume { volume_of: key.to_string(), volume_span: VolumeSpan::Period, short_volume: finite(r.short_volume), total_volume: r.total_volume.and_then(finite), volume_pct: r.volume_pct.and_then(finite) }),
    }
}

/// The average daily volume in the listing's own
/// market over the period its short report covers. FINRA publishes the
/// average itself; Canada's total is divided by the days the Canadian market
/// actually traded, counted from the index series the app keeps.
pub fn average_volume(conn: &rusqlite::Connection, rec: &Shorts) -> Option<f64> {
    if rec.market == ShortMarket::Us {
        return rec.average_volume;
    }
    let total = rec.total_volume;
    if !truthy(total) || !rec.volume_of.contains('/') {
        return None;
    }
    let (start, end) = rec.volume_of.split_once('/').unwrap();
    let days = bagholder_store::tables::benchmark_days(conn, "TSX", start, end).unwrap_or(0);
    if days != 0 { finite(total.unwrap() / days as f64) } else { None }
}

/// The position against that average daily volume.
pub fn days_to_cover(conn: &rusqlite::Connection, rec: &Shorts) -> Option<f64> {
    let shares = rec.shares;
    let average = average_volume(conn, rec);
    if truthy(shares) && truthy(average) { Some(round1(shares.unwrap() / average.unwrap())) } else { None }
}

/// Everything published about one listing's short
/// selling, `None` where the listing is on a market no one publishes for.
pub fn for_listing(
    conn: &rusqlite::Connection,
    symbol: &str,
    exchange: &str,
    currency: &str,
    today: &str,
    trend: bool,
    name: &str,
) -> Option<Shorts> {
    let sym = symbol.trim().to_uppercase();
    let market = market_of(&sym, exchange, currency)?;
    let (position, volume) = if market == ShortMarket::Us {
        (us_position(&sym, today), us_volume(&sym, today))
    } else {
        (ca_position(&sym, exchange, today), ca_volume(conn, &sym, exchange, currency, today))
    };
    Some(finish(conn, position, volume, &sym, exchange, trend, market, |asof| ca_series(&sym, exchange, asof, today, SERIES), |issuer| {
        float_shares(conn, &sym, exchange, currency, if name.trim().is_empty() { issuer } else { name.trim() }, today)
    }))
}

/// The part of `for_listing` after the regulators have answered, split out so
/// it can be run on fixed answers.
#[allow(clippy::too_many_arguments)]
pub fn finish<S, F>(
    conn: &rusqlite::Connection,
    position: Position,
    volume: Option<ShortVolume>,
    sym: &str,
    exchange: &str,
    trend: bool,
    market: ShortMarket,
    series: S,
    float_of: F,
) -> Shorts
where
    S: FnOnce(&str) -> Vec<ShortPoint>,
    F: FnOnce(&str) -> Option<f64>,
{
    // a listing the app knew no venue for takes the one the regulator's own
    // report gives it, and the issuer's name with it
    let venue = position.venue.trim().to_uppercase();
    let issuer = position.issuer.trim().to_string();
    let ex = exchange.trim().to_uppercase();
    let exchange_out = if !ex.is_empty() {
        ex
    } else {
        CA_VENUE_NAMES.iter().find(|(k, _)| *k == venue).map(|(_, v)| v.to_string()).unwrap_or_default()
    };
    // a ticker is not a name: the issuer the report names is what tells a fund
    // from a company, and so what its short position is measured against
    let floated = float_of(&issuer).and_then(finite);
    let shares = position.shares;
    let of_float = if truthy(floated) && truthy(shares) { finite(shares.unwrap() / floated.unwrap() * 100.0) } else { None };
    let series_out = if market == ShortMarket::Ca && trend { Some(series(&position.as_of)) } else { position.series };
    let mut rec = Shorts {
        symbol: sym.to_string(),
        exchange: exchange_out,
        market,
        name: issuer,
        as_of: position.as_of,
        shares,
        previous: position.previous,
        previous_of: position.previous_of,
        change: position.change,
        float: floated,
        of_float,
        average_volume: position.average_volume,
        days_to_cover: None,
        volume_of: volume.as_ref().map(|v| v.volume_of.clone()).unwrap_or_default(),
        volume_span: volume.as_ref().map(|v| v.volume_span),
        short_volume: volume.as_ref().and_then(|v| v.short_volume),
        total_volume: volume.as_ref().and_then(|v| v.total_volume),
        volume_pct: volume.as_ref().and_then(|v| v.volume_pct),
        series: series_out,
    };
    rec.average_volume = average_volume(conn, &rec);
    rec.days_to_cover = days_to_cover(conn, &rec);
    rec
}
