//! Links between records (`docs/architecture.md` §6, "Reconciliation"): one set
//! of records superseding another, many to many.
//!
//! When records are superseded their transactions leave the book (their
//! revisions stay) and every trade anchored on one of them moves to the
//! superseding records' transaction for the same account, instrument and
//! direction (and, for an option, opening or closing): the earliest by instant, then by record id, then by leg. A chain
//! (A by B, then B by C) ends with every anchor on C. A trade with no
//! counterpart is orphaned with the reason, its journal kept.

use std::collections::BTreeMap;

use rusqlite::params;

use bagholder_core::record::RecordState;
use bagholder_core::transaction::Transaction;
use bagholder_core::{LinkId, RecordId, TransactionId};

use bagholder_core::journal::Opening;

use crate::records::same_opening;
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
            let removed = self.hand_over(from, to)?;
            for r in from {
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
            // a record gone from the count may have stated a succession
            self.settle_successions()?;
            Ok((link, removed))
        })
    }

    /// Hand what stands on the `from` records' transactions to the `to` records':
    /// every trade anchored on one moves to the earliest of theirs with the same
    /// opening, else is orphaned, and each adjustment likewise; the `from`
    /// records' transactions leave the book. The transactions that left.
    fn hand_over(&self, from: &[RecordId], to: &[RecordId]) -> Result<Vec<TransactionId>> {
        let mut removed = Vec::new();
        let mut targets: Vec<Transaction> = Vec::new();
        for r in to {
            targets.extend(self.transactions_of(*r)?);
        }
        // the earliest first: by instant (an unknown instant last), record, leg
        targets.sort_by(|a, b| {
            (a.occurred_at.is_none(), a.occurred_at, a.id.record, &a.id.leg).cmp(&(b.occurred_at.is_none(), b.occurred_at, b.id.record, &b.id.leg))
        });
        let mut explained: BTreeMap<TransactionId, TransactionId> = BTreeMap::new();
        for r in from {
            let theirs = self.transactions_of(*r)?;
            for (trade, anchor) in self.trades_anchored_on(*r)? {
                let Some(opening) = theirs.iter().find(|t| t.id == anchor.transaction) else {
                    self.orphan(trade, "the record it opened on had no such transaction")?;
                    continue;
                };
                let mut moved = false;
                for t in targets.iter().filter(|t| same_opening(opening, t)) {
                    if self.trade_on(&Opening { transaction: t.id.clone(), instrument: anchor.instrument })?.is_none() {
                        self.move_anchor(trade, &t.id, anchor.instrument)?;
                        moved = true;
                        break;
                    }
                }
                if !moved {
                    self.orphan(trade, "the record it opened on was replaced by records with no opening like it")?;
                }
            }
            // an adjustment explaining one of its transactions follows it the
            // same way, never onto a transaction another one moved to explains
            for (adjustment, applies) in self.adjustments_applying_to(*r)? {
                if from.contains(&adjustment.record) {
                    // it goes with its own record
                    continue;
                }
                let free = |t: &&Transaction| explained.get(&t.id).is_none_or(|o| *o == applies);
                let target = theirs.iter().find(|t| t.id == applies).and_then(|old| targets.iter().filter(free).find(|t| same_opening(old, t)));
                match target {
                    Some(t) => {
                        explained.insert(t.id.clone(), applies.clone());
                        self.move_adjustment(&adjustment, &t.id)?;
                    }
                    None => self.adjustment_target_gone(&adjustment, &applies, "was replaced by records with no transaction like it")?,
                }
            }
            removed.extend(theirs.into_iter().map(|t| t.id));
            self.clear_derived(*r)?;
        }
        Ok(removed)
    }

    /// Take a record's place back from it: the records it superseded are live again,
    /// derived by `mapping` from their own latest payloads, what stands on its
    /// transactions goes back to theirs, the link it was made by is gone, and it is
    /// removed. One that superseded none is removed as its source's removal removes
    /// it (`mark_removed`). Only a record that took the place of others alone, in a
    /// link of its own, and only of `mapping`'s source, is taken back.
    pub fn withdraw(&self, record: RecordId, mapping: &dyn crate::mapping::Mapping, at: jiff::Timestamp) -> Result<crate::records::Changes> {
        self.atomically(|| {
            if self.record(record)?.state != RecordState::Live {
                return Ok(crate::records::Changes::default());
            }
            let replaced = self.replaced(record)?;
            if replaced.is_empty() {
                return self.mark_removed(record, at);
            }
            let links: Vec<String> = {
                let mut stmt = self.conn().prepare_cached("SELECT link_id FROM link_records WHERE record_id = ? AND side = 'to'")?;
                let rows = stmt.query_map([record.to_string()], |r| r.get(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for link in &links {
                let others: i64 = self.conn().query_row("SELECT count(*) FROM link_records WHERE link_id = ? AND side = 'to' AND record_id <> ?", params![link, record.to_string()], |r| r.get(0))?;
                if others > 0 {
                    return Err(BookError::Refused(format!("record {record} took the place of others together with other records, so it is not taken back alone")));
                }
            }
            for r in &replaced {
                let f = self.record(*r)?;
                if f.state != RecordState::Superseded || f.source != mapping.source() {
                    return Err(BookError::Refused(format!("record {r}, which record {record} took the place of, is {} of {}, not one {} brings back", f.state, f.source.as_str(), mapping.source().as_str())));
                }
            }
            for link in &links {
                self.conn().execute("DELETE FROM link_records WHERE link_id = ?", [link])?;
                self.conn().execute("DELETE FROM links WHERE id = ?", [link])?;
            }
            let mut changes = crate::records::Changes::default();
            for r in &replaced {
                let connection = self.record(*r)?.connection;
                self.set_state(*r, RecordState::Live, at)?;
                let (payload, _) = self.latest_revision(*r)?;
                changes.extend(self.derive(*r, connection, &payload, mapping, None, at)?);
            }
            changes.removed.extend(self.hand_over(&[record], &replaced)?);
            self.set_state(record, RecordState::Removed, at)?;
            self.settle_successions()?;
            Ok(changes)
        })
    }

    /// The records `record` superseded, if it superseded any.
    pub fn replaced(&self, record: RecordId) -> Result<Vec<RecordId>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT f.record_id FROM link_records t JOIN link_records f ON f.link_id = t.link_id AND f.side = 'from'
             WHERE t.record_id = ? AND t.side = 'to' ORDER BY f.record_id",
        )?;
        let ids = stmt.query_map([record.to_string()], |r| r.get::<_, String>(0))?;
        ids.map(|s| text::parsed("link_records", "record_id", &s?, RecordId::parse)).collect()
    }

    /// The transactions standing now for the record `source` knew by `key`: its
    /// own while it is live, else those of the live records that replaced it,
    /// each replacement followed to its end. None where no such record is kept,
    /// or it and what replaced it were removed.
    pub fn standing_for(&self, source: &bagholder_core::SourceName, key: &str) -> Result<Vec<Transaction>> {
        let mut stmt = self.conn().prepare_cached("SELECT id FROM source_records WHERE source = ? AND source_key = ? ORDER BY id")?;
        let found = stmt.query_map(params![source.as_str(), key], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        let mut todo: Vec<RecordId> = found.iter().map(|s| text::parsed("source_records", "id", s, RecordId::parse)).collect::<Result<_>>()?;
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        while let Some(r) = todo.pop() {
            if !seen.insert(r) {
                continue;
            }
            match self.record(r)?.state {
                RecordState::Live => out.extend(self.transactions_of(r)?),
                RecordState::Superseded => todo.extend(self.superseded_by(r)?),
                _ => {}
            }
        }
        Ok(out)
    }

    /// Whether `record` has superseded a record of `source`: it has taken the
    /// place of one already, and takes no other's.
    pub fn supersedes_from(&self, record: RecordId, source: &bagholder_core::SourceName) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM link_records t JOIN link_records f ON f.link_id = t.link_id AND f.side = 'from'
                JOIN source_records r ON r.id = f.record_id WHERE t.record_id = ?1 AND t.side = 'to' AND r.source = ?2)",
            params![record.to_string(), source.as_str()],
            |r| r.get(0),
        )?)
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
