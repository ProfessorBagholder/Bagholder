//! TMX Money: the quote for a Canadian listing, asked under the form of its own
//! venue.
//!
//! A ticker is written differently on each venue -- bare on the TSX and the
//! Venture, `:CNX` on the CSE, `:AQL` on Cboe Canada, `:US` for a US listing --
//! and the listing's venue picks the form. A form that answers nothing is that
//! listing's miss: another venue's answer would be another listing (plan
//! `market-data-keyed-by-listing`, Approach, 5). A symbol given with no venue is
//! asked under every Canadian form, and every listing that answers is returned
//! with its venue, never one picked (`tmx_listings`).

use serde_json::{json, Value};

use crate::http::{post_json, FetchError, TMX_HEADERS};
use crate::quotes::SourceQuote;
use bagholder_model::lenient;
use bagholder_store::market::{DistributionRecord, QuoteRecord};
use serde::Deserialize;

pub const TMX_URL: &str = "https://app-money.tmx.com/graphql";

pub const TMX_QUOTE_QUERY: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }";

/// The venue a TMX quote names, in the app's own
/// words.
/// A mark in TMX's name for an exchange, the venue's words, and the market identifier
/// code those words are read as: first match wins, so a venue whose name holds
/// another's (NYSE American, NYSE Arca) comes before it.
pub const EXCHANGE_NAMES: [(&str, &str, &str); 11] = [
    ("VENTURE", "TSX-V", "XTSX"),
    ("TORONTO", "TSX", "XTSE"),
    ("CANADIAN SECURITIES", "CSE", "XCNQ"),
    ("CBOE", "Cboe Canada", "NEOE"),
    ("NEO", "Cboe Canada", "NEOE"),
    ("NASDAQ", "NASDAQ", "XNAS"),
    ("NYSE AMERICAN", "NYSE American", "XASE"),
    ("AMEX", "NYSE American", "XASE"),
    ("NYSE ARCA", "NYSE Arca", "ARCX"),
    ("NYSE", "NYSE", "XNYS"),
    ("NEW YORK", "NYSE", "XNYS"),
];

pub fn tmx_venue(name: &str) -> String {
    let up = name.to_uppercase();
    for (mark, venue, _) in EXCHANGE_NAMES {
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

fn ask(symbol: &str) -> Result<Value, FetchError> {
    let payload = json!({
        "operationName": "getQuoteBySymbol",
        "variables": {"symbol": symbol, "locale": "en"},
        "query": TMX_QUOTE_QUERY,
    });
    post_json(TMX_URL, &payload, &TMX_HEADERS)
}

/// TMX's quote under a listing's own form; a request that fails is the failure.
pub fn fetch_tmx_quote(form: &str) -> Result<Option<SourceQuote>, FetchError> {
    ask(form).map(|d| parse_tmx_quote(&d))
}

/// Every Canadian listing TMX knows a symbol given with no venue as: the symbol
/// asked under each Canadian venue's form, and each answer whose quote names the
/// venue the form asks for returned with that venue. Never one picked: a symbol
/// listed on two venues is two listings. The public directories cover the TSX and
/// Nasdaq registries alone, so this is how a CSE or Cboe Canada listing is found.
pub fn tmx_listings(symbol: &str) -> Result<Vec<bagholder_model::wire::SymbolMatch>, FetchError> {
    let bare = tmx_bare(&bagholder_model::venues::tmx_symbol(symbol));
    if bare.is_empty() || bare.contains(' ') {
        return Ok(vec![]);
    }
    let mut forms: Vec<&str> = Vec::new();
    for mic in bagholder_core::listing::CANADA_MICS {
        if let Some(form) = bagholder_sources::venue::tmx_suffix(mic) {
            if !forms.contains(&form) {
                forms.push(form);
            }
        }
    }
    let mut out = Vec::new();
    for form in forms {
        let Some(q) = fetch_tmx_quote(&format!("{bare}{form}"))? else { continue };
        if !bagholder_sources::venue::tmx_venue_matches(form, &q.exchange) {
            continue;
        }
        let venue = tmx_venue(&q.exchange);
        let Some(mic) = bagholder_sources::venue::mic_of(&venue) else { continue };
        // the venue's own currency where TMX states none, by the app's one venue table
        let currency = if !q.currency.is_empty() {
            q.currency
        } else {
            match bagholder_sources::venue::currency_of(mic) {
                Some(c) => c.as_str().to_string(),
                None => continue,
            }
        };
        let name = if q.name.is_empty() { bare.clone() } else { q.name };
        out.push(bagholder_model::wire::SymbolMatch { symbol: bare.clone(), name, exchange: venue, currency, ..Default::default() });
    }
    Ok(out)
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
pub fn fetch_tmx(symbol: &str, exchange: &str) -> Result<(Option<SourceQuote>, Vec<DistributionRecord>), FetchError> {
    let form = match tmx_record_symbol(symbol, exchange) { Some(s) => s, None => return Ok((None, vec![])) };
    let quote = fetch_tmx_quote(&form)?;
    let payload = json!({
        "operationName": "getDividendsForSymbol",
        "variables": {"symbol": form, "page": 1, "batch": TMX_BATCH},
        "query": TMX_DIVIDENDS_QUERY,
    });
    let divs = parse_tmx_dividends(&post_json(TMX_URL, &payload, &TMX_HEADERS)?);
    Ok((quote, divs))
}
