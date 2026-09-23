//! Identity in the book (`docs/architecture.md` §5): broker connections, accounts
//! and their broker ids, instruments with their references, dated names and
//! option terms, and issuers.
//!
//! An instrument is found only by a reference that identifies it (a strong one,
//! or a connection-scoped one within its connection), never by a bare symbol.
//! When a record's references name two different instruments, or a reference is
//! already another instrument's, nothing is merged and nothing is picked: the
//! record gets a problem.

use rusqlite::{params, OptionalExtension};

use bagholder_core::account::{Account, AccountKind, AccountRef, AccountStatus, AccountType, Connection, Registration};
use bagholder_core::instrument::{Instrument, InstrumentKind, Issuer, Name, OptionRight, OptionTerms, RefScheme, Reference};
use bagholder_core::record::Problem;
use bagholder_core::{AccountId, Broker, ConnectionId, InstrumentId, IssuerId, SourceName, TransactionId};

use crate::mapping::{InstrumentDraft, NameDraft};
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

    pub fn instruments(&self) -> Result<Vec<Instrument>> {
        let mut stmt = self.conn().prepare_cached("SELECT id FROM instruments ORDER BY created_at, rowid")?;
        let ids: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        ids.iter().map(|s| self.instrument(text::parsed("instruments", "id", s, InstrumentId::parse)?)).collect()
    }

    /// The instrument an identifying reference names, if any. A routing reference
    /// names none: it only says how to ask a source (`instrument_routes`).
    pub fn instrument_by_ref(&self, r: &Reference) -> Result<Option<InstrumentId>> {
        if !r.identifies() {
            return Ok(None);
        }
        let found: Option<String> = self
            .conn()
            .query_row("SELECT instrument_id FROM instrument_refs WHERE scheme = ? AND value = ?", params![r.scheme.to_text(), r.value], |row| row.get(0))
            .optional()?;
        found.map(|s| text::parsed("instrument_refs", "instrument_id", &s, InstrumentId::parse)).transpose()
    }

    /// Every reference an instrument has: those that identify it, then the ways to
    /// ask sources for it.
    pub fn instrument_refs(&self, id: InstrumentId) -> Result<Vec<Reference>> {
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

    /// The instruments a routing reference leads to (it may be several).
    pub fn instruments_routed_by(&self, r: &Reference) -> Result<Vec<InstrumentId>> {
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id FROM instrument_routes WHERE scheme = ? AND value = ? ORDER BY instrument_id")?;
        let ids = stmt.query_map(params![r.scheme.to_text(), r.value], |row| row.get::<_, String>(0))?;
        ids.map(|s| text::parsed("instrument_routes", "instrument_id", &s?, InstrumentId::parse)).collect()
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
        match self.instrument_by_ref(r)? {
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
    pub fn names(&self, id: InstrumentId) -> Result<Vec<Name>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT s.symbol, s.venue_mic, s.venue_name, s.name, s.day, r.source FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live'
             WHERE s.instrument_id = ? ORDER BY s.day, s.symbol, s.rowid",
        )?;
        type Row = (String, Option<String>, Option<String>, Option<String>, String, String);
        let rows = stmt.query_map([id.to_string()], |r| -> rusqlite::Result<Row> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)) })?;
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
        Ok(out)
    }

    pub fn option_terms(&self, id: InstrumentId) -> Result<Option<OptionTerms>> {
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
    pub(crate) fn resolve_instrument(&self, draft: &InstrumentDraft, seen_by: &TransactionId, at: jiff::Timestamp) -> Result<(std::result::Result<InstrumentId, Problem>, Vec<Problem>)> {
        let source = &self.record(seen_by.record)?.source;
        let mut notes = Vec::new();
        let identifying: Vec<&Reference> = draft.refs.iter().filter(|r| r.identifies()).collect();
        if identifying.is_empty() {
            return Ok((Err(Problem::new("instrument-unidentified", "the record names an instrument by nothing that identifies it")), notes));
        }
        let mut found: Vec<(InstrumentId, &Reference)> = Vec::new();
        for r in &identifying {
            if let Some(id) = self.instrument_by_ref(r)? {
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
            match self.instrument_by_ref(r)? {
                Some(holder) if holder == id => {}
                Some(other) => {
                    return Ok((Err(Problem::new("reference-conflict", format!("{} {} already names instrument {other}", r.scheme, r.value))), notes));
                }
                None => self.add_instrument_ref(id, r)?,
            }
        }
        if let Some(name) = &draft.name {
            self.record_sighting(id, name, seen_by)?;
        }
        if let Some(opt) = &draft.option {
            let (underlying, more) = self.resolve_instrument(&opt.underlying, seen_by, at)?;
            notes.extend(more);
            let underlying = match underlying {
                Ok(u) => u,
                Err(p) => return Ok((Err(p), notes)),
            };
            match self.option_terms(id)? {
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
                    let same = terms.underlying == underlying && terms.expiry == opt.expiry && terms.strike == opt.strike && terms.right == opt.right;
                    if !same {
                        return Ok((Err(Problem::new("option-terms-conflict", format!("the record states other terms for option {id} than the book holds"))), notes));
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

    /// Note that a record's leg called the instrument this, on its day.
    fn record_sighting(&self, id: InstrumentId, n: &NameDraft, seen_by: &TransactionId) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO instrument_sightings(record_id, leg, instrument_id, symbol, venue_mic, venue_name, name, day) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![seen_by.record.to_string(), seen_by.leg.as_str(), id.to_string(), n.symbol, n.venue_mic, n.venue_name, n.name, day(n.seen)],
        )?;
        Ok(())
    }
}
