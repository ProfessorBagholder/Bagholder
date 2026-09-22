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
use crate::quotes::SourceQuote;
use bagholder_model::lenient;
use bagholder_model::value::field_s;
use bagholder_store::market::{DistributionRecord, QuoteRecord};
use serde::Deserialize;

pub const TMX_URL: &str = "https://app-money.tmx.com/graphql";

pub const TMX_QUOTE_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }";

const FORMS_CAD: [&str; 3] = ["", ":CNX", ":AQL"];
const FORMS_USD: [&str; 1] = [":US"];

/// The venue a quote must name for that form to be
/// the right one.
const VENUE_OF_FORM: [(&str, &[&str]); 4] = [
    ("", &["TORONTO STOCK EXCHANGE", "TSX VENTURE"]),
    (":CNX", &["CANADIAN SECURITIES EXCHANGE"]),
    (":AQL", &["CBOE", "NEO"]),
    (":US", &["NYSE", "NASDAQ", "NEW YORK"]),
];

pub const RESOLVE_RETRY_DAYS: i64 = 1;

/// The venue a TMX quote names, in the app's own
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

pub fn tmx_venue(name: &str) -> String {
    let up = name.to_uppercase();
    for (mark, venue) in EXCHANGE_NAMES {
        if up.contains(mark) {
            return venue.to_string();
        }
    }
    String::new()
}

/// The ticker without its venue suffix.
pub fn tmx_bare(key: &str) -> String {
    key.split(':').next().unwrap_or("").to_string()
}

/// `getQuoteBySymbol`'s answer, as far as a quote reads it.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TmxQuote {
    #[serde(deserialize_with = "lenient::maybe_number")]
    price: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    price_change: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    percent_change: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    prev_close: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    currency: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    dividend_amount: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    dividend_frequency: String,
    #[serde(deserialize_with = "lenient::text")]
    ex_dividend_date: String,
    #[serde(deserialize_with = "lenient::text")]
    name: String,
    #[serde(deserialize_with = "lenient::text")]
    exchange_name: String,
}

/// TMX's quote. An empty answer is no answer; a halted listing answers with no price.
pub fn parse_tmx_quote(data: &Value) -> Option<SourceQuote> {
    let q = data.get("data")?.get("getQuoteBySymbol")?;
    if q.as_object()?.is_empty() {
        return None;
    }
    let q = TmxQuote::deserialize(q).ok()?;
    Some(SourceQuote {
        quote: QuoteRecord {
            price: q.price,
            price_change: q.price_change,
            percent_change: q.percent_change,
            prev_close: q.prev_close,
            dividend_amount: q.dividend_amount,
            dividend_frequency: q.dividend_frequency,
            ex_dividend_date: q.ex_dividend_date.chars().take(10).collect(),
        },
        currency: q.currency,
        name: q.name,
        exchange: q.exchange_name,
    })
}

fn ask(symbol: &str) -> Value {
    let payload = json!({
        "operationName": "getQuoteBySymbol",
        "variables": {"symbol": symbol, "locale": "en"},
        "query": TMX_QUOTE_QUERY,
    });
    post_json(TMX_URL, &payload, &TMX_HEADERS).unwrap_or(Value::Null)
}

/// The form TMX answered to for this symbol, when one
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

/// Which of TMX's forms answers, checked by the venue
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

/// The remembered or given form first; when it answers
/// nothing, the form TMX resolves for the symbol instead.
pub fn tmx_lookup<T, F>(conn: &rusqlite::Connection, key: &str, today: &str, f: F) -> (Option<T>, String)
where
    F: Fn(&str) -> Option<T>,
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

/// The TMX lookup for a lookup that can fail: a failure is passed straight
/// back, so it stops there -- nothing is resolved and nothing
/// is remembered on the strength of a request that did not get an answer.
pub fn tmx_lookup_try<T, F, E>(conn: &rusqlite::Connection, key: &str, today: &str, f: F) -> Result<(Option<T>, String), E>
where
    F: Fn(&str) -> Result<Option<T>, E>,
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

pub fn fetch_tmx_quote(conn: &rusqlite::Connection, tmx_sym: &str, today: &str) -> Option<SourceQuote> {
    tmx_lookup(conn, tmx_sym, today, |k| parse_tmx_quote(&ask(k))).0
}

/// The listing TMX knows a bare ticker as.
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
    let venue = tmx_venue(&q.exchange);
    if venue.is_empty() {
        return None;
    }
    let name = if q.name.is_empty() { bare.clone() } else { q.name };
    let currency = if !q.currency.is_empty() {
        q.currency
    } else if venue == "NYSE" || venue == "NASDAQ" {
        "USD".into()
    } else {
        "CAD".into()
    };
    Some(json!({"symbol": bare, "name": name, "exchange": venue, "currency": currency}))
}

/// The symbol a Canadian listing's declared
/// distribution record is filed under.
pub fn tmx_record_symbol(symbol: &str, exchange: &str) -> Option<String> {
    let s = bagholder_model::venues::tmx_symbol(symbol);
    if s.is_empty() {
        return None;
    }
    bagholder_model::venues::tmx_form(exchange, "CAD").map(|form| format!("{}{}", s, form))
}

pub fn is_canadian_listing(exchange: &str, currency: &str) -> bool {
    const CANADIAN: [&str; 7] = ["TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE"];
    let ex = exchange.trim().to_uppercase();
    if !ex.is_empty() {
        return CANADIAN.contains(&ex.as_str());
    }
    currency.trim().to_uppercase() == "CAD"
}

/// One row of `getDividendsForSymbol`.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TmxDividend {
    #[serde(deserialize_with = "lenient::text")]
    ex_date: String,
    #[serde(deserialize_with = "lenient::text")]
    payable_date: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    amount: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    currency: String,
}

/// The dividend rows TMX files against a symbol: a row with no ex-date or no
/// positive amount is not a distribution.
pub fn parse_tmx_dividends(data: &Value) -> Vec<DistributionRecord> {
    let rows = data.get("data").and_then(|d| d.get("dividends")).and_then(|b| b.get("dividends")).unwrap_or(&Value::Null);
    lenient::rows::<TmxDividend>(rows)
        .into_iter()
        .filter_map(|r| {
            let ex: String = r.ex_date.chars().take(10).collect();
            let a = r.amount.filter(|a| ex.len() == 10 && *a > 0.0)?;
            Some(DistributionRecord { ex_date: ex, pay_date: r.payable_date.chars().take(10).collect(), amount: Some(a), currency: r.currency })
        })
        .collect()
}

pub const TMX_DIVIDENDS_QUERY: &str = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate payableDate amount currency } } }";

pub const TMX_BATCH: i64 = 24;

/// The quote and the declared distribution history for one
/// Canadian listing. The exchange picks the form of the symbol.
pub fn fetch_tmx(conn: &rusqlite::Connection, symbol: &str, exchange: &str, today: &str) -> (Option<SourceQuote>, Vec<DistributionRecord>) {
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
