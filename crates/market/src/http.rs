//! Reading from the public sources, and remembering which of them answered.
//!
//! One connection per host is kept open between requests. Every read used to
//! open a new TLS connection, and for a request this small the handshake is
//! the whole cost -- which is what pinned a small board at a full core while
//! the archive caught up.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub use crate::client::Error as FetchError;

pub const TIMEOUT_SEC: u64 = 30;

/// `market.UA`.
pub const UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// `market.SOURCE_LABELS`, in the order the health list is reported in.
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

const HOST_NEEDLES: [(&str, &str); 9] = [
    ("tmx", "tmx.com"),
    ("yahoo", "yahoo.com"),
    ("coinbase", "coinbase.com"),
    ("cboe", "cboe.com"),
    ("boc", "bankofcanada.ca"),
    ("fred", "stlouisfed.org"),
    ("stooq", "stooq.com"),
    ("finra", "finra.org"),
    ("ciro", "ciro.ca"),
];


/// `market.source_of_url`.
pub fn source_of_url(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    for (key, needle) in HOST_NEEDLES {
        if host.contains(needle) {
            return key.to_string();
        }
    }
    if host.is_empty() { "other".to_string() } else { host }
}

/// `market.describe_failure`: a failure in words a user can act on.
pub fn describe_failure(e: &FetchError) -> String {
    match e {
        FetchError::Status(429) => "refused the request (too many)".into(),
        FetchError::Status(c) => format!("answered with an error ({})", c),
        FetchError::Transport(m) if m.contains("backing off") => {
            "refused the request; asked again in ten minutes".into()
        }
        _ => "could not be reached".into(),
    }
}

struct Health {
    ok: bool,
    at: String,
    error: String,
}

fn health() -> &'static Mutex<BTreeMap<String, Health>> {
    static H: OnceLock<Mutex<BTreeMap<String, Health>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// `market.note_source`.
pub fn note_source(name: &str, ok: bool, error: Option<&FetchError>) {
    let at = crate::now_stamp();
    // a failure with nothing to say for itself reads as Python's does
    let error = if ok { String::new() } else { error.map(describe_failure).unwrap_or_else(|| "could not be reached".into()) };
    health().lock().unwrap().insert(name.to_string(), Health { ok, at, error });
}

/// `market.source_health`: every source touched since start, in a fixed order.
pub fn source_health() -> Vec<Value> {
    let snap = health().lock().unwrap();
    SOURCE_LABELS
        .iter()
        .filter_map(|(k, name)| {
            snap.get(*k).map(|h| json!({"key": k, "name": name, "ok": h.ok, "at": h.at, "error": h.error}))
        })
        .collect()
}


/// `market._get_text`. A 404 is a symbol the source does not carry, not the
/// source failing, so it is not recorded against its health.
pub fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String, FetchError> {
    let default: Vec<(&str, &str)> = vec![("User-Agent", UA), ("Accept", "text/csv,application/json,*/*;q=0.8")];
    let hdrs = if headers.is_empty() { &default[..] } else { headers };
    match crate::client::request("GET", url, hdrs, None, Duration::from_secs(TIMEOUT_SEC)) {
        Ok(resp) => {
            note_source(&source_of_url(url), true, None);
            Ok(resp.text())
        }
        Err(err) => {
            if err.code() != Some(404) {
                note_source(&source_of_url(url), false, Some(&err));
            }
            Err(err)
        }
    }
}

/// `market._post_json`.
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
    match crate::client::request("POST", url, &hdrs, Some(body.as_bytes()), Duration::from_secs(TIMEOUT_SEC)) {
        Ok(resp) => {
            note_source(&source_of_url(url), true, None);
            serde_json::from_str(&resp.text()).map_err(|e| FetchError::Transport(e.to_string()))
        }
        Err(err) => {
            note_source(&source_of_url(url), false, Some(&err));
            Err(err)
        }
    }
}

/// The headers TMX's GraphQL endpoint expects.
pub const TMX_HEADERS: [(&str, &str); 3] = [
    ("locale", "en"),
    ("Origin", "https://money.tmx.com"),
    ("Referer", "https://money.tmx.com/"),
];
