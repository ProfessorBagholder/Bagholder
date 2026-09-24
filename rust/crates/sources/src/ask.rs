//! Asking a source and reading its answer into an [`Outcome`]: the status and
//! the body, before any adapter reads a field.
//!
//! - 200 is an answer, read as the adapter needs it (JSON, text).
//! - 429, and a 503 that names a `Retry-After`, is a refusal (the limiter has
//!   already rested the host).
//! - A status the adapter names as its source's "not carried" (Valet's 404 for a
//!   series it does not publish) is that.
//! - Anything else is no answer: the source is unreachable for this request.

use bagholder_core::json::Value;
use bagholder_net::{retry_after, Ask, Net, Reply};

use crate::outcome::Outcome;
use crate::reply::{self, Keyed, Mismatch, Shape, ShapeChange};

/// What a request came to before its body is read.
pub fn send(net: &Net, ask: &Ask, not_carried: &[u16]) -> Outcome<Reply> {
    let reply = match net.send(ask) {
        Ok(r) => r,
        Err(e) => return e.into(),
    };
    status(reply, not_carried)
}

/// The outcome a reply's status says, the reply itself when it is an answer.
pub fn status(reply: Reply, not_carried: &[u16]) -> Outcome<Reply> {
    match reply.status {
        200 => Outcome::Answered(reply),
        s if not_carried.contains(&s) => Outcome::NotCarried(format!("status {s}")),
        429 => Outcome::Refused { status: Some(429), retry_after: reply.header("retry-after").and_then(|v| retry_after(v, reply.received_at)) },
        503 if reply.header("retry-after").is_some() => Outcome::Refused { status: Some(503), retry_after: reply.header("retry-after").and_then(|v| retry_after(v, reply.received_at)) },
        s => Outcome::Unreachable(format!("status {s} from {}", reply.url)),
    }
}

/// A body as text: UTF-8, or a mismatch.
pub fn text(body: &[u8]) -> Result<&str, Mismatch> {
    std::str::from_utf8(body).map_err(|e| Mismatch { path: String::new(), why: format!("not UTF-8 text: {e}") })
}

/// A body as one JSON value.
pub fn json(body: &[u8]) -> Result<Value, Mismatch> {
    reply::parse(text(body)?)
}

/// The shape change of a JSON reply against the shape its recorded replies
/// carry, if any, the objects `keyed` names read as data-keyed.
pub fn noticed(value: &Value, recorded: &Shape, keyed: &[Keyed]) -> Option<ShapeChange> {
    reply::shape_change(recorded, &reply::shape_keyed(value, keyed))
}

/// The recorded replies' shape from its committed form: one path per line.
pub fn recorded_shape(paths: &str) -> Shape {
    paths.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|p| (if p == "." { String::new() } else { p.to_string() }, Default::default())).collect()
}

/// A shape's committed form: one path per line, the root written `.`.
pub fn shape_text(shape: &Shape) -> String {
    let mut out = String::new();
    for p in shape.keys() {
        out.push_str(if p.is_empty() { "." } else { p });
        out.push('\n');
    }
    out
}
