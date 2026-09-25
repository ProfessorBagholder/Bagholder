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
    /// The newest statement of units, and the day they are as of.
    pub units: Option<(jiff::civil::Date, BTreeMap<InstrumentId, Dec>)>,
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

    /// That `account` is linked to `to`, as the broker states it.
    pub fn link_accounts(&self, account: AccountId, to: AccountId, read: &ReadId) -> Result<()> {
        self.conn().execute("INSERT OR IGNORE INTO account_links (account_id, linked_to, read_id) VALUES (?1, ?2, ?3)", params![account.to_string(), to.to_string(), read.0])?;
        Ok(())
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

    /// Store a statement of units as of a day.
    pub fn store_units(&self, account: AccountId, as_of: jiff::civil::Date, lines: &[UnitsLine], read: &ReadId, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            let id = new_uuid(at).to_string();
            self.conn().execute("INSERT INTO statements (id, account_id, kind, as_of_day, read_id) VALUES (?1, ?2, 'units', ?3, ?4)", params![id, account.to_string(), day(as_of), read.0])?;
            for l in lines {
                self.conn().execute(
                    "INSERT INTO statement_units (statement_id, instrument_id, quantity, book_value, book_value_currency) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, l.instrument.to_string(), l.quantity.to_text(), l.book_value.map(|m| m.amount.to_text()), l.book_value.map(|m| m.currency.to_string())],
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
        let cash: Option<(String, String)> = self
            .conn()
            .query_row("SELECT id, stated_at FROM statements WHERE account_id = ?1 AND kind = 'cash' ORDER BY stated_at DESC, id DESC LIMIT 1", params![a], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((id, at)) = cash {
            let mut m = BTreeMap::new();
            let mut st = self.conn().prepare("SELECT currency, amount FROM statement_cash WHERE statement_id = ?1")?;
            for r in st.query_map(params![id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (c, v) = r?;
                m.insert(text::currency("statement_cash", "currency", &c)?, text::dec("statement_cash", "amount", &v)?);
            }
            out.cash = Some((text::instant("statements", "stated_at", &at)?, m));
        }
        let units: Option<(String, String)> = self
            .conn()
            .query_row("SELECT id, as_of_day FROM statements WHERE account_id = ?1 AND kind = 'units' ORDER BY as_of_day DESC, id DESC LIMIT 1", params![a], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((id, d)) = units {
            let mut m = BTreeMap::new();
            let mut st = self.conn().prepare("SELECT instrument_id, quantity FROM statement_units WHERE statement_id = ?1")?;
            for r in st.query_map(params![id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (i, q) = r?;
                m.insert(text::parsed("statement_units", "instrument_id", &i, InstrumentId::parse)?, text::dec("statement_units", "quantity", &q)?);
            }
            out.units = Some((text::date("statements", "as_of_day", &d)?, m));
        }
        out.activity_read_at = self.activity_read_at(account)?;
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
