//! Debts held outside the broker (`docs/plans/debts-held-outside-wealthsimple.md`,
//! migration 33): each outside account the broker's net worth feature states,
//! kept read by read; the person's answers for the
//! payees no rule settled; and the withdrawals whose payee a match reads.

use rusqlite::params;

use bagholder_core::{ConnectionId, Currency, Dec, Leg, RecordId, TransactionId};

use crate::text;
use crate::{Book, Result};

/// An outside account as one read of the net worth feature states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutsideDebt {
    pub entity_id: String,
    pub kind: String,
    pub name: String,
    pub institution: String,
    pub last4: Option<String>,
    /// Its balance as the feature states it, in `currency`: a debt's balance owed
    /// is positive, one in credit negative.
    pub balance: Dec,
    pub currency: Currency,
    pub as_of: Option<jiff::Timestamp>,
}

/// A withdrawal's payee, as its broker row states it: the row's raw fields, for
/// the broker's matching to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Withdrawal {
    pub transaction: TransactionId,
    pub connection: Option<ConnectionId>,
    /// The record's newest payload, as the broker sent it.
    pub payload: String,
}

impl Book {
    /// Keep what one read of the net worth feature states: every outside account,
    /// under the read's moment. An earlier read stays as it was.
    pub fn keep_outside_debts(&self, connection: ConnectionId, read_at: jiff::Timestamp, debts: &[OutsideDebt]) -> Result<()> {
        // a read stating what the newest one stated is no new statement
        if self.outside_debts_of(connection)? == debts {
            return Ok(());
        }
        let tx = self.conn().unchecked_transaction()?;
        for d in debts {
            tx.execute(
                "INSERT OR REPLACE INTO outside_debts (connection_id, read_at, entity_id, kind, name, institution, last4, balance, currency, as_of) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![connection.to_string(), text::at(read_at), d.entity_id, d.kind, d.name, d.institution, d.last4, d.balance.to_string(), d.currency.as_str(), d.as_of.map(text::at)],
            )?;
        }
        // a read that states none keeps its moment, so "none owed" is a statement too
        if debts.is_empty() {
            tx.execute(
                "INSERT OR REPLACE INTO outside_debts (connection_id, read_at, entity_id, kind, name, institution, last4, balance, currency, as_of) VALUES (?, ?, '', '', '', '', NULL, '0', 'CAD', NULL)",
                params![connection.to_string(), text::at(read_at)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep net worth by day as one read states it, in place of the last.
    pub fn keep_net_worth_days(&self, days: &[(jiff::civil::Date, Dec, Currency)], at: jiff::Timestamp) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute("DELETE FROM net_worth_days", [])?;
        for (d, v, c) in days {
            tx.execute("INSERT INTO net_worth_days (day, amount, currency, read_at) VALUES (?, ?, ?, ?)", params![text::day(*d), v.to_string(), c.as_str(), text::at(at)])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Net worth by day, oldest first, and when it was last read.
    pub fn net_worth_days(&self) -> Result<Vec<(jiff::civil::Date, Dec, Currency)>> {
        let mut stmt = self.conn().prepare("SELECT day, amount, currency FROM net_worth_days ORDER BY day")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let t = "net_worth_days";
        rows.map(|row| {
            let (d, v, c) = row?;
            Ok((text::date(t, "day", &d)?, text::dec(t, "amount", &v)?, text::currency(t, "currency", &c)?))
        })
        .collect()
    }

    /// When net worth by day was last read, if ever.
    pub fn net_worth_read_at(&self) -> Result<Option<jiff::Timestamp>> {
        let at: Option<String> = self.conn().query_row("SELECT MAX(read_at) FROM net_worth_days", [], |r| r.get(0))?;
        text::opt_instant("net_worth_days", "read_at", at)
    }

    /// What a fingerprint of the outside debts and the answers about them reads
    /// now: it moves with every read kept and every answer given.
    pub fn debts_print(&self) -> Result<String> {
        Ok(self.conn().query_row(
            "SELECT COALESCE((SELECT MAX(read_at) FROM outside_debts), '') || '|' || (SELECT COUNT(*) FROM repayment_answers) || '|' || COALESCE((SELECT MAX(answered_at) FROM repayment_answers), '') || '|' || COALESCE((SELECT MAX(read_at) FROM net_worth_days), '')",
            [],
            |r| r.get::<_, String>(0),
        )?)
    }

    /// What each connection's newest read states is owed outside, every connection's.
    pub fn outside_debts(&self) -> Result<Vec<OutsideDebt>> {
        self.outside_debts_where(None)
    }

    /// What one connection's newest read states.
    fn outside_debts_of(&self, connection: ConnectionId) -> Result<Vec<OutsideDebt>> {
        self.outside_debts_where(Some(connection))
    }

    fn outside_debts_where(&self, connection: Option<ConnectionId>) -> Result<Vec<OutsideDebt>> {
        let mut stmt = self.conn().prepare(
            "SELECT entity_id, kind, name, institution, last4, balance, currency, as_of FROM outside_debts d
             WHERE entity_id <> '' AND read_at = (SELECT MAX(read_at) FROM outside_debts n WHERE n.connection_id = d.connection_id)
             AND (?1 IS NULL OR d.connection_id = ?1)
             ORDER BY entity_id",
        )?;
        let rows = stmt.query_map([connection.map(|c| c.to_string())], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, Option<String>>(7)?)))?;
        let t = "outside_debts";
        rows.map(|row| {
            let (entity_id, kind, name, institution, last4, balance, currency, as_of) = row?;
            Ok(OutsideDebt { entity_id, kind, name, institution, last4, balance: text::dec(t, "balance", &balance)?, currency: text::currency(t, "currency", &currency)?, as_of: text::opt_instant(t, "as_of", as_of)? })
        })
        .collect()
    }

    /// The person's answer for a payee: the debt its payments repay, or none.
    pub fn answer_repayment(&self, payee_key: &str, debt: Option<&str>, at: jiff::Timestamp) -> Result<()> {
        self.conn().execute(
            "INSERT INTO repayment_answers (payee_key, debt_id, answered_at) VALUES (?, ?, ?) ON CONFLICT (payee_key) DO UPDATE SET debt_id = excluded.debt_id, answered_at = excluded.answered_at",
            params![payee_key, debt, text::at(at)],
        )?;
        Ok(())
    }

    /// Every answer the person gave, by payee.
    pub fn repayment_answers(&self) -> Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn().prepare("SELECT payee_key, debt_id FROM repayment_answers ORDER BY payee_key")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every live withdrawal of `source`, with its record's newest payload.
    pub fn withdrawals_of(&self, source: &str) -> Result<Vec<Withdrawal>> {
        let mut stmt = self.conn().prepare(
            "SELECT t.record_id, t.leg, s.connection_id, v.payload FROM transactions t
             JOIN source_records s ON s.id = t.record_id AND s.state = 'live' AND s.source = ?
             JOIN record_revisions v ON v.record_id = t.record_id AND v.revision = (SELECT MAX(revision) FROM record_revisions w WHERE w.record_id = t.record_id)
             WHERE t.kind = 'withdrawal'
             ORDER BY t.record_id, t.leg",
        )?;
        let rows = stmt.query_map([source], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?)))?;
        let t = "transactions";
        rows.map(|row| {
            let (record, leg, connection, payload) = row?;
            Ok(Withdrawal {
                transaction: TransactionId::new(text::parsed(t, "record_id", &record, RecordId::parse)?, text::parsed(t, "leg", &leg, Leg::parse)?),
                connection: text::opt_parsed("source_records", "connection_id", connection, ConnectionId::parse)?,
                payload,
            })
        })
        .collect()
    }
}
