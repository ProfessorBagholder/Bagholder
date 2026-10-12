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

/// A book from before migration 25 holding a bracket with its entry order and its
/// log (rows that point at the bracket) reaches the latest schema with every row and
/// reference kept: migration 25 builds the brackets table again (owner's book,
/// 2026-10-04: "FOREIGN KEY constraint failed" on 2.1.0).
#[test]
fn a_book_holding_a_bracket_with_its_orders_migrates_past_the_brackets_rebuild() {
    use bagholder_book::schema::SCHEMA;
    use bagholder_sqlite::migrate::{self, Schema};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.db");
    static BEFORE: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    let before = BEFORE.get_or_init(|| Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &SCHEMA.migrations[..24] });
    {
        let (conn, _) = migrate::open(before, &path, "2.0.16", "2026-10-01T00:00:00Z".parse().unwrap()).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(
            "INSERT INTO brackets (id, broker, broker_account, broker_security, symbol, currency, created_at, phase, updated_at)
               VALUES ('bracket-1', 'wealthsimple', 'acct', 'sec', 'SHOP', 'USD', '2026-10-01T14:00:00Z', 'guarding', '2026-10-01T14:00:00Z');
             INSERT INTO bracket_events (bracket_id, seq, at, asker, kind, body, refused)
               VALUES ('bracket-1', 0, '2026-10-01T14:00:00Z', 'person', 'created', '{\"quantity\": \"10\", \"stop\": null, \"target\": null}', NULL);
             INSERT INTO orders (id, broker, broker_account, broker_security, symbol, currency, side, order_type, quantity, time_in_force, bracket_id, role, request, created_at, state, filled, updated_at)
               VALUES ('order-1', 'wealthsimple', 'acct', 'sec', 'SHOP', 'USD', 'buy', 'limit', '10', 'day', 'bracket-1', 'entry', '{}', '2026-10-01T14:00:00Z', 'filled', '10', '2026-10-01T14:00:00Z');",
        )
        .unwrap();
    }
    let (conn, done) = migrate::open(&SCHEMA, &path, "test", "2026-10-04T00:00:00Z".parse().unwrap()).unwrap();
    assert_eq!((done.from, done.to), (24, SCHEMA.latest()));
    let kept: (i64, i64, i64) = conn
        .query_row("SELECT (SELECT count(*) FROM brackets), (SELECT count(*) FROM bracket_events), (SELECT count(*) FROM orders WHERE bracket_id = 'bracket-1')", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(kept, (1, 1, 1));
    let broken: i64 = conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
    assert_eq!(broken, 0);
}

/// splitmix64: the same seed, the same book.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }
}

/// Migration 26 folds a book's statements into periods and the book answers what
/// it answered before (`docs/plans/stage-3-the-brokers-statements.md`): a book at
/// version 25 holding generated statements for several accounts (instants out of
/// order and tied, their text with fractions of every length, holds whose amount is
/// known or not), migrated, states for every account the newest cash and when, the
/// cash and holds at its last full read, its buying power and its value now exactly
/// as a model of the rows it held gives them: each the newest by its read's instant,
/// then the order the reads were written in.
#[test]
fn migration_26_keeps_what_every_account_s_statements_answer() {
    use bagholder_core::{AccountId, Currency, Dec};
    use std::collections::BTreeMap;
    for seed in 0..40u64 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.db");
        let partial: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..25] }));
        let (conn, _) = migrate::open(partial, &path, "test", t0()).unwrap();
        conn.execute("INSERT INTO broker_connections (id, broker, label, created_at) VALUES ('c', 'wealthsimple', 'kept', 'x')", []).unwrap();
        let accounts: Vec<AccountId> = (0..3).map(|n| AccountId::parse(&format!("00000000-0000-7000-8000-00000000000{n}")).unwrap()).collect();
        for a in &accounts {
            conn.execute("INSERT INTO accounts (id, connection_id, unrecognised_type, status, created_at) VALUES (?1, 'c', 'x', 'open', 'x')", [a.to_string()]).unwrap();
        }
        conn.execute("INSERT INTO source_records (id, connection_id, source, source_key, state, derived_version, first_received_at, state_changed_at) VALUES ('00000000-0000-7000-8000-0000000000aa', 'c', 'wealthsimple', 'k', 'live', 1, 'x', 'x')", []).unwrap();
        let mut rng = Rng(seed);
        let base: jiff::Timestamp = "2026-10-01T10:00:00Z".parse().unwrap();
        // (account, instant, the order written, cash, hold amount, power, value)
        type Row = (usize, jiff::Timestamp, usize, i64, Option<&'static str>, i64, i64);
        let mut rows: Vec<Row> = Vec::new();
        let fractions = [0i64, 500_000_000, 450_401_000, 1, 999_999_999];
        let mut uid = 0u32;
        let mut id = || {
            uid += 1;
            format!("00000000-0000-7000-8000-{uid:012x}")
        };
        for k in 0..(10 + rng.below(30) as usize) {
            let acc = rng.below(3) as usize;
            let at = base.checked_add(jiff::SignedDuration::new(rng.below(6) as i64, fractions[rng.below(5) as usize] as i32)).unwrap();
            let (cash, hold, power, value) = (rng.below(2) as i64, [None, Some("5"), Some("NULL")][rng.below(3) as usize], rng.below(2) as i64, rng.below(2) as i64);
            let a = accounts[acc].to_string();
            for part in ["cash", "buying-power", "net-value"] {
                let read = id();
                conn.execute("INSERT INTO broker_reads (id, connection_id, part, at) VALUES (?1, 'c', ?2, ?3)", rusqlite::params![read, part, at.to_string()]).unwrap();
                match part {
                    "cash" => {
                        let s = id();
                        conn.execute("INSERT INTO statements (id, account_id, kind, stated_at, read_id) VALUES (?1, ?2, 'cash', ?3, ?4)", rusqlite::params![s, a, at.to_string(), read]).unwrap();
                        conn.execute("INSERT INTO statement_cash (statement_id, currency, amount) VALUES (?1, 'CAD', ?2)", rusqlite::params![s, cash.to_string()]).unwrap();
                        if let Some(h) = hold {
                            let amount = (h != "NULL").then_some(h);
                            conn.execute("INSERT INTO statement_holds (statement_id, record_id, kind, currency, amount) VALUES (?1, '00000000-0000-7000-8000-0000000000aa', 'buy', 'CAD', ?2)", rusqlite::params![s, amount]).unwrap();
                        }
                    }
                    "buying-power" => {
                        conn.execute("INSERT INTO buying_power (account_id, stated_at, amount, currency, read_id) VALUES (?1, ?2, ?3, 'CAD', ?4)", rusqlite::params![a, at.to_string(), power.to_string(), read]).unwrap();
                    }
                    _ => {
                        conn.execute("INSERT OR REPLACE INTO account_values (account_id, stated_at, amount, currency, read_id) VALUES (?1, ?2, ?3, 'CAD', ?4)", rusqlite::params![a, at.to_string(), value.to_string(), read]).unwrap();
                    }
                }
            }
            rows.push((acc, at, k, cash, hold, power, value));
        }
        // statements of units as of a day, days out of order and tied: (account, day, the order written, quantity)
        conn.execute("INSERT INTO instruments(id, kind, currency, created_at) VALUES ('00000000-0000-7000-8000-0000000000bb', 'security', 'CAD', 'x')", []).unwrap();
        let mut units: Vec<(usize, jiff::civil::Date, usize, i64)> = Vec::new();
        for k in 0..rng.below(8) as usize {
            let (acc, day, quantity) = (rng.below(3) as usize, jiff::civil::date(2026, 9, 28 + rng.below(3) as i8), 1 + rng.below(9) as i64);
            let (read, s) = (id(), id());
            conn.execute("INSERT INTO broker_reads (id, connection_id, part, at) VALUES (?1, 'c', 'units', ?2)", rusqlite::params![read, base.to_string()]).unwrap();
            conn.execute("INSERT INTO statements (id, account_id, kind, as_of_day, read_id) VALUES (?1, ?2, 'units', ?3, ?4)", rusqlite::params![s, accounts[acc].to_string(), day.to_string(), read]).unwrap();
            conn.execute("INSERT INTO statement_units (statement_id, instrument_id, quantity) VALUES (?1, '00000000-0000-7000-8000-0000000000bb', ?2)", rusqlite::params![s, quantity.to_string()]).unwrap();
            units.push((acc, day, k, quantity));
        }
        // a full read of each account's activity at an instant among them
        let mut read_at: BTreeMap<usize, jiff::Timestamp> = BTreeMap::new();
        for (n, a) in accounts.iter().enumerate() {
            let at = base.checked_add(jiff::SignedDuration::new(rng.below(7) as i64, fractions[rng.below(5) as usize] as i32)).unwrap();
            conn.execute("INSERT INTO activity_reads (account_id, read_at, complete) VALUES (?1, ?2, 1)", rusqlite::params![a.to_string(), at.to_string()]).unwrap();
            read_at.insert(n, at);
        }
        drop(conn);
        let (book, done) = Book::open(&path, "test", t0()).unwrap();
        assert_eq!(done.to, MIGRATIONS.len() as u32);
        for (n, a) in accounts.iter().enumerate() {
            let mut mine: Vec<&Row> = rows.iter().filter(|r| r.0 == n).collect();
            // the model: by instant, then the order written; a value now replaced at one instant
            mine.sort_by_key(|r| (r.1, r.2));
            let s = book.stated_latest(*a).unwrap();
            let cad = |v: i64| BTreeMap::from([(Currency::CAD, Dec::from_int(v))]);
            let newest = mine.last();
            assert_eq!(s.cash, newest.map(|r| (r.1, cad(r.3))), "seed {seed}, account {n}: newest cash");
            assert_eq!(s.buying_power.map(|(at, p)| (at, p.unwrap().amount)), newest.map(|r| (r.1, Dec::from_int(r.5))), "seed {seed}, account {n}: buying power");
            assert_eq!(s.net_value_now.map(|(at, v)| (at, v.amount)), newest.map(|r| (r.1, Dec::from_int(r.6))), "seed {seed}, account {n}: value now");
            let at_read = mine.iter().rev().find(|r| r.1 <= read_at[&n]);
            assert_eq!(s.activity_read_at, Some(read_at[&n]));
            assert_eq!(s.cash_read, at_read.map(|r| cad(r.3)), "seed {seed}, account {n}: the cash at its full read");
            let holds: Vec<Option<Dec>> = s.cash_read_holds.iter().map(|h| h.amount).collect();
            let want: Vec<Option<Dec>> = at_read.and_then(|r| r.4).map(|h| (h != "NULL").then(|| Dec::from_int(5))).into_iter().collect();
            assert_eq!(holds, want, "seed {seed}, account {n}: the holds at its full read");
            // the units: the newest by the day they are as of, then the order written
            let held = bagholder_core::InstrumentId::parse("00000000-0000-7000-8000-0000000000bb").unwrap();
            let newest_units = units.iter().filter(|u| u.0 == n).max_by_key(|u| (u.1, u.2)).map(|u| (u.1, BTreeMap::from([(held, Dec::from_int(u.3))])));
            assert_eq!(s.units, newest_units, "seed {seed}, account {n}: the units");
        }
    }
}
