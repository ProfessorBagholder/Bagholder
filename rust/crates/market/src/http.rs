//! Reading from the public sources.
//!
//! One connection per host is kept open between requests. Every read used to
//! open a new TLS connection, and for a request this small the handshake is
//! the whole cost -- which is what pinned a small board at a full core while
//! the archive caught up.
//!
//! Every request here waits its turn on the process's one limiter
//! (`bagholder_net::machine::net`), at its URL's host: the host's gap between
//! two requests, and its rest after a refusal (429, or 503 with a
//! `Retry-After`, honoured). A caller that wants a host spaced sets the host's
//! pace ([`pace_host`]) and takes no turn of its own.

use bagholder_net::{Ask, NetError, Pace};
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

/// Space `host`'s requests at least `gap` apart on the one limiter, from now on
/// (a host's gap is only ever raised, so the most careful caller's holds).
pub fn pace_host(host: &str, gap: Duration) {
    let l = bagholder_net::machine::global();
    let pace = l.pace(host);
    if pace.gap < gap {
        l.configure(host, Pace { gap, ..pace });
    }
}

/// One request on the one limiter: the answer, or an HTTP error as its status.
fn send(ask: &Ask) -> Result<bagholder_net::Reply, FetchError> {
    let reply = bagholder_net::machine::net().send(ask).map_err(|e| match e {
        NetError::Resting { host, until } => {
            FetchError::Transport(format!("{host}: refused a request; not asked again before {until}"))
        }
        NetError::Unreachable(m) => FetchError::Transport(m),
    })?;
    if reply.status >= 400 {
        return Err(FetchError::Status(reply.status));
    }
    Ok(reply)
}

pub fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String, FetchError> {
    get_bytes(url, headers).map(|b| String::from_utf8_lossy(&b).to_string())
}

/// The body as sent (a spreadsheet, a PDF).
pub fn get_bytes(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>, FetchError> {
    let default: Vec<(&str, &str)> = vec![("User-Agent", UA), ("Accept", "text/csv,application/json,*/*;q=0.8")];
    let hdrs = if headers.is_empty() { &default[..] } else { headers };
    let ask = Ask { timeout: Duration::from_secs(TIMEOUT_SEC), ..Ask::get(url, hdrs) };
    send(&ask).map(|r| r.body)
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
    let ask = Ask { timeout: Duration::from_secs(TIMEOUT_SEC), ..Ask::post(url, &hdrs, body.as_bytes()) };
    let reply = send(&ask)?;
    serde_json::from_str(&String::from_utf8_lossy(&reply.body)).map_err(|e| FetchError::Transport(e.to_string()))
}

/// The headers TMX's GraphQL endpoint expects.
pub const TMX_HEADERS: [(&str, &str); 3] = [
    ("locale", "en"),
    ("Origin", "https://money.tmx.com"),
    ("Referer", "https://money.tmx.com/"),
];
