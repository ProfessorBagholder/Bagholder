//! TMX Money: what its quote says a Canadian listing is (its name, venue and
//! currency), asked under the form of its own venue.
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
use bagholder_model::lenient;
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
    #[serde(deserialize_with = "lenient::text")]
    currency: String,
    #[serde(deserialize_with = "lenient::text")]
    name: String,
    #[serde(deserialize_with = "lenient::text")]
    exchange_name: String,
}

/// What TMX's quote says a listing is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TmxListing {
    pub name: String,
    /// TMX's name for the exchange.
    pub exchange: String,
    pub currency: String,
}

/// What TMX's quote says a listing is. An empty answer is no answer.
pub fn parse_tmx_listing(data: &Value) -> Option<TmxListing> {
    let q = data.get("data")?.get("getQuoteBySymbol")?;
    if q.as_object()?.is_empty() {
        return None;
    }
    let q = TmxQuote::deserialize(q).ok()?;
    Some(TmxListing { name: q.name, exchange: q.exchange_name, currency: q.currency })
}

fn ask(symbol: &str) -> Result<Value, FetchError> {
    let payload = json!({
        "operationName": "getQuoteBySymbol",
        "variables": {"symbol": symbol, "locale": "en"},
        "query": TMX_QUOTE_QUERY,
    });
    post_json(TMX_URL, &payload, &TMX_HEADERS)
}

/// What TMX's quote under a listing's own form says it is; a request that fails
/// is the failure.
pub fn fetch_tmx_listing(form: &str) -> Result<Option<TmxListing>, FetchError> {
    ask(form).map(|d| parse_tmx_listing(&d))
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
        let Some(q) = fetch_tmx_listing(&format!("{bare}{form}"))? else { continue };
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
