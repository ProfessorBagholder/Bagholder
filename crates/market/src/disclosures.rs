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
    /// Anything else that went wrong, with Python's `"%s: %s" % (type, e)`
    /// shape already applied.
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

/// `disclosures.clean`: tags to spaces, whitespace collapsed.
pub fn clean(text: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let t = re(&TAGS, r"<[^>]+>").replace_all(text, " ");
    let t = re(&WS, r"\s+").replace_all(&t, " ");
    bagholder_model::pytext::py_strip(&t).to_string()
}

fn name_tokens(name: &str) -> std::collections::HashSet<String> {
    static NOISE: OnceLock<Regex> = OnceLock::new();
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    let lower = name.to_lowercase();
    let n = re(&NOISE, r"(?i)\b(inc|corp|corporation|ltd|limited|co|company|plc|the|sa|nv|ag|llc|lp|trust|fund|holdings?)\b").replace_all(&lower, " ");
    re(&SPLIT, r"[^a-z0-9]+").split(&n).filter(|t| t.chars().count() > 1).map(|t| t.to_string()).collect()
}

/// `disclosures.names_match`: whether two issuer names plausibly denote the
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

/// `disclosures.available`: true when at least one source can be reached.
pub fn available() -> bool {
    crate::sedar::available() || crate::edgar::available()
}

/// `disclosures.fetch`: every covering source's filings for one instrument,
/// merged newest first, with each source's outcome beside them. A source that
/// fails is recorded and skipped; the others still return.
pub fn fetch(symbol: &str, name: &str, exchange: &str, currency: &str, limit: usize, profile_no: &str) -> Value {
    let mut items: Vec<Value> = Vec::new();
    let mut sources = Map::new();

    // SEDAR+ first, then EDGAR, as PROVIDERS lists them
    let sedar_covered = crate::sedar::available() && crate::sedar::covers(symbol, exchange, currency);
    if !sedar_covered {
        sources.insert(crate::sedar::SOURCE.into(), json!({"available": crate::sedar::available(), "matched": false, "filer": false, "count": 0, "error": ""}));
    } else {
        match crate::sedar::fetch(symbol, name, exchange, currency, crate::sedar::SEARCH_LIMIT, profile_no) {
            Ok(got) => {
                let n = got.len();
                items.extend(got);
                sources.insert(crate::sedar::SOURCE.into(), json!({"available": true, "matched": n > 0, "filer": n > 0, "count": n, "error": ""}));
            }
            Err(e) => {
                sources.insert(crate::sedar::SOURCE.into(), outcome_of(&e));
            }
        }
    }

    let edgar_covered = crate::edgar::covers(symbol, exchange, currency);
    if !edgar_covered {
        sources.insert(crate::edgar::SOURCE.into(), json!({"available": true, "matched": false, "filer": false, "count": 0, "error": ""}));
    } else {
        match crate::edgar::fetch(symbol, name, exchange, currency, 200) {
            Ok(got) => {
                let n = got.len();
                items.extend(got);
                let filer = n > 0 || crate::edgar::has_filer(symbol, name, exchange, currency);
                sources.insert(crate::edgar::SOURCE.into(), json!({"available": true, "matched": n > 0, "filer": filer, "count": n, "error": ""}));
            }
            Err(e) => {
                sources.insert(crate::edgar::SOURCE.into(), outcome_of(&e));
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

/// `disclosures.document`: the document a stored row points at, from its
/// source. (bytes, content type).
pub fn document(row: &Value) -> Fetched<(Vec<u8>, String)> {
    let src = row.get("source").and_then(|v| v.as_str()).unwrap_or("");
    match src {
        s if s == crate::sedar::SOURCE => crate::sedar::document(row),
        s if s == crate::edgar::SOURCE => crate::edgar::document(row),
        _ => Err(SourceError::Unavailable(format!("no provider for source {}", py_repr(src)))),
    }
}

/// `disclosures.content`: the document's readable substance, past a cover form
/// where the provider can tell.
pub fn content(row: &Value) -> Fetched<(Vec<u8>, String)> {
    let src = row.get("source").and_then(|v| v.as_str()).unwrap_or("");
    match src {
        s if s == crate::sedar::SOURCE => crate::sedar::document(row),
        s if s == crate::edgar::SOURCE => crate::edgar::content(row),
        _ => Err(SourceError::Unavailable(format!("no provider for source {}", py_repr(src)))),
    }
}

/// `disclosures.enrichment`: a provider's deterministic title and summary for
/// a structured filing it can parse exactly, or None.
pub fn enrichment(row: &Value) -> Option<Value> {
    match row.get("source").and_then(|v| v.as_str()).unwrap_or("") {
        s if s == crate::edgar::SOURCE => crate::edgar::enrichment(row),
        _ => None,
    }
}

/// `disclosures.categorize`: a stored row's category re-derived from its type,
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

/// Python's `repr()` of a string, for the messages that quote one.
pub fn py_repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{}\"", s.replace('\\', "\\\\"))
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}
