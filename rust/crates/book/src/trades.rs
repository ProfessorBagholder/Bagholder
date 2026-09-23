//! Trades, the journal and the groups (`docs/architecture.md` §5, "Trade").
//!
//! A trade is anchored on its opening transaction and keeps its id through the
//! record behind it being derived again, superseded, or corrected. When the
//! anchor is gone the trade is orphaned with the reason, and its journal and
//! group places are kept for the person to re-attach.

use rusqlite::{params, OptionalExtension};

use bagholder_core::journal::{Anchor, Grade, Group, JournalEntry, JournalSubject, Opening, Trade};
use bagholder_core::transaction::Kind;
use bagholder_core::{GroupId, InstrumentId, Leg, RecordId, TradeId, TransactionId};

use crate::text::{self, at as at_text};
use crate::{new_uuid, Book, BookError, Result};

impl Book {
    /// The trade opened by `opening`, made the first time it is asked for. The
    /// opening must be a transaction in the book that moves a position of the
    /// instrument named: its own instrument, or, for an assignment or an
    /// exercise, the underlying its contract delivers. `legacy_key` is the key an
    /// earlier version of the app knew the trade by, where it was imported.
    pub fn open_trade(&self, opening: &Opening, legacy_key: Option<&str>, at: jiff::Timestamp) -> Result<TradeId> {
        self.atomically(|| {
            if let Some(key) = legacy_key {
                if let Some(t) = self.trade_by_legacy_key(key)? {
                    return Ok(t);
                }
            }
            if let Some(t) = self.trade_on(opening)? {
                if let Some(key) = legacy_key {
                    self.conn().execute("UPDATE trades SET legacy_key = ? WHERE id = ? AND legacy_key IS NULL", params![key, t.to_string()])?;
                }
                return Ok(t);
            }
            let tx = &opening.transaction;
            match self.transaction(tx)? {
                None => return Err(BookError::Refused(format!("no transaction {tx} to open a trade on"))),
                // a trade opens on a position moving: a dividend or a deposit opens nothing
                Some(t) if t.instrument.is_none() || t.quantity.is_none_or(|q| q.is_zero()) => {
                    return Err(BookError::Refused(format!("transaction {tx} moves no position, so it opens no trade")));
                }
                Some(t) if t.instrument != Some(opening.instrument) => {
                    let delivers = matches!(t.kind, Kind::OptionAssignment | Kind::OptionExercise)
                        && t.instrument.map(|i| self.option_terms(i)).transpose()?.flatten().is_some_and(|terms| terms.underlying == opening.instrument);
                    if !delivers {
                        return Err(BookError::Refused(format!("transaction {tx} moves no position of instrument {}", opening.instrument)));
                    }
                }
                Some(_) => {}
            }
            let id = TradeId::from_uuid(new_uuid(at));
            self.conn().execute(
                "INSERT INTO trades(id, anchor_record, anchor_leg, anchor_instrument, legacy_key, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                params![id.to_string(), tx.record.to_string(), tx.leg.as_str(), opening.instrument.to_string(), legacy_key, at_text(at)],
            )?;
            Ok(id)
        })
    }

    /// A trade an earlier version of the app knew by `legacy_key` and whose
    /// opening cannot be found: kept orphaned, with the reason, so what the person
    /// wrote on it is not lost. The same key again is the same trade.
    pub fn orphan_trade(&self, legacy_key: &str, reason: &str, at: jiff::Timestamp) -> Result<TradeId> {
        self.atomically(|| {
            if let Some(t) = self.trade_by_legacy_key(legacy_key)? {
                return Ok(t);
            }
            let id = TradeId::from_uuid(new_uuid(at));
            self.conn().execute(
                "INSERT INTO trades(id, orphaned_reason, legacy_key, created_at) VALUES (?, ?, ?, ?)",
                params![id.to_string(), reason, legacy_key, at_text(at)],
            )?;
            Ok(id)
        })
    }

    pub(crate) fn orphan(&self, trade: TradeId, reason: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL, orphaned_reason = ? WHERE id = ?",
            params![reason, trade.to_string()],
        )?;
        Ok(())
    }

    /// Orphan a trade whose round trip a correction joined into another's: the
    /// engine names it (`bagholder_engine::identity::Identity::joined`), and its
    /// journal is kept for the person to re-attach.
    pub fn orphan_joined(&self, trade: TradeId, keeps: TradeId) -> Result<()> {
        self.atomically(|| {
            drop(self.trade(trade)?);
            self.orphan(trade, &format!("its round trip joined trade {keeps}"))
        })
    }

    /// The anchor moved to the counterpart transaction; the instrument it opened
    /// is the same.
    pub(crate) fn move_anchor(&self, trade: TradeId, to: &TransactionId) -> Result<()> {
        self.conn().execute(
            "UPDATE trades SET anchor_record = ?, anchor_leg = ?, orphaned_reason = NULL WHERE id = ?",
            params![to.record.to_string(), to.leg.as_str(), trade.to_string()],
        )?;
        Ok(())
    }

    /// The trade anchored on `opening`, if any.
    pub fn trade_on(&self, opening: &Opening) -> Result<Option<TradeId>> {
        let found: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM trades WHERE anchor_record = ? AND anchor_leg = ? AND anchor_instrument = ?",
                params![opening.transaction.record.to_string(), opening.transaction.leg.as_str(), opening.instrument.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        found.map(|s| text::parsed("trades", "id", &s, TradeId::parse)).transpose()
    }

    pub fn trade_by_legacy_key(&self, key: &str) -> Result<Option<TradeId>> {
        let found: Option<String> = self.conn().query_row("SELECT id FROM trades WHERE legacy_key = ?", [key], |r| r.get(0)).optional()?;
        found.map(|s| text::parsed("trades", "id", &s, TradeId::parse)).transpose()
    }

    pub fn trade(&self, id: TradeId) -> Result<Trade> {
        self.trades_where("WHERE id = ?", [id.to_string()])?.pop().ok_or_else(|| BookError::Refused(format!("no trade {id}")))
    }

    pub fn trades(&self) -> Result<Vec<Trade>> {
        self.trades_where("", [])
    }

    fn trades_where(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Trade>> {
        let sql = format!("SELECT id, anchor_record, anchor_leg, anchor_instrument, orphaned_reason, legacy_key FROM trades {filter} ORDER BY created_at, rowid");
        let mut stmt = self.conn().prepare_cached(&sql)?;
        type Row = (String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);
        let rows = stmt.query_map(args, |r| -> rusqlite::Result<Row> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)) })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, record, leg, instrument, orphaned, legacy_key) = row?;
            let anchor = match (record, leg, instrument, orphaned) {
                (Some(r), Some(l), Some(i), None) => Anchor::Opening(Opening {
                    transaction: TransactionId::new(text::parsed("trades", "anchor_record", &r, RecordId::parse)?, text::parsed("trades", "anchor_leg", &l, Leg::parse)?),
                    instrument: text::parsed("trades", "anchor_instrument", &i, InstrumentId::parse)?,
                }),
                (None, None, None, Some(why)) => Anchor::Orphaned(why),
                (r, l, i, o) => return Err(text::corrupt("trades", "anchor_record", &format!("{r:?} {l:?} {i:?} {o:?}"), "neither anchored on an instrument nor orphaned")),
            };
            out.push(Trade { id: text::parsed("trades", "id", &id, TradeId::parse)?, anchor, legacy_key });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // the journal
    // ------------------------------------------------------------------

    fn subject_columns(subject: JournalSubject) -> (Option<String>, Option<String>) {
        match subject {
            JournalSubject::Trade(t) => (Some(t.to_string()), None),
            JournalSubject::Group(g) => (None, Some(g.to_string())),
        }
    }

    fn journal_row(&self, subject: JournalSubject) -> Result<Option<(i64, String, Option<String>)>> {
        let (trade, group) = Book::subject_columns(subject);
        Ok(self
            .conn()
            .query_row(
                "SELECT id, thesis, grade FROM journal WHERE trade_id IS ?1 AND group_id IS ?2",
                params![trade, group],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    }

    /// Write what the person wrote on a trade or a group; an empty entry removes it.
    pub fn set_journal(&self, subject: JournalSubject, entry: &JournalEntry, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            match subject {
                JournalSubject::Trade(t) => drop(self.trade(t)?),
                JournalSubject::Group(g) => {
                    if !self.groups()?.iter().any(|x| x.id == g) {
                        return Err(BookError::Refused(format!("no group {g}")));
                    }
                }
            }
            if let Some((id, _, _)) = self.journal_row(subject)? {
                self.conn().execute("DELETE FROM journal_tags WHERE journal_id = ?", [id])?;
                self.conn().execute("DELETE FROM journal WHERE id = ?", [id])?;
            }
            if entry.is_empty() {
                return Ok(());
            }
            let (trade, group) = Book::subject_columns(subject);
            self.conn().execute(
                "INSERT INTO journal(trade_id, group_id, thesis, grade, updated_at) VALUES (?, ?, ?, ?, ?)",
                params![trade, group, entry.thesis, entry.grade.map(|g| g.as_str()), at_text(at)],
            )?;
            let id = self.conn().last_insert_rowid();
            for (i, tag) in entry.tags.iter().enumerate() {
                self.conn().execute("INSERT INTO journal_tags(journal_id, position, tag) VALUES (?, ?, ?)", params![id, i as i64, tag])?;
            }
            Ok(())
        })
    }

    pub fn journal(&self, subject: JournalSubject) -> Result<Option<JournalEntry>> {
        let Some((id, thesis, grade)) = self.journal_row(subject)? else { return Ok(None) };
        let mut stmt = self.conn().prepare_cached("SELECT tag FROM journal_tags WHERE journal_id = ? ORDER BY position")?;
        let tags = stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(Some(JournalEntry { thesis, grade: text::opt_parsed("journal", "grade", grade, Grade::parse)?, tags }))
    }

    /// Every journal entry, with what it is written on.
    pub fn journal_entries(&self) -> Result<Vec<(JournalSubject, JournalEntry)>> {
        let mut stmt = self.conn().prepare_cached("SELECT trade_id, group_id FROM journal ORDER BY updated_at, rowid")?;
        let rows: Vec<(Option<String>, Option<String>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (trade, group) in rows {
            let subject = match (trade, group) {
                (Some(t), None) => JournalSubject::Trade(text::parsed("journal", "trade_id", &t, TradeId::parse)?),
                (None, Some(g)) => JournalSubject::Group(text::parsed("journal", "group_id", &g, GroupId::parse)?),
                (t, g) => return Err(text::corrupt("journal", "trade_id", &format!("{t:?} {g:?}"), "on both a trade and a group, or on neither")),
            };
            if let Some(entry) = self.journal(subject)? {
                out.push((subject, entry));
            }
        }
        Ok(out)
    }

    /// Journal entries on orphaned trades, for the person to re-attach, with why.
    pub fn orphaned_journal(&self) -> Result<Vec<(Trade, JournalEntry)>> {
        let mut out = Vec::new();
        for (subject, entry) in self.journal_entries()? {
            if let JournalSubject::Trade(id) = subject {
                let trade = self.trade(id)?;
                if matches!(trade.anchor, Anchor::Orphaned(_)) {
                    out.push((trade, entry));
                }
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // groups
    // ------------------------------------------------------------------

    /// A group of trades the person made. `legacy_key` is its id in an earlier
    /// version of the app; the same key again is the same group.
    pub fn add_group(&self, members: &[TradeId], locked: bool, legacy_key: Option<&str>, at: jiff::Timestamp) -> Result<GroupId> {
        self.atomically(|| {
            if let Some(key) = legacy_key {
                let found: Option<String> = self.conn().query_row("SELECT id FROM trade_groups WHERE legacy_key = ?", [key], |r| r.get(0)).optional()?;
                if let Some(id) = found {
                    return text::parsed("trade_groups", "id", &id, GroupId::parse);
                }
            }
            let id = GroupId::from_uuid(new_uuid(at));
            self.conn().execute(
                "INSERT INTO trade_groups(id, locked, legacy_key, created_at) VALUES (?, ?, ?, ?)",
                params![id.to_string(), locked as i64, legacy_key, at_text(at)],
            )?;
            for (i, t) in members.iter().enumerate() {
                self.conn().execute(
                    "INSERT INTO trade_group_members(group_id, position, trade_id) VALUES (?, ?, ?)",
                    params![id.to_string(), i as i64, t.to_string()],
                )?;
            }
            Ok(id)
        })
    }

    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut stmt = self.conn().prepare_cached("SELECT id, locked FROM trade_groups ORDER BY created_at, rowid")?;
        let rows: Vec<(String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (id, locked) in rows {
            let gid = text::parsed("trade_groups", "id", &id, GroupId::parse)?;
            let mut m = self.conn().prepare_cached("SELECT trade_id FROM trade_group_members WHERE group_id = ? ORDER BY position")?;
            let members = m
                .query_map([&id], |r| r.get::<_, String>(0))?
                .map(|s| text::parsed("trade_group_members", "trade_id", &s?, TradeId::parse))
                .collect::<Result<_>>()?;
            out.push(Group { id: gid, locked: locked == 1, members });
        }
        Ok(out)
    }
}
