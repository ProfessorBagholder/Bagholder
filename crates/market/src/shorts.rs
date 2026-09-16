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

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::http::{get_text, note_source, post_json, FetchError, TIMEOUT_SEC};
use bagholder_model::dates;
use bagholder_model::pytext::{csv_records, py_float, splitlines};
use bagholder_model::value::s as vs;

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

// --- Python's numbers -----------------------------------------------------------

/// `shorts._num`: `float(_s(v).replace(",", "").strip())`, None where it is
/// not a number.
pub fn num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Bool(_)) => None,
        Some(Value::String(t)) => py_float(&t.replace(',', "")),
        Some(other) => py_float(&vs(Some(other)).replace(',', "")),
    }
}

fn truthy(v: Option<f64>) -> bool {
    matches!(v, Some(x) if x != 0.0)
}

fn opt(v: Option<f64>) -> Value {
    match v {
        Some(x) if x.is_finite() => json!(x),
        // json.dumps writes NaN and Infinity; serde_json cannot, and a figure
        // that is not one is no figure
        _ => Value::Null,
    }
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

// --- which regulator, if any, publishes for a listing -------------------------

/// `shorts.market_of`: "us", "ca", or "" for an instrument no one reports short
/// selling on -- a coin, an index, a futures or currency contract, an option.
pub fn market_of(symbol: &str, exchange: &str, currency: &str) -> &'static str {
    let sym = symbol.trim().to_uppercase();
    let ex = exchange.trim().to_uppercase();
    if sym.is_empty()
        || bagholder_model::symbols::is_option_symbol(&sym)
        || ex == "CRYPTO"
        || bagholder_model::instruments::find(&sym, &ex).is_some()
    {
        return "";
    }
    match bagholder_model::venues::tmx_form(&ex, currency) {
        None => "",
        Some(":US") => "us",
        Some(_) => "ca",
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

/// `shorts.position_dates`: the reporting dates of the twice-monthly position
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

/// `shorts.volume_periods`: the half-month periods the Canadian volume report
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

/// `shorts.trading_days`: the days the US volume file could be for, newest
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

#[derive(Clone)]
pub struct Held {
    pub key: String,
    pub rows: Map<String, Value>,
    at: Instant,
}

fn files() -> &'static Mutex<HashMap<String, Held>> {
    static F: OnceLock<Mutex<HashMap<String, Held>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `shorts._table`: a file every listing is looked up in, fetched at most once
/// every FILE_HOURS. A fetch that fails keeps what was already read rather
/// than emptying it.
fn table<F>(name: &str, build: F) -> Held
where
    F: FnOnce() -> Option<(String, Map<String, Value>)>,
{
    if let Some(held) = files().lock().unwrap().get(name) {
        if held.at.elapsed() < Duration::from_secs(FILE_HOURS * 3600) {
            return held.clone();
        }
    }
    let built = build();
    let mut f = files().lock().unwrap();
    let held = match built {
        None => {
            let mut h = f.get(name).cloned().unwrap_or(Held { key: String::new(), rows: Map::new(), at: Instant::now() });
            h.at = Instant::now();
            h
        }
        Some((key, rows)) => Held { key, rows, at: Instant::now() },
    };
    f.insert(name.to_string(), held.clone());
    held
}

/// `shorts.parse_us_volume`: FINRA's daily file, one pipe-separated line per
/// symbol, a trailer at the end.
pub fn parse_us_volume(text: &str) -> Map<String, Value> {
    let mut rows = Map::new();
    for line in splitlines(text).into_iter().skip(1) {
        let parts: Vec<&str> = line.trim().split('|').collect();
        if parts.len() < 5 {
            continue;
        }
        let sym = parts[1].trim().to_uppercase();
        let short = py_float(&parts[2].replace(',', ""));
        let total = py_float(&parts[4].replace(',', ""));
        if !sym.is_empty() && short.is_some() && truthy(total) {
            rows.insert(sym, json!({"shortVolume": opt(short), "totalVolume": opt(total)}));
        }
    }
    rows
}

fn us_volume_file(today: &str) -> Option<(String, Map<String, Value>)> {
    for d in trading_days(today, TRIES) {
        let url = US_VOLUME_URL.replace("{}", &compact(&d));
        let rows = match get_text(&url, &headers()) {
            Ok(t) => parse_us_volume(&t),
            Err(_) => continue,
        };
        if !rows.is_empty() {
            return Some((d, rows));
        }
    }
    None
}

/// `shorts.parse_ca_positions`: CIRO's position report -- issue name, symbol,
/// venue, shares short, net change.
pub fn parse_ca_positions(grid: &[Vec<Value>]) -> Map<String, Value> {
    let mut rows = Map::new();
    for r in grid {
        if r.len() < 5 {
            continue;
        }
        let sym = vs(Some(&r[1])).trim().to_uppercase();
        let shares = num(Some(&r[3]));
        if sym.is_empty() || shares.is_none() {
            continue;
        }
        rows.insert(sym, json!({
            "venue": vs(Some(&r[2])).trim().to_uppercase(),
            "shares": opt(shares),
            "change": opt(num(Some(&r[4]))),
            "name": vs(Some(&r[0])).trim(),
        }));
    }
    rows
}

fn get_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    crate::client::request("GET", url, &headers(), None, Duration::from_secs(TIMEOUT_SEC)).map(|r| r.body)
}

fn ca_position_file(today: &str) -> Option<(String, Map<String, Value>)> {
    for d in position_dates(today, TRIES) {
        let url = CA_POSITION_URL.replace("{}", &compact(&d));
        let rows = get_bytes(&url)
            .map_err(|e| e.to_string())
            .and_then(|raw| crate::xls::table(&raw))
            .map(|grid| parse_ca_positions(&grid));
        match rows {
            Err(_) => {
                note_source("ciro", false, Some(&FetchError::Transport(format!("no report for {}", d))));
                continue;
            }
            Ok(rows) if !rows.is_empty() => {
                note_source("ciro", true, None);
                return Some((d, rows));
            }
            Ok(_) => {}
        }
    }
    None
}

/// `shorts.ca_traded`: a Canadian listing's own volume over the report's
/// period, from TMX's daily series under the venue's own form.
pub fn ca_traded(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, span: &str, today: &str) -> Option<f64> {
    let (start, end) = match span.split_once('/') { Some((a, b)) => (a.to_string(), b.to_string()), None => (span.to_string(), String::new()) };
    if end.is_empty() {
        return None;
    }
    let code = crate::quotes::tmx_quote_symbol(symbol, exchange, currency)?;
    let ask = |form: &str| -> Result<Option<Value>, FetchError> {
        let data = post_json(crate::tmx::TMX_URL, &json!({
            "operationName": "getTimeSeriesData",
            "variables": {"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end},
            "query": TMX_HISTORY_QUERY,
        }), &crate::http::TMX_HEADERS)?;
        let bars = crate::parse::parse_tmx_history(&data);
        Ok(if bars.is_empty() { None } else { Some(Value::Array(bars)) })
    };
    let bars = match crate::tmx::tmx_lookup_try(conn, &code, today, ask) {
        Ok((b, _)) => b.and_then(|v| v.as_array().cloned()).unwrap_or_default(),
        Err(e) => {
            eprintln!("bagholder shorts: {} traded volume failed: {}", symbol, e);
            return None;
        }
    };
    let traded: Vec<f64> = bars.iter().filter_map(|b| num(b.get("volume")).filter(|v| *v != 0.0)).collect();
    if traded.is_empty() { None } else { Some(traded.iter().sum()) }
}

/// `shorts.parse_ca_volume`: CIRO's short sale summary, the short part of a
/// period's trading, per listing.
pub fn parse_ca_volume(text: &str) -> Result<Map<String, Value>, String> {
    let mut rows = Map::new();
    for r in csv_records(text.trim_start_matches('\u{feff}'))? {
        let r = Value::Object(r);
        let sym = field(&r, "Security").trim().to_uppercase();
        let short = num(get(&r, "Short Traded Volume"));
        let pct = num(get(&r, "% Total Traded Volume"));
        if sym.is_empty() || short.is_none() {
            continue;
        }
        let total = if truthy(pct) { Some(short.unwrap() / pct.unwrap() * 100.0) } else { None };
        rows.insert(sym, json!({
            "venue": field(&r, "Listing Market").trim().to_uppercase(),
            "shortVolume": opt(short),
            "volumePct": opt(pct),
            "totalVolume": opt(total),
        }));
    }
    Ok(rows)
}

fn ca_volume_file(today: &str) -> Option<(String, Map<String, Value>)> {
    for (start, end) in volume_periods(today, TRIES) {
        let url = CA_VOLUME_URL.replacen("{}", &compact(&start), 1).replacen("{}", &compact(&end), 1);
        let rows = match get_text(&url, &headers()) {
            Ok(t) => match parse_ca_volume(&t) { Ok(r) => r, Err(_) => continue },
            Err(_) => continue,
        };
        if !rows.is_empty() {
            return Some((format!("{}/{}", start, end), rows));
        }
    }
    None
}

/// `shorts._ca_positions_on`: one dated Canadian report, kept for the session
/// so a run of them is read once.
fn ca_positions_on(day: &str) -> Map<String, Value> {
    let name = format!("ca_position:{}", day);
    if let Some(h) = files().lock().unwrap().get(&name) {
        return h.rows.clone();
    }
    let rows = get_bytes(&CA_POSITION_URL.replace("{}", &compact(day)))
        .map_err(|e| e.to_string())
        .and_then(|raw| crate::xls::table(&raw))
        .map(|g| parse_ca_positions(&g))
        .unwrap_or_default();
    files().lock().unwrap().insert(name, Held { key: day.to_string(), rows: rows.clone(), at: Instant::now() });
    rows
}

/// `shorts.ca_series`: the listing's position across the last reports, oldest
/// first. Canada publishes one file per reporting date, so each is read on its
/// own and kept.
pub fn ca_series(symbol: &str, exchange: &str, asof: &str, today: &str, back: usize) -> Vec<Value> {
    let sym = symbol.trim().to_uppercase();
    let mut out: Vec<Value> = Vec::new();
    for day in position_dates(today, back) {
        if !asof.is_empty() && day.as_str() > asof {
            continue;
        }
        let rows = ca_positions_on(&day);
        if let Some(row) = rows.get(&sym) {
            if venue_fits(&field(row, "venue"), exchange) {
                out.push(json!({"date": day, "shares": row.get("shares").cloned().unwrap_or(Value::Null)}));
            }
        }
    }
    out.sort_by_key(|x| field(x, "date"));
    out
}

// --- the float -----------------------------------------------------------------

struct Yahoo {
    session: crate::browser::Session,
    crumb: String,
}

fn yahoo() -> &'static Mutex<Option<Yahoo>> {
    static Y: OnceLock<Mutex<Option<Yahoo>>> = OnceLock::new();
    Y.get_or_init(|| Mutex::new(None))
}

/// `shorts._yahoo_session`: a session that can read Yahoo's statistics. Its
/// own TLS handshake is the gate, so it goes through the browser helper;
/// without it the float is simply unknown, as it is for a listing Yahoo does
/// not carry.
fn yahoo_open(slot: &mut Option<Yahoo>) -> bool {
    if slot.is_some() {
        return true;
    }
    let mut session = match crate::browser::Session::new() { Some(s) => s, None => return false };
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

/// `shorts._cboe_units`: the units a fund listed on Cboe Canada has in issue,
/// from that venue's own directory -- its market capitalisation divided by its
/// last price, which gives back the count the exchange put in, whole for every
/// listing it carries.
fn cboe_units(symbol: &str) -> Option<f64> {
    let held = table("cboe_listings", || {
        let text = get_text(CA_CBOE_URL, &headers()).ok()?;
        let doc: Value = serde_json::from_str(&text).ok()?;
        let mut rows = Map::new();
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
                rows.insert(field(&r, "symbol").trim().to_uppercase(), json!(count.round_ties_even()));
            }
        }
        Some(("cboe".to_string(), rows))
    });
    held.rows.get(&symbol.trim().to_uppercase()).and_then(|v| v.as_f64())
}

/// `shorts._fund_units`: the units an exchange-traded fund has in issue, from
/// the market's own source. For a fund this is the float, not a stand-in.
fn fund_units(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, today: &str) -> Option<f64> {
    let sym = symbol.trim().to_uppercase();
    if market_of(&sym, exchange, currency) == "us" {
        // the US count comes from Yahoo with the float
        return None;
    }
    let mut count: Option<f64> = None;
    if let Some(code) = crate::quotes::tmx_quote_symbol(&sym, exchange, currency) {
        let ask = |form: &str| -> Result<Option<Value>, FetchError> {
            let answered = post_json(crate::tmx::TMX_URL, &json!({
                "operationName": "getQuoteBySymbol",
                "variables": {"symbol": form, "locale": "en"},
                "query": TMX_UNITS_QUERY,
            }), &crate::http::TMX_HEADERS)?;
            let q = answered.get("data").and_then(|d| d.get("getQuoteBySymbol")).cloned().unwrap_or(Value::Null);
            Ok(match &q { Value::Object(m) if !m.is_empty() => Some(q), _ => None })
        };
        match crate::tmx::tmx_lookup_try(conn, &code, today, ask) {
            Ok((q, _)) => count = q.and_then(|q| num(q.get("shareOutStanding"))).filter(|c| *c != 0.0),
            Err(e) => eprintln!("bagholder shorts: {} units failed: {}", sym, e),
        }
    }
    if truthy(count) {
        return count;
    }
    // the venue is asked for its own listings only: a count taken from the
    // wrong venue would be the wrong fund's
    let ex = exchange.trim().to_uppercase();
    if !CA_VENUES[3].1.contains(&ex.as_str()) {
        return None;
    }
    cboe_units(&sym)
}

struct Float {
    value: Option<f64>,
    at: Instant,
}

fn floats() -> &'static Mutex<HashMap<String, Float>> {
    static F: OnceLock<Mutex<HashMap<String, Float>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `shorts.float_shares`: what a short position is measured against, the
/// shares actually available to trade. For a company that is the free float
/// Yahoo publishes, never swapped for the shares in issue; a fund's units in
/// issue are its float, and stand in where no float is published for one.
pub fn float_shares(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, name: &str, today: &str) -> Option<f64> {
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
    let ccy = if currency.trim().is_empty() { if where_ == "us" { "USD".to_string() } else { "CAD".to_string() } } else { currency.trim().to_string() };
    let mut count: Option<f64> = None;
    {
        let mut slot = yahoo().lock().unwrap();
        if yahoo_open(&mut slot) {
            let y = slot.as_mut().unwrap();
            let mut forms = crate::quotes::yahoo_forms(&json!({"symbol": sym, "exchange": exchange, "currency": ccy}));
            if forms.is_empty() {
                forms = vec![bagholder_model::venues::tmx_symbol(&sym)];
            }
            for form in forms {
                if !crate::quotes::yahoo_turn() {
                    continue;
                }
                let url = YAHOO_STATS_URL.replacen("{}", &form, 1).replacen("{}", &y.crumb, 1);
                let answered = match y.session.get(&url, Duration::from_secs(TIMEOUT_SEC)) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("bagholder shorts: {} float from yahoo failed: {}", form, e);
                        continue;
                    }
                };
                if answered.status == 429 {
                    crate::quotes::yahoo_back_off();
                    continue;
                }
                if answered.status != 200 {
                    continue;
                }
                let doc: Value = match serde_json::from_str(&answered.text()) {
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
    }
    if !truthy(count) && fund {
        count = fund_units(conn, &sym, exchange, &ccy, today);
    }
    let count = count.filter(|c| *c != 0.0);
    floats().lock().unwrap().insert(key, Float { value: count, at: Instant::now() });
    count
}

/// Python's truthiness for what a JSON answer carries.
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

/// `shorts._venue_fits`: whether a row's venue is the listing's.
pub fn venue_fits(code: &str, exchange: &str) -> bool {
    let ex = exchange.trim().to_uppercase();
    if ex.is_empty() {
        return true;
    }
    let code = code.trim().to_uppercase();
    CA_VENUES.iter().find(|(k, _)| *k == code).map(|(_, v)| v.contains(&ex.as_str())).unwrap_or(false)
}

// --- one listing ---------------------------------------------------------------

/// `shorts.parse_us_position`: FINRA's answer into the newest settlement and
/// the run of them. Split from the request so the two can be compared on the
/// same answer.
pub fn parse_us_position(answered: &Value) -> Value {
    let mut rows: Vec<Value> = answered
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.is_object() && truthy_json(r.get("settlementDate").unwrap_or(&Value::Null)))
        .collect();
    if rows.is_empty() {
        return json!({});
    }
    rows.sort_by(|a, b| field(b, "settlementDate").cmp(&field(a, "settlementDate")));
    let r = &rows[0];
    let date10 = |x: &Value| field(x, "settlementDate").chars().take(10).collect::<String>();
    let mut ascending = rows.clone();
    ascending.sort_by_key(|x| field(x, "settlementDate"));
    let series: Vec<Value> = ascending
        .iter()
        .filter(|x| num(x.get("currentShortPositionQuantity")).is_some())
        .map(|x| json!({"date": date10(x), "shares": opt(num(x.get("currentShortPositionQuantity")))}))
        .collect();
    json!({
        "asOf": date10(r),
        "shares": opt(num(r.get("currentShortPositionQuantity"))),
        "previous": opt(num(r.get("previousShortPositionQuantity"))),
        "change": opt(num(r.get("changePreviousNumber"))),
        "previousOf": if rows.len() > 1 { date10(&rows[1]) } else { String::new() },
        "averageVolume": opt(num(r.get("averageDailyVolumeQuantity"))),
        // every settlement FINRA answered with, oldest first
        "series": series,
    })
}

/// `shorts.us_position`: the newest settlement FINRA has for a US listing.
pub fn us_position(symbol: &str, today: &str) -> Value {
    let body = json!({
        "limit": 20,
        "compareFilters": [{"fieldName": "symbolCode", "fieldValue": symbol.trim().to_uppercase(), "compareType": "EQUAL"}],
        "dateRangeFilters": [{"fieldName": "settlementDate", "startDate": dates::shift_date(today, -150), "endDate": today}],
    });
    match post_json(US_POSITION_URL, &body, &[("Accept", "application/json")]) {
        Ok(answered) => parse_us_position(&answered),
        Err(e) => {
            eprintln!("bagholder shorts: {} position from finra failed: {}", symbol, e);
            json!({})
        }
    }
}

/// `shorts.us_volume`: the last trading day's short volume for a US listing.
pub fn us_volume(symbol: &str, today: &str) -> Value {
    let held = table("us_volume", || us_volume_file(today));
    us_volume_from(&held.key, &held.rows, symbol)
}

pub fn us_volume_from(key: &str, rows: &Map<String, Value>, symbol: &str) -> Value {
    let row = match rows.get(&symbol.trim().to_uppercase()) { Some(r) if truthy_json(r) => r, _ => return json!({}) };
    let short = num(row.get("shortVolume"));
    let total = num(row.get("totalVolume"));
    let pct = if truthy(total) { Some(short.unwrap_or(0.0) / total.unwrap() * 100.0) } else { None };
    json!({
        "volumeOf": key, "volumeSpan": "day",
        "shortVolume": opt(short), "totalVolume": opt(total), "volumePct": opt(pct),
    })
}

/// `shorts.ca_position`.
pub fn ca_position(symbol: &str, exchange: &str, today: &str) -> Value {
    let held = table("ca_position", || ca_position_file(today));
    ca_position_from(&held.key, &held.rows, symbol, exchange, today)
}

pub fn ca_position_from(key: &str, rows: &Map<String, Value>, symbol: &str, exchange: &str, today: &str) -> Value {
    let row = match rows.get(&symbol.trim().to_uppercase()) { Some(r) if truthy_json(r) => r, _ => return json!({}) };
    if !venue_fits(&field(row, "venue"), exchange) {
        return json!({});
    }
    let shares = num(row.get("shares"));
    let change = num(row.get("change"));
    // the report before this one is the previous reporting date, which is what
    // the change is against
    let earlier: Vec<String> = position_dates(today, TRIES).into_iter().filter(|d| d.as_str() < key).collect();
    json!({
        "asOf": key,
        "shares": opt(shares),
        "change": opt(change),
        "previous": opt(change.map(|c| shares.unwrap_or(0.0) - c)),
        "previousOf": earlier.first().cloned().unwrap_or_default(),
        // the report names the venue and the issuer
        "venue": field(row, "venue").trim().to_uppercase(),
        "issuer": field(row, "name").trim(),
    })
}

/// `shorts.ca_volume`: the short part of a Canadian listing's trading over the
/// report's period. A listing the report does not carry was not sold short in
/// it, so its short volume is none of its trading rather than unknown, and
/// what it did trade comes from the exchange.
pub fn ca_volume(conn: &rusqlite::Connection, symbol: &str, exchange: &str, currency: &str, today: &str) -> Value {
    let held = table("ca_volume", || ca_volume_file(today));
    if held.key.is_empty() {
        return json!({});
    }
    let row = held.rows.get(&symbol.trim().to_uppercase()).filter(|r| truthy_json(r));
    ca_volume_from(&held.key, row, exchange, || ca_traded(conn, symbol, exchange, currency, &held.key, today))
}

pub fn ca_volume_from<F: FnOnce() -> Option<f64>>(key: &str, row: Option<&Value>, exchange: &str, traded: F) -> Value {
    match row {
        Some(r) if !venue_fits(&field(r, "venue"), exchange) => json!({}),
        None => match traded() {
            Some(t) if t != 0.0 => json!({
                "volumeOf": key, "volumeSpan": "period", "shortVolume": 0.0, "totalVolume": opt(Some(t)), "volumePct": 0.0,
            }),
            _ => json!({}),
        },
        Some(r) => json!({
            "volumeOf": key, "volumeSpan": "period",
            "shortVolume": r.get("shortVolume").cloned().unwrap_or(Value::Null),
            "totalVolume": r.get("totalVolume").cloned().unwrap_or(Value::Null),
            "volumePct": r.get("volumePct").cloned().unwrap_or(Value::Null),
        }),
    }
}

/// `shorts.average_volume`: the average daily volume in the listing's own
/// market over the period its short report covers. FINRA publishes the
/// average itself; Canada's total is divided by the days the Canadian market
/// actually traded, counted from the index series the app keeps.
pub fn average_volume(conn: &rusqlite::Connection, rec: &Value) -> Option<f64> {
    if field(rec, "market") == "us" {
        return num(rec.get("averageVolume"));
    }
    let total = num(rec.get("totalVolume"));
    let span = field(rec, "volumeOf");
    if !truthy(total) || !span.contains('/') {
        return None;
    }
    let (start, end) = span.split_once('/').unwrap();
    let days = bagholder_store::tables::benchmark_days(conn, "TSX", start, end).unwrap_or(0);
    if days != 0 { Some(total.unwrap() / days as f64) } else { None }
}

/// `shorts.days_to_cover`: the position against that average daily volume.
pub fn days_to_cover(conn: &rusqlite::Connection, rec: &Value) -> Option<f64> {
    let shares = num(rec.get("shares"));
    let average = average_volume(conn, rec);
    if truthy(shares) && truthy(average) { Some(round1(shares.unwrap() / average.unwrap())) } else { None }
}

/// `shorts.for_listing`: everything published about one listing's short
/// selling, {} where nothing is.
pub fn for_listing(
    conn: &rusqlite::Connection,
    symbol: &str,
    exchange: &str,
    currency: &str,
    today: &str,
    trend: bool,
    name: &str,
) -> Value {
    let sym = symbol.trim().to_uppercase();
    let where_ = market_of(&sym, exchange, currency);
    if where_.is_empty() {
        return json!({});
    }
    let mut rec: Map<String, Value> = Map::new();
    let (a, b) = if where_ == "us" {
        (us_position(&sym, today), us_volume(&sym, today))
    } else {
        (ca_position(&sym, exchange, today), ca_volume(conn, &sym, exchange, currency, today))
    };
    for part in [a, b] {
        if let Value::Object(m) = part {
            for (k, v) in m {
                rec.insert(k, v);
            }
        }
    }
    finish(conn, rec, &sym, exchange, currency, today, trend, name, where_, |asof| ca_series(&sym, exchange, asof, today, SERIES), |issuer| {
        float_shares(conn, &sym, exchange, currency, if name.trim().is_empty() { issuer } else { name.trim() }, today)
    })
}

/// The part of `for_listing` after the regulators have answered, split out so
/// it can be run on fixed answers.
#[allow(clippy::too_many_arguments)]
pub fn finish<S, F>(
    conn: &rusqlite::Connection,
    mut rec: Map<String, Value>,
    sym: &str,
    exchange: &str,
    _currency: &str,
    _today: &str,
    trend: bool,
    _name: &str,
    where_: &str,
    series: S,
    float_of: F,
) -> Value
where
    S: FnOnce(&str) -> Vec<Value>,
    F: FnOnce(&str) -> Option<f64>,
{
    if where_ == "ca" && trend {
        let asof = vs(rec.get("asOf").filter(|v| !v.is_null()));
        rec.insert("series".into(), Value::Array(series(&asof)));
    }
    // the record names its own listing, as a stored one does; a listing the
    // app knew no venue for takes the one the regulator's own report gives it,
    // and the issuer's name with it
    let venue = vs(rec.shift_remove("venue").as_ref().filter(|v| !v.is_null())).trim().to_uppercase();
    let issuer = vs(rec.shift_remove("issuer").as_ref().filter(|v| !v.is_null())).trim().to_string();
    let ex = exchange.trim().to_uppercase();
    let exchange_out = if !ex.is_empty() {
        ex
    } else {
        CA_VENUE_NAMES.iter().find(|(k, _)| *k == venue).map(|(_, v)| v.to_string()).unwrap_or_default()
    };
    rec.insert("symbol".into(), json!(sym));
    rec.insert("exchange".into(), json!(exchange_out));
    rec.insert("market".into(), json!(where_));
    rec.insert("source".into(), json!(if where_ == "us" { "FINRA" } else { "CIRO" }));
    if !issuer.is_empty() {
        rec.insert("name".into(), json!(issuer));
    }
    // a ticker is not a name: the issuer the report names is what tells a fund
    // from a company, and so what its short position is measured against
    let floated = float_of(&issuer);
    rec.insert("float".into(), opt(floated));
    let shares = num(rec.get("shares"));
    let of_float = if truthy(floated) && truthy(shares) { Some(shares.unwrap() / floated.unwrap() * 100.0) } else { None };
    rec.insert("ofFloat".into(), opt(of_float));
    let as_value = Value::Object(rec.clone());
    rec.insert("averageVolume".into(), opt(average_volume(conn, &as_value)));
    let as_value = Value::Object(rec.clone());
    rec.insert("daysToCover".into(), opt(days_to_cover(conn, &as_value)));
    Value::Object(rec)
}
