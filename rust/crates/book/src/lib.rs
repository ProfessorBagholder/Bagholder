//! The book (`docs/architecture.md` §5, §6): what the person's money did, what
//! they wrote, and every fact a figure was computed from. Nothing in it can be
//! fetched again, so nothing in it is ever lost: records keep every revision, a
//! record leaves the count only by a link or its source's reported removal, and
//! a note whose trade is gone is kept, orphaned, for the person to re-attach.
//!
//! The book never reads the clock: every write is given the time it happened.

pub mod canon;
pub mod changes;
pub mod clear;
pub mod debts;
pub mod facts;
pub mod identity;
pub mod links;
pub mod mapping;
pub mod names;
pub mod notices;
pub mod fills;
pub mod orders;
pub mod person;
pub mod pooled;
pub mod records;
pub mod schema;
pub mod settings;
pub mod statements;
pub mod trades;
pub mod watched;
pub mod zones;

mod text;

use std::fmt;
use std::path::Path;

use rusqlite::Connection;
#[cfg(feature = "oracles")]
use bagholder_core::InstrumentId as InstrumentIdOf;

pub use bagholder_sqlite::migrate::Migrated;

/// The file the book is kept in, inside the data folder.
pub const BOOK_FILE: &str = "book.db";

#[derive(Debug)]
pub enum BookError {
    Sqlite(rusqlite::Error),
    /// The file could not be opened or brought to this version's schema.
    Migrate(bagholder_sqlite::migrate::MigrateError),
    /// A stored value that does not read back: the file was changed by something
    /// other than the book.
    Corrupt { table: &'static str, column: &'static str, value: String, why: String },
    /// A payload that is not JSON the book can keep.
    Payload(canon::CanonError),
    /// A request the book refuses: an unknown record, a record in the wrong state,
    /// a mapping for another source.
    Refused(String),
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookError::Sqlite(e) => write!(f, "the book: {e}"),
            BookError::Migrate(e) => write!(f, "{e}"),
            BookError::Corrupt { table, column, value, why } => {
                write!(f, "the book holds a value it cannot read in {table}.{column} ({value:?}): {why}")
            }
            BookError::Payload(e) => write!(f, "{e}"),
            BookError::Refused(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for BookError {}

impl From<rusqlite::Error> for BookError {
    fn from(e: rusqlite::Error) -> Self {
        BookError::Sqlite(e)
    }
}

impl From<bagholder_sqlite::migrate::MigrateError> for BookError {
    fn from(e: bagholder_sqlite::migrate::MigrateError) -> Self {
        BookError::Migrate(e)
    }
}

pub type Result<T> = std::result::Result<T, BookError>;

/// An open book.
pub struct Book {
    conn: Connection,
    zones: zones::Zones,
}

impl Book {
    /// Open the book at `path`, made when it does not exist, and bring it to this
    /// version's schema (snapshotting it first when it has to be migrated). `at`
    /// is now; `app_version` is this build's.
    pub fn open(path: &Path, app_version: &str, at: jiff::Timestamp) -> Result<(Book, Migrated)> {
        Book::open_mapped(path, app_version, at, &[])
    }

    /// `open`, with the mappings a migration's code step reads records with
    /// (migration 37's split of the accounts the import pooled, `pooled`): a
    /// book that needs a mapping not among them is left as it was, and refused.
    pub fn open_mapped(path: &Path, app_version: &str, at: jiff::Timestamp, mappings: &[&dyn mapping::Mapping]) -> Result<(Book, Migrated)> {
        let split = |c: &rusqlite::Connection| pooled::step(c, mappings, at);
        let (conn, done) = bagholder_sqlite::migrate::open_with(&schema::SCHEMA, path, app_version, at, &[(37, &split)])?;
        // every reference between the book's tables is held by SQLite itself
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // a REPLACE fires the delete triggers of the row it replaces: the change log
        // and the names' due rows see the row that went (brief 28, 1)
        conn.pragma_update(None, "recursive_triggers", "ON")?;
        let book = Book { conn, zones: zones::Zones::new() };
        // what a migration marked settled once (`names`). An open with no migration
        // takes no lock, so it never waits on another connection's write: a due row
        // a write outside the book's own transactions left is read by folding until
        // the next write's commit settles it, and a head that fails keeps its rows
        if done.from != done.to {
            book.atomically(|| Ok(()))?;
        }
        Ok((book, done))
    }

    /// The book in the data folder `home`.
    pub fn open_in(home: &Path, app_version: &str, at: jiff::Timestamp) -> Result<(Book, Migrated)> {
        Book::open(&home.join(BOOK_FILE), app_version, at)
    }

    /// `open_mapped`, the book in the data folder `home`.
    pub fn open_in_mapped(home: &Path, app_version: &str, at: jiff::Timestamp, mappings: &[&dyn mapping::Mapping]) -> Result<(Book, Migrated)> {
        Book::open_mapped(&home.join(BOOK_FILE), app_version, at, mappings)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The connection, for a test that counts what each table holds.
    #[doc(hidden)]
    pub fn conn_for_tests(&self) -> &Connection {
        &self.conn
    }

    /// Run `work` as one transaction: all of it, or none of it (an error or a
    /// panic rolls it back). Nested calls join the transaction already open. As
    /// `bagholder_sqlite::atomically`, it begins IMMEDIATE, so a writer waits for
    /// its turn at the start rather than failing half way. A caller that writes many
    /// records in a row commits them a group at a time with it.
    pub fn atomically<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            return work();
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Immediate)?;
        let out = work()?; // dropping `tx` on an error or a panic rolls back
        // the names around every sighting the work changed, kept before it commits
        let failed = self.settle_names()?;
        // test-only reads, kept out of a probe's measured window
        #[cfg(feature = "oracles")]
        if !bagholder_sqlite::work::measuring() {
            let due: Vec<String> = self.conn.prepare("SELECT DISTINCT instrument_id FROM names_due")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            for d in due {
                // a head that cannot be read is the id's own failure
                let id = InstrumentIdOf::parse(&d).ok().map(|i| self.canonical(i).unwrap_or(i));
                assert!(id.is_some_and(|h| failed.contains(&h)), "a write through the book left the names of {d} unsettled");
            }
        }
        let _ = failed;
        tx.commit()?;
        Ok(out)
    }

    /// Run `reads` as one read transaction: every read in it sees the book as it
    /// stood at its first read, whatever another connection commits meanwhile
    /// (WAL's snapshot). Nested calls join the transaction already open. For a
    /// reader that must see what the change log names and the rows it names at one
    /// moment (`changes`).
    pub fn snapshot<T>(&self, reads: impl FnOnce() -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            return reads();
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Deferred)?;
        let out = reads()?;
        tx.commit()?;
        Ok(out)
    }
}

/// Where a trade or a group stands in the book's own order: when it was made,
/// then its row's number.
pub type Rank = (String, i64);

/// A new id for something stored at `at`: a UUID v7 from that time and random bits.
pub(crate) fn new_uuid(at: jiff::Timestamp) -> uuid::Uuid {
    let secs = at.as_second().max(0) as u64;
    let nanos = at.subsec_nanosecond().max(0) as u32;
    uuid::Uuid::new_v7(uuid::Timestamp::from_unix(uuid::NoContext, secs, nanos))
}
