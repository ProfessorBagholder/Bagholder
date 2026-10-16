//! How a listing is named on TMX: `tmx_symbol` and `tmx_form`.
//!
//! The model needs both to key an exposure record and to tell one listing from
//! another, so they live here rather than in the market fetcher.

/// `exposure.SHARE_KEY`: the cache key for a classified listing is
/// `share:<ListingKey>`, a fund's own record `fund:<ListingKey>`: two listings of
/// one ticker are two records.
pub const SHARE_KEY: &str = "share:";
pub const FUND_KEY: &str = "fund:";

/// The record key of a listing classified as a share.
pub fn share_key(key: &bagholder_core::ListingKey) -> String {
    format!("{SHARE_KEY}{key}")
}

/// The record key of a fund's own breakdown.
pub fn fund_key(key: &bagholder_core::ListingKey) -> String {
    format!("{FUND_KEY}{key}")
}

/// The share record of `symbol` on the venue `exchange` names; none for a venue
/// no listing is keyed by, which no record is kept for.
pub fn share_key_of(symbol: &str, exchange: &str) -> Option<String> {
    bagholder_core::ListingKey::on(symbol, exchange).ok().map(|k| share_key(&k))
}

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

/// `tmx_form`: TMX's suffix for the venue `exchange` names, in words, as a code
/// or as a key's `US`; `None` for a venue TMX does not carry or none known here.
/// A currency never stands in for a venue (plan `market-data-keyed-by-listing`).
pub fn tmx_form(exchange: &str) -> Option<&'static str> {
    match bagholder_core::listing::key_venue(exchange)? {
        bagholder_core::listing::US => Some(":US"),
        "XTSE" | "XTSX" | "XATS" => Some(""),
        "XCNQ" => Some(":CNX"),
        "NEOE" => Some(":AQL"),
        _ => None,
    }
}

