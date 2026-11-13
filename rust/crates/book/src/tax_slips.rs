//! The tax slips a broker issued the person (migration 046): each document's text
//! kept as it came less what identifies the person (`SPEC.md` tax-slips-5), read once.
//! A slip the broker amends is a document of its own, and for an account, a kind
//! and a year the one the broker made last is the slip (`SPEC.md` tax-slips-3); the
//! ones before it stay kept, and are not read. A slip whose name and address could
//! not be taken out is kept with no text, under the version of the redaction that
//! could not, and read again only once that version changes.

use std::collections::BTreeSet;

use rusqlite::params;

use bagholder_core::ConnectionId;

use crate::statements::ReadId;
use crate::{Book, Result};

/// A slip as a broker's documents list and its file state it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxSlip {
    /// The broker's own id for the document.
    pub document: String,
    /// The broker's own id for the account it was issued for.
    pub account_key: String,
    /// Its kind, as the broker names it (`t3`, `t5`, `t5008`).
    pub kind: String,
    pub year: i16,
    /// When the broker made it, as it wrote it: texts of one broker sort as time.
    pub created: String,
    /// Its file's text, as `pdftotext -layout` lays its pages out, what identifies
    /// the person taken out; none where that could not be done. Never shown: only
    /// the figures read from it are.
    pub text: Option<String>,
}

impl Book {
    /// Keep a slip read in `read`, under the version of the redaction it was
    /// written by. A document kept with its text stays as it was: its file is
    /// fixed, so it is never read twice. One kept without text takes the text a
    /// later redaction gives it.
    pub fn keep_tax_slip(&self, connection: ConnectionId, slip: &TaxSlip, redactor: i64, read: &ReadId) -> Result<()> {
        self.conn().execute(
            "INSERT INTO tax_slips (connection_id, document, account_key, kind, year, created, text, redactor, read_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (connection_id, document) DO UPDATE SET text = excluded.text, redactor = excluded.redactor, read_id = excluded.read_id
             WHERE tax_slips.text IS NULL",
            params![connection.to_string(), slip.document, slip.account_key, slip.kind, slip.year, slip.created, slip.text, redactor, read.0],
        )?;
        Ok(())
    }

    /// The documents of a connection's slips held, not to be read again under the
    /// redaction `redactor`: each kept with its text, and each kept without it by
    /// that same version (the ones amended since among them).
    pub fn tax_slip_documents(&self, connection: ConnectionId, redactor: i64) -> Result<BTreeSet<String>> {
        let mut q = self.conn().prepare("SELECT document FROM tax_slips WHERE connection_id = ?1 AND (text IS NOT NULL OR redactor = ?2)")?;
        let rows = q.query_map(params![connection.to_string(), redactor], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// A connection's slips in force: for each account, kind and year, the one the
    /// broker made last (two made at one instant told apart by the broker's id for
    /// them, so the answer never depends on the order they were kept in), its text
    /// none where it could not be kept; by year, kind and account.
    pub fn tax_slips(&self, connection: ConnectionId) -> Result<Vec<TaxSlip>> {
        let mut q = self.conn().prepare(
            "SELECT t.document, t.account_key, t.kind, t.year, t.created, t.text FROM tax_slips t
             WHERE t.connection_id = ?1 AND NOT EXISTS (
                 SELECT 1 FROM tax_slips u
                 WHERE u.connection_id = t.connection_id AND u.account_key = t.account_key AND u.kind = t.kind AND u.year = t.year
                   AND (u.created > t.created OR (u.created = t.created AND u.document > t.document)))
             ORDER BY t.year, t.kind, t.account_key",
        )?;
        let rows = q.query_map(params![connection.to_string()], |r| {
            Ok(TaxSlip { document: r.get(0)?, account_key: r.get(1)?, kind: r.get(2)?, year: r.get(3)?, created: r.get(4)?, text: r.get(5)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}
