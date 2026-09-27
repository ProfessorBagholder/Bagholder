//! Identity in the book (`docs/architecture.md` §5): broker connections, accounts
//! and their broker ids, instruments with their references, dated names and
//! option terms, and issuers.
//!
//! An instrument is found only by a reference that identifies it (a strong one,
//! or a connection-scoped one within its connection), never by a bare symbol.
//! When a record's references name two different instruments, or a reference is
//! already another instrument's, nothing is merged and nothing is picked: the
//! record gets a problem. The one join is a source's own: an id it states
//! retired by a corporate action and the id it trades the same listing under
//! now are read as one instrument (`Book::settle_successions`). The join is
//! decided from the records each time and merges nothing: each id keeps its
//! instrument, and a later row stating the succession otherwise parts them.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};

use bagholder_core::account::{Account, AccountKind, AccountRef, AccountStatus, AccountType, Connection, Registration};
use bagholder_core::instrument::{Instrument, InstrumentKind, Issuer, Name, OptionRight, OptionTerms, RefScheme, Reference};
use bagholder_core::record::Problem;
use bagholder_core::{AccountId, Broker, ConnectionId, InstrumentId, IssuerId, SourceName, TransactionId};

use crate::mapping::{InstrumentDraft, Mapping, NameDraft, StandingDraft};
use crate::text::{self, at as at_text, day};
use crate::{new_uuid, Book, BookError, Result};

impl Book {
    // ------------------------------------------------------------------
    // connections
    // ------------------------------------------------------------------

    pub fn add_connection(&self, broker: &Broker, label: &str, at: jiff::Timestamp) -> Result<ConnectionId> {
        let id = ConnectionId::from_uuid(new_uuid(at));
        self.conn().execute(
            "INSERT INTO broker_connections(id, broker, label, created_at) VALUES (?, ?, ?, ?)",
            params![id.to_string(), broker.as_str(), label, at_text(at)],
        )?;
        Ok(id)
    }

    pub fn connections(&self) -> Result<Vec<Connection>> {
        let mut stmt = self.conn().prepare_cached("SELECT id, broker, label FROM broker_connections ORDER BY created_at, rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, broker, label) = row?;
            out.push(Connection {
                id: text::parsed("broker_connections", "id", &id, ConnectionId::parse)?,
                broker: text::parsed("broker_connections", "broker", &broker, Broker::parse)?,
                label,
            });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // accounts
    // ------------------------------------------------------------------

    /// A new account with its broker ids. An id another account already holds is
    /// refused: two accounts are never made one by an id they share.
    pub fn add_account(
        &self,
        connection: ConnectionId,
        refs: &[AccountRef],
        account_type: &AccountType,
        status: AccountStatus,
        nickname: Option<&str>,
        at: jiff::Timestamp,
    ) -> Result<AccountId> {
        self.atomically(|| {
            for r in refs {
                if let Some(other) = self.account_by_ref(r)? {
                    return Err(BookError::Refused(format!("the {} account id {} is already account {other}", r.broker, r.value)));
                }
            }
            let id = AccountId::from_uuid(new_uuid(at));
            let (kind, registration, managed, joint, unrecognised) = match account_type {
                AccountType::Known { kind, registration, managed, joint } => {
                    (Some(kind.as_str()), Some(registration.as_str()), Some(*managed as i64), Some(*joint as i64), None)
                }
                AccountType::Unrecognised(words) => (None, None, None, None, Some(words.as_str())),
            };
            self.conn().execute(
                "INSERT INTO accounts(id, connection_id, kind, registration, managed, joint, unrecognised_type, status, nickname, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![id.to_string(), connection.to_string(), kind, registration, managed, joint, unrecognised, status.as_str(), nickname, at_text(at)],
            )?;
            for r in refs {
                self.conn().execute(
                    "INSERT INTO account_refs(scheme, value, account_id) VALUES (?, ?, ?)",
                    params![r.scheme_text(), r.value, id.to_string()],
                )?;
            }
            Ok(id)
        })
    }

    /// Add a broker id to an account. One another account holds is refused.
    pub fn add_account_ref(&self, account: AccountId, r: &AccountRef) -> Result<()> {
        match self.account_by_ref(r)? {
            Some(holder) if holder == account => Ok(()),
            Some(other) => Err(BookError::Refused(format!("the {} account id {} is already account {other}", r.broker, r.value))),
            None => {
                self.conn().execute(
                    "INSERT INTO account_refs(scheme, value, account_id) VALUES (?, ?, ?)",
                    params![r.scheme_text(), r.value, account.to_string()],
                )?;
                Ok(())
            }
        }
    }

    pub fn account_by_ref(&self, r: &AccountRef) -> Result<Option<AccountId>> {
        let found: Option<String> = self
            .conn()
            .query_row("SELECT account_id FROM account_refs WHERE scheme = ? AND value = ?", params![r.scheme_text(), r.value], |row| row.get(0))
            .optional()?;
        found.map(|s| text::parsed("account_refs", "account_id", &s, AccountId::parse)).transpose()
    }

    pub fn account(&self, id: AccountId) -> Result<Account> {
        self.accounts_where("WHERE id = ?", params![id.to_string()])?
            .pop()
            .ok_or_else(|| BookError::Refused(format!("no account {id}")))
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        self.accounts_where("", [])
    }

    fn accounts_where(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Account>> {
        let sql = format!(
            "SELECT id, connection_id, kind, registration, managed, joint, unrecognised_type, status, nickname FROM accounts {filter} ORDER BY created_at, rowid"
        );
        let mut stmt = self.conn().prepare_cached(&sql)?;
        type Row = (String, String, Option<String>, Option<String>, Option<i64>, Option<i64>, Option<String>, String, Option<String>);
        let rows = stmt.query_map(args, |r| -> rusqlite::Result<Row> {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, connection, kind, registration, managed, joint, unrecognised, status, nickname) = row?;
            let account_type = match (kind, unrecognised) {
                (Some(kind), None) => AccountType::Known {
                    kind: text::parsed("accounts", "kind", &kind, AccountKind::parse)?,
                    registration: text::parsed("accounts", "registration", registration.as_deref().unwrap_or(""), Registration::parse)?,
                    managed: managed == Some(1),
                    joint: joint == Some(1),
                },
                (None, Some(words)) => AccountType::Unrecognised(words),
                (k, u) => return Err(text::corrupt("accounts", "kind", &format!("{k:?} {u:?}"), "a kind and a broker type at once, or neither")),
            };
            out.push(Account {
                id: text::parsed("accounts", "id", &id, AccountId::parse)?,
                connection: text::parsed("accounts", "connection_id", &connection, ConnectionId::parse)?,
                account_type,
                status: text::parsed("accounts", "status", &status, AccountStatus::parse)?,
                nickname,
            });
        }
        Ok(out)
    }

    /// The broker ids an account is known by.
    pub fn account_refs(&self, id: AccountId) -> Result<Vec<AccountRef>> {
        let mut stmt = self.conn().prepare_cached("SELECT scheme, value FROM account_refs WHERE account_id = ? ORDER BY scheme, value")?;
        let rows = stmt.query_map([id.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (scheme, value) = row?;
            out.push(AccountRef { broker: text::parsed("account_refs", "scheme", &scheme, AccountRef::parse_scheme)?, value });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // issuers
    // ------------------------------------------------------------------

    pub fn add_issuer(&self, name: &str, at: jiff::Timestamp) -> Result<IssuerId> {
        let id = IssuerId::from_uuid(new_uuid(at));
        self.conn().execute("INSERT INTO issuers(id, name, created_at) VALUES (?, ?, ?)", params![id.to_string(), name, at_text(at)])?;
        Ok(id)
    }

    /// Put a listing under its issuer. A listing already under another issuer is
    /// refused rather than moved.
    pub fn attach_issuer(&self, instrument: InstrumentId, issuer: IssuerId) -> Result<()> {
        let current = self.instrument(instrument)?.issuer;
        match current {
            Some(i) if i == issuer => Ok(()),
            Some(other) => Err(BookError::Refused(format!("instrument {instrument} is already under issuer {other}"))),
            None => {
                self.conn().execute("UPDATE instruments SET issuer_id = ? WHERE id = ?", params![issuer.to_string(), instrument.to_string()])?;
                Ok(())
            }
        }
    }

    pub fn issuer(&self, id: IssuerId) -> Result<Issuer> {
        let name: String = self
            .conn()
            .query_row("SELECT name FROM issuers WHERE id = ?", [id.to_string()], |r| r.get(0))
            .optional()?
            .ok_or_else(|| BookError::Refused(format!("no issuer {id}")))?;
        Ok(Issuer { id, name })
    }

    /// The listings under an issuer.
    pub fn listings(&self, issuer: IssuerId) -> Result<Vec<InstrumentId>> {
        let mut stmt = self.conn().prepare_cached("SELECT id FROM instruments WHERE issuer_id = ? ORDER BY created_at, rowid")?;
        let ids = stmt.query_map([issuer.to_string()], |r| r.get::<_, String>(0))?;
        ids.map(|s| text::parsed("instruments", "id", &s?, InstrumentId::parse)).collect()
    }

    // ------------------------------------------------------------------
    // instruments
    // ------------------------------------------------------------------

    pub fn instrument(&self, id: InstrumentId) -> Result<Instrument> {
        let row: Option<(String, String, Option<String>)> = self
            .conn()
            .query_row("SELECT kind, currency, issuer_id FROM instruments WHERE id = ?", [id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let (kind, currency, issuer) = row.ok_or_else(|| BookError::Refused(format!("no instrument {id}")))?;
        Ok(Instrument {
            id,
            kind: text::parsed("instruments", "kind", &kind, InstrumentKind::parse)?,
            currency: text::currency("instruments", "currency", &currency)?,
            issuer: text::opt_parsed("instruments", "issuer_id", issuer, IssuerId::parse)?,
        })
    }

    /// Every instrument as the book reads them: one joined into another by a
    /// succession is read as that one, and is not listed.
    pub fn instruments(&self) -> Result<Vec<Instrument>> {
        let mut stmt = self.conn().prepare_cached("SELECT id FROM instruments WHERE id NOT IN (SELECT instrument_id FROM instrument_joins) ORDER BY created_at, rowid")?;
        let ids: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        ids.iter().map(|s| self.instrument(text::parsed("instruments", "id", s, InstrumentId::parse)?)).collect()
    }

    /// The instrument an identifying reference names, if any, as the book reads
    /// it: where a succession joins the reference's own instrument to another,
    /// the one they are read as (`settle_successions`). A routing reference
    /// names none: it only says how to ask a source (`instrument_routes`).
    pub fn instrument_by_ref(&self, r: &Reference) -> Result<Option<InstrumentId>> {
        self.own_instrument(r)?.map(|id| self.canonical(id)).transpose()
    }

    /// The instrument an identifying reference names itself, before any
    /// succession joins it to another: what a record, a statement or a choice
    /// made under the reference is kept against, so that parting a succession
    /// again leaves each on its own.
    pub fn own_instrument(&self, r: &Reference) -> Result<Option<InstrumentId>> {
        if !r.identifies() {
            return Ok(None);
        }
        let found: Option<String> = self
            .conn()
            .query_row("SELECT instrument_id FROM instrument_refs WHERE scheme = ? AND value = ?", params![r.scheme.to_text(), r.value], |row| row.get(0))
            .optional()?;
        found.map(|s| text::parsed("instrument_refs", "instrument_id", &s, InstrumentId::parse)).transpose()
    }

    /// The instrument `id` is read as: the one a succession joins it into, else
    /// itself.
    pub fn canonical(&self, id: InstrumentId) -> Result<InstrumentId> {
        let into: Option<String> = self
            .conn()
            .query_row("SELECT into_id FROM instrument_joins WHERE instrument_id = ?", [id.to_string()], |r| r.get(0))
            .optional()?;
        into.map_or(Ok(id), |s| text::parsed("instrument_joins", "into_id", &s, InstrumentId::parse))
    }

    /// Every instrument a succession joins into another, and the one it is read as.
    pub fn joined(&self) -> Result<BTreeMap<InstrumentId, InstrumentId>> {
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id, into_id FROM instrument_joins")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (a, b) = row?;
            out.insert(
                text::parsed("instrument_joins", "instrument_id", &a, InstrumentId::parse)?,
                text::parsed("instrument_joins", "into_id", &b, InstrumentId::parse)?,
            );
        }
        Ok(out)
    }

    /// Every reference an instrument is known by, with those of every
    /// instrument read as one with it: those that identify it (an id its source
    /// states still traded under before one it states retired), then the ways to
    /// ask sources for it.
    pub fn instrument_refs(&self, id: InstrumentId) -> Result<Vec<Reference>> {
        let head = self.canonical(id)?;
        let group = group_sql("?1");
        let mut out: Vec<Reference> = Vec::new();
        for (table, sql) in [
            (
                "instrument_refs",
                format!(
                    "SELECT r.scheme, r.value FROM instrument_refs r WHERE r.instrument_id IN {group}
                     ORDER BY EXISTS (SELECT 1 FROM security_standings s WHERE s.scheme = r.scheme AND s.value = r.value AND s.standing = 'retired-by-event'), r.scheme, r.value"
                ),
            ),
            ("instrument_routes", format!("SELECT DISTINCT scheme, value FROM instrument_routes WHERE instrument_id IN {group} ORDER BY scheme, value")),
        ] {
            let mut stmt = self.conn().prepare_cached(&sql)?;
            let rows = stmt.query_map([head.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (scheme, value) = row?;
                out.push(Reference { scheme: text::parsed(table, "scheme", &scheme, RefScheme::parse)?, value });
            }
        }
        Ok(out)
    }

    /// The references the instrument holds itself, identifying ones first: the
    /// broker's own id for it, where a source states which of its moves are
    /// which.
    pub fn own_refs(&self, id: InstrumentId) -> Result<Vec<Reference>> {
        let mut out = Vec::new();
        for (table, sql) in [
            ("instrument_refs", "SELECT scheme, value FROM instrument_refs WHERE instrument_id = ? ORDER BY scheme, value"),
            ("instrument_routes", "SELECT scheme, value FROM instrument_routes WHERE instrument_id = ? ORDER BY scheme, value"),
        ] {
            let mut stmt = self.conn().prepare_cached(sql)?;
            let rows = stmt.query_map([id.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (scheme, value) = row?;
                out.push(Reference { scheme: text::parsed(table, "scheme", &scheme, RefScheme::parse)?, value });
            }
        }
        Ok(out)
    }

    /// The instruments a routing reference leads to (it may be several), as the
    /// book reads them.
    pub fn instruments_routed_by(&self, r: &Reference) -> Result<Vec<InstrumentId>> {
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id FROM instrument_routes WHERE scheme = ? AND value = ? ORDER BY instrument_id")?;
        let ids = stmt.query_map(params![r.scheme.to_text(), r.value], |row| row.get::<_, String>(0))?;
        let mut out = BTreeSet::new();
        for s in ids {
            out.insert(self.canonical(text::parsed("instrument_routes", "instrument_id", &s?, InstrumentId::parse)?)?);
        }
        Ok(out.into_iter().collect())
    }

    /// Add a reference to an instrument. An identifying one another instrument
    /// holds is refused; a routing one is simply added.
    pub fn add_instrument_ref(&self, id: InstrumentId, r: &Reference) -> Result<()> {
        if !r.identifies() {
            self.conn().execute(
                "INSERT OR IGNORE INTO instrument_routes(instrument_id, scheme, value) VALUES (?, ?, ?)",
                params![id.to_string(), r.scheme.to_text(), r.value],
            )?;
            return Ok(());
        }
        match self.own_instrument(r)? {
            Some(holder) if holder == id => Ok(()),
            Some(other) => Err(BookError::Refused(format!("{} {} already names instrument {other}", r.scheme, r.value))),
            None => {
                self.conn().execute(
                    "INSERT INTO instrument_refs(scheme, value, instrument_id) VALUES (?, ?, ?)",
                    params![r.scheme.to_text(), r.value, id.to_string()],
                )?;
                Ok(())
            }
        }
    }

    /// What the instrument was called, oldest first: each unbroken run of live
    /// records' sightings under one symbol and venue is one name, from its first
    /// day to its last. A ticker that changes and changes back is three names.
    /// The sightings of every instrument read as one with it count.
    pub fn names(&self, id: InstrumentId) -> Result<Vec<Name>> {
        let head = self.canonical(id)?;
        let mut stmt = self.conn().prepare_cached(&format!(
            "SELECT s.symbol, s.venue_mic, s.venue_name, s.name, s.day, r.source FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live'
             WHERE s.instrument_id IN {} ORDER BY s.day, s.symbol, s.rowid",
            group_sql("?1")
        ))?;
        type Row = (String, Option<String>, Option<String>, Option<String>, String, String);
        let rows = stmt.query_map([head.to_string()], |r| -> rusqlite::Result<Row> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)) })?;
        let mut out: Vec<Name> = Vec::new();
        for row in rows {
            let (symbol, venue_mic, venue_name, name, day_text, source) = row?;
            let seen = text::date("instrument_sightings", "day", &day_text)?;
            let source = text::parsed("source_records", "source", &source, SourceName::parse)?;
            match out.last_mut() {
                Some(last) if last.symbol == symbol && last.venue_mic == venue_mic => {
                    // the latest sighting's words stand for the run
                    last.last_seen = seen;
                    last.venue_name = venue_name.or(last.venue_name.take());
                    last.name = name.or(last.name.take());
                    last.source = source;
                }
                _ => out.push(Name { symbol, venue_mic, venue_name, name, first_seen: seen, last_seen: seen, source }),
            }
        }
        if out.is_empty() && self.listing_named(id)?.is_none() {
            // neither a record nor the person's pick names it: what a source's own
            // description of it does, its own first
            let sql = format!(
                "SELECT symbol, venue_mic, venue_name, name, day, source FROM instruments_described WHERE instrument_id IN {} ORDER BY instrument_id = ?1 DESC, day DESC, instrument_id LIMIT 1",
                group_sql("?1")
            );
            let found: Option<Row> = self.conn().query_row(&sql, [head.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).optional()?;
            if let Some((symbol, venue_mic, venue_name, name, day_text, source)) = found {
                let seen = text::date("instruments_described", "day", &day_text)?;
                let source = text::parsed("instruments_described", "source", &source, SourceName::parse)?;
                out.push(Name { symbol, venue_mic, venue_name, name, first_seen: seen, last_seen: seen, source });
            }
        }
        Ok(out)
    }

    /// A contract's terms as the book reads them: its own, else those of an
    /// instrument read as one with it; the underlying as the book reads it.
    pub fn option_terms(&self, id: InstrumentId) -> Result<Option<OptionTerms>> {
        let head = self.canonical(id)?;
        let sql = format!("SELECT instrument_id FROM option_terms WHERE instrument_id IN {} ORDER BY instrument_id = ?1 DESC, instrument_id LIMIT 1", group_sql("?1"));
        let held: Option<String> = self.conn().query_row(&sql, [head.to_string()], |r| r.get(0)).optional()?;
        let Some(held) = held else { return Ok(None) };
        let Some(mut terms) = self.own_option_terms(text::parsed("option_terms", "instrument_id", &held, InstrumentId::parse)?)? else { return Ok(None) };
        terms.underlying = self.canonical(terms.underlying)?;
        Ok(Some(terms))
    }

    /// The terms kept for the instrument itself, the underlying as its record named it.
    pub(crate) fn own_option_terms(&self, id: InstrumentId) -> Result<Option<OptionTerms>> {
        type Row = (String, String, String, String, Option<String>, String);
        let row: Option<Row> = self
            .conn()
            .query_row(
                "SELECT underlying_id, expiry, strike, option_right, multiplier, source FROM option_terms WHERE instrument_id = ?",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        let Some((underlying, expiry, strike, right, multiplier, source)) = row else { return Ok(None) };
        Ok(Some(OptionTerms {
            underlying: text::parsed("option_terms", "underlying_id", &underlying, InstrumentId::parse)?,
            expiry: text::date("option_terms", "expiry", &expiry)?,
            strike: text::dec("option_terms", "strike", &strike)?,
            right: text::parsed("option_terms", "option_right", &right, OptionRight::parse)?,
            multiplier: text::opt_dec("option_terms", "multiplier", multiplier)?,
            source: text::parsed("option_terms", "source", &source, SourceName::parse)?,
        }))
    }

    /// The instrument `draft` describes: found by its identifying references, or
    /// made the first time it is seen. `Err` is a problem that stops the record's
    /// transactions (a conflict between references, a kind or currency that
    /// differs from the instrument's); the `Vec` holds problems that do not.
    ///
    /// The instrument is the one the draft's own references name, before any
    /// succession joins it to another (`own_instrument`). `splits` are the
    /// instruments the repair of an earlier build's merge split off, each with
    /// the one it was merged into (`repair_merged_successions`).
    pub(crate) fn resolve_instrument(&self, draft: &InstrumentDraft, seen_by: &TransactionId, splits: Option<&Splits>, at: jiff::Timestamp) -> Result<(std::result::Result<InstrumentId, Problem>, Vec<Problem>)> {
        let source = self.record(seen_by.record)?.source;
        self.resolve_instrument_from(draft, &source, Some(seen_by), splits, at)
    }

    /// The instrument a broker's statement names that no record does (a
    /// holding a broker states with no row that brought it): found or made
    /// from the broker's own description of it, as a record's would be. A name
    /// is kept only from a record, where it was seen.
    pub fn instrument_stated(&self, draft: &InstrumentDraft, source: &SourceName, at: jiff::Timestamp) -> Result<std::result::Result<InstrumentId, Problem>> {
        self.atomically(|| {
            let found = self.resolve_instrument_from(draft, source, None, None, at)?.0;
            // the name the source's description gives it, which names it where no
            // record does; a contract is named by its terms, and the name its
            // description carries is its underlying's
            if let (Ok(id), Some(n), None) = (&found, &draft.name, &draft.option) {
                self.conn().execute(
                    "INSERT INTO instruments_described(instrument_id, source, symbol, venue_mic, venue_name, name, day) VALUES (?, ?, ?, ?, ?, ?, ?)
                     ON CONFLICT(instrument_id) DO UPDATE SET source = excluded.source, symbol = excluded.symbol, venue_mic = excluded.venue_mic,
                        venue_name = excluded.venue_name, name = excluded.name, day = excluded.day",
                    params![id.to_string(), source.as_str(), n.symbol, n.venue_mic, n.venue_name, n.name, day(n.seen)],
                )?;
            }
            self.settle_successions()?;
            Ok(found)
        })
    }

    fn resolve_instrument_from(
        &self,
        draft: &InstrumentDraft,
        source: &SourceName,
        seen_by: Option<&TransactionId>,
        splits: Option<&Splits>,
        at: jiff::Timestamp,
    ) -> Result<(std::result::Result<InstrumentId, Problem>, Vec<Problem>)> {
        let mut notes = Vec::new();
        let identifying: Vec<&Reference> = draft.refs.iter().filter(|r| r.identifies()).collect();
        if identifying.is_empty() {
            return Ok((Err(Problem::new("instrument-unidentified", "the record names an instrument by nothing that identifies it")), notes));
        }
        let mut found: Vec<(InstrumentId, &Reference)> = Vec::new();
        for r in &identifying {
            if let Some(id) = self.own_instrument(r)? {
                if !found.iter().any(|(f, _)| *f == id) {
                    found.push((id, r));
                }
            }
        }
        let id = match found.as_slice() {
            [] => {
                let id = InstrumentId::from_uuid(new_uuid(at));
                self.conn().execute(
                    "INSERT INTO instruments(id, kind, currency, created_at) VALUES (?, ?, ?, ?)",
                    params![id.to_string(), draft.kind.as_str(), draft.currency.as_str(), at_text(at)],
                )?;
                id
            }
            [(id, _)] => {
                let existing = self.instrument(*id)?;
                if existing.kind != draft.kind {
                    return Ok((Err(Problem::new("instrument-kind-conflict", format!(
                        "the record calls instrument {id} a {}, but it is a {}", draft.kind, existing.kind
                    ))), notes));
                }
                if existing.currency != draft.currency {
                    return Ok((Err(Problem::new("instrument-currency-conflict", format!(
                        "the record prices instrument {id} in {}, but it is priced in {}", draft.currency, existing.currency
                    ))), notes));
                }
                *id
            }
            [(a, ra), (b, rb), ..] => {
                return Ok((Err(Problem::new("instrument-conflict", format!(
                    "the record's references name two instruments: {} {} is {a}, {} {} is {b}", ra.scheme, ra.value, rb.scheme, rb.value
                ))), notes));
            }
        };
        // the references the instrument does not hold yet; one another holds is a
        // conflict, and nothing is merged
        for r in &draft.refs {
            match self.own_instrument(r)? {
                Some(holder) if holder == id => {}
                Some(other) => {
                    return Ok((Err(Problem::new("reference-conflict", format!("{} {} already names instrument {other}", r.scheme, r.value))), notes));
                }
                None => self.add_instrument_ref(id, r)?,
            }
        }
        if let (Some(name), Some(seen_by)) = (&draft.name, seen_by) {
            self.record_sighting(id, name, seen_by)?;
        }
        if let Some(s) = &draft.standing {
            self.note_standing(draft, s)?;
        }
        if let Some(opt) = &draft.option {
            let (underlying, more) = self.resolve_instrument_from(&opt.underlying, source, seen_by, splits, at)?;
            notes.extend(more);
            let underlying = match underlying {
                Ok(u) => u,
                Err(p) => return Ok((Err(p), notes)),
            };
            match self.own_option_terms(id)? {
                None => {
                    self.conn().execute(
                        "INSERT INTO option_terms(instrument_id, underlying_id, expiry, strike, option_right, multiplier, source) VALUES (?, ?, ?, ?, ?, ?, ?)",
                        params![
                            id.to_string(),
                            underlying.to_string(),
                            day(opt.expiry),
                            opt.strike.to_text(),
                            opt.right.as_str(),
                            opt.multiplier.map(|m| m.to_text()),
                            source.as_str()
                        ],
                    )?;
                }
                Some(terms) => {
                    // the underlying the record names, or one read as one with
                    // it; the repair of a merge puts back the one it names
                    let split_back = splits.is_some_and(|s| s.get(&underlying) == Some(&terms.underlying));
                    let same_underlying = terms.underlying == underlying || split_back || self.canonical(terms.underlying)? == self.canonical(underlying)?;
                    let same = same_underlying && terms.expiry == opt.expiry && terms.strike == opt.strike && terms.right == opt.right;
                    if !same {
                        return Ok((Err(Problem::new("option-terms-conflict", format!("the record states other terms for option {id} than the book holds"))), notes));
                    }
                    if split_back {
                        self.conn().execute("UPDATE option_terms SET underlying_id = ? WHERE instrument_id = ?", params![underlying.to_string(), id.to_string()])?;
                    }
                    match (terms.multiplier, opt.multiplier) {
                        (None, Some(m)) => {
                            self.conn().execute("UPDATE option_terms SET multiplier = ? WHERE instrument_id = ?", params![m.to_text(), id.to_string()])?;
                        }
                        (Some(a), Some(b)) if a != b => {
                            return Ok((Err(Problem::new("option-terms-conflict", format!("the record states a multiplier of {b} for option {id}, the book holds {a}"))), notes));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok((Ok(id), notes))
    }

    /// Keep what a source states of one of its own ids, with the listing it
    /// named as it said so.
    fn note_standing(&self, draft: &InstrumentDraft, s: &StandingDraft) -> Result<()> {
        let (symbol, venue_mic) = match &draft.name {
            Some(n) => (Some(n.symbol.as_str()), n.venue_mic.as_deref()),
            None => (None, None),
        };
        self.conn().execute(
            "INSERT INTO security_standings(scheme, value, standing, kind, currency, symbol, venue_mic) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(scheme, value, standing) DO UPDATE SET kind = excluded.kind, currency = excluded.currency, symbol = excluded.symbol, venue_mic = excluded.venue_mic",
            params![s.of.scheme.to_text(), s.of.value, s.standing.as_str(), draft.kind.as_str(), draft.currency.as_str(), symbol, venue_mic],
        )?;
        Ok(())
    }

    /// Decide again which instruments are one by a source's own succession
    /// (`docs/architecture.md` §5, "Matching across sources"), from what the
    /// records and standings state now: an id the source states retired by a
    /// corporate action and the one id of the same source stated live, never
    /// stated retired, with the retired id's symbol, venue and currency (the
    /// listing trading on under a new id) are read as one instrument. With no
    /// such live id or several, or where a corporate event row gives up the
    /// retired id's units (a consolidation, a split, a change of code states
    /// its own succession), they stay two. Nothing is joined on a symbol alone:
    /// the source's own word that the id was retired is what joins them.
    ///
    /// Nothing is merged: each id keeps its own instrument, and everything kept
    /// under it stays there; the book reads a joined instrument as the one
    /// first seen of those it is joined with. A later row that states the
    /// succession otherwise (the event row arriving after the ids were joined)
    /// parts them again here, and the book reads as if they had never been
    /// joined. Run after every write that can change what it reads.
    pub fn settle_successions(&self) -> Result<Successions> {
        self.atomically(|| {
            let retired: Vec<(String, String, String, String, String, String)> = {
                let mut stmt = self.conn().prepare_cached(
                    "SELECT scheme, value, kind, currency, symbol, venue_mic FROM security_standings
                     WHERE standing = 'retired-by-event' AND symbol IS NOT NULL AND venue_mic IS NOT NULL ORDER BY scheme, value",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            // each pair of instruments a succession makes one, then each set of
            // instruments the pairs connect
            let mut parent: BTreeMap<InstrumentId, InstrumentId> = BTreeMap::new();
            fn root(parent: &BTreeMap<InstrumentId, InstrumentId>, mut x: InstrumentId) -> InstrumentId {
                while let Some(p) = parent.get(&x).filter(|p| **p != x) {
                    x = *p;
                }
                x
            }
            for (scheme, value, kind, currency, symbol, venue_mic) in retired {
                let successors: Vec<String> = {
                    let mut stmt = self.conn().prepare_cached(
                        "SELECT s.value FROM security_standings s
                         WHERE s.scheme = ? AND s.standing = 'live' AND s.kind = ? AND s.currency = ? AND s.symbol = ? AND s.venue_mic = ? AND s.value <> ?
                           AND NOT EXISTS (SELECT 1 FROM security_standings r WHERE r.scheme = s.scheme AND r.value = s.value AND r.standing = 'retired-by-event')
                         ORDER BY s.value",
                    )?;
                    let rows = stmt.query_map(params![scheme, kind, currency, symbol, venue_mic, value], |r| r.get(0))?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                let [successor] = successors.as_slice() else { continue };
                let scheme = text::parsed("security_standings", "scheme", &scheme, RefScheme::parse)?;
                let Some(old) = self.own_instrument(&Reference::new(scheme.clone(), value))? else { continue };
                let Some(new) = self.own_instrument(&Reference::new(scheme, successor.clone()))? else { continue };
                if old == new || self.event_gives_up(old)? {
                    continue;
                }
                let (a, b) = (self.instrument(old)?, self.instrument(new)?);
                if a.kind != b.kind || a.currency != b.currency {
                    continue;
                }
                let (ra, rb) = (root(&parent, old), root(&parent, new));
                parent.entry(ra).or_insert(ra);
                parent.entry(rb).or_insert(rb);
                if ra != rb {
                    parent.insert(ra.max(rb), ra.min(rb));
                }
            }
            let mut sets: BTreeMap<InstrumentId, Vec<InstrumentId>> = BTreeMap::new();
            for x in parent.keys() {
                sets.entry(root(&parent, *x)).or_default().push(*x);
            }
            // each set is read as the instrument of it first seen
            let mut wanted: BTreeMap<InstrumentId, InstrumentId> = BTreeMap::new();
            for members in sets.into_values() {
                let mut seen = Vec::new();
                for m in &members {
                    let (created, rowid): (String, i64) =
                        self.conn().query_row("SELECT created_at, rowid FROM instruments WHERE id = ?", [m.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
                    seen.push((text::instant("instruments", "created_at", &created)?, rowid, *m));
                }
                seen.sort();
                let head = seen[0].2;
                for (_, _, m) in &seen[1..] {
                    wanted.insert(*m, head);
                }
            }
            let held = self.joined()?;
            if wanted == held {
                return Ok(Successions::default());
            }
            self.conn().execute("DELETE FROM instrument_joins", [])?;
            for (m, head) in &wanted {
                self.conn().execute("INSERT INTO instrument_joins(instrument_id, into_id) VALUES (?, ?)", params![m.to_string(), head.to_string()])?;
            }
            let joined: Vec<(InstrumentId, InstrumentId)> = wanted.iter().filter(|(m, h)| held.get(m) != Some(h)).map(|(m, h)| (*h, *m)).collect();
            let parted: Vec<(InstrumentId, InstrumentId)> = held.iter().filter(|(m, h)| wanted.get(m) != Some(h)).map(|(m, h)| (*h, *m)).collect();
            // a contract whose records name its underlying by two ids that are
            // now two instruments again is derived again at the next derivation,
            // where a record stating the other one is a conflict of terms
            for (h, m) in &parted {
                self.conn().execute(
                    "UPDATE source_records SET derived_version = 0 WHERE state = 'live' AND id IN
                        (SELECT t.record_id FROM transactions t JOIN option_terms o ON o.instrument_id = t.instrument_id WHERE o.underlying_id IN (?1, ?2))",
                    params![h.to_string(), m.to_string()],
                )?;
            }
            Ok(Successions { joined, parted })
        })
    }

    /// Whether a corporate event row gives up units of the instrument: its
    /// succession is then what that row states.
    fn event_gives_up(&self, id: InstrumentId) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM transactions WHERE instrument_id = ? AND kind = 'corporate-event' AND quantity LIKE '-%')",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }

    /// Put apart what an earlier build merged for a succession, once: that build
    /// made one instrument of a retired id and its successor by moving every
    /// reference, transaction, statement line and choice of the one onto the
    /// other and deleting it, so a corporate event row arriving later could not
    /// part them. Each broker id beyond the first on one instrument gets its own
    /// instrument again, the records on the merged one are derived again under
    /// `mappings` (each leg lands on the instrument of the id it names, its trade
    /// following it), and the successions are settled from the records. What the
    /// merge moved without saying from where (a watch, a tile, a fact) stays on
    /// the instrument first seen, which the book reads it as while the two are
    /// one; a statement of units it added up under two instruments now apart is
    /// dropped for the next pull to state again.
    pub fn repair_merged_successions(&self, mappings: &[&dyn Mapping], at: jiff::Timestamp) -> Result<()> {
        const REPAIR: &str = "successions-unmerged";
        if self.repaired(REPAIR)? {
            return Ok(());
        }
        self.atomically(|| {
            let merged: Vec<(String, String)> = {
                let mut stmt = self.conn().prepare(
                    "SELECT instrument_id, scheme FROM instrument_refs WHERE scheme LIKE 'broker-security:%'
                     GROUP BY instrument_id, scheme HAVING COUNT(*) > 1 ORDER BY instrument_id, scheme",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            let mut splits: Splits = BTreeMap::new();
            for (instrument, scheme) in &merged {
                let into = text::parsed("instrument_refs", "instrument_id", instrument, InstrumentId::parse)?;
                let held = self.instrument(into)?;
                // the id the instrument was made for was added first
                let values: Vec<String> = {
                    let mut stmt = self.conn().prepare("SELECT value FROM instrument_refs WHERE instrument_id = ? AND scheme = ? ORDER BY rowid")?;
                    let rows = stmt.query_map(params![instrument, scheme], |r| r.get(0))?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                for value in &values[1..] {
                    let id = InstrumentId::from_uuid(new_uuid(at));
                    self.conn().execute(
                        "INSERT INTO instruments(id, kind, currency, created_at) VALUES (?, ?, ?, ?)",
                        params![id.to_string(), held.kind.as_str(), held.currency.as_str(), at_text(at)],
                    )?;
                    self.conn().execute("UPDATE instrument_refs SET instrument_id = ? WHERE scheme = ? AND value = ?", params![id.to_string(), scheme, value])?;
                    splits.insert(id, into);
                }
            }
            if !splits.is_empty() {
                let into: BTreeSet<InstrumentId> = splits.values().copied().collect();
                let mut records: Vec<(String, Option<String>, String)> = Vec::new();
                for i in &into {
                    let mut stmt = self.conn().prepare(
                        "SELECT r.id, r.connection_id, r.source FROM source_records r WHERE r.state = 'live' AND (
                            EXISTS (SELECT 1 FROM transactions t WHERE t.record_id = r.id
                                    AND (t.instrument_id = ?1 OR t.instrument_id IN (SELECT instrument_id FROM option_terms WHERE underlying_id = ?1)))
                            OR EXISTS (SELECT 1 FROM instrument_sightings s WHERE s.record_id = r.id AND s.instrument_id = ?1)
                            OR EXISTS (SELECT 1 FROM adjustment_legs l WHERE l.record_id = r.id AND (l.from_instrument = ?1 OR l.to_instrument = ?1)))
                         ORDER BY r.first_received_at, r.rowid",
                    )?;
                    let rows = stmt.query_map([i.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                    for row in rows {
                        let row = row?;
                        if !records.contains(&row) {
                            records.push(row);
                        }
                    }
                }
                for (id, connection, source) in records {
                    let record = text::parsed("source_records", "id", &id, bagholder_core::RecordId::parse)?;
                    let connection = text::opt_parsed("source_records", "connection_id", connection, ConnectionId::parse)?;
                    let Some(mapping) = mappings.iter().find(|m| m.source().as_str() == source) else {
                        return Err(BookError::Refused(format!("record {record} is {source}'s, and no mapping of it was given to derive it again")));
                    };
                    let (payload, _) = self.latest_revision(record)?;
                    self.derive(record, connection, &payload, *mapping, Some(&splits), at)?;
                }
                // a trade anchored on the merged instrument through an underlying
                // or an adjustment moves to the one its opening names now
                let anchored: Vec<(String, String)> = {
                    let mut stmt = self.conn().prepare("SELECT id, anchor_instrument FROM trades WHERE anchor_instrument IS NOT NULL")?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                for (trade, anchor) in anchored {
                    let anchor = text::parsed("trades", "anchor_instrument", &anchor, InstrumentId::parse)?;
                    if !into.contains(&anchor) {
                        continue;
                    }
                    let trade = text::parsed("trades", "id", &trade, bagholder_core::TradeId::parse)?;
                    let bagholder_core::journal::Anchor::Opening(opening) = self.trade(trade)?.anchor else { continue };
                    let named = self.opening_instruments(&opening.transaction)?;
                    if named.contains(&anchor) {
                        continue;
                    }
                    let moved: Vec<&InstrumentId> = named.iter().filter(|n| splits.get(n) == Some(&anchor)).collect();
                    if let [to] = moved.as_slice() {
                        self.conn().execute("UPDATE trades SET anchor_instrument = ? WHERE id = ?", params![to.to_string(), trade.to_string()])?;
                    }
                }
            }
            self.settle_successions()?;
            // a statement of units the merge added up under instruments now apart
            // again cannot be split: which id each unit was stated under is not
            // kept. It is dropped, so the next pull states the holdings anew
            // rather than the check reading the sum against one of them.
            for (split, into) in &splits {
                if self.canonical(*split)? != self.canonical(*into)? {
                    let merged: Vec<String> = {
                        let mut stmt = self.conn().prepare("SELECT DISTINCT statement_id FROM statement_units WHERE instrument_id = ?")?;
                        let rows = stmt.query_map([into.to_string()], |r| r.get(0))?;
                        rows.collect::<rusqlite::Result<_>>()?
                    };
                    for statement in merged {
                        self.conn().execute("DELETE FROM statement_units WHERE statement_id = ?", [&statement])?;
                        self.conn().execute("DELETE FROM statements WHERE id = ?", [&statement])?;
                    }
                }
            }
            self.record_repair(REPAIR, at)
        })
    }

    /// Note that a record's leg called the instrument this, on its day.
    fn record_sighting(&self, id: InstrumentId, n: &NameDraft, seen_by: &TransactionId) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO instrument_sightings(record_id, leg, instrument_id, symbol, venue_mic, venue_name, name, day) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![seen_by.record.to_string(), seen_by.leg.as_str(), id.to_string(), n.symbol, n.venue_mic, n.venue_name, n.name, day(n.seen)],
        )?;
        Ok(())
    }
}

/// The instruments read as one with the instrument whose id is bound to `param`
/// (the one they are read as): itself and every one joined into it, as SQL.
pub(crate) fn group_sql(param: &str) -> String {
    format!("(SELECT {param} UNION SELECT instrument_id FROM instrument_joins WHERE into_id = {param})")
}

/// The instruments a repair split off an earlier build's merge, each with the
/// one it had been merged into (`Book::repair_merged_successions`).
pub(crate) type Splits = BTreeMap<InstrumentId, InstrumentId>;

/// What settling the successions changed: pairs (the instrument read as, the
/// one read as it) newly joined, and those parted again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Successions {
    pub joined: Vec<(InstrumentId, InstrumentId)>,
    pub parted: Vec<(InstrumentId, InstrumentId)>,
}
