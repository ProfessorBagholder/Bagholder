//! How a listing is named on TMX: `tmx_symbol` and `tmx_form`.
//!
//! The model needs both to key an exposure record and to tell one listing from
//! another, so they live here rather than in the market fetcher.

pub const US_EXCHANGES: [&str; 9] =
    ["NASDAQ", "NYSE", "NYSE AMERICAN", "NYSE ARCA", "BATS", "AMEX", "ARCA", "CBOE", "IEX"];
pub const CBOE_CANADA_EXCHANGES: [&str; 2] = ["CBOE CANADA", "NEO"];

/// `exposure.SHARE_KEY`: the cache key for a classified listing is
/// `share:<TICKER>:<venue form>`.
pub const SHARE_KEY: &str = "share:";

/// `tmx_symbol`: Wealthsimple's Canadian tickers already match TMX
/// Money's, so only the venue suffix comes off.
pub fn tmx_symbol(symbol: &str) -> String {
    let mut s = symbol.trim().to_uppercase();
    for suffix in [".TO", ".V", ".CN", ".NE"] {
        if s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    s
}

/// `tmx_form`: TMX's suffix for a listing venue, or `None` when TMX
/// does not carry it.
pub fn tmx_form(exchange: &str, currency: &str) -> Option<&'static str> {
    let ex = exchange.trim().to_uppercase();
    let ccy = currency.trim().to_uppercase();
    if US_EXCHANGES.contains(&ex.as_str()) || (ex.is_empty() && ccy == "USD") {
        return Some(":US");
    }
    if CBOE_CANADA_EXCHANGES.contains(&ex.as_str()) {
        return Some(":AQL");
    }
    if ex == "CSE" {
        return Some(":CNX");
    }
    if ex == "TSX" || ex == "TSX-V" || ex == "TSXV" {
        return Some("");
    }
    // a venue TMX does not name (an ATS such as Alpha, or none at all): start
    // from the currency's usual form and let the lookup settle it
    if ccy == "CAD" {
        return Some("");
    }
    if ccy == "USD" {
        return Some(":US");
    }
    None
}

/// `watch_exposure_key`.
pub fn watch_exposure_key(symbol: &str, exchange: &str, currency: &str) -> String {
    format!("{}{}:{}", SHARE_KEY, tmx_symbol(symbol), tmx_form(exchange, currency).unwrap_or(""))
}
