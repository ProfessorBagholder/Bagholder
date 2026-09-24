//! Reading from the public sources.
//!
//! One connection per host is kept open between requests. Every read used to
//! open a new TLS connection, and for a request this small the handshake is
//! the whole cost -- which is what pinned a small board at a full core while
//! the archive caught up.

use serde_json::Value;
use std::time::Duration;

pub use bagholder_net::client::Error as FetchError;

pub const TIMEOUT_SEC: u64 = 30;

pub const UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Each source's label, as a failure names it.
pub const SOURCE_LABELS: [(&str, &str); 9] = [
    ("tmx", "TMX Money"),
    ("yahoo", "Yahoo Finance"),
    ("coinbase", "Coinbase"),
    ("cboe", "Cboe"),
    ("boc", "Bank of Canada"),
    ("fred", "FRED"),
    ("stooq", "Stooq"),
    ("finra", "FINRA"),
    ("ciro", "CIRO"),
];

/// A failure in words a user can act on.
pub fn describe_failure(e: &FetchError) -> String {
    match e {
        FetchError::Status(429) => "refused the request (too many)".into(),
        FetchError::Status(c) => format!("answered with an error ({})", c),
        FetchError::Transport(m) if m.contains("not asked again before") => {
            "refused the request; asked again when its rest is over".into()
        }
        _ => "could not be reached".into(),
    }
}

pub fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String, FetchError> {
    let default: Vec<(&str, &str)> = vec![("User-Agent", UA), ("Accept", "text/csv,application/json,*/*;q=0.8")];
    let hdrs = if headers.is_empty() { &default[..] } else { headers };
    bagholder_net::client::request("GET", url, hdrs, None, Duration::from_secs(TIMEOUT_SEC)).map(|r| r.text())
}

pub fn post_json(url: &str, payload: &Value, headers: &[(&str, &str)]) -> Result<Value, FetchError> {
    let body = serde_json::to_string(payload).unwrap_or_default();
    let mut hdrs: Vec<(&str, &str)> = vec![
        ("User-Agent", UA),
        ("Content-Type", "application/json"),
        ("Accept", "*/*"),
    ];
    // `hdrs.update(headers)`: a caller's header replaces the default of the
    // same name rather than being sent beside it. FINRA answers two Accept
    // headers with CSV.
    for (k, v) in headers {
        match hdrs.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(k)) {
            Some(slot) => *slot = (k, v),
            None => hdrs.push((k, v)),
        }
    }
    let resp = bagholder_net::client::request("POST", url, &hdrs, Some(body.as_bytes()), Duration::from_secs(TIMEOUT_SEC))?;
    serde_json::from_str(&resp.text()).map_err(|e| FetchError::Transport(e.to_string()))
}

/// The headers TMX's GraphQL endpoint expects.
pub const TMX_HEADERS: [(&str, &str); 3] = [
    ("locale", "en"),
    ("Origin", "https://money.tmx.com"),
    ("Referer", "https://money.tmx.com/"),
];
