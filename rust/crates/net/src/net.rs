//! Asking a host: through the limiter, on the clock handed in, by the transport
//! the host needs.

use crate::browser;
use crate::client;
use crate::clock::Clock;
use crate::limiter::{Limiter, Resting};
use jiff::Timestamp;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How a host is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// The HTTP client (OpenSSL's handshake).
    Direct,
    /// The helper that presents a browser's handshake, for a host that gates on
    /// it (SEDAR+, Global X). Its session keeps cookies for its life.
    Browser,
}

pub struct Ask<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a [u8]>,
    pub via: Via,
    pub timeout: Duration,
}

impl<'a> Ask<'a> {
    pub fn get(url: &'a str, headers: &'a [(&'a str, &'a str)]) -> Ask<'a> {
        Ask { method: "GET", url, headers, body: None, via: Via::Direct, timeout: Duration::from_secs(30) }
    }

    pub fn post(url: &'a str, headers: &'a [(&'a str, &'a str)], body: &'a [u8]) -> Ask<'a> {
        Ask { method: "POST", url, headers, body: Some(body), via: Via::Direct, timeout: Duration::from_secs(30) }
    }

    pub fn via(self, via: Via) -> Ask<'a> {
        Ask { via, ..self }
    }
}

/// A host's answer, whatever its status.
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    /// Where the request ended up after redirects (the asked URL where the
    /// transport does not say).
    pub url: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// When the answer arrived, on the clock handed in.
    pub received_at: Timestamp,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// The host refused recently and is left alone until then; nothing was sent.
    Resting { host: String, until: Timestamp },
    /// The host could not be reached, or the transport failed.
    Unreachable(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Resting { host, until } => write!(f, "{host} refused a request and is not asked again before {until}"),
            NetError::Unreachable(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for NetError {}

/// The host of a URL, lower-cased.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    host.to_ascii_lowercase()
}

/// A `Retry-After` value as the time to wait from `now`: seconds, or an HTTP
/// date. Anything else says nothing.
pub fn retry_after(value: &str, now: Timestamp) -> Option<Duration> {
    let v = value.trim();
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        return v.parse::<u64>().ok().map(Duration::from_secs);
    }
    let at = jiff::fmt::rfc2822::parse(v).ok()?.timestamp();
    Some(if at > now { Duration::try_from(now.duration_until(at)).unwrap_or(Duration::ZERO) } else { Duration::ZERO })
}

/// What a transport answers: the status, where the request ended up, the
/// headers (names lower-cased) and the body.
pub type Answer = (u16, String, Vec<(String, String)>, Vec<u8>);

/// Something that answers requests in place of the network: recorded replies in
/// tests. The limiter and the clock still apply to every request.
pub trait Transport: Send + Sync {
    fn answer(&self, ask: &Ask) -> Result<Answer, NetError>;
}

/// The network as the sources see it.
pub struct Net {
    clock: Arc<dyn Clock>,
    limiter: Arc<Limiter>,
    browser: Mutex<Option<browser::Session>>,
    replaced: Option<Box<dyn Transport>>,
}

impl Net {
    pub fn new(clock: Arc<dyn Clock>, limiter: Arc<Limiter>) -> Net {
        Net { clock, limiter, browser: Mutex::new(None), replaced: None }
    }

    /// A network whose every request is answered by `transport`.
    pub fn answered_by(clock: Arc<dyn Clock>, limiter: Arc<Limiter>, transport: Box<dyn Transport>) -> Net {
        Net { clock, limiter, browser: Mutex::new(None), replaced: Some(transport) }
    }

    pub fn clock(&self) -> &dyn Clock {
        &*self.clock
    }

    pub fn limiter(&self) -> &Limiter {
        &self.limiter
    }

    /// One request. Any status comes back as a reply; a refusal (429, or 503
    /// with a `Retry-After`) also rests the host, for as long as it says.
    pub fn send(&self, ask: &Ask) -> Result<Reply, NetError> {
        let host = host_of(ask.url);
        // told to stay off the network: nothing leaves, and the refusal says so in
        // the one wording a record of it can tell apart (`client::OFFLINE`)
        if self.replaced.is_none() && client::offline() && !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
            return Err(NetError::Unreachable(client::OFFLINE.into()));
        }
        self.limiter.turn(&host, &*self.clock).map_err(|Resting { until }| NetError::Resting { host: host.clone(), until })?;
        let (status, url, headers, body) = match (&self.replaced, ask.via) {
            (Some(t), _) => t.answer(ask)?,
            (None, Via::Direct) => {
                let r = client::request_any(ask.method, ask.url, ask.headers, ask.body, ask.timeout).map_err(|e| NetError::Unreachable(e.to_string()))?;
                (r.status, ask.url.to_string(), r.headers, r.body)
            }
            (None, Via::Browser) => {
                let mut slot = self.browser.lock().unwrap_or_else(|e| e.into_inner());
                if slot.is_none() {
                    *slot = Some(browser::Session::new().ok_or_else(|| NetError::Unreachable("the browser helper is not installed".into()))?);
                }
                let body = ask.body.map(|b| String::from_utf8_lossy(b).into_owned());
                let got = slot.as_mut().unwrap().request(ask.method, ask.url, ask.headers, body.as_deref(), ask.timeout, true);
                match got {
                    Ok(a) => (a.status, a.url, a.headers.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect(), a.body),
                    Err(e) => {
                        // a helper that failed is started afresh next time
                        *slot = None;
                        return Err(NetError::Unreachable(e));
                    }
                }
            }
        };
        let received_at = self.clock.now();
        let reply = Reply { status, url, headers, body, received_at };
        let named = reply.header("retry-after").and_then(|v| retry_after(v, received_at));
        if status == 429 || (status == 503 && named.is_some()) {
            self.limiter.refused(&host, received_at, named);
        }
        Ok(reply)
    }
}
