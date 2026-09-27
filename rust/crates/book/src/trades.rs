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

/// What `Book::reattach` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reattached {
    Attached,
    /// Left orphaned, with this reason.
    Kept(String),
}

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
            self.check_opening(opening)?;
            let id = TradeId::from_uuid(new_uuid(at));
            self.conn().execute(
                "INSERT INTO trades(id, anchor_record, anchor_leg, anchor_instrument, legacy_key, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                params![id.to_string(), opening.transaction.record.to_string(), opening.transaction.leg.as_str(), self.own_anchor(opening)?.to_string(), legacy_key, at_text(at)],
            )?;
            Ok(id)
        })
    }

    /// Refused unless `opening` is a transaction in the book that moves a position
    /// of the instrument named: its own instrument, or, for an assignment or an
    /// exercise, the underlying its contract delivers.
    fn check_opening(&self, opening: &Opening) -> Result<()> {
        let tx = &opening.transaction;
        let Some(t) = self.transaction(tx)? else {
            return Err(BookError::Refused(format!("no transaction {tx} to open a trade on")));
        };
        let opening = &Opening { transaction: tx.clone(), instrument: self.canonical(opening.instrument)? };
        // a corporate event opens what an adjustment on it says it moved: a
        // spin-off's child, a stock dividend on a marker stating no units
        let adjusted = t.kind == Kind::CorporateEvent && self.adjustment_moves(tx, opening.instrument)?;
        // a trade opens on a position moving: a dividend or a deposit of cash opens nothing
        if !adjusted && (t.instrument.is_none() || t.quantity.is_none_or(|q| q.is_zero())) {
            return Err(BookError::Refused(format!("transaction {tx} moves no position, so it opens no trade")));
        }
        if !adjusted && t.instrument != Some(opening.instrument) {
            let delivers = matches!(t.kind, Kind::OptionAssignment | Kind::OptionExercise)
                && t.instrument.map(|i| self.option_terms(i)).transpose()?.flatten().is_some_and(|terms| terms.underlying == opening.instrument);
            if !delivers {
                return Err(BookError::Refused(format!("transaction {tx} moves no position of instrument {}", opening.instrument)));
            }
        }
        Ok(())
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

    /// Whether `trade` is anchored as the engine saw it when it named the trade:
    /// the engine reads the book, and a write in between (a pull moving anchors to
    /// the broker's rows) makes what it named stale, so nothing is done on it and
    /// the next settling decides again.
    fn anchored_as(&self, trade: TradeId, seen: &Opening) -> Result<bool> {
        Ok(matches!(self.trade(trade)?.anchor, Anchor::Opening(ref o) if o == seen))
    }

    /// Orphan a trade whose round trip a correction joined into another's: the
    /// engine names it (`bagholder_engine::identity::Identity::joined`), and its
    /// journal is kept for the person to re-attach. Done only while the trade is
    /// anchored on `seen`, the opening the engine saw it on; whether it was done.
    pub fn orphan_joined(&self, trade: TradeId, keeps: TradeId, seen: &Opening) -> Result<bool> {
        self.atomically(|| {
            if !self.anchored_as(trade, seen)? {
                return Ok(false);
            }
            self.orphan(trade, &format!("its round trip joined trade {keeps}"))?;
            Ok(true)
        })
    }

    /// Orphan a trade whose anchor opens no round trip any more and is a fill of
    /// none it can move to (the engine names it, `Identity::unclaimed`); its
    /// journal is kept. Done only while the trade is anchored on `seen`; whether
    /// it was done.
    pub fn orphan_unclaimed(&self, trade: TradeId, seen: &Opening) -> Result<bool> {
        self.atomically(|| {
            if !self.anchored_as(trade, seen)? {
                return Ok(false);
            }
            self.orphan(trade, "the transaction it opened on no longer opens a round trip")?;
            Ok(true)
        })
    }

    /// Move a trade to the opening of the round trip it is now a fill of (the
    /// engine names it, `Identity::moved`): its id, journal and group places are
    /// kept (`SPEC.md` §2, Trade). Done only while the trade is anchored on `seen`
    /// and no trade is anchored on `to`; whether it was done. `to` must open a
    /// position of its instrument, as an opening given to `open_trade` must.
    pub fn move_trade(&self, trade: TradeId, seen: &Opening, to: &Opening) -> Result<bool> {
        self.atomically(|| {
            if !self.anchored_as(trade, seen)? || self.trade_on(to)?.is_some() {
                return Ok(false);
            }
            self.anchor_on(trade, to)?;
            Ok(true)
        })
    }

    /// Anchor `trade`, orphaned or not, on `to`, which must open a position of its
    /// instrument; no other trade may be anchored there.
    pub(crate) fn anchor_on(&self, trade: TradeId, to: &Opening) -> Result<()> {
        self.check_opening(to)?;
        if let Some(other) = self.trade_on(to)?.filter(|t| *t != trade) {
            return Err(BookError::Refused(format!("trade {other} is anchored on {} already", to.transaction)));
        }
        self.conn().execute(
            "UPDATE trades SET anchor_record = ?, anchor_leg = ?, anchor_instrument = ?, orphaned_reason = NULL WHERE id = ?",
            params![to.transaction.record.to_string(), to.transaction.leg.as_str(), self.own_anchor(to)?.to_string(), trade.to_string()],
        )?;
        Ok(())
    }

    /// The anchor moved to the counterpart transaction; the instrument it opened
    /// is the same, `instrument` as the counterpart names it.
    pub(crate) fn move_anchor(&self, trade: TradeId, to: &TransactionId, instrument: InstrumentId) -> Result<()> {
        let instrument = self.own_anchor(&Opening { transaction: to.clone(), instrument: self.canonical(instrument)? })?;
        self.conn().execute(
            "UPDATE trades SET anchor_record = ?, anchor_leg = ?, anchor_instrument = ?, orphaned_reason = NULL WHERE id = ?",
            params![to.record.to_string(), to.leg.as_str(), instrument.to_string(), trade.to_string()],
        )?;
        Ok(())
    }

    /// The instrument an opening is kept against: of those its transaction names
    /// itself (its own instrument, the underlying its contract delivers, what an
    /// adjustment on it moves), the one the book reads as the opening's. A
    /// succession joining or parting instruments then leaves the anchor on the
    /// instrument its record names.
    fn own_anchor(&self, opening: &Opening) -> Result<InstrumentId> {
        let wanted = self.canonical(opening.instrument)?;
        for i in self.opening_instruments(&opening.transaction)? {
            if self.canonical(i)? == wanted {
                return Ok(i);
            }
        }
        Ok(opening.instrument)
    }

    /// The instruments a transaction can open a position of, as its record names
    /// them: its own, the underlying its contract delivers, and each an
    /// adjustment on it moves units into.
    pub(crate) fn opening_instruments(&self, tx: &TransactionId) -> Result<Vec<InstrumentId>> {
        let mut out = Vec::new();
        if let Some(i) = self.own_transaction(tx)?.and_then(|t| t.instrument) {
            out.push(i);
            if let Some(terms) = self.own_option_terms(i)? {
                out.push(terms.underlying);
            }
        }
        out.extend(self.adjustment_targets(tx)?);
        Ok(out)
    }

    /// Put an orphaned trade back on the round trip opened by `to`, its id and
    /// journal kept: a repair of what an earlier build orphaned though its round
    /// trip still stood. A trade the book gave that round trip since (with no
    /// note and in no group, so nothing of the person's is on it) gives way to
    /// it; one with a note of its own, or in a group, keeps the round trip, and
    /// the orphan stays orphaned with that reason.
    pub fn reattach(&self, orphan: TradeId, to: &Opening) -> Result<Reattached> {
        self.atomically(|| {
            if !matches!(self.trade(orphan)?.anchor, Anchor::Orphaned(_)) {
                return Err(BookError::Refused(format!("trade {orphan} is not orphaned")));
            }
            if let Some(holder) = self.trade_on(to)? {
                let why = if self.journal(JournalSubject::Trade(holder))?.is_some() {
                    Some(format!("the round trip it was written on has a note of its own, on trade {holder}"))
                } else if self.groups()?.iter().any(|g| g.members.contains(&holder)) {
                    Some(format!("the round trip it was written on is trade {holder}, which is in a group"))
                } else {
                    None
                };
                if let Some(why) = why {
                    self.conn().execute("UPDATE trades SET orphaned_reason = ? WHERE id = ?", params![why, orphan.to_string()])?;
                    return Ok(Reattached::Kept(why));
                }
                self.conn().execute("DELETE FROM trades WHERE id = ?", [holder.to_string()])?;
            }
            self.anchor_on(orphan, to)?;
            Ok(Reattached::Attached)
        })
    }

    /// Whether the repair `name` has been done on this book.
    pub fn repaired(&self, name: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT 1 FROM settings WHERE key = ?", [format!("repair.{name}")], |_| Ok(())).optional()?.is_some())
    }

    /// Record the repair `name` as done, so it is not done again.
    pub fn record_repair(&self, name: &str, at: jiff::Timestamp) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings(key, value, source, set_at) VALUES (?1, 'done', 'bagholder', ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, source = excluded.source, set_at = excluded.set_at",
            params![format!("repair.{name}"), at_text(at)],
        )?;
        Ok(())
    }

    /// The trade anchored on `opening`, if any: on its transaction, and on an
    /// instrument the book reads as the opening's.
    pub fn trade_on(&self, opening: &Opening) -> Result<Option<TradeId>> {
        let wanted = self.canonical(opening.instrument)?;
        let mut stmt = self.conn().prepare_cached("SELECT id, anchor_instrument FROM trades WHERE anchor_record = ? AND anchor_leg = ? ORDER BY created_at, rowid")?;
        let rows = stmt.query_map(params![opening.transaction.record.to_string(), opening.transaction.leg.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, instrument) = row?;
            if self.canonical(text::parsed("trades", "anchor_instrument", &instrument, InstrumentId::parse)?)? == wanted {
                return text::parsed("trades", "id", &id, TradeId::parse).map(Some);
            }
        }
        Ok(None)
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

    /// Trades, each anchored on the instrument the book reads its anchor as.
    fn trades_where(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Trade>> {
        let sql = format!(
            "SELECT id, anchor_record, anchor_leg, COALESCE(j.into_id, anchor_instrument), orphaned_reason, legacy_key
             FROM trades LEFT JOIN instrument_joins j ON j.instrument_id = trades.anchor_instrument {filter} ORDER BY trades.created_at, trades.rowid"
        );
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
