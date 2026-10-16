//! The disclosures pipeline: a listing's regulatory filings, normalized to a
//! single shape, merged newest first.
//!
//! Filings belong to a filer, an issuer as a regulator numbers it (an SEC CIK, a
//! SEDAR+ profile). A listing reaches its own regulator's filer by that source's
//! own key: a US listing its CIK in the SEC's ticker-and-exchange list, a
//! Canadian listing the one SEDAR+ profile whose legal name is the issuer name its
//! exchange gives. A Canadian and a US listing are one company only where the
//! TSX's interlisted list pairs them; then each shows both filers' documents. No
//! name comparison joins anything (plan `market-data-keyed-by-listing`, Approach, 2).
//!
//! A source is a regulator's filing system (SEDAR+ for Canada, SEC EDGAR for
//! the US); a category is the kind of disclosure, the same across sources. A
//! document is `FiledDocument`, its id prefixed with the source ("sedar:",
//! "sec:") so a stored row routes back to the provider that can download it.

use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use bagholder_core::ListingKey;
use bagholder_store::feeds::{FiledDocument, Filer, Regulator};

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

fn sort_key(item: &FiledDocument) -> (String, Regulator) {
    let date = if item.date.is_empty() { item.date_text.clone() } else { item.date.clone() };
    (date, item.source)
}

/// True when at least one source can be reached.
pub fn available() -> bool {
    crate::sedar::available() || crate::edgar::available()
}

/// What a reading made of a document: its title, a sentence, and whether it
/// is read for good.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Enrichment {
    pub subject: String,
    pub summary: String,
    #[serde(rename = "final", skip_serializing_if = "std::ops::Not::not")]
    pub final_: bool,
}

/// One source's outcome in a gathering.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SourceOutcome {
    pub available: bool,
    pub matched: bool,
    pub filer: bool,
    pub count: usize,
    pub error: String,
}

/// Every covering source's documents, newest first, with each source's
/// outcome.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Gathered {
    pub items: Vec<FiledDocument>,
    pub sources: BTreeMap<Regulator, SourceOutcome>,
}

/// The regulators, as the pipeline asks them: which filer a listing's own
/// regulator lists it under, and a filer's documents. A trait so a test stands
/// in for the regulators.
pub trait Regulators {
    /// The regulator a listing files with, and its filer there or why none
    /// settles; `None` for a listing no regulator here covers (a coin, an index).
    fn filer(&self, listing: &ListingKey, today: &str) -> Option<(Regulator, Fetched<Result<String, String>>)>;
    /// A filer's documents, newest first.
    fn documents(&self, filer: &Filer, limit: usize) -> Fetched<Vec<FiledDocument>>;
}

/// The regulators themselves.
pub struct Live;

impl Regulators for Live {
    fn filer(&self, listing: &ListingKey, today: &str) -> Option<(Regulator, Fetched<Result<String, String>>)> {
        if listing.us() {
            let found = crate::edgar::filer_of(listing).map(|f| match f {
                Some((cik, _)) => Ok(cik.to_string()),
                None => Err(format!("The SEC lists no filer for {} on a US exchange.", listing.symbol())),
            });
            return Some((Regulator::Sec, found));
        }
        if listing.canadian() {
            let found = issuer_name(listing).and_then(|n| match n {
                Ok(name) => crate::sedar::profile_of(&name, today),
                Err(why) => Ok(Err(why)),
            });
            return Some((Regulator::Sedar, found));
        }
        None
    }

    fn documents(&self, filer: &Filer, limit: usize) -> Fetched<Vec<FiledDocument>> {
        match filer.source {
            Regulator::Sec => {
                let cik: i64 = filer.id.parse().map_err(|_| SourceError::Other(format!("{} is not a CIK", repr_quoted(&filer.id))))?;
                crate::edgar::fetch(cik, limit)
            }
            Regulator::Sedar => crate::sedar::fetch_profile(&filer.id, limit),
        }
    }
}

/// The issuer's name a Canadian listing's exchange gives: the TSX directory's
/// company record that holds the symbol, for the TSX and the TSX Venture; TMX's
/// quote's name for the CSE and Cboe Canada. Why none, where none does.
pub fn issuer_name(listing: &ListingKey) -> Fetched<Result<String, String>> {
    let failed = |e: crate::http::FetchError| SourceError::Unavailable(format!("{}", crate::http::describe_failure(&e)));
    match listing.venue() {
        "XTSE" | "XTSX" => Ok(crate::search::tsx_issuer_name(listing).map_err(failed)?.ok_or_else(|| format!("The TSX's directory names no single company holding {}.", listing.symbol()))),
        _ => {
            let form = bagholder_sources::venue::tmx_form(listing.symbol(), listing.venue()).ok_or_else(|| SourceError::Other(format!("no TMX form for {listing}")))?;
            let q = crate::tmx::fetch_tmx_quote(&form).map_err(failed)?;
            Ok(q.map(|q| q.name).filter(|n| !n.trim().is_empty()).ok_or_else(|| format!("TMX Money names no company for {}.", listing.symbol())))
        }
    }
}

/// A listing's filings gathered as the regulators answer now: its own filer's,
/// and, where the TSX's interlisted list pairs it with `partner`, the partner's
/// filer's, merged newest first, with each regulator's outcome beside them. A
/// regulator that fails is recorded and skipped; the other still returns.
pub fn gather(regulators: &dyn Regulators, listing: &ListingKey, partner: Option<&ListingKey>, today: &str, limit: usize) -> Gathered {
    let mut items: Vec<FiledDocument> = Vec::new();
    let mut sources: BTreeMap<Regulator, SourceOutcome> = BTreeMap::new();
    for l in std::iter::once(listing).chain(partner) {
        let Some((source, found)) = regulators.filer(l, today) else { continue };
        let outcome = match found {
            Ok(Ok(id)) => match regulators.documents(&Filer { source, id }, limit) {
                Ok(got) => {
                    let n = got.len();
                    items.extend(got);
                    SourceOutcome { available: true, matched: n > 0, filer: true, count: n, error: String::new() }
                }
                Err(e) => outcome_of(&e),
            },
            Ok(Err(why)) => SourceOutcome { available: true, matched: false, filer: false, count: 0, error: why },
            Err(e) => outcome_of(&e),
        };
        sources.insert(source, outcome);
    }
    items.sort_by(|a, b| sort_key(b).cmp(&sort_key(a)));
    items.dedup_by(|a, b| a.source == b.source && a.id == b.id);
    items.truncate(limit.max(1));
    Gathered { items, sources }
}

fn outcome_of(e: &SourceError) -> SourceOutcome {
    match e {
        SourceError::Unavailable(m) => SourceOutcome { available: false, matched: false, filer: false, count: 0, error: m.clone() },
        SourceError::Other(m) => SourceOutcome { available: true, matched: false, filer: false, count: 0, error: m.clone() },
    }
}

/// The document a stored row points at, from its
/// source. (bytes, content type).
pub fn document(row: &FiledDocument) -> Fetched<(Vec<u8>, String)> {
    match row.source {
        Regulator::Sedar => crate::sedar::document(row),
        Regulator::Sec => crate::edgar::document(row),
    }
}

/// The document's readable substance, past a cover form
/// where the provider can tell.
pub fn content(row: &FiledDocument) -> Fetched<(Vec<u8>, String)> {
    match row.source {
        Regulator::Sedar => crate::sedar::document(row),
        Regulator::Sec => crate::edgar::content(row),
    }
}

/// A provider's deterministic title and summary for
/// a structured filing it can parse exactly, or None.
/// The title a row carries without anything being fetched: the name of the
/// form it is, or of the document it is. None when only a reading can say.
pub fn quick_title(row: &FiledDocument) -> Option<String> {
    match row.source {
        Regulator::Sec => crate::formnames::title_of(&row.form),
        Regulator::Sedar => crate::sedar::enrichment(row).map(|e| e.subject),
    }
}

pub fn enrichment(row: &FiledDocument) -> Option<Enrichment> {
    match row.source {
        Regulator::Sec => crate::edgar::enrichment(row),
        Regulator::Sedar => crate::sedar::enrichment(row),
    }
}

/// A stored row's category re-derived from its type,
/// so a change to a mapping applies on read. Falls back to the stored one --
/// today it always reads as a string.
pub fn categorize(row: &FiledDocument) -> String {
    let derived = match row.source {
        Regulator::Sedar => crate::sedar::categorize(row),
        Regulator::Sec => crate::edgar::categorize(row),
    };
    if derived.is_empty() { row.category.clone() } else { derived }
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
