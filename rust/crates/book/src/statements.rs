//! What a broker states beside its rows (`docs/plans/stage-3b-wealthsimple.md`,
//! "The book's new tables"): each read, the links between accounts, each
//! account's value per day, its cash and units at a moment, when its activity
//! was last read in full, and the two sides of a move of holdings. Kept as the
//! broker stated them: a later statement is kept beside an earlier one, never
//! over it, and the newest is the one read.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};

use bagholder_core::{AccountId, ConnectionId, Currency, Dec, InstrumentId, Leg, Money, RecordId, TransactionId};

use crate::text::{self, at as at_text, day};
use crate::{new_uuid, Book, Result};

/// One read of a broker: what the rows it stated came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadId(pub String);

/// An account's value and net deposits on a day, as the broker states them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountDay {
    pub day: jiff::civil::Date,
    pub net_value: Money,
    pub net_deposits: Money,
}

/// What the broker last stated about one account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stated {
    /// Each day's value and net deposits, from the newest read of the day.
    pub days: BTreeMap<jiff::civil::Date, (Money, Money)>,
    /// The newest statement of cash, and when it was stated.
    pub cash: Option<(jiff::Timestamp, BTreeMap<Currency, Dec>)>,
    /// The newest statement of cash made at or before the last full read of the
    /// activity: every fill it reflects is on the record, so the book's cash is
    /// checked against it.
    pub cash_read: Option<(jiff::Timestamp, BTreeMap<Currency, Dec>)>,
    /// What the broker held against the account when it stated `cash_read`.
    pub cash_read_holds: Vec<bagholder_core::hold::Hold>,
    /// The newest statement of what the account is worth now, and when.
    pub net_value_now: Option<(jiff::Timestamp, Money)>,
    /// The newest statement of units, and the day they are as of.
    pub units: Option<(jiff::civil::Date, BTreeMap<InstrumentId, Dec>)>,
    /// What that statement states each instrument's units are worth, where it
    /// states every line of it in one currency.
    pub unit_values: BTreeMap<InstrumentId, Money>,
    /// When the account's activity was last read in full.
    pub activity_read_at: Option<jiff::Timestamp>,
    /// The newest statement of what it can borrow, when it was stated: an
    /// amount, or the broker's reason it cannot say.
    pub buying_power: Option<(jiff::Timestamp, std::result::Result<Money, String>)>,
}

/// A position as a statement of units states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitsLine {
    pub instrument: InstrumentId,
    pub quantity: Dec,
    /// The broker's own book value, kept as its statement, never a cost.
    pub book_value: Option<Money>,
    /// What the broker states the position is worth.
    pub value: Option<Money>,
}

impl Book {
    /// Note a read of `part` (`accounts`, `activity:<account>`, …).
    pub fn broker_read(&self, connection: ConnectionId, part: &str, at: jiff::Timestamp) -> Result<ReadId> {
        let id = new_uuid(at).to_string();
        self.conn().execute("INSERT INTO broker_reads (id, connection_id, part, at) VALUES (?1, ?2, ?3, ?4)", params![id, connection.to_string(), part, at_text(at)])?;
        Ok(ReadId(id))
    }

    /// When `part` of a connection was last read, if ever.
    pub fn last_read(&self, connection: ConnectionId, part: &str) -> Result<Option<jiff::Timestamp>> {
        let at: Option<String> = self.conn().query_row("SELECT MAX(at) FROM broker_reads WHERE connection_id = ?1 AND part = ?2", params![connection.to_string(), part], |r| r.get(0))?;
        at.map(|t| text::instant("broker_reads", "at", &t)).transpose()
    }

    /// Keep an account's statement for the month starting `month`, as the
    /// broker issued it. A month is read once and kept: a second copy of the
    /// same month is not stored.
    pub fn keep_monthly_statement(&self, connection: ConnectionId, account_key: &str, month: jiff::civil::Date, payload: &str, read: &ReadId) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO monthly_statements (connection_id, account_key, month, payload, read_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![connection.to_string(), account_key, month.to_string(), payload, read.0],
        )?;
        Ok(())
    }

    /// Every statement kept for one of the broker's accounts, by month, oldest first.
    pub fn monthly_statements(&self, connection: ConnectionId, account_key: &str) -> Result<Vec<(jiff::civil::Date, String)>> {
        let mut stmt = self.conn().prepare("SELECT month, payload FROM monthly_statements WHERE connection_id = ?1 AND account_key = ?2 ORDER BY month")?;
        let rows = stmt.query_map(params![connection.to_string(), account_key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (m, p) = r?;
            out.push((text::parsed("monthly_statements", "month", &m, |d: &str| d.parse::<jiff::civil::Date>())?, p));
        }
        Ok(out)
    }

    /// That `account` is linked to `to`, as the broker states it.
    pub fn link_accounts(&self, account: AccountId, to: AccountId, read: &ReadId) -> Result<()> {
        self.conn().execute("INSERT OR IGNORE INTO account_links (account_id, linked_to, read_id) VALUES (?1, ?2, ?3)", params![account.to_string(), to.to_string(), read.0])?;
        Ok(())
    }

    /// The accounts of a connection that back a margin account, as one read of the
    /// accounts states them: what it no longer states is no longer so.
    pub fn store_margin_backing(&self, connection: ConnectionId, backing: &[(AccountId, AccountId)], read: &ReadId) -> Result<()> {
        self.atomically(|| {
            self.conn().execute(
                "DELETE FROM margin_backing WHERE account_id IN (SELECT id FROM accounts WHERE connection_id = ?1)",
                params![connection.to_string()],
            )?;
            for (account, margin) in backing {
                self.conn().execute(
                    "INSERT INTO margin_backing (account_id, margin_account_id, read_id) VALUES (?1, ?2, ?3)",
                    params![account.to_string(), margin.to_string(), read.0],
                )?;
            }
            Ok(())
        })
    }

    /// Every statement of an account's units read after `since`, newest first:
    /// when it was read, and the units of `instrument` it states (none where it
    /// does not list it), with those of every instrument read as one with it.
    pub fn units_reads(&self, account: AccountId, instrument: InstrumentId, since: jiff::Timestamp) -> Result<Vec<(jiff::Timestamp, Option<Dec>)>> {
        let head = self.canonical(instrument)?;
        let mut st = self.conn().prepare(&format!(
            "SELECT r.at, s.id, u.quantity FROM statements s JOIN broker_reads r ON r.id = s.read_id
             LEFT JOIN statement_units u ON u.statement_id = s.id AND u.instrument_id IN {}
             WHERE s.account_id = ?1 AND s.kind = 'units' AND r.at > ?3 ORDER BY r.at DESC, s.id DESC",
            crate::identity::group_sql("?2")
        ))?;
        let rows = st.query_map(params![account.to_string(), head.to_string(), at_text(since)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
        })?;
        let mut out: Vec<(String, jiff::Timestamp, Option<Dec>)> = Vec::new();
        for row in rows {
            let (at, statement, q) = row?;
            let q = q.map(|q| text::dec("statement_units", "quantity", &q)).transpose()?;
            match out.last_mut() {
                Some((id, _, held)) if *id == statement => {
                    if let Some(q) = q {
                        *held = Some(held.unwrap_or(Dec::ZERO).checked_add(q).map_err(|e| crate::BookError::Refused(format!("a statement's units too large to add: {e}")))?);
                    }
                }
                _ => out.push((statement, text::instant("broker_reads", "at", &at)?, q)),
            }
        }
        Ok(out.into_iter().map(|(_, at, q)| (at, q)).collect())
    }

    /// Each account that backs a margin account, and the margin account.
    pub fn margin_backing(&self) -> Result<BTreeMap<AccountId, AccountId>> {
        let mut st = self.conn().prepare("SELECT account_id, margin_account_id FROM margin_backing ORDER BY account_id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|r| {
            let (a, b) = r?;
            Ok((text::parsed("margin_backing", "account_id", &a, AccountId::parse)?, text::parsed("margin_backing", "margin_account_id", &b, AccountId::parse)?))
        })
        .collect()
    }

    pub fn account_links(&self) -> Result<Vec<(AccountId, AccountId)>> {
        let mut st = self.conn().prepare("SELECT account_id, linked_to FROM account_links ORDER BY account_id, linked_to")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|r| {
            let (a, b) = r?;
            Ok((text::parsed("account_links", "account_id", &a, AccountId::parse)?, text::parsed("account_links", "linked_to", &b, AccountId::parse)?))
        })
        .collect()
    }

    /// Store the days a read stated. A day stated as before writes nothing; a
    /// day stated differently is kept beside the earlier statement. Returns the
    /// days whose value changed.
    pub fn store_account_days(&self, account: AccountId, days: &[AccountDay], read: &ReadId) -> Result<Vec<jiff::civil::Date>> {
        self.atomically(|| {
            let mut changed = Vec::new();
            for d in days {
                let newest: Option<(String, String, String)> = self
                    .conn()
                    .query_row(
                        "SELECT net_value, net_deposits, currency FROM account_days a JOIN broker_reads r ON r.id = a.read_id WHERE account_id = ?1 AND day = ?2 ORDER BY r.at DESC, r.id DESC LIMIT 1",
                        params![account.to_string(), day(d.day)],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let this = (d.net_value.amount.to_text(), d.net_deposits.amount.to_text(), d.net_value.currency.to_string());
                if newest.as_ref() == Some(&this) {
                    continue;
                }
                if newest.is_some() {
                    changed.push(d.day);
                }
                self.conn().execute(
                    "INSERT INTO account_days (account_id, day, net_value, net_deposits, currency, read_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![account.to_string(), day(d.day), this.0, this.1, this.2, read.0],
                )?;
            }
            Ok(changed)
        })
    }

    /// The last day whose value is stored for `account`.
    pub fn last_account_day(&self, account: AccountId) -> Result<Option<jiff::civil::Date>> {
        let d: Option<String> = self.conn().query_row("SELECT MAX(day) FROM account_days WHERE account_id = ?1", params![account.to_string()], |r| r.get(0))?;
        d.map(|s| text::date("account_days", "day", &s)).transpose()
    }

    /// Store a statement of cash: each currency's amount, stated at `at`.
    pub fn store_cash(&self, account: AccountId, stated_at: jiff::Timestamp, cash: &BTreeMap<Currency, Dec>, read: &ReadId) -> Result<()> {
        self.atomically(|| {
            let id = new_uuid(stated_at).to_string();
            self.conn().execute("INSERT INTO statements (id, account_id, kind, stated_at, read_id) VALUES (?1, ?2, 'cash', ?3, ?4)", params![id, account.to_string(), at_text(stated_at), read.0])?;
            for (c, a) in cash {
                self.conn().execute("INSERT INTO statement_cash (statement_id, currency, amount) VALUES (?1, ?2, ?3)", params![id, c.to_string(), a.to_text()])?;
            }
            // what its live records hold against the account as the cash is stated
            self.conn().execute(
                "INSERT INTO statement_holds (statement_id, record_id, kind, currency, instrument_id, amount, quantity, premium)
                 SELECT ?1, h.record_id, h.kind, h.currency, h.instrument_id, h.amount, h.quantity, h.premium
                 FROM record_holds h JOIN source_records r ON r.id = h.record_id AND r.state = 'live' WHERE h.account_id = ?2",
                params![id, account.to_string()],
            )?;
            // a working order with a fill against it already holds only its rest, which
            // its row does not state: a fill the app booked from its own order, or
            // another live record of the same order that moved anything (brief 18)
            let mut st = self.conn().prepare(
                "SELECT DISTINCT s.record_id, o.filled FROM statement_holds s
                 JOIN record_orders ro ON ro.record_id = s.record_id
                 LEFT JOIN orders o ON o.id = ro.order_id OR o.broker_id = ro.order_id
                 WHERE s.statement_id = ?1 AND s.amount IS NOT NULL AND (o.id IS NOT NULL OR EXISTS (
                     SELECT 1 FROM record_orders other JOIN source_records r ON r.id = other.record_id AND r.state = 'live'
                     JOIN transactions t ON t.record_id = other.record_id
                     WHERE other.order_id = ro.order_id AND other.record_id <> s.record_id))",
            )?;
            let rows: Vec<(String, Option<String>)> = st.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            for (record, filled) in rows {
                let filled_some = match filled {
                    Some(f) => text::dec("orders", "filled", &f)?.is_positive(),
                    // no order of the app's: another record of the order moved something
                    None => true,
                };
                if filled_some {
                    self.conn().execute("UPDATE statement_holds SET amount = NULL WHERE statement_id = ?1 AND record_id = ?2", params![id, record])?;
                }
            }
            Ok(())
        })
    }

    /// Store what the account can borrow as the broker states it now: an amount,
    /// or its reason it cannot say.
    pub fn store_buying_power(&self, account: AccountId, stated_at: jiff::Timestamp, stated: &std::result::Result<Money, String>, read: &ReadId) -> Result<()> {
        let (amount, currency, why) = match stated {
            Ok(m) => (Some(m.amount.to_text()), Some(m.currency.to_string()), None),
            Err(w) => (None, None, Some(w.as_str())),
        };
        self.conn().execute(
            "INSERT INTO buying_power (account_id, stated_at, amount, currency, unavailable, read_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![account.to_string(), at_text(stated_at), amount, currency, why, read.0],
        )?;
        Ok(())
    }

    /// Store what the broker states an account is worth now.
    pub fn store_net_value(&self, account: AccountId, at: jiff::Timestamp, value: Money, read: &ReadId) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO account_values (account_id, stated_at, amount, currency, read_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![account.to_string(), text::at(at), value.amount.to_text(), value.currency.to_string(), read.0],
        )?;
        Ok(())
    }

    /// Store a statement of units as of a day.
    pub fn store_units(&self, account: AccountId, as_of: jiff::civil::Date, lines: &[UnitsLine], read: &ReadId, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            let id = new_uuid(at).to_string();
            self.conn().execute("INSERT INTO statements (id, account_id, kind, as_of_day, read_id) VALUES (?1, ?2, 'units', ?3, ?4)", params![id, account.to_string(), day(as_of), read.0])?;
            for l in lines {
                self.conn().execute(
                    "INSERT INTO statement_units (statement_id, instrument_id, quantity, book_value, book_value_currency, value, value_currency) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, l.instrument.to_string(), l.quantity.to_text(), l.book_value.map(|m| m.amount.to_text()), l.book_value.map(|m| m.currency.to_string()), l.value.map(|m| m.amount.to_text()), l.value.map(|m| m.currency.to_string())],
                )?;
            }
            Ok(())
        })
    }

    /// Note a read of an account's activity: `complete` when every page of it
    /// was read without a failure.
    pub fn note_activity_read(&self, account: AccountId, read_at: jiff::Timestamp, complete: bool) -> Result<()> {
        self.conn().execute("INSERT OR REPLACE INTO activity_reads (account_id, read_at, complete) VALUES (?1, ?2, ?3)", params![account.to_string(), at_text(read_at), complete as i64])?;
        Ok(())
    }

    /// When the account's activity was last read in full.
    pub fn activity_read_at(&self, account: AccountId) -> Result<Option<jiff::Timestamp>> {
        let t: Option<String> = self.conn().query_row("SELECT MAX(read_at) FROM activity_reads WHERE account_id = ?1 AND complete = 1", params![account.to_string()], |r| r.get(0))?;
        t.map(|s| text::instant("activity_reads", "read_at", &s)).transpose()
    }

    /// That `out` and `into` are the two sides of one move of holdings.
    pub fn link_transfer(&self, out: &TransactionId, into: &TransactionId) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO transfer_links (out_record, out_leg, in_record, in_leg) VALUES (?1, ?2, ?3, ?4)",
            params![out.record.to_string(), out.leg.as_str(), into.record.to_string(), into.leg.as_str()],
        )?;
        Ok(())
    }

    pub fn transfer_links(&self) -> Result<Vec<(TransactionId, TransactionId)>> {
        let mut st = self.conn().prepare("SELECT out_record, out_leg, in_record, in_leg FROM transfer_links ORDER BY out_record, out_leg")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        rows.map(|r| {
            let (a, b, c, d) = r?;
            let t = "transfer_links";
            Ok((
                TransactionId::new(text::parsed(t, "out_record", &a, RecordId::parse)?, text::parsed(t, "out_leg", &b, Leg::parse)?),
                TransactionId::new(text::parsed(t, "in_record", &c, RecordId::parse)?, text::parsed(t, "in_leg", &d, Leg::parse)?),
            ))
        })
        .collect()
    }

    /// What the broker held against an account when it stated the cash of `statement`.
    fn statement_holds(&self, statement: &str) -> Result<Vec<bagholder_core::hold::Hold>> {
        let t = "statement_holds";
        let mut st = self.conn().prepare_cached("SELECT record_id, kind, currency, instrument_id, amount, quantity, premium FROM statement_holds WHERE statement_id = ?1 ORDER BY record_id")?;
        let rows = st.query_map(params![statement], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?, r.get::<_, Option<String>>(6)?))
        })?;
        let dec = |col: &'static str, v: Option<String>| v.map(|v| text::dec(t, col, &v)).transpose();
        rows.map(|r| {
            let (record, kind, currency, instrument, amount, quantity, premium) = r?;
            Ok(bagholder_core::hold::Hold {
                record: text::parsed(t, "record_id", &record, RecordId::parse)?,
                kind: text::parsed(t, "kind", &kind, bagholder_core::hold::HoldKind::parse)?,
                currency: currency.map(|c| text::currency(t, "currency", &c)).transpose()?,
                instrument: instrument.map(|i| text::parsed(t, "instrument_id", &i, InstrumentId::parse)).transpose()?,
                amount: dec("amount", amount)?,
                quantity: dec("quantity", quantity)?,
                premium: dec("premium", premium)?,
            })
        })
        .collect()
    }

    /// What the broker last stated about `account`.
    pub fn stated(&self, account: AccountId) -> Result<Stated> {
        let a = account.to_string();
        let mut out = Stated::default();
        // each day's newest statement
        let mut st = self.conn().prepare(
            "SELECT d.day, d.net_value, d.net_deposits, d.currency FROM account_days d JOIN broker_reads r ON r.id = d.read_id WHERE d.account_id = ?1 ORDER BY d.day, r.at, r.id",
        )?;
        let rows = st.query_map(params![a], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        for r in rows {
            let (d, v, n, c) = r?;
            let t = "account_days";
            let c = text::currency(t, "currency", &c)?;
            out.days.insert(text::date(t, "day", &d)?, (Money::new(text::dec(t, "net_value", &v)?, c), Money::new(text::dec(t, "net_deposits", &n)?, c)));
        }
        out.activity_read_at = self.activity_read_at(account)?;
        let cash_of = |id: &str| -> Result<BTreeMap<Currency, Dec>> {
            let mut m = BTreeMap::new();
            let mut st = self.conn().prepare_cached("SELECT currency, amount FROM statement_cash WHERE statement_id = ?1")?;
            for r in st.query_map(params![id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (c, v) = r?;
                m.insert(text::currency("statement_cash", "currency", &c)?, text::dec("statement_cash", "amount", &v)?);
            }
            Ok(m)
        };
        // newest first, by the instant rather than its text (whose fraction's
        // length varies): the newest, and the newest the last full read covers
        let mut st = self.conn().prepare("SELECT id, stated_at FROM statements WHERE account_id = ?1 AND kind = 'cash'")?;
        let mut stated: Vec<(jiff::Timestamp, String)> = vec![];
        for r in st.query_map(params![a], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, at) = r?;
            stated.push((text::instant("statements", "stated_at", &at)?, id));
        }
        stated.sort_unstable_by(|x, y| y.cmp(x));
        if let Some((at, id)) = stated.first() {
            out.cash = Some((*at, cash_of(id)?));
        }
        if let Some(read) = out.activity_read_at {
            if let Some((at, id)) = stated.iter().find(|(at, _)| *at <= read) {
                out.cash_read = Some((*at, cash_of(id)?));
                out.cash_read_holds = self.statement_holds(id)?;
            }
        }
        let value: Option<(String, String, String)> = self
            .conn()
            .query_row("SELECT stated_at, amount, currency FROM account_values WHERE account_id = ?1 ORDER BY stated_at DESC LIMIT 1", params![a], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        if let Some((at, amount, currency)) = value {
            out.net_value_now = Some((text::instant("account_values", "stated_at", &at)?, Money::new(text::dec("account_values", "amount", &amount)?, text::parsed("account_values", "currency", &currency, Currency::parse)?)));
        }
        let units: Option<(String, String)> = self
            .conn()
            .query_row("SELECT id, as_of_day FROM statements WHERE account_id = ?1 AND kind = 'units' ORDER BY as_of_day DESC, id DESC LIMIT 1", params![a], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((id, d)) = units {
            // each line under the instrument the book reads it as: the units of
            // instruments read as one added up
            let joined = self.joined()?;
            let mut m: BTreeMap<InstrumentId, Dec> = BTreeMap::new();
            // each instrument's value, none once a line of it states none or another currency
            let mut values: BTreeMap<InstrumentId, Option<Money>> = BTreeMap::new();
            let mut st = self.conn().prepare("SELECT instrument_id, quantity, value, value_currency FROM statement_units WHERE statement_id = ?1")?;
            type Line = (String, String, Option<String>, Option<String>);
            for r in st.query_map(params![id], |r| -> rusqlite::Result<Line> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)) })? {
                let (i, q, v, vc) = r?;
                let i = text::parsed("statement_units", "instrument_id", &i, InstrumentId::parse)?;
                let q = text::dec("statement_units", "quantity", &q)?;
                let head = joined.get(&i).copied().unwrap_or(i);
                let e = m.entry(head).or_insert(Dec::ZERO);
                *e = e.checked_add(q).map_err(|e| crate::BookError::Refused(format!("a statement's units too large to add: {e}")))?;
                let value = match (v, vc) {
                    (Some(v), Some(c)) => Some(Money::new(text::dec("statement_units", "value", &v)?, text::parsed("statement_units", "value_currency", &c, Currency::parse)?)),
                    _ => None,
                };
                let slot = values.entry(head).or_insert(Some(Money::zero(value.map(|v| v.currency).unwrap_or(Currency::CAD))));
                *slot = match (*slot, value) {
                    (Some(a), Some(v)) if a.currency == v.currency => a.amount.checked_add(v.amount).ok().map(|x| Money::new(x, a.currency)),
                    _ => None,
                };
            }
            out.unit_values = values.into_iter().filter_map(|(i, v)| v.map(|v| (i, v))).collect();
            out.units = Some((text::date("statements", "as_of_day", &d)?, m));
        }
        let bp: Option<(String, Option<String>, Option<String>, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT b.stated_at, b.amount, b.currency, b.unavailable FROM buying_power b JOIN broker_reads r ON r.id = b.read_id WHERE b.account_id = ?1 ORDER BY b.stated_at DESC, r.at DESC, r.id DESC LIMIT 1",
                params![a],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((at, amount, currency, why)) = bp {
            let t = "buying_power";
            let stated = match (amount, currency, why) {
                (Some(v), Some(c), None) => Ok(Money::new(text::dec(t, "amount", &v)?, text::currency(t, "currency", &c)?)),
                (None, None, Some(w)) => Err(w),
                _ => return Err(crate::BookError::Corrupt { table: t, column: "unavailable", value: a.clone(), why: "states both an amount and a reason, or neither".into() }),
            };
            out.buying_power = Some((text::instant(t, "stated_at", &at)?, stated));
        }
        Ok(out)
    }
}
