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

/// A dated report's body, or `None` where the publisher states the report is not
/// published yet, as each states it: a 404; a redirect away from the file to the
/// site's own page (CIRO sends an unpublished report to its not-found page, which
/// itself answers 403); or the storage service's own refusal of a key it does not
/// hold (FINRA's files are behind one that answers a missing key with an
/// `AccessDenied` error document). Any other refusal is the failure it is.
pub fn get_published(url: &str, headers: &[(&str, &str)]) -> Result<Option<Vec<u8>>, FetchError> {
    let ask = Ask { timeout: Duration::from_secs(TIMEOUT_SEC), ..Ask::get(url, headers) };
    let reply = bagholder_net::machine::net().send(&ask).map_err(|e| match e {
        NetError::Resting { host, until } => FetchError::Transport(format!("{host}: refused a request; not asked again before {until}")),
        NetError::Unreachable(m) => FetchError::Transport(m),
    })?;
    if not_published(url, &reply) {
        return Ok(None);
    }
    if reply.status >= 400 {
        return Err(FetchError::Status(reply.status));
    }
    Ok(Some(reply.body))
}

/// Whether `reply` is the publisher saying the file at `url` does not exist yet.
pub fn not_published(url: &str, reply: &bagholder_net::Reply) -> bool {
    let path = |u: &str| u.split_once("://").map(|(_, rest)| rest.split_once('/').map_or("", |(_, p)| p).split(['?', '#']).next().unwrap_or("").to_string()).unwrap_or_default();
    let redirected_away = path(&reply.url) != path(url);
    let storage_says_no_key = reply.status == 403 && {
        let body = String::from_utf8_lossy(&reply.body);
        body.trim_start().starts_with("<?xml") && (body.contains("<Code>AccessDenied</Code>") || body.contains("<Code>NoSuchKey</Code>"))
    };
    reply.status == 404 || redirected_away || storage_says_no_key
}

/// The body as sent (a spreadsheet, a PDF).
pub fn get_bytes(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>, FetchError> {
    let default: Vec<(&str, &str)> = vec![("User-Agent", UA), ("Accept", "text/csv,application/json,*/*;q=0.8")];
    let hdrs = if headers.is_empty() { &default[..] } else { headers };
    let ask = Ask { timeout: Duration::from_secs(TIMEOUT_SEC), ..Ask::get(url, hdrs) };
    send(&ask).map(|r| r.body)
}

pub fn post_json(url: &str, payload: &Value, headers: &[(&str, &str)]) -> Result<Value, FetchError> {
    let body = serde_json::to_string(payload).expect("a JSON value always serializes");
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

#[cfg(test)]
mod tests {
    use super::not_published;

    fn reply(status: u16, url: &str, body: &str) -> bagholder_net::Reply {
        bagholder_net::Reply { status, url: url.into(), headers: vec![], body: body.as_bytes().to_vec(), received_at: "2026-10-02T15:00:00Z".parse().unwrap() }
    }

    /// Each way a publisher states that a dated report is not out yet is no failure;
    /// every other refusal is.
    #[test]
    fn a_report_not_published_yet_is_told_from_a_failure() {
        let file = "https://publisher.example/files/20261015_report.xls";
        // a 404, at the file itself
        assert!(not_published(file, &reply(404, file, "")));
        // sent to the site's own page, whatever that page answers
        for status in [200, 403, 404] {
            assert!(not_published(file, &reply(status, "https://publisher.example/404", "<html>")), "{status}");
        }
        // a storage service refusing a key it does not hold
        for code in ["AccessDenied", "NoSuchKey"] {
            let body = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{code}</Code><Message>m</Message></Error>");
            assert!(not_published(file, &reply(403, file, &body)), "{code}");
        }
        // the file answered, or refused for any other reason: not this
        assert!(!not_published(file, &reply(200, file, "data")));
        assert!(!not_published(file, &reply(403, file, "<html>Attention Required</html>")));
        assert!(!not_published(file, &reply(429, file, "")));
        assert!(!not_published(file, &reply(503, file, "")));
        // the same file asked with a query string is the same file
        assert!(!not_published(&format!("{file}?v=1"), &reply(200, file, "data")));
    }
}
