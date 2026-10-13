//! The record's changes as the book logs them (migration 028,
//! `docs/plans/stage-3-the-record-by-change.md`, A): every write to a table the
//! engine's ledger or its adjustments are read from appends, in the writer's own
//! transaction, the keys whose read it changed. A reader applies what is past the
//! last sequence it applied and reads each named thing by its key.

use std::collections::BTreeSet;

use bagholder_core::{AccountId, GroupId, InstrumentId, RecordId, TradeId, TransactionId};
use rusqlite::{params, OptionalExtension};

use crate::{text, Book, Result};

/// What changed in the record past a sequence, by key, and the sequence it was
/// read to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The last sequence read: the reader has applied everything up to it once it
    /// applies these.
    pub upto: i64,
    pub records: BTreeSet<RecordId>,
    pub instruments: BTreeSet<InstrumentId>,
    pub accounts: BTreeSet<AccountId>,
    /// A trade's own row or its journal entry.
    pub trades: BTreeSet<TradeId>,
    /// A group's own row, its members or its journal entry.
    pub groups: BTreeSet<GroupId>,
    /// The transactions whose adjustment changed.
    pub adjustments: BTreeSet<TransactionId>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.instruments.is_empty() && self.accounts.is_empty() && self.trades.is_empty() && self.groups.is_empty() && self.adjustments.is_empty()
    }
}

impl Book {
    /// The last sequence the log has handed out: a reader built from the book as
    /// it stands now has applied everything up to it. Read in the same snapshot as
    /// that build (`Book::snapshot`).
    pub fn changes_upto(&self) -> Result<i64> {
        // the log's own counter, which AUTOINCREMENT keeps past every row ever
        // written, pruned or not
        let n: Option<i64> = self.conn().query_row("SELECT seq FROM sqlite_sequence WHERE name = 'ledger_changes'", [], |r| r.get(0)).optional()?;
        Ok(n.unwrap_or(0))
    }

    /// Everything logged past `after`, by key.
    pub fn changes_since(&self, after: i64) -> Result<Changes> {
        let mut out = Changes { upto: after, ..Changes::default() };
        let mut st = self.conn().prepare_cached("SELECT seq, kind, key FROM ledger_changes WHERE seq > ?1 ORDER BY seq")?;
        let mut rows = st.query(params![after])?;
        while let Some(r) = rows.next()? {
            let (seq, kind, key): (i64, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
            out.upto = out.upto.max(seq);
            let t = "ledger_changes";
            match kind.as_str() {
                "record" => {
                    out.records.insert(text::parsed(t, "key", &key, RecordId::parse)?);
                }
                "instrument" => {
                    out.instruments.insert(text::parsed(t, "key", &key, InstrumentId::parse)?);
                }
                "account" => {
                    out.accounts.insert(text::parsed(t, "key", &key, AccountId::parse)?);
                }
                "trade" => {
                    out.trades.insert(text::parsed(t, "key", &key, TradeId::parse)?);
                }
                "group" => {
                    out.groups.insert(text::parsed(t, "key", &key, GroupId::parse)?);
                }
                "adjustment" => {
                    out.adjustments.insert(text::parsed(t, "key", &key, TransactionId::parse)?);
                }
                other => return Err(crate::BookError::Refused(format!("ledger_changes holds a kind {other:?} no reader knows"))),
            }
        }
        // a row kept under an instrument a succession joins into another is read
        // under that one (its names, refs and terms through the join): both move
        let members: Vec<InstrumentId> = out.instruments.iter().copied().collect();
        for i in members {
            out.instruments.insert(self.canonical(i)?);
        }
        // the counter, not the last row: a sequence handed out to a row since
        // pruned is still past
        out.upto = out.upto.max(self.changes_upto()?);
        Ok(out)
    }

    /// The log's rows up to `upto` dropped: the reader has applied them.
    pub fn forget_changes(&self, upto: i64) -> Result<()> {
        self.conn().execute("DELETE FROM ledger_changes WHERE seq <= ?1", params![upto])?;
        Ok(())
    }
}
