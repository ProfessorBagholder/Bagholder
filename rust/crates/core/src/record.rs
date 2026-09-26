//! Source records: what a source reported, kept as received (`docs/architecture.md` §6).

use crate::ids::{ConnectionId, RecordId};
use crate::names::SourceName;

text_enum! {
    RecordState "record state" {
        /// Counted: its transactions are in the book.
        Live = "live",
        /// Replaced by another record through a link; its transactions have left
        /// the book, its revisions are kept.
        Superseded = "superseded",
        /// Its source reported it removed.
        Removed = "removed",
    }
}

/// A record, without its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRecord {
    pub id: RecordId,
    /// The connection it came through; empty for what the person entered.
    pub connection: Option<ConnectionId>,
    pub source: SourceName,
    /// The source's own id for it.
    pub source_key: String,
    pub state: RecordState,
    /// The latest revision's number, from 1.
    pub revision: u32,
}

/// Something wrong with a record that the person should see: a fact it does not
/// state, a shape the mapping does not know, a conflict between references.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Problem {
    /// A short fixed word for the kind of problem (`quantity-not-stated`), for
    /// counting and for tests.
    pub code: String,
    /// What is wrong, in words the person can read.
    pub detail: String,
}

impl Problem {
    pub fn new(code: &str, detail: impl Into<String>) -> Problem {
        Problem { code: code.to_string(), detail: detail.into() }
    }
}
