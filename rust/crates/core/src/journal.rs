//! Trades, the journal the person keeps on them, and the groups they make.
//!
//! A trade's id is assigned when its opening transaction is first stored, and
//! the trade is anchored to that transaction. The anchor moves when the record
//! behind it is superseded (a broker's row replacing a provisional fill), and
//! survives the record's transactions being derived again. A trade whose anchor
//! is gone is orphaned, with the reason, and its journal is kept for the person
//! to re-attach: a note is never dropped.

use crate::ids::{GroupId, InstrumentId, TradeId, TransactionId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trade {
    pub id: TradeId,
    pub anchor: Anchor,
    /// The key an earlier version of the app knew it by, where it was imported.
    pub legacy_key: Option<String>,
}

impl Trade {
    /// Why the trade is orphaned, if it is.
    pub fn orphaned_reason(&self) -> Option<String> {
        match &self.anchor {
            Anchor::Orphaned(why) => Some(why.clone()),
            Anchor::Opening(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Anchored on its opening.
    Opening(Opening),
    /// Its opening transaction is gone; why, in words the person can read.
    Orphaned(String),
}

/// What opened a trade: a transaction and the instrument it opened. One
/// transaction can open two (an assignment closes the contract's round trip and
/// opens the underlying's), so the transaction alone is not enough.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Opening {
    pub transaction: TransactionId,
    pub instrument: InstrumentId,
}

text_enum! {
    Grade "grade" {
        A = "A",
        B = "B",
        C = "C",
        F = "F",
    }
}

/// What a journal entry is written on: one trade, or a group of trades the person
/// put together (which the screens show as one trade).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JournalSubject {
    Trade(TradeId),
    Group(GroupId),
}

/// What the person wrote about a trade or a group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JournalEntry {
    pub thesis: String,
    pub grade: Option<Grade>,
    pub tags: Vec<String>,
}

impl JournalEntry {
    /// An entry with nothing in it is no entry.
    pub fn is_empty(&self) -> bool {
        self.thesis.trim().is_empty() && self.grade.is_none() && self.tags.is_empty()
    }
}

/// Trades the person put together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: GroupId,
    pub locked: bool,
    pub members: Vec<TradeId>,
}
