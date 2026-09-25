//! The book (`docs/architecture.md` §5, §6): what the person's money did, what
//! they wrote, and every fact a figure was computed from. Nothing in it can be
//! fetched again, so nothing in it is ever lost: records keep every revision, a
//! record leaves the count only by a link or its source's reported removal, and
//! a note whose trade is gone is kept, orphaned, for the person to re-attach.
//!
//! The book never reads the clock: every write is given the time it happened.

pub mod canon;
pub mod facts;
pub mod identity;
pub mod import;
pub mod links;
pub mod mapping;
pub mod person;
pub mod records;
pub mod schema;
pub mod settings;
pub mod statements;
pub mod trades;
pub mod zones;

mod text;

use std::fmt;
use std::path::Path;

use rusqlite::Connection;

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
        let (conn, done) = bagholder_sqlite::migrate::open(&schema::SCHEMA, path, app_version, at)?;
        // every reference between the book's tables is held by SQLite itself
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok((Book { conn, zones: zones::Zones::new() }, done))
    }

    /// The book in the data folder `home`.
    pub fn open_in(home: &Path, app_version: &str, at: jiff::Timestamp) -> Result<(Book, Migrated)> {
        Book::open(&home.join(BOOK_FILE), app_version, at)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Run `work` as one transaction: all of it, or none of it (an error or a
    /// panic rolls it back). Nested calls join the transaction already open. As
    /// `bagholder_sqlite::atomically`, it begins IMMEDIATE, so a writer waits for
    /// its turn at the start rather than failing half way.
    pub(crate) fn atomically<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            return work();
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Immediate)?;
        let out = work()?; // dropping `tx` on an error or a panic rolls back
        tx.commit()?;
        Ok(out)
    }
}

/// A new id for something stored at `at`: a UUID v7 from that time and random bits.
pub(crate) fn new_uuid(at: jiff::Timestamp) -> uuid::Uuid {
    let secs = at.as_second().max(0) as u64;
    let nanos = at.subsec_nanosecond().max(0) as u32;
    uuid::Uuid::new_v7(uuid::Timestamp::from_unix(uuid::NoContext, secs, nanos))
}
