//! TMX Money: the quote for a Canadian listing, and which of TMX's forms of a
//! symbol actually answers.
//!
//! A ticker is written differently on each Canadian venue -- bare on the TSX
//! and the Venture, `:CNX` on the CSE, `:AQL` on Cboe Canada, `:US` for a US
//! listing -- and nothing in the book says which. The form that answers is
//! found once by asking, checked against the venue the quote itself names, and
//! remembered; a miss is remembered for a day so a delisted ticker is not
//! asked about on every refresh.

use serde_json::{json, Value};

use crate::http::{post_json, TMX_HEADERS};
use crate::parse::opt;
use bagholder_model::value::{field_s, get};

pub const TMX_URL: &str = "https://app-money.tmx.com/graphql";

pub const TMX_QUOTE_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }";

/// `market.TMX_FORMS`.
const FORMS_CAD: [&str; 3] = ["", ":CNX", ":AQL"];
const FORMS_USD: [&str; 1] = [":US"];

/// `market.TMX_VENUE_OF_FORM`: the venue a quote must name for that form to be
/// the right one.
const VENUE_OF_FORM: [(&str, &[&str]); 4] = [
    ("", &["TORONTO STOCK EXCHANGE", "TSX VENTURE"]),
    (":CNX", &["CANADIAN SECURITIES EXCHANGE"]),
    (":AQL", &["CBOE", "NEO"]),
    (":US", &["NYSE", "NASDAQ", "NEW YORK"]),
];

/// `market.TMX_RESOLVE_RETRY_DAYS`.
pub const RESOLVE_RETRY_DAYS: i64 = 1;

/// `market.TMX_EXCHANGE_NAMES`: the venue a TMX quote names, in the app's own
/// words.
const EXCHANGE_NAMES: [(&str, &str); 8] = [
    ("VENTURE", "TSX-V"),
    ("TORONTO", "TSX"),
    ("CANADIAN SECURITIES", "CSE"),
    ("CBOE", "Cboe Canada"),
    ("NEO", "Cboe Canada"),
    ("NASDAQ", "NASDAQ"),
    ("NYSE", "NYSE"),
    ("NEW YORK", "NYSE"),
];

/// `market.tmx_venue`.
pub fn tmx_venue(name: &str) -> String {
    let up = name.to_uppercase();
    for (mark, venue) in EXCHANGE_NAMES {
        if up.contains(mark) {
            return venue.to_string();
        }
    }
    String::new()
}

/// `market.tmx_bare`: the ticker without its venue suffix.
pub fn tmx_bare(key: &str) -> String {
    key.split(':').next().unwrap_or("").to_string()
}

/// `market.parse_tmx_quote`.
pub fn parse_tmx_quote(data: &Value) -> Option<Value> {
    let q = data.get("data")?.get("getQuoteBySymbol")?;
    let m = q.as_object()?;
    if m.is_empty() {
        return None;
    }
    let ex: String = field_s(q, "exDividendDate").chars().take(10).collect();
    Some(json!({
        "price": q.get("price").cloned().unwrap_or(Value::Null),
        "priceChange": q.get("priceChange").cloned().unwrap_or(Value::Null),
        "percentChange": q.get("percentChange").cloned().unwrap_or(Value::Null),
        "prevClose": q.get("prevClose").cloned().unwrap_or(Value::Null),
        "currency": field_s(q, "currency"),
        "dividendAmount": q.get("dividendAmount").cloned().unwrap_or(Value::Null),
        "dividendFrequency": field_s(q, "dividendFrequency"),
        "exDividendDate": ex,
        "name": field_s(q, "name"),
        "exchange": field_s(q, "exchangeName"),
    }))
}

fn ask(symbol: &str) -> Value {
    let payload = json!({
        "operationName": "getQuoteBySymbol",
        "variables": {"symbol": symbol, "locale": "en"},
        "query": TMX_QUOTE_QUERY,
    });
    post_json(TMX_URL, &payload, &TMX_HEADERS).unwrap_or(Value::Null)
}

/// `market.tmx_remembered`: the form TMX answered to for this symbol, when one
/// has been remembered.
pub fn tmx_remembered(conn: &rusqlite::Connection, key: &str) -> String {
    if key.is_empty() || key.starts_with('^') {
        return key.to_string();
    }
    let bare = tmx_bare(key);
    let v = bagholder_store::tables::get_meta(conn, &format!("tmx_form:{}", bare), "").unwrap_or_default();
    if let Some(rest) = v.strip_prefix('@') {
        return format!("{}{}", bare, rest);
    }
    key.to_string()
}

/// `market.tmx_resolve`: which of TMX's forms answers, checked by the venue
/// its quote names. Remembered for good; a miss remembered for a day.
pub fn tmx_resolve(conn: &rusqlite::Connection, key: &str, today: &str) -> String {
    if key.is_empty() || key.starts_with('^') {
        return key.to_string();
    }
    let bare = tmx_bare(key);
    let suffix = &key[bare.len()..];
    let base: Vec<&str> = if suffix == ":US" { FORMS_USD.to_vec() } else { FORMS_CAD.to_vec() };
    // the record's own form first, when TMX has one for it
    let forms: Vec<&str> = if base.contains(&suffix) {
        let mut f = vec![suffix];
        f.extend(base.iter().filter(|x| **x != suffix));
        f
    } else {
        base
    };

    let meta_key = format!("tmx_form:{}", bare);
    let v = bagholder_store::tables::get_meta(conn, &meta_key, "").unwrap_or_default();
    if let Some(rest) = v.strip_prefix('@') {
        return format!("{}{}", bare, rest);
    }
    if let Some(when) = v.strip_prefix("none@") {
        let cutoff = bagholder_model::dates::shift_date(today, -RESOLVE_RETRY_DAYS);
        if when > cutoff.as_str() {
            return String::new();
        }
    }

    for form in forms {
        let cand = format!("{}{}", bare, form);
        let q = ask(&cand);
        let venue = q
            .get("data")
            .and_then(|d| d.get("getQuoteBySymbol"))
            .map(|g| field_s(g, "exchangeName").to_uppercase())
            .unwrap_or_default();
        let wanted = VENUE_OF_FORM.iter().find(|(f, _)| *f == form).map(|(_, v)| *v).unwrap_or(&[]);
        if !venue.is_empty() && wanted.iter().any(|w| venue.contains(w)) {
            let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("@{}", form));
            return cand;
        }
    }
    let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("none@{}", today));
    String::new()
}

/// `market.tmx_lookup`: the remembered or given form first; when it answers
/// nothing, the form TMX resolves for the symbol instead.
pub fn tmx_lookup<F>(conn: &rusqlite::Connection, key: &str, today: &str, f: F) -> (Option<Value>, String)
where
    F: Fn(&str) -> Option<Value>,
{
    let first = tmx_remembered(conn, key);
    let r = f(&first);
    if r.is_some() || key.is_empty() || key.starts_with('^') {
        return (r, first);
    }
    let alt = tmx_resolve(conn, key, today);
    if !alt.is_empty() && alt != first {
        return (f(&alt), alt);
    }
    (r, first)
}

/// `market.tmx_lookup` for a lookup that can fail: a failure is raised past
/// the lookup in Python, so it stops there -- nothing is resolved and nothing
/// is remembered on the strength of a request that did not get an answer.
pub fn tmx_lookup_try<F, E>(conn: &rusqlite::Connection, key: &str, today: &str, f: F) -> Result<(Option<Value>, String), E>
where
    F: Fn(&str) -> Result<Option<Value>, E>,
{
    let first = tmx_remembered(conn, key);
    let r = f(&first)?;
    if r.is_some() || key.is_empty() || key.starts_with('^') {
        return Ok((r, first));
    }
    let alt = tmx_resolve(conn, key, today);
    if !alt.is_empty() && alt != first {
        return Ok((f(&alt)?, alt));
    }
    Ok((r, first))
}

/// `market.fetch_tmx_quote`.
pub fn fetch_tmx_quote(conn: &rusqlite::Connection, tmx_sym: &str, today: &str) -> Option<Value> {
    tmx_lookup(conn, tmx_sym, today, |k| parse_tmx_quote(&ask(k))).0
}

/// `market.tmx_listing`: the listing TMX knows a bare ticker as.
///
/// The public directories cover the TSX and Nasdaq registries alone, so this
/// is how a CSE or Cboe Canada listing is found by name.
pub fn tmx_listing(conn: &rusqlite::Connection, symbol: &str, today: &str) -> Option<Value> {
    let bare = tmx_bare(&bagholder_model::venues::tmx_symbol(symbol));
    if bare.is_empty() || bare.contains(' ') {
        return None;
    }
    let form = tmx_resolve(conn, &bare, today);
    if form.is_empty() {
        return None;
    }
    let q = parse_tmx_quote(&ask(&form))?;
    let venue = tmx_venue(&field_s(&q, "exchange"));
    if venue.is_empty() {
        return None;
    }
    let name = { let n = field_s(&q, "name"); if n.is_empty() { bare.clone() } else { n } };
    let currency = {
        let c = field_s(&q, "currency");
        if !c.is_empty() { c } else if venue == "NYSE" || venue == "NASDAQ" { "USD".into() } else { "CAD".into() }
    };
    Some(json!({"symbol": bare, "name": name, "exchange": venue, "currency": currency}))
}

/// `market.tmx_record_symbol`: the symbol a Canadian listing's declared
/// distribution record is filed under.
pub fn tmx_record_symbol(symbol: &str, exchange: &str) -> Option<String> {
    let s = bagholder_model::venues::tmx_symbol(symbol);
    if s.is_empty() {
        return None;
    }
    bagholder_model::venues::tmx_form(exchange, "CAD").map(|form| format!("{}{}", s, form))
}

/// `market.is_canadian_listing`.
pub fn is_canadian_listing(exchange: &str, currency: &str) -> bool {
    const CANADIAN: [&str; 7] = ["TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE"];
    let ex = exchange.trim().to_uppercase();
    if !ex.is_empty() {
        return CANADIAN.contains(&ex.as_str());
    }
    currency.trim().to_uppercase() == "CAD"
}

/// The dividend rows TMX files against a symbol.
pub fn parse_tmx_dividends(data: &Value) -> Vec<Value> {
    let block = match data.get("data").and_then(|d| d.get("dividends")) { Some(b) => b, None => return vec![] };
    let rows = match block.get("dividends").and_then(|v| v.as_array()) { Some(r) => r, None => return vec![] };
    let mut out = Vec::new();
    for r in rows {
        if !r.is_object() {
            continue;
        }
        let ex: String = field_s(r, "exDate").chars().take(10).collect();
        let amount = opt(get(r, "amount"));
        match amount {
            Some(a) if ex.len() == 10 && a > 0.0 => out.push(json!({
                "exDate": ex,
                "payDate": field_s(r, "payableDate").chars().take(10).collect::<String>(),
                "amount": a,
                "currency": field_s(r, "currency"),
            })),
            _ => continue,
        }
    }
    out
}

pub const TMX_DIVIDENDS_QUERY: &str = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate payableDate amount currency } } }";

/// `market.TMX_BATCH`.
pub const TMX_BATCH: i64 = 24;

/// `market.fetch_tmx`: the quote and the declared distribution history for one
/// Canadian listing. The exchange picks the form of the symbol.
pub fn fetch_tmx(conn: &rusqlite::Connection, symbol: &str, exchange: &str, today: &str) -> (Option<Value>, Vec<Value>) {
    let sym = match tmx_record_symbol(symbol, exchange) { Some(s) => s, None => return (None, vec![]) };
    let (quote, form) = tmx_lookup(conn, &sym, today, |k| parse_tmx_quote(&ask(k)));
    let payload = json!({
        "operationName": "getDividendsForSymbol",
        "variables": {"symbol": form, "page": 1, "batch": TMX_BATCH},
        "query": TMX_DIVIDENDS_QUERY,
    });
    let divs = match post_json(TMX_URL, &payload, &TMX_HEADERS) {
        Ok(d) => parse_tmx_dividends(&d),
        Err(_) => vec![],
    };
    (quote, divs)
}
