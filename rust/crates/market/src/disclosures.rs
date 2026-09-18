//! The disclosures pipeline: one instrument's regulatory filings gathered from
//! every source that covers it, normalized to a single shape, merged newest
//! first.
//!
//! A source is a regulator's filing system (SEDAR+ for Canada, SEC EDGAR for
//! the US); a category is the kind of disclosure, the same across sources. An
//! item is `{id, source, category, date, dateText, type, title, size, url}`,
//! its id prefixed with the source ("sedar:", "sec:") so a stored row routes
//! back to the provider that can download it.

use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

pub const FINANCIALS: &str = "Financials";
pub const EVENTS: &str = "Material events";
pub const GOVERNANCE: &str = "Governance";
pub const OFFERINGS: &str = "Offerings";
pub const INSIDER: &str = "Insider & ownership";
pub const NEWS: &str = "News releases";
pub const OTHER: &str = "Other";
pub const CATEGORIES: [&str; 7] = [FINANCIALS, EVENTS, GOVERNANCE, OFFERINGS, INSIDER, NEWS, OTHER];

/// A source could not be reached, or what it needs is not installed.
#[derive(Debug, Clone)]
pub enum SourceError {
    Unavailable(String),
    /// Anything else that went wrong, as `kind: message`.
    Other(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Unavailable(m) | SourceError::Other(m) => write!(f, "{}", m),
        }
    }
}

pub type Fetched<T> = Result<T, SourceError>;

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

/// Tags to spaces, whitespace collapsed.
pub fn clean(text: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let t = re(&TAGS, r"<[^>]+>").replace_all(text, " ");
    let t = re(&WS, r"\s+").replace_all(&t, " ");
    bagholder_model::textrules::trim_space(&t).to_string()
}

fn name_tokens(name: &str) -> std::collections::HashSet<String> {
    static NOISE: OnceLock<Regex> = OnceLock::new();
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    let lower = name.to_lowercase();
    let n = re(&NOISE, r"(?i)\b(inc|corp|corporation|ltd|limited|co|company|plc|the|sa|nv|ag|llc|lp|trust|fund|holdings?)\b").replace_all(&lower, " ");
    re(&SPLIT, r"[^a-z0-9]+").split(&n).filter(|t| t.chars().count() > 1).map(|t| t.to_string()).collect()
}

/// Whether two issuer names plausibly denote the
/// same company, used to reject a ticker that collides with an unrelated filer
/// in another market.
pub fn names_match(a: &str, b: &str) -> bool {
    let (ta, tb) = (name_tokens(a), name_tokens(b));
    if ta.is_empty() || tb.is_empty() {
        return false;
    }
    let overlap = ta.intersection(&tb).count();
    overlap > 0 && overlap as f64 >= ta.len().min(tb.len()) as f64 * 0.5
}

fn sort_key(item: &Value) -> (String, String) {
    let s = |k: &str| item.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let date = { let d = s("date"); if d.is_empty() { s("dateText") } else { d } };
    (date, s("source"))
}

/// True when at least one source can be reached.
pub fn available() -> bool {
    crate::sedar::available() || crate::edgar::available()
}

/// A filing source.
pub trait Provider {
    fn source(&self) -> &str;
    fn available(&self) -> bool;
    fn covers(&self, symbol: &str, exchange: &str, currency: &str) -> bool;
    fn fetch(&self, symbol: &str, name: &str, exchange: &str, currency: &str, profile_no: &str) -> Fetched<Vec<Value>>;
    /// Whether the source knows a filer even when it listed nothing.
    fn has_filer(&self, _symbol: &str, _name: &str, _exchange: &str, _currency: &str) -> bool {
        false
    }
    fn document(&self, row: &Value) -> Fetched<(Vec<u8>, String)>;
}

struct Sedar;
impl Provider for Sedar {
    fn source(&self) -> &str { crate::sedar::SOURCE }
    fn available(&self) -> bool { crate::sedar::available() }
    fn covers(&self, s: &str, e: &str, c: &str) -> bool { crate::sedar::covers(s, e, c) }
    fn fetch(&self, s: &str, n: &str, e: &str, c: &str, p: &str) -> Fetched<Vec<Value>> {
        crate::sedar::fetch(s, n, e, c, crate::sedar::SEARCH_LIMIT, p)
    }
    fn document(&self, row: &Value) -> Fetched<(Vec<u8>, String)> { crate::sedar::document(row) }
}

struct Edgar;
impl Provider for Edgar {
    fn source(&self) -> &str { crate::edgar::SOURCE }
    fn available(&self) -> bool { crate::edgar::available() }
    fn covers(&self, s: &str, e: &str, c: &str) -> bool { crate::edgar::covers(s, e, c) }
    fn fetch(&self, s: &str, n: &str, e: &str, c: &str, _p: &str) -> Fetched<Vec<Value>> {
        crate::edgar::fetch(s, n, e, c, 200)
    }
    fn has_filer(&self, s: &str, n: &str, e: &str, c: &str) -> bool { crate::edgar::has_filer(s, n, e, c) }
    fn document(&self, row: &Value) -> Fetched<(Vec<u8>, String)> { crate::edgar::document(row) }
}

/// SEDAR+ first, then EDGAR.
pub fn providers() -> [&'static dyn Provider; 2] {
    [&Sedar, &Edgar]
}

/// Every covering source's filings for one instrument,
/// merged newest first, with each source's outcome beside them. A source that
/// fails is recorded and skipped; the others still return.
pub fn fetch(symbol: &str, name: &str, exchange: &str, currency: &str, limit: usize, profile_no: &str) -> Value {
    fetch_from(&providers(), symbol, name, exchange, currency, limit, profile_no)
}

/// `fetch` over the providers given.
pub fn fetch_from(providers: &[&dyn Provider], symbol: &str, name: &str, exchange: &str, currency: &str, limit: usize, profile_no: &str) -> Value {
    let mut items: Vec<Value> = Vec::new();
    let mut sources = Map::new();
    for p in providers {
        let avail = p.available();
        if !(avail && p.covers(symbol, exchange, currency)) {
            sources.insert(p.source().into(), json!({"available": avail, "matched": false, "filer": false, "count": 0, "error": ""}));
            continue;
        }
        match p.fetch(symbol, name, exchange, currency, profile_no) {
            Ok(got) => {
                let n = got.len();
                items.extend(got);
                let filer = n > 0 || p.has_filer(symbol, name, exchange, currency);
                sources.insert(p.source().into(), json!({"available": true, "matched": n > 0, "filer": filer, "count": n, "error": ""}));
            }
            Err(e) => {
                sources.insert(p.source().into(), outcome_of(&e));
            }
        }
    }
    // `sort(key=..., reverse=True)` is stable: equal keys keep their order
    items.sort_by(|a, b| sort_key(b).cmp(&sort_key(a)));
    items.truncate(limit.max(1));
    json!({"items": items, "sources": sources})
}

fn outcome_of(e: &SourceError) -> Value {
    match e {
        SourceError::Unavailable(m) => json!({"available": false, "matched": false, "filer": false, "count": 0, "error": m}),
        SourceError::Other(m) => json!({"available": true, "matched": false, "filer": false, "count": 0, "error": m}),
    }
}

/// The document a stored row points at, from its
/// source. (bytes, content type).
pub fn document(row: &Value) -> Fetched<(Vec<u8>, String)> {
    document_from(&providers(), row)
}

/// `document` over the providers given.
pub fn document_from(providers: &[&dyn Provider], row: &Value) -> Fetched<(Vec<u8>, String)> {
    let src = row.get("source").and_then(|v| v.as_str()).unwrap_or("");
    match providers.iter().find(|p| p.source() == src) {
        Some(p) => p.document(row),
        None => Err(SourceError::Unavailable(format!("no provider for source {}", repr_quoted(src)))),
    }
}

/// The document's readable substance, past a cover form
/// where the provider can tell.
pub fn content(row: &Value) -> Fetched<(Vec<u8>, String)> {
    let src = row.get("source").and_then(|v| v.as_str()).unwrap_or("");
    match src {
        s if s == crate::sedar::SOURCE => crate::sedar::document(row),
        s if s == crate::edgar::SOURCE => crate::edgar::content(row),
        _ => Err(SourceError::Unavailable(format!("no provider for source {}", repr_quoted(src)))),
    }
}

/// A provider's deterministic title and summary for
/// a structured filing it can parse exactly, or None.
pub fn enrichment(row: &Value) -> Option<Value> {
    match row.get("source").and_then(|v| v.as_str()).unwrap_or("") {
        s if s == crate::edgar::SOURCE => crate::edgar::enrichment(row),
        s if s == crate::sedar::SOURCE => crate::sedar::enrichment(row),
        _ => None,
    }
}

/// A stored row's category re-derived from its type,
/// so a change to a mapping applies on read. Falls back to the stored one.
pub fn categorize(row: &Value) -> Value {
    let stored = row.get("category").cloned().unwrap_or(Value::Null);
    let derived = match row.get("source").and_then(|v| v.as_str()).unwrap_or("") {
        s if s == crate::sedar::SOURCE => crate::sedar::categorize(row),
        s if s == crate::edgar::SOURCE => crate::edgar::categorize(row),
        _ => return stored,
    };
    if derived.is_empty() { stored } else { json!(derived) }
}

/// A string quoted for a message: single quotes, or double quotes when it
/// holds a single quote and no double quote, backslashes escaped.
pub fn repr_quoted(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{}\"", s.replace('\\', "\\\\"))
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}
