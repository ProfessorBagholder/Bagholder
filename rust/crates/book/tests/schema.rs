//! The book's schema: every version is built from its migrations exactly as
//! committed, a released migration is never edited, and every earlier version
//! reaches the latest with its rows intact.

mod common;

use bagholder_book::schema::{MIGRATIONS, SCHEMA};
use bagholder_book::Book;
use bagholder_sqlite::migrate::{self, Schema};
use common::*;

/// The schema version `n` produces, from migrations 1 to `n`.
fn built(n: usize) -> String {
    let dir = tempfile::tempdir().unwrap();
    let partial = Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..n] };
    // a `Schema` needs `'static` migrations; the slice of the static array is
    let partial: &'static Schema = Box::leak(Box::new(partial));
    let (conn, _) = migrate::open(partial, &dir.path().join("book.db"), "test", t0()).unwrap();
    migrate::schema_text(&conn).unwrap()
}

#[test]
fn each_version_is_exactly_the_schema_committed_for_it() {
    for n in 1..=MIGRATIONS.len() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("schema/v{n}.sql"));
        let text = built(n);
        if std::env::var("BAGHOLDER_BLESS").is_ok() {
            std::fs::write(&path, &text).unwrap();
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} is not committed", path.display()));
        assert_eq!(text, committed, "schema {n} differs from {}: a released migration was edited", path.display());
    }
}

#[test]
fn every_earlier_version_reaches_the_latest_with_its_rows() {
    for n in 1..MIGRATIONS.len() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.db");
        let partial: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..n] }));
        let (conn, _) = migrate::open(partial, &path, "test", t0()).unwrap();
        conn.execute("INSERT INTO broker_connections(id, broker, label, created_at) VALUES ('c', 'wealthsimple', 'kept', 'x')", []).unwrap();
        drop(conn);
        let (book, done) = Book::open(&path, "test", t0()).unwrap();
        assert_eq!((done.from, done.to), (n as u32, MIGRATIONS.len() as u32));
        assert!(done.snapshot.is_some());
        drop(book);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let label: String = conn.query_row("SELECT label FROM broker_connections WHERE id = 'c'", [], |r| r.get(0)).unwrap();
        assert_eq!(label, "kept");
    }
}

#[test]
fn the_header_and_the_record_of_migrations_agree() {
    let f = Fixture::new();
    let conn = rusqlite::Connection::open(f.dir.path().join("book.db")).unwrap();
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    let applied: Vec<u32> = conn.prepare("SELECT number FROM schema_migrations ORDER BY number").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(version as usize, MIGRATIONS.len());
    assert_eq!(applied, (1..=MIGRATIONS.len() as u32).collect::<Vec<_>>());
    let app: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0)).unwrap();
    assert_eq!(app, SCHEMA.application_id);
}

#[test]
fn an_earlier_apps_database_is_not_opened_as_a_book() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TABLE activities (id TEXT); INSERT INTO activities VALUES ('a');").unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(Book::open(&path, "test", t0()), Err(bagholder_book::BookError::Migrate(migrate::MigrateError::Foreign { .. }))));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn the_schema_refuses_what_it_must_never_hold() {
    let f = Fixture::new();
    let a = f.account(&["acc-1"]);
    let conn = rusqlite::Connection::open(f.dir.path().join("book.db")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    conn.execute(
        "INSERT INTO source_records(id, connection_id, source, source_key, state, derived_version, first_received_at, state_changed_at) VALUES ('r', NULL, 'person', 'k', 'live', 1, 'x', 'x')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO instruments(id, kind, currency, created_at) VALUES ('i', 'security', 'CAD', 'x')", []).unwrap();
    let tx = |cols: &str, vals: &str| conn.execute(&format!("INSERT INTO transactions(record_id, leg, mapping_version, account_id, trade_date, kind{cols}) VALUES ('r', 'trade', 1, '{a}', '2026-01-02', 'buy'{vals})"), []);
    assert!(tx(", cash", ", '1'").is_err(), "an amount without its currency");
    assert!(tx(", fee", ", '1'").is_err(), "a fee without its currency");
    assert!(tx(", quantity", ", '1'").is_err(), "a quantity without an instrument");
    assert!(tx(", instrument_id, price, price_currency", ", 'i', '1', 'CAD'").is_err(), "a price without a quantity");
    assert!(tx(", effect", ", 'sideways'").is_err(), "an effect that is neither open nor close");
    assert!(tx(", instrument_id, quantity, cash, cash_currency", ", 'i', '1', '-1', 'CAD'").is_ok());
    // a transaction on an account the book does not have
    assert!(conn.execute("INSERT INTO transactions(record_id, leg, mapping_version, account_id, trade_date, kind) VALUES ('r', 'other', 1, 'nobody', '2026-01-02', 'fee')", []).is_err());
}

#[test]
fn a_stored_value_the_book_did_not_write_is_an_error_naming_where() {
    let f = Fixture::new();
    f.account(&["a1"]);
    f.store(&Spelled::v(1), "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1.5", "2026-01-02T15:00:00Z")]));
    let conn = rusqlite::Connection::open(f.dir.path().join("book.db")).unwrap();
    for bad in ["-1.50", "1e3", "abc"] {
        conn.execute("UPDATE transactions SET cash = ?", [bad]).unwrap();
        match f.book.transactions() {
            Err(bagholder_book::BookError::Corrupt { table, column, value, .. }) => assert_eq!((table, column, value.as_str()), ("transactions", "cash", bad)),
            other => panic!("{bad}: {other:?}"),
        }
    }
}
