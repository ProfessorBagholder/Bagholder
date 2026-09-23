//! Links between records (`docs/architecture.md` §6, "Reconciliation"): one set
//! of records superseding another, many to many.
//!
//! When records are superseded their transactions leave the book (their
//! revisions stay) and every trade anchored on one of them moves to the
//! superseding records' transaction for the same account, instrument and
//! direction (and, for an option, opening or closing): the earliest by instant, then by record id, then by leg. A chain
//! (A by B, then B by C) ends with every anchor on C. A trade with no
//! counterpart is orphaned with the reason, its journal kept.

use rusqlite::params;

use bagholder_core::record::RecordState;
use bagholder_core::transaction::Transaction;
use bagholder_core::{LinkId, RecordId, TransactionId};

use crate::records::opening_key;
use crate::text::{self, at as at_text};
use crate::{new_uuid, Book, BookError, Result};

impl Book {
    /// Supersede the `from` records by the `to` records. Both sets must be live and
    /// must not share a record.
    pub fn supersede(&self, from: &[RecordId], to: &[RecordId], reason: &str, at: jiff::Timestamp) -> Result<LinkId> {
        Ok(self.supersede_counting(from, to, reason, at)?.0)
    }

    /// As `supersede`, with the transactions that left the book.
    pub(crate) fn supersede_counting(&self, from: &[RecordId], to: &[RecordId], reason: &str, at: jiff::Timestamp) -> Result<(LinkId, Vec<TransactionId>)> {
        self.atomically(|| {
            let mut removed = Vec::new();
            if from.is_empty() || to.is_empty() {
                return Err(BookError::Refused("a link needs records on both sides".into()));
            }
            for r in from.iter().chain(to) {
                if self.record(*r)?.state != RecordState::Live {
                    return Err(BookError::Refused(format!("record {r} is not live, so it cannot be linked")));
                }
            }
            if let Some(both) = from.iter().find(|r| to.contains(r)) {
                return Err(BookError::Refused(format!("record {both} is on both sides of the link")));
            }
            let mut targets: Vec<Transaction> = Vec::new();
            for r in to {
                targets.extend(self.transactions_of(*r)?);
            }
            // the earliest first: by instant (an unknown instant last), record, leg
            targets.sort_by(|a, b| {
                (a.occurred_at.is_none(), a.occurred_at, a.id.record, &a.id.leg).cmp(&(b.occurred_at.is_none(), b.occurred_at, b.id.record, &b.id.leg))
            });
            for r in from {
                let theirs = self.transactions_of(*r)?;
                for (trade, anchor) in self.trades_anchored_on(*r)? {
                    let Some(opening) = theirs.iter().find(|t| t.id == anchor) else {
                        self.orphan(trade, "the record it opened on had no such transaction")?;
                        continue;
                    };
                    let key = opening_key(opening);
                    let mut moved = false;
                    for t in targets.iter().filter(|t| opening_key(t) == key) {
                        if self.trade_on(&t.id)?.is_none() {
                            self.move_anchor(trade, &t.id)?;
                            moved = true;
                            break;
                        }
                    }
                    if !moved {
                        self.orphan(trade, "the record it opened on was replaced by records with no opening like it")?;
                    }
                }
                removed.extend(theirs.into_iter().map(|t| t.id));
                self.clear_derived(*r)?;
                self.set_state(*r, RecordState::Superseded, at)?;
            }
            let link = LinkId::from_uuid(new_uuid(at));
            self.conn().execute(
                "INSERT INTO links(id, kind, reason, created_at) VALUES (?, 'supersedes', ?, ?)",
                params![link.to_string(), reason, at_text(at)],
            )?;
            for (side, records) in [("from", from), ("to", to)] {
                for r in records {
                    self.conn().execute(
                        "INSERT INTO link_records(link_id, record_id, side) VALUES (?, ?, ?)",
                        params![link.to_string(), r.to_string(), side],
                    )?;
                }
            }
            Ok((link, removed))
        })
    }

    /// Whether `record` has superseded others.
    pub fn has_superseded(&self, record: RecordId) -> Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM link_records WHERE record_id = ? AND side = 'to'",
            [record.to_string()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// The records that superseded `record`, if it was superseded.
    pub fn superseded_by(&self, record: RecordId) -> Result<Vec<RecordId>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT t.record_id FROM link_records f JOIN link_records t ON t.link_id = f.link_id AND t.side = 'to'
             WHERE f.record_id = ? AND f.side = 'from' ORDER BY t.record_id",
        )?;
        let ids = stmt.query_map([record.to_string()], |r| r.get::<_, String>(0))?;
        ids.map(|s| text::parsed("link_records", "record_id", &s?, RecordId::parse)).collect()
    }
}
