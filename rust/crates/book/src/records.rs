//! Source records and the transactions derived from them (`docs/architecture.md` §6).
//!
//! A record is what a source reported, kept as received (in canonical JSON) with
//! every revision. Receiving the same payload again writes nothing; a different
//! one appends a revision and derives the record's transactions again. A record
//! leaves the count only when its source reports it removed (`mark_removed`) or
//! another record supersedes it (`links.rs`); one missing from a pull is left alone.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};

use bagholder_core::journal::Opening;
use bagholder_core::record::{Problem, RecordState, SourceRecord};
use bagholder_core::transaction::{Effect, Kind, Transaction};
use bagholder_core::{AccountId, ConnectionId, Dec, InstrumentId, Leg, MappingVersion, Money, RecordId, SourceName, TradeId, TransactionId};

use crate::mapping::{Draft, MapContext, Mapping};
use crate::text::{self, at as at_text, day};
use crate::{canon, new_uuid, Book, BookError, Result};

/// A record as a source hands it to the book.
pub struct Incoming<'a> {
    pub connection: Option<ConnectionId>,
    /// The source's own id for it.
    pub source_key: &'a str,
    /// The record as the source sent it, as JSON.
    pub payload: &'a str,
    /// Other ids it is known by (`scheme`, `value`), so a later source can find it.
    pub refs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    New,
    /// A different payload: the new revision's number.
    Revised(u32),
    /// The same payload as the latest revision: nothing was written.
    Unchanged,
}

/// Which transactions a write added, changed and removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: Vec<TransactionId>,
    pub changed: Vec<TransactionId>,
    pub removed: Vec<TransactionId>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    fn extend(&mut self, other: Changes) {
        self.added.extend(other.added);
        self.changed.extend(other.changed);
        self.removed.extend(other.removed);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub record: RecordId,
    pub outcome: Outcome,
    pub changes: Changes,
}

/// The way a transaction moves a position: in (1), out (-1), or neither (0).
pub(crate) fn direction(t: &Transaction) -> i8 {
    match t.quantity {
        Some(q) if q.is_positive() => 1,
        Some(q) if q.is_negative() => -1,
        Some(_) => 0,
        None => match t.kind {
            Kind::Buy => 1,
            Kind::Sell => -1,
            _ => 0,
        },
    }
}

/// What makes a transaction the same opening as another: its account, its
/// instrument, the way it moves the position (and its kind: an assignment's
/// delivery is not a purchase).
pub(crate) fn opening_key(t: &Transaction) -> (AccountId, Option<InstrumentId>, i8, Kind) {
    (t.account, t.instrument, direction(t), t.kind)
}

/// Whether `b` is the same opening as `a`: the same key, and, for an option,
/// not the other effect where both state one (a buy to open and a buy to close
/// both move it in). An effect one of them does not state does not tell them
/// apart.
pub(crate) fn same_opening(a: &Transaction, b: &Transaction) -> bool {
    opening_key(a) == opening_key(b)
        && match (a.effect, b.effect) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}

/// Whether two transactions say the same thing (their mapping version aside).
fn same_content(a: &Transaction, b: &Transaction) -> bool {
    Transaction { mapping: b.mapping.clone(), ..a.clone() } == *b
}

impl Book {
    /// Store a record: new, a new revision, or nothing when the payload is the
    /// same. A new or revised live record's transactions are derived by `mapping`.
    pub fn store(&self, mapping: &dyn Mapping, incoming: &Incoming, at: jiff::Timestamp) -> Result<Stored> {
        self.atomically(|| self.store_inner(mapping, incoming, at))
    }

    /// Store a record that replaces others, in one transaction, so the new record
    /// and the ones it replaces are never counted together (`supersede`). The same
    /// delivery again changes nothing: a record this one already replaced is left
    /// as it is. One replaced by another record is refused.
    pub fn store_superseding(&self, mapping: &dyn Mapping, incoming: &Incoming, replaces: &[RecordId], reason: &str, at: jiff::Timestamp) -> Result<Stored> {
        self.atomically(|| {
            let mut stored = self.store_inner(mapping, incoming, at)?;
            let mut pending = Vec::new();
            for r in replaces {
                match self.record(*r)?.state {
                    RecordState::Live => pending.push(*r),
                    RecordState::Superseded if self.superseded_by(*r)?.contains(&stored.record) => {}
                    state => return Err(BookError::Refused(format!("record {r} is {state}, not replaceable by record {}", stored.record))),
                }
            }
            if !pending.is_empty() {
                let (_, removed) = self.supersede_counting(&pending, &[stored.record], reason, at)?;
                stored.changes.removed.extend(removed);
            }
            Ok(stored)
        })
    }

    fn store_inner(&self, mapping: &dyn Mapping, incoming: &Incoming, at: jiff::Timestamp) -> Result<Stored> {
        let source = mapping.source();
        let payload = canon::canonical(incoming.payload).map_err(BookError::Payload)?;
        let existing = self.record_by_key(incoming.connection, &source, incoming.source_key)?;
        let (record, outcome) = match existing {
            None => {
                let id = RecordId::from_uuid(new_uuid(at));
                self.conn().execute(
                    "INSERT INTO source_records(id, connection_id, source, source_key, state, derived_version, first_received_at, state_changed_at)
                     VALUES (?1, ?2, ?3, ?4, 'live', ?5, ?6, ?6)",
                    params![id.to_string(), incoming.connection.map(|c| c.to_string()), source.as_str(), incoming.source_key, mapping.version(), at_text(at)],
                )?;
                self.add_revision(id, 1, &payload, at)?;
                (id, Outcome::New)
            }
            Some(id) => {
                let (latest, n) = self.latest_revision(id)?;
                if latest == payload {
                    (id, Outcome::Unchanged)
                } else {
                    self.add_revision(id, n + 1, &payload, at)?;
                    (id, Outcome::Revised(n + 1))
                }
            }
        };
        for (scheme, value) in &incoming.refs {
            self.conn().execute("INSERT OR IGNORE INTO record_refs(scheme, value, record_id) VALUES (?, ?, ?)", params![scheme, value, record.to_string()])?;
        }
        let changes = if outcome != Outcome::Unchanged && self.record(record)?.state == RecordState::Live {
            self.derive(record, incoming.connection, &payload, mapping, at)?
        } else {
            Changes::default()
        };
        Ok(Stored { record, outcome, changes })
    }

    fn add_revision(&self, id: RecordId, n: u32, payload: &str, at: jiff::Timestamp) -> Result<()> {
        self.conn().execute(
            "INSERT INTO record_revisions(record_id, revision, received_at, payload) VALUES (?, ?, ?, ?)",
            params![id.to_string(), n, at_text(at), payload],
        )?;
        Ok(())
    }

    fn latest_revision(&self, id: RecordId) -> Result<(String, u32)> {
        self.conn()
            .query_row(
                "SELECT payload, revision FROM record_revisions WHERE record_id = ? ORDER BY revision DESC LIMIT 1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| text::corrupt("record_revisions", "record_id", &id.to_string(), "a record with no revision"))
    }

    /// Derive a live record's transactions from `payload` again, replacing what
    /// it had, and move or orphan the trades anchored on it.
    fn derive(&self, record: RecordId, connection: Option<ConnectionId>, payload: &str, mapping: &dyn Mapping, at: jiff::Timestamp) -> Result<Changes> {
        let source = mapping.source();
        let version = mapping.version();
        let ctx = MapContext { connection, record: record.clone(), zones: &self.zones };
        let mapped = mapping.map(&ctx, payload);
        let mut problems = mapped.problems.clone();

        // what the record said before goes; resolve every leg, or none: a record
        // is never half counted
        self.conn().execute("DELETE FROM instrument_sightings WHERE record_id = ?", [record.to_string()])?;
        self.conn().execute_batch("SAVEPOINT derive")?;
        let resolved = self.resolve_legs(record, &mapped.legs, &source, version, at);
        let rows = match resolved {
            Ok((Ok(rows), notes)) => {
                self.conn().execute_batch("RELEASE derive")?;
                problems.extend(notes);
                rows
            }
            Ok((Err(blocking), _)) => {
                self.conn().execute_batch("ROLLBACK TO derive; RELEASE derive")?;
                problems.extend(blocking);
                Vec::new()
            }
            Err(e) => {
                self.conn().execute_batch("ROLLBACK TO derive; RELEASE derive")?;
                return Err(e);
            }
        };

        let before: BTreeMap<Leg, Transaction> = self.transactions_of(record)?.into_iter().map(|t| (t.id.leg.clone(), t)).collect();
        // the sightings the resolution just wrote are the record's new ones; its
        // transactions and problems are replaced below
        self.conn().execute("DELETE FROM transactions WHERE record_id = ?", [record.to_string()])?;
        self.conn().execute("DELETE FROM record_problems WHERE record_id = ?", [record.to_string()])?;
        for t in &rows {
            self.insert_transaction(t)?;
        }
        problems.extend(self.write_adjustments(record, &mapped.adjustments)?);
        self.add_problems(record, &problems)?;
        self.conn().execute("UPDATE source_records SET derived_version = ? WHERE id = ?", params![version, record.to_string()])?;

        let after: BTreeMap<Leg, &Transaction> = rows.iter().map(|t| (t.id.leg.clone(), t)).collect();
        // an adjustment another record holds on a transaction this one no longer has
        for (adjustment, applies) in self.adjustments_applying_to(record)? {
            if adjustment.record != record && !after.contains_key(&applies.leg) {
                self.adjustment_target_gone(&adjustment, &applies, "is no longer on its record")?;
            }
        }
        let mut changes = Changes::default();
        for (leg, t) in &after {
            match before.get(leg) {
                None => changes.added.push(t.id.clone()),
                Some(old) if !same_content(old, t) => changes.changed.push(t.id.clone()),
                Some(_) => {}
            }
        }
        for (leg, old) in &before {
            if !after.contains_key(leg) {
                changes.removed.push(old.id.clone());
            }
        }
        // a trade stays anchored only on an opening that is still the same opening
        for (trade, anchor) in self.trades_anchored_on(record)? {
            let reason = match (before.get(&anchor.transaction.leg), after.get(&anchor.transaction.leg)) {
                (_, None) => Some(format!("the record it opened on no longer has its {} transaction", anchor.transaction.leg)),
                (Some(old), Some(new)) if !same_opening(old, new) => {
                    Some("the record it opened on now says another account, instrument or direction".to_string())
                }
                _ => None,
            };
            if let Some(reason) = reason {
                self.orphan(trade, &reason)?;
            }
        }
        Ok(changes)
    }

    /// Each draft as a transaction, its account and instrument resolved. `Err`
    /// holds the problems that stop the record's transactions.
    #[allow(clippy::type_complexity)]
    fn resolve_legs(&self, record: RecordId, legs: &[Draft], source: &SourceName, version: u32, at: jiff::Timestamp) -> Result<(std::result::Result<Vec<Transaction>, Vec<Problem>>, Vec<Problem>)> {
        let mut rows = Vec::new();
        let mut blocking = Vec::new();
        let mut notes = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for d in legs {
            if !seen.insert(d.leg.clone()) {
                blocking.push(Problem::new("duplicate-leg", format!("the mapping names two legs {}", d.leg)));
                continue;
            }
            let Some(account) = self.account_by_ref(&d.account)? else {
                blocking.push(Problem::new("account-unknown", format!("no account has the {} id {}", d.account.broker, d.account.value)));
                continue;
            };
            let instrument = match &d.instrument {
                None => None,
                Some(draft) => {
                    let (found, more) = self.resolve_instrument(draft, &TransactionId::new(record, d.leg.clone()), at)?;
                    notes.extend(more);
                    match found {
                        Ok(id) => Some(id),
                        Err(p) => {
                            blocking.push(p);
                            continue;
                        }
                    }
                }
            };
            if d.quantity.is_some() && instrument.is_none() {
                blocking.push(Problem::new("quantity-without-instrument", format!("the {} leg moves a quantity of no instrument", d.leg)));
                continue;
            }
            if d.price.is_some() && d.quantity.is_none() {
                blocking.push(Problem::new("price-without-quantity", format!("the {} leg has a price and no quantity", d.leg)));
                continue;
            }
            rows.push(Transaction {
                id: TransactionId::new(record, d.leg.clone()),
                mapping: MappingVersion { source: source.clone(), version },
                account,
                occurred_at: d.occurred_at,
                trade_date: d.trade_date,
                settle_date: d.settle_date,
                kind: d.kind,
                effect: d.effect,
                instrument,
                quantity: d.quantity,
                price: d.price,
                cash: d.cash,
                fee: d.fee,
                fx_rate: d.fx_rate,
            });
        }
        Ok((if blocking.is_empty() { Ok(rows) } else { Err(blocking) }, notes))
    }

    fn insert_transaction(&self, t: &Transaction) -> Result<()> {
        let amount = |m: Option<Money>| m.map(|m| (m.amount.to_text(), m.currency.to_string())).unzip();
        let (price, price_cur) = amount(t.price);
        let (cash, cash_cur) = amount(t.cash);
        let (fee, fee_cur) = amount(t.fee);
        self.conn().execute(
            "INSERT INTO transactions(record_id, leg, mapping_version, account_id, occurred_at, trade_date, settle_date, kind, effect, instrument_id,
                                      quantity, price, price_currency, cash, cash_currency, fee, fee_currency, fx_rate)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                t.id.record.to_string(),
                t.id.leg.as_str(),
                t.mapping.version,
                t.account.to_string(),
                t.occurred_at.map(at_text),
                day(t.trade_date),
                t.settle_date.map(day),
                t.kind.as_str(),
                t.effect.map(|e| e.as_str()),
                t.instrument.map(|i| i.to_string()),
                t.quantity.map(|q| q.to_text()),
                price,
                price_cur,
                cash,
                cash_cur,
                fee,
                fee_cur,
                t.fx_rate.map(|r| r.to_text()),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn add_problems(&self, record: RecordId, problems: &[Problem]) -> Result<()> {
        for p in problems {
            self.conn().execute("INSERT INTO record_problems(record_id, code, detail) VALUES (?, ?, ?)", params![record.to_string(), p.code, p.detail])?;
        }
        Ok(())
    }

    /// Derive again every live record of `mapping`'s source whose transactions or
    /// problems came from an older version of it, and say what moved. Records
    /// derived with this version already are left alone.
    pub fn rederive(&self, mapping: &dyn Mapping, at: jiff::Timestamp) -> Result<Changes> {
        self.atomically(|| {
            let mut stmt = self.conn().prepare(
                "SELECT id, connection_id FROM source_records WHERE source = ? AND state = 'live' AND derived_version < ? ORDER BY first_received_at, rowid",
            )?;
            let stale: Vec<(String, Option<String>)> = stmt
                .query_map(params![mapping.source().as_str(), mapping.version()], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut changes = Changes::default();
            for (id, connection) in stale {
                let record = text::parsed("source_records", "id", &id, RecordId::parse)?;
                let connection = text::opt_parsed("source_records", "connection_id", connection, ConnectionId::parse)?;
                let (payload, _) = self.latest_revision(record)?;
                changes.extend(self.derive(record, connection, &payload, mapping, at)?);
            }
            Ok(changes)
        })
    }

    /// The record's source reported it removed: its transactions leave the book
    /// and trades anchored on them are orphaned. A record that had replaced
    /// others does not bring them back; that is a problem for the person.
    pub fn mark_removed(&self, record: RecordId, at: jiff::Timestamp) -> Result<Changes> {
        self.atomically(|| {
            let r = self.record(record)?;
            if r.state == RecordState::Removed {
                return Ok(Changes::default());
            }
            let removed: Vec<TransactionId> = self.transactions_of(record)?.into_iter().map(|t| t.id).collect();
            for (trade, _) in self.trades_anchored_on(record)? {
                self.orphan(trade, "the source removed the record it opened on")?;
            }
            for (adjustment, applies) in self.adjustments_applying_to(record)? {
                if adjustment.record != record {
                    self.adjustment_target_gone(&adjustment, &applies, "was removed by its source")?;
                }
            }
            self.clear_derived(record)?;
            self.set_state(record, RecordState::Removed, at)?;
            if self.has_superseded(record)? {
                self.add_problems(record, &[Problem::new(
                    "superseding-record-removed",
                    "the source removed a record that had replaced others; the ones it replaced stay replaced",
                )])?;
            }
            Ok(Changes { removed, ..Changes::default() })
        })
    }

    /// Take out of the book everything derived from a record: its transactions,
    /// its problems, its sightings. Its revisions stay.
    pub(crate) fn clear_derived(&self, record: RecordId) -> Result<()> {
        for table in ["transactions", "record_problems", "instrument_sightings"] {
            self.conn().execute(&format!("DELETE FROM {table} WHERE record_id = ?"), [record.to_string()])?;
        }
        self.clear_adjustments(record)
    }

    pub(crate) fn set_state(&self, record: RecordId, state: RecordState, at: jiff::Timestamp) -> Result<()> {
        self.conn().execute(
            "UPDATE source_records SET state = ?, state_changed_at = ? WHERE id = ?",
            params![state.as_str(), at_text(at), record.to_string()],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // reading
    // ------------------------------------------------------------------

    pub fn record(&self, id: RecordId) -> Result<SourceRecord> {
        let row: Option<(Option<String>, String, String, String)> = self
            .conn()
            .query_row("SELECT connection_id, source, source_key, state FROM source_records WHERE id = ?", [id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let (connection, source, source_key, state) = row.ok_or_else(|| BookError::Refused(format!("no record {id}")))?;
        let (_, revision) = self.latest_revision(id)?;
        Ok(SourceRecord {
            id,
            connection: text::opt_parsed("source_records", "connection_id", connection, ConnectionId::parse)?,
            source: text::parsed("source_records", "source", &source, SourceName::parse)?,
            source_key,
            state: text::parsed("source_records", "state", &state, RecordState::parse)?,
            revision,
        })
    }

    pub fn record_by_key(&self, connection: Option<ConnectionId>, source: &SourceName, key: &str) -> Result<Option<RecordId>> {
        let found: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM source_records WHERE COALESCE(connection_id, '') = ? AND source = ? AND source_key = ?",
                params![connection.map(|c| c.to_string()).unwrap_or_default(), source.as_str(), key],
                |r| r.get(0),
            )
            .optional()?;
        found.map(|s| text::parsed("source_records", "id", &s, RecordId::parse)).transpose()
    }

    /// The value a record is known by in `scheme`, if any.
    pub fn record_ref(&self, record: RecordId, scheme: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM record_refs WHERE record_id = ? AND scheme = ? ORDER BY value LIMIT 1", params![record.to_string(), scheme], |r| r.get(0))
            .optional()?)
    }

    /// The records known by `value` in `scheme`.
    pub fn records_by_ref(&self, scheme: &str, value: &str) -> Result<Vec<RecordId>> {
        let mut stmt = self.conn().prepare_cached("SELECT record_id FROM record_refs WHERE scheme = ? AND value = ? ORDER BY record_id")?;
        let ids = stmt.query_map(params![scheme, value], |r| r.get::<_, String>(0))?;
        ids.map(|s| text::parsed("record_refs", "record_id", &s?, RecordId::parse)).collect()
    }

    /// Every revision of a record, oldest first: its number, when it arrived, its payload.
    pub fn revisions(&self, id: RecordId) -> Result<Vec<(u32, jiff::Timestamp, String)>> {
        let mut stmt = self.conn().prepare_cached("SELECT revision, received_at, payload FROM record_revisions WHERE record_id = ? ORDER BY revision")?;
        let rows = stmt.query_map([id.to_string()], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (n, received, payload) = row?;
            out.push((n, text::instant("record_revisions", "received_at", &received)?, payload));
        }
        Ok(out)
    }

    /// Every transaction in the book (live records only have any), by account and day.
    pub fn transactions(&self) -> Result<Vec<Transaction>> {
        self.transactions_where("", [])
    }

    pub fn transactions_of(&self, record: RecordId) -> Result<Vec<Transaction>> {
        self.transactions_where("WHERE t.record_id = ?", [record.to_string()])
    }

    pub fn transaction(&self, id: &TransactionId) -> Result<Option<Transaction>> {
        Ok(self.transactions_where("WHERE t.record_id = ? AND t.leg = ?", [id.record.to_string(), id.leg.to_string()])?.pop())
    }

    fn transactions_where(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Transaction>> {
        let sql = format!(
            "SELECT t.record_id, t.leg, r.source, t.mapping_version, t.account_id, t.occurred_at, t.trade_date, t.settle_date, t.kind, t.effect,
                    t.instrument_id, t.quantity, t.price, t.price_currency, t.cash, t.cash_currency, t.fee, t.fee_currency, t.fx_rate
             FROM transactions t JOIN source_records r ON r.id = t.record_id {filter}
             ORDER BY t.account_id, t.trade_date, t.occurred_at, t.record_id, t.leg"
        );
        let mut stmt = self.conn().prepare_cached(&sql)?;
        let mut rows = stmt.query(args)?;
        let mut out = Vec::new();
        const T: &str = "transactions";
        while let Some(r) = rows.next()? {
            let s = |i: usize| r.get::<_, String>(i);
            let o = |i: usize| r.get::<_, Option<String>>(i);
            let record = text::parsed(T, "record_id", &s(0)?, RecordId::parse)?;
            let leg = text::parsed(T, "leg", &s(1)?, Leg::parse)?;
            out.push(Transaction {
                id: TransactionId::new(record, leg),
                mapping: MappingVersion { source: text::parsed("source_records", "source", &s(2)?, SourceName::parse)?, version: r.get(3)? },
                account: text::parsed(T, "account_id", &s(4)?, AccountId::parse)?,
                occurred_at: text::opt_instant(T, "occurred_at", o(5)?)?,
                trade_date: text::date(T, "trade_date", &s(6)?)?,
                settle_date: text::opt_date(T, "settle_date", o(7)?)?,
                kind: text::parsed(T, "kind", &s(8)?, Kind::parse)?,
                effect: text::opt_parsed(T, "effect", o(9)?, Effect::parse)?,
                instrument: text::opt_parsed(T, "instrument_id", o(10)?, InstrumentId::parse)?,
                quantity: text::opt_dec(T, "quantity", o(11)?)?,
                price: text::money(T, "price", o(12)?, o(13)?)?,
                cash: text::money(T, "cash", o(14)?, o(15)?)?,
                fee: text::money(T, "fee", o(16)?, o(17)?)?,
                fx_rate: text::opt_dec(T, "fx_rate", o(18)?)?,
            });
        }
        Ok(out)
    }

    pub fn problems_of(&self, record: RecordId) -> Result<Vec<Problem>> {
        let mut stmt = self.conn().prepare_cached("SELECT code, detail FROM record_problems WHERE record_id = ? ORDER BY rowid")?;
        let rows = stmt.query_map([record.to_string()], |r| Ok(Problem { code: r.get(0)?, detail: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every problem the person should see: on live records, and on removed ones
    /// (whose only problems are about their removal). A superseded record's
    /// problems no longer matter: it is not counted.
    pub fn problems(&self) -> Result<Vec<(RecordId, Problem)>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT p.record_id, p.code, p.detail FROM record_problems p JOIN source_records r ON r.id = p.record_id
             WHERE r.state IN ('live', 'removed') ORDER BY p.record_id, p.rowid",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, Problem { code: r.get(1)?, detail: r.get(2)? })))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, p) = row?;
            out.push((text::parsed("record_problems", "record_id", &id, RecordId::parse)?, p));
        }
        Ok(out)
    }

    /// Every live record's source key: what orders the transactions no instant
    /// separates (`bagholder_engine::ledger`).
    pub fn live_record_keys(&self) -> Result<BTreeMap<RecordId, String>> {
        let mut stmt = self.conn().prepare_cached("SELECT id, source_key FROM source_records WHERE state = 'live'")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (id, key) = row?;
            out.insert(text::parsed("source_records", "id", &id, RecordId::parse)?, key);
        }
        Ok(out)
    }

    /// The live records known by a value in `scheme` that another source than
    /// `except` stated, with that value: an imported record of a broker's row
    /// that the broker's own record has not replaced.
    pub fn live_records_by_scheme(&self, scheme: &str, except: &SourceName) -> Result<Vec<(RecordId, String)>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT r.id, x.value FROM record_refs x JOIN source_records r ON r.id = x.record_id
             WHERE x.scheme = ? AND r.state = 'live' AND r.source != ? ORDER BY r.first_received_at, r.rowid",
        )?;
        let rows = stmt.query_map(params![scheme, except.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, value) = row?;
            out.push((text::parsed("source_records", "id", &id, RecordId::parse)?, value));
        }
        Ok(out)
    }

    /// How many records of a source the book has ever kept, whatever became of them.
    pub fn record_count(&self, source: &SourceName) -> Result<usize> {
        let n: i64 = self.conn().query_row("SELECT COUNT(*) FROM source_records WHERE source = ?", [source.as_str()], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// The live records of a source, oldest first.
    pub fn live_records(&self, source: &SourceName) -> Result<Vec<RecordId>> {
        let mut stmt = self.conn().prepare_cached("SELECT id FROM source_records WHERE source = ? AND state = 'live' ORDER BY first_received_at, rowid")?;
        let ids = stmt.query_map([source.as_str()], |r| r.get::<_, String>(0))?;
        ids.map(|s| text::parsed("source_records", "id", &s?, RecordId::parse)).collect()
    }

    /// The records that have superseded a record of `source`: each has taken
    /// the place of one already, and takes no other's.
    pub fn superseding(&self, source: &SourceName) -> Result<Vec<RecordId>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT DISTINCT t.record_id FROM link_records f
             JOIN link_records t ON t.link_id = f.link_id AND t.side = 'to'
             JOIN source_records r ON r.id = f.record_id
             WHERE f.side = 'from' AND r.source = ? ORDER BY t.record_id",
        )?;
        let ids = stmt.query_map([source.as_str()], |r| r.get::<_, String>(0))?;
        ids.map(|s| text::parsed("link_records", "record_id", &s?, RecordId::parse)).collect()
    }

    /// The trades anchored on a record's transactions, with their anchors.
    pub(crate) fn trades_anchored_on(&self, record: RecordId) -> Result<Vec<(TradeId, Opening)>> {
        let mut stmt = self.conn().prepare_cached("SELECT id, anchor_leg, anchor_instrument FROM trades WHERE anchor_record = ? ORDER BY created_at, rowid")?;
        let rows = stmt.query_map([record.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, leg, instrument) = row?;
            out.push((
                text::parsed("trades", "id", &id, TradeId::parse)?,
                Opening {
                    transaction: TransactionId::new(record, text::parsed("trades", "anchor_leg", &leg, Leg::parse)?),
                    instrument: text::parsed("trades", "anchor_instrument", &instrument, InstrumentId::parse)?,
                },
            ));
        }
        Ok(out)
    }
}

/// The quantity of each (account, instrument) the live transactions hold: what a
/// position count would read, and what nothing counted twice keeps equal to the
/// sum of the live transactions (the invariant `supersede` is tested against).
pub fn positions(transactions: &[Transaction]) -> std::result::Result<BTreeMap<(AccountId, InstrumentId), Dec>, bagholder_core::DecError> {
    let mut out: BTreeMap<(AccountId, InstrumentId), Dec> = BTreeMap::new();
    for t in transactions {
        if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
            let e = out.entry((t.account, i)).or_insert(Dec::ZERO);
            *e = e.checked_add(q)?;
        }
    }
    Ok(out)
}
