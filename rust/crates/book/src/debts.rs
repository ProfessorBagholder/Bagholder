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
    /// What is owed, positive, in `currency`.
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

    /// What each connection's newest read states is owed outside, every connection's.
    pub fn outside_debts(&self) -> Result<Vec<OutsideDebt>> {
        let mut stmt = self.conn().prepare(
            "SELECT entity_id, kind, name, institution, last4, balance, currency, as_of FROM outside_debts d
             WHERE entity_id <> '' AND read_at = (SELECT MAX(read_at) FROM outside_debts n WHERE n.connection_id = d.connection_id)
             ORDER BY entity_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, Option<String>>(7)?)))?;
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
