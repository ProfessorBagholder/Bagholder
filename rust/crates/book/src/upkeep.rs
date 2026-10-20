//! Book upkeep (`docs/plans/remove-the-earlier-app.md`, Part 2): work done once
//! on a book an earlier build left holding what this one no longer keeps. It
//! changes no table; it runs only when its condition holds, and once done it is
//! recorded in the book's settings, so it never runs again.
//!
//! The earlier app's rows: the records the first start copied from the earlier
//! (Python) app's database, each superseded since by the broker's own row or
//! removed by its source. Removing them takes nothing out of the count: a record
//! that is not live has no transactions. While one is still live it counts, so
//! nothing is removed until the broker's own row supersedes it.

use std::collections::BTreeSet;

use rusqlite::params;

use bagholder_core::journal::Opening;
use bagholder_core::record::RecordState;
use bagholder_core::{RecordId, TradeId, TransactionId};

use crate::text::{self, at as at_text};
use crate::{Book, BookError, Result};

/// The setting that says the earlier app's rows were removed, and when.
pub const EARLIER_APP_ROWS_REMOVED: &str = "upkeep.earlier-app-rows-removed";

/// The records the earlier app's database was copied into, by their source.
const EARLIER_APP_ROWS: &str = "SELECT id FROM source_records WHERE source = 'bagholder-import'";

/// What `Book::remove_earlier_app_rows` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EarlierAppRows {
    /// Removed before: nothing was done.
    RemovedBefore,
    /// This many still count: nothing was done, and the step runs again at the
    /// next start.
    StillLive(usize),
    /// Removed now.
    Removed {
        records: usize,
        /// Links left with nothing on one side, removed with them.
        links: usize,
        /// Trades still anchored on one, moved to the row that superseded it.
        trades_moved: usize,
    },
}

impl EarlierAppRows {
    /// What was done, in words, for the log; none when nothing was to be done.
    pub fn said(&self) -> Option<String> {
        match self {
            EarlierAppRows::RemovedBefore => None,
            EarlierAppRows::StillLive(n) => Some(format!(
                "{n} of the earlier app's rows still count{}, so none of them was removed: each is removed once the broker's own row replaces it",
                if *n == 1 { "s" } else { "" }
            )),
            EarlierAppRows::Removed { records: 0, .. } => None,
            EarlierAppRows::Removed { records, links, trades_moved } => {
                Some(format!("the earlier app's {records} rows were removed from the book, with {links} links to them; {trades_moved} trades still on one were moved to the row that replaced it"))
            }
        }
    }
}

impl Book {
    /// Remove the earlier app's rows and everything derived from them, once:
    /// their revisions, refs, problems, holds, sightings, adjustments, order and
    /// transfer links, any transaction, the links left with nothing on one side,
    /// and the keys the earlier app knew trades and groups by. Done only when none
    /// of them is live. A trade still anchored on one first moves to the
    /// transaction of the live row that superseded it which opens its instrument;
    /// where there is not exactly one, the step is refused, naming each such
    /// trade, and the book is left as it was.
    pub fn remove_earlier_app_rows(&self, at: jiff::Timestamp) -> Result<EarlierAppRows> {
        self.atomically(|| {
            if self.setting(EARLIER_APP_ROWS_REMOVED)?.is_some() {
                return Ok(EarlierAppRows::RemovedBefore);
            }
            let live: i64 = self.conn().query_row(&format!("SELECT COUNT(*) FROM source_records WHERE state = 'live' AND id IN ({EARLIER_APP_ROWS})"), [], |r| r.get(0))?;
            if live > 0 {
                return Ok(EarlierAppRows::StillLive(live as usize));
            }
            let records: Vec<RecordId> = {
                let mut stmt = self.conn().prepare(&format!("{EARLIER_APP_ROWS} ORDER BY id"))?;
                let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                ids.iter().map(|s| text::parsed("source_records", "id", s, RecordId::parse)).collect::<Result<_>>()?
            };
            let mut unplaced = Vec::new();
            let mut trades_moved = 0;
            for r in &records {
                for (trade, anchor) in self.trades_anchored_on(*r)? {
                    match self.superseding_opening(*r, trade, &anchor)? {
                        Ok(to) => {
                            self.anchor_on(trade, &to)?;
                            trades_moved += 1;
                        }
                        Err(why) => unplaced.push(format!("trade {trade}, on {}: {why}", anchor.transaction)),
                    }
                }
            }
            if !unplaced.is_empty() {
                return Err(BookError::Refused(format!("the earlier app's rows were kept, since a trade on one has no single row to move to: {}", unplaced.join("; "))));
            }
            let c = self.conn();
            let of = format!("IN ({EARLIER_APP_ROWS})");
            for table in ["instrument_sightings", "record_revisions", "record_refs", "record_problems", "record_holds", "statement_holds", "record_orders", "adjustment_legs", "adjustments", "transactions", "link_records"] {
                c.execute(&format!("DELETE FROM {table} WHERE record_id {of}"), [])?;
            }
            c.execute(&format!("DELETE FROM transfer_links WHERE out_record {of} OR in_record {of}"), [])?;
            // a link with nothing left on one side supersedes nothing
            let one_sided = "SELECT id FROM links l WHERE NOT EXISTS (SELECT 1 FROM link_records f WHERE f.link_id = l.id AND f.side = 'from')
                                OR NOT EXISTS (SELECT 1 FROM link_records t WHERE t.link_id = l.id AND t.side = 'to')";
            c.execute(&format!("DELETE FROM link_records WHERE link_id IN ({one_sided})"), [])?;
            let links = c.execute(&format!("DELETE FROM links WHERE id IN ({one_sided})"), [])?;
            let removed = c.execute(&format!("DELETE FROM source_records WHERE id {of}"), [])?;
            c.execute("UPDATE trades SET legacy_key = NULL WHERE legacy_key IS NOT NULL", [])?;
            c.execute("UPDATE trade_groups SET legacy_key = NULL WHERE legacy_key IS NOT NULL", [])?;
            c.execute(
                "INSERT INTO settings(key, value, source, set_at) VALUES (?1, ?2, 'bagholder', ?3)",
                params![EARLIER_APP_ROWS_REMOVED, format!("{removed} rows, {links} links, {trades_moved} trades moved"), at_text(at)],
            )?;
            Ok(EarlierAppRows::Removed { records: removed, links, trades_moved })
        })
    }

    /// The opening a trade anchored on `record`, which is not live, moves to: the
    /// one transaction of the live records that superseded it, followed to their
    /// end, that opens the trade's instrument and carries no other trade. Why not,
    /// where there is none or more than one.
    fn superseding_opening(&self, record: RecordId, trade: TradeId, anchor: &Opening) -> Result<std::result::Result<Opening, String>> {
        let wanted = self.canonical(anchor.instrument)?;
        let mut todo = vec![record];
        let mut seen = BTreeSet::new();
        let mut found: Vec<TransactionId> = Vec::new();
        while let Some(r) = todo.pop() {
            if !seen.insert(r) {
                continue;
            }
            match self.record(r)?.state {
                RecordState::Live => {
                    for t in self.transactions_of(r)? {
                        let opens = self.opening_instruments(&t.id)?.into_iter().map(|i| self.canonical(i)).collect::<Result<Vec<_>>>()?.contains(&wanted);
                        let free = self.trade_on(&Opening { transaction: t.id.clone(), instrument: anchor.instrument })?.is_none_or(|t| t == trade);
                        if opens && free {
                            found.push(t.id);
                        }
                    }
                }
                RecordState::Superseded => todo.extend(self.superseded_by(r)?),
                _ => {}
            }
        }
        Ok(match found.as_slice() {
            [one] => Ok(Opening { transaction: one.clone(), instrument: anchor.instrument }),
            [] => Err("no row that replaced it opens its instrument".into()),
            many => Err(format!("{} rows that replaced it open its instrument", many.len())),
        })
    }
}
