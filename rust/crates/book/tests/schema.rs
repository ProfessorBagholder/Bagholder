//! The book's schema: every version is built from its migrations exactly as
//! committed, a released migration is never edited, and every earlier version
//! reaches the latest with its rows intact.

mod common;

use bagholder_book::schema::{MIGRATIONS, SCHEMA};
use bagholder_book::Book;
use bagholder_sqlite::migrate::{self, Schema};
use bagholder_core::account::{AccountRef, AccountStatus, AccountType};
use bagholder_core::Broker;
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
fn a_file_that_is_not_a_book_is_not_opened_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.db");
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
        // migration 37 forgets every full read, so the next sync reads whole: the
        // reads noted again, as that sync would
        for (n, a) in accounts.iter().enumerate() {
            assert_eq!(book.activity_read_at(*a).unwrap(), None);
            book.note_activity_read(*a, read_at[&n], true).unwrap();
        }
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

/// A book stored before migration 30 kept a sighting that states no venue as a
/// name of its own, on no venue; migrated, its names are the rule's (a sighting
/// stating none reads as on the venue before it), every instrument refolded once.
#[test]
fn a_book_from_before_migration_30_has_its_names_refolded_by_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.db");
    let partial: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..29] }));
    {
        let (c, _) = migrate::open(partial, &path, "test", t0()).unwrap();
        let id = "01a0ce23-1200-7000-8000-000000000001";
        c.execute_batch(&format!(
            "INSERT INTO instruments (id, kind, currency, created_at) VALUES ('{id}', 'security', 'CAD', '2026-03-01T00:00:00Z');
             INSERT INTO source_records (id, connection_id, source, source_key, state, derived_version, first_received_at, state_changed_at) VALUES
               ('01a0ce23-1200-7000-8000-0000000000a1', NULL, 'test-broker', 'a', 'live', 1, '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z'),
               ('01a0ce23-1200-7000-8000-0000000000a2', NULL, 'csv', 'b', 'live', 1, '2026-03-05T00:00:00Z', '2026-03-05T00:00:00Z');
             INSERT INTO instrument_sightings (record_id, leg, instrument_id, symbol, venue_mic, venue_name, name, day) VALUES
               ('01a0ce23-1200-7000-8000-0000000000a1', 'trade', '{id}', 'ZQV', 'XTSE', 'TSX', NULL, '2026-03-01'),
               ('01a0ce23-1200-7000-8000-0000000000a2', 'trade', '{id}', 'ZQV', NULL, NULL, NULL, '2026-03-05');
             -- the names as the fold before migration 30 kept them, nothing left due
             DELETE FROM instrument_names;
             INSERT INTO instrument_names VALUES
               ('{id}', '2026-03-01', 'ZQV', 1, '2026-03-01', 'ZQV', 1, 'ZQV', 'XTSE', 'TSX', NULL, 'test-broker'),
               ('{id}', '2026-03-05', 'ZQV', 2, '2026-03-05', 'ZQV', 2, 'ZQV', NULL, NULL, NULL, 'csv');
             DELETE FROM names_due;"
        ))
        .unwrap();
    }
    let (book, _) = Book::open(&path, "test", t0()).unwrap();
    let id = bagholder_core::InstrumentId::parse("01a0ce23-1200-7000-8000-000000000001").unwrap();
    let names: Vec<(String, Option<String>, Option<String>, String, String)> =
        book.names(id).unwrap().into_iter().map(|n| (n.symbol, n.venue_mic, n.venue_name, n.first_seen.to_string(), n.last_seen.to_string())).collect();
    assert_eq!(names, [("ZQV".to_string(), Some("XTSE".to_string()), Some("TSX".to_string()), "2026-03-01".to_string(), "2026-03-05".to_string())]);
}

/// What a book holds of the broker's and the person's, before migration 37: one
/// book per kind of brief 75 §2, built through the book's own writes. `pooled`:
/// the five broker ids on two accounts, of two and three ids, as the import
/// built them; else one account each. `imported`: the import's records in every
/// state beside the broker's, as a book that ran the import holds them.
struct Seeded {
    dir: tempfile::TempDir,
}

fn seeded(pooled: bool, imported: bool) -> Seeded {
    use bagholder_core::journal::{JournalEntry, JournalSubject};
    use serde_json::json;
    let f = Fixture::new();
    let t = |key: &str| -> AccountRef { AccountRef::new(Broker::named("wealthsimple"), key) };
    let ty = AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Tfsa, managed: false, joint: false };
    if pooled {
        f.book.add_account(f.connection, &[t("ws-1"), t("ws-2")], &ty, AccountStatus::Open, Some("Pooled two"), t0()).unwrap();
        f.book.add_account(f.connection, &[t("ws-3"), t("ws-4"), t("ws-5")], &ty, AccountStatus::Closed, Some("Pooled three"), t0()).unwrap();
    } else {
        f.book.add_account(f.connection, &[t("ws-1")], &ty, AccountStatus::Open, Some("Pooled two"), t0()).unwrap();
        f.book.add_account(f.connection, &[t("ws-2")], &ty, AccountStatus::Open, Some("Pooled two"), t0()).unwrap();
        for k in ["ws-3", "ws-4", "ws-5"] {
            f.book.add_account(f.connection, &[t(k)], &ty, AccountStatus::Closed, Some("Pooled three"), t0()).unwrap();
        }
    }
    let broker = Spelled::v(1);
    let store = |m: &Spelled, key: &str, v: serde_json::Value| f.book.store(m, &f.incoming(key, &v.to_string()), t0()).unwrap().record;
    let deposit = |a: &str, cash: &str, day: &str| json!({"leg": "cash", "account": a, "kind": "deposit", "cash": cash, "date": day});
    // the broker's own records, in every account
    let r1 = store(&broker, "b-1", legs(vec![deposit("ws-1", "1000", "2026-01-02"), buy("ws-1", share("CA0000000001", "ZQA"), "10", "-100", "2026-01-05T15:00:00Z")]));
    let r2 = store(&broker, "b-2", legs(vec![deposit("ws-2", "500", "2026-01-02"), buy("ws-2", share("CA0000000002", "ZQB"), "5", "-50", "2026-01-06T15:00:00Z")]));
    store(&broker, "b-3", legs(vec![deposit("ws-3", "300", "2026-01-03")]));
    let r4 = store(&broker, "b-4", legs(vec![deposit("ws-4", "400", "2026-01-03"), buy("ws-4", share("CA0000000004", "ZQD"), "4", "-40", "2026-01-07T15:00:00Z")]));
    store(&broker, "b-5", json!({"hold": {"account": "ws-5", "kind": "buy", "currency": "CAD", "amount": "25"}}));
    let (t1, t2, t4) = (f.book.open_trade(&f.opens(r1), t0()).unwrap(), f.book.open_trade(&f.opens(r2), t0()).unwrap(), f.book.open_trade(&f.opens(r4), t0()).unwrap());
    let note = |thesis: &str, tags: &[&str]| JournalEntry { thesis: thesis.into(), grade: None, tags: tags.iter().map(|s| s.to_string()).collect() };
    f.book.set_journal(JournalSubject::Trade(t1), &note("why one", &["a", "b"]), t0()).unwrap();
    let g = f.book.add_group(&[t2, t4], false, t0()).unwrap();
    f.book.set_journal(JournalSubject::Group(g), &note("why the two", &["c"]), t0()).unwrap();
    // a record of the person's own, superseded by the broker's row of it
    let csv = Spelled { source: "csv", version: 1 };
    let c1 = store(&csv, "c-1", legs(vec![deposit("ws-2", "70", "2026-01-09")]));
    let import = Spelled { source: "bagholder-import", version: 1 };
    if imported {
        // live import records: one booking a transfer out, one a trade with a note on it
        store(&import, "i-live", legs(vec![json!({"leg": "cash", "account": "ws-3", "kind": "transfer-out", "cash": "-300", "date": "2026-01-10"})]));
        let held = store(&import, "i-trade", legs(vec![buy("ws-1", share("CA0000000009", "ZQZ"), "1", "-9", "2026-01-08T15:00:00Z")]));
        let on = f.book.open_trade(&f.opens(held), t0()).unwrap();
        f.book.set_journal(JournalSubject::Trade(on), &note("why the import's", &[]), t0()).unwrap();
        // superseded by the broker's own row, and removed
        let sup = store(&import, "i-sup", legs(vec![deposit("ws-4", "60", "2026-01-11")]));
        f.book.store_superseding(&broker, &f.incoming("b-6", &legs(vec![deposit("ws-4", "60", "2026-01-11")]).to_string()), &[sup], "the broker's own row", t0()).unwrap();
        let rem = store(&import, "i-rem", legs(vec![deposit("ws-5", "1", "2026-01-12")]));
        f.book.mark_removed(rem, t0()).unwrap();
        // a link whose from side holds the person's record and an import record
        let il = store(&import, "i-link", legs(vec![deposit("ws-2", "70", "2026-01-09")]));
        f.book.store_superseding(&broker, &f.incoming("b-7", &legs(vec![deposit("ws-2", "70", "2026-01-09")]).to_string()), &[c1, il], "the broker's own row", t0()).unwrap();
    } else {
        store(&broker, "b-6", legs(vec![deposit("ws-4", "60", "2026-01-11")]));
        f.book.store_superseding(&broker, &f.incoming("b-7", &legs(vec![deposit("ws-2", "70", "2026-01-09")]).to_string()), &[c1], "the broker's own row", t0()).unwrap();
    }
    // a row in every table of the person's own
    let c = f.book.conn_for_tests();
    let inst: String = c.query_row("SELECT instrument_id FROM instrument_refs WHERE value = 'CA0000000001'", [], |r| r.get(0)).unwrap();
    c.execute_batch(&format!(
        "INSERT INTO brackets (id, broker, broker_account, broker_security, symbol, currency, created_at, phase, updated_at) VALUES ('bk-1', 'wealthsimple', 'ws-1', 'sec-1', 'ZQA', 'CAD', 'x', 'ended', 'x');
         INSERT INTO bracket_events (bracket_id, seq, at, asker, kind, body) VALUES ('bk-1', 1, 'x', 'person', 'imported', '{{}}');
         INSERT INTO orders (id, broker, broker_account, broker_security, symbol, currency, side, order_type, quantity, time_in_force, bracket_id, role, request, created_at, state, filled, updated_at)
            VALUES ('o-1', 'wealthsimple', 'ws-1', 'sec-1', 'ZQA', 'CAD', 'buy', 'limit', '1', 'day', 'bk-1', 'entry', '{{}}', 'x', 'filled', '1', 'x');
         INSERT INTO order_events (order_id, seq, at, asker, kind, body) VALUES ('o-1', 1, 'x', 'person', 'imported', '{{}}');
         INSERT INTO notifications (at, kind, key, title) VALUES ('x', 'fill', 'n-1', 'Filled');
         INSERT INTO told (scope, event, at) VALUES ('news:ZQA@TSX', 'e-1', 'x');
         INSERT INTO push_subscriptions (endpoint, p256dh, auth, made_at) VALUES ('https://push.example/1', 'k', 'a', 'x');
         INSERT INTO watched (instrument_id, added_at) VALUES ('{inst}', 'x');
         INSERT INTO tiles (instrument_id, position) VALUES ('{inst}', 1);"
    ))
    .unwrap();
    Seeded { dir: f.dir }
}

/// The seeded book as a version-36 file holds it: every row copied into a file
/// built from migrations 1 to 36, the change log's triggers set aside while the
/// rows go in, so it holds exactly the rows the book wrote. Then what only a
/// version-36 book can hold: the import's keys on a trade and a group, every
/// account's full read, and the marks of the carries and repairs.
fn as_v36(seeded: &Seeded) -> std::path::PathBuf {
    let path = seeded.dir.path().join("v36.db");
    let v36: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..36] }));
    let (conn, _) = migrate::open(v36, &path, "test", t0()).unwrap();
    // the rows go in table by table: each reference holds once all are in
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute("ATTACH DATABASE ?1 AS src", [seeded.dir.path().join("book.db").to_string_lossy()]).unwrap();
    let triggers: Vec<(String, String)> = conn.prepare("SELECT name, sql FROM main.sqlite_master WHERE type = 'trigger'").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    for (name, _) in &triggers {
        conn.execute_batch(&format!("DROP TRIGGER main.\"{name}\"")).unwrap();
    }
    let tables: Vec<String> = conn.prepare("SELECT name FROM main.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'schema_migrations'").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    let columns = |schema: &str, t: &str| -> Vec<String> { conn.prepare(&format!("SELECT name FROM pragma_table_info('{t}', '{schema}')")).unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect() };
    for t in &tables {
        let theirs = columns("src", t);
        let both: Vec<String> = columns("main", t).into_iter().filter(|c| theirs.contains(c)).map(|c| format!("\"{c}\"")).collect();
        // a row a migration seeded (`successions_due`) is the seeded book's own
        conn.execute_batch(&format!("DELETE FROM main.\"{t}\"; INSERT INTO main.\"{t}\" ({0}) SELECT {0} FROM src.\"{t}\"", both.join(", "))).unwrap();
    }
    for (_, sql) in &triggers {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch(
        "UPDATE trades SET legacy_key = 'old-' || rowid;
         UPDATE trade_groups SET legacy_key = 'old-group-' || rowid;
         INSERT INTO activity_reads (account_id, read_at, complete, at_ns) SELECT id, '2026-01-20T00:00:00Z', 1, 1 FROM accounts;
         INSERT INTO settings (key, value, source, set_at) VALUES ('carried.notices', '1', 'app', 'x'), ('following.carried', '1', 'app', 'x'),
            ('carried.told-by-listing', '1', 'app', 'x'), ('repair.carried-notes-reattached', '1', 'app', 'x'), ('statements.said', '1', 'app', 'x');",
    )
    .unwrap();
    drop(conn);
    path
}

/// Each broker record's legs as the book holds them: its key and leg, the broker
/// id of the account the leg is on, and what it moves.
fn broker_legs(book: &Book) -> std::collections::BTreeSet<(String, String, String, String)> {
    let c = book.conn_for_tests();
    c.prepare(
        "SELECT r.source_key, t.leg, (SELECT GROUP_CONCAT(value) FROM account_refs WHERE account_id = t.account_id), t.kind || ' ' || COALESCE(t.quantity, '') || ' ' || COALESCE(t.cash, '')
         FROM transactions t JOIN source_records r ON r.id = t.record_id WHERE r.source = 'test-broker'",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Every row of the person's own tables, as text.
fn persons_rows(c: &rusqlite::Connection) -> Vec<String> {
    let mut out = Vec::new();
    for t in ["journal", "journal_tags", "trade_groups", "trade_group_members", "orders", "order_events", "brackets", "bracket_events", "notifications", "told", "push_subscriptions", "watched", "tiles"] {
        let cols: Vec<String> = c.prepare(&format!("SELECT name FROM pragma_table_info('{t}') WHERE name <> 'legacy_key'")).unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        let sql = format!("SELECT {} FROM \"{t}\" ORDER BY 1", cols.iter().map(|c| format!("quote(\"{c}\")")).collect::<Vec<_>>().join(" || '|' || "));
        for row in c.prepare(&sql).unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap() {
            out.push(format!("{t}: {}", row.unwrap()));
        }
    }
    out
}

/// Migration 37 (brief 75 §1, §2), over a book of every kind: the import's
/// records go in every state with all they derived; a trade anchored on a live
/// one is orphaned with its note kept; the person's rows stay field for field;
/// the CSV record a link still holds stays superseded; the columns kept for the
/// import go, and every full read, and the marks; each account the import pooled
/// is split to one per broker id with every trade's anchor unchanged; and the
/// broker's records stand exactly as in a book that never ran the import.
#[test]
fn migration_37_leaves_only_the_broker_s_and_the_person_s_records_and_a_migrated_book_equals_a_fresh_one() {
    let fresh = seeded(false, false);
    let (fresh_book, _) = Book::open_in(fresh.dir.path(), "test", t0()).unwrap();
    for (pooled, imported) in [(false, false), (false, true), (true, false), (true, true)] {
        let what = format!("pooled {pooled}, imported {imported}");
        let s = seeded(pooled, imported);
        let path = as_v36(&s);
        let (before, persons) = {
            let c = bagholder_sqlite::open_db(&path).unwrap();
            let anchors: Vec<(String, String)> = c.prepare("SELECT id, COALESCE(anchor_record, '') || '/' || COALESCE(anchor_leg, '') FROM trades ORDER BY id").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            (anchors, persons_rows(&c))
        };
        // a pooled book cannot be split without the mappings: refused, left at 36
        if pooled {
            assert!(Book::open(&path, "test", t0()).is_err(), "{what}");
            assert_eq!(migrate::version(&bagholder_sqlite::open_db(&path).unwrap()).unwrap(), 36, "{what}");
        }
        // opened again later, with them (a snapshot is named for its second)
        let (book, done) = Book::open_mapped(&path, "test", at("2026-09-23T12:00:01Z"), &[&Spelled::v(1)]).unwrap();
        assert_eq!((done.from, done.to), (36, MIGRATIONS.len() as u32), "{what}");
        let c = book.conn_for_tests();
        let count = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM source_records WHERE source = 'bagholder-import'"), 0, "{what}");
        for (t, col) in [("transactions", "record_id"), ("record_revisions", "record_id"), ("record_refs", "record_id"), ("record_problems", "record_id"), ("record_holds", "record_id"), ("link_records", "record_id"), ("adjustments", "record_id")] {
            assert_eq!(count(&format!("SELECT COUNT(*) FROM {t} WHERE {col} NOT IN (SELECT id FROM source_records)")), 0, "{what}: {t}");
        }
        assert_eq!(count("SELECT COUNT(*) FROM pragma_table_info('trades') WHERE name = 'legacy_key'") + count("SELECT COUNT(*) FROM pragma_table_info('trade_groups') WHERE name = 'legacy_key'"), 0, "{what}");
        assert_eq!(count("SELECT COUNT(*) FROM activity_reads"), 0, "{what}");
        assert_eq!(count("SELECT COUNT(*) FROM settings WHERE key IN ('carried.notices', 'following.carried', 'carried.told-by-listing', 'repair.carried-notes-reattached', 'statements.said')"), 0, "{what}");
        // one account per broker id
        assert_eq!(count("SELECT COUNT(*) FROM (SELECT account_id FROM account_refs GROUP BY account_id HAVING COUNT(*) > 1)"), 0, "{what}");
        assert_eq!(count("SELECT COUNT(*) FROM accounts"), 5, "{what}");
        // the broker's records as a fresh book holds them
        assert_eq!(broker_legs(&book), broker_legs(&fresh_book), "{what}");
        assert_eq!(persons_rows(c), persons, "{what}: the person's rows, field for field");
        // every trade on a broker record keeps its anchor; the import's is orphaned with its note
        let after: std::collections::BTreeMap<String, (String, Option<String>)> = c
            .prepare("SELECT id, COALESCE(anchor_record, '') || '/' || COALESCE(anchor_leg, ''), orphaned_reason FROM trades")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (id, anchor) in &before {
            match after.get(id) {
                Some((a, None)) => assert_eq!(a, anchor, "{what}: trade {id} keeps its anchor"),
                Some((a, Some(why))) => {
                    assert!(imported && a == "/", "{what}: trade {id} orphaned: {why}");
                    assert_eq!(count(&format!("SELECT COUNT(*) FROM journal WHERE trade_id = '{id}' AND thesis = 'why the import''s'")), 1, "{what}: its note kept");
                }
                None => panic!("{what}: trade {id} gone"),
            }
        }
        // the person's record a link still holds stays superseded
        assert_eq!(c.query_row("SELECT state FROM source_records WHERE source_key = 'c-1'", [], |r| r.get::<_, String>(0)).unwrap(), "superseded", "{what}");
        assert_eq!(count("SELECT COUNT(*) FROM links l WHERE NOT EXISTS (SELECT 1 FROM link_records f WHERE f.link_id = l.id AND f.side = 'from')"), 0, "{what}");
        drop(book);
    }
}

/// Each broker record's legs with the broker ids of the account each is on,
/// sorted, as a connection holds them.
fn legs_by_ids(c: &rusqlite::Connection) -> std::collections::BTreeSet<(String, String, String)> {
    c.prepare(
        "SELECT r.source_key, t.leg, (SELECT GROUP_CONCAT(value, ',') FROM (SELECT value FROM account_refs WHERE account_id = t.account_id ORDER BY value))
         FROM transactions t JOIN source_records r ON r.id = t.record_id",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Migration 39: a book migration 37 split, whose ids the broker states as
/// linked (as an account given a new id is), is one account per linked set
/// again, each record's legs on the account of the same ids as before the
/// split, every trade's anchor and every note unchanged, open where any id is;
/// the set's history goes, for the next sync to read whole. A book whose
/// accounts the broker links to none is left exactly as it was.
#[test]
fn migration_39_makes_each_set_the_broker_links_one_account_again() {
    for linked in [true, false] {
        let what = format!("linked {linked}");
        let s = seeded(true, false);
        let path = as_v36(&s);
        let pooled = legs_by_ids(&bagholder_sqlite::open_db(&path).unwrap());
        // at version 38: split by migration 37, then the broker's links read
        let v38: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..38] }));
        let mapping = Spelled::v(1);
        let split = |c: &rusqlite::Connection| bagholder_book::pooled::step(c, &[&mapping], t0());
        let (c, _) = migrate::open_with(v38, &path, "test", t0(), &[(37, &split)]).unwrap();
        let account = |key: &str| -> String { c.query_row("SELECT account_id FROM account_refs WHERE value = ?1", [key], |r| r.get(0)).unwrap() };
        if linked {
            let read: String = c.query_row("SELECT id FROM broker_reads LIMIT 1", [], |r| r.get(0)).unwrap_or_else(|_| {
                c.execute("INSERT INTO broker_reads (id, connection_id, part, at) SELECT 'read-1', connection_id, 'accounts', 'x' FROM accounts LIMIT 1", []).unwrap();
                "read-1".into()
            });
            for (a, b) in [("ws-1", "ws-2"), ("ws-2", "ws-1"), ("ws-3", "ws-4"), ("ws-4", "ws-5")] {
                c.execute("INSERT INTO account_links (account_id, linked_to, read_id) VALUES (?1, ?2, ?3)", rusqlite::params![account(a), account(b), read]).unwrap();
            }
            c.execute("INSERT INTO activity_reads (account_id, read_at, complete) VALUES (?1, 'x', 1)", [account("ws-2")]).unwrap();
        }
        c.execute("UPDATE accounts SET status = 'open' WHERE id = ?1", [account("ws-5")]).unwrap();
        let (persons, anchors, legs_split, accounts_split) = (
            persons_rows(&c),
            c.prepare("SELECT id, COALESCE(anchor_record, '') || '/' || COALESCE(anchor_leg, '') FROM trades ORDER BY id").unwrap().query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>(),
            legs_by_ids(&c),
            c.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get::<_, i64>(0)).unwrap(),
        );
        assert_eq!(accounts_split, 5, "{what}: split to one account per id");
        drop(c);
        let (book, done) = Book::open(&path, "test", at("2026-10-24T12:00:01Z")).unwrap();
        assert_eq!((done.from, done.to), (38, MIGRATIONS.len() as u32), "{what}");
        let c = book.conn_for_tests();
        let count = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(persons_rows(c), persons, "{what}: the person's rows, field for field");
        let after: Vec<(String, String)> = c.prepare("SELECT id, COALESCE(anchor_record, '') || '/' || COALESCE(anchor_leg, '') FROM trades ORDER BY id").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(after, anchors, "{what}: every trade's anchor");
        if linked {
            assert_eq!(count("SELECT COUNT(*) FROM accounts"), 2, "{what}");
            assert_eq!(legs_by_ids(c), pooled, "{what}: each leg on the account of the same ids as before the split");
            let status = |key: &str| -> String { c.query_row("SELECT a.status FROM accounts a JOIN account_refs r ON r.account_id = a.id WHERE r.value = ?1", [key], |r| r.get(0)).unwrap() };
            assert_eq!(status("ws-3"), "open", "{what}: open where any of its ids is");
            assert_eq!(count("SELECT COUNT(*) FROM account_links") + count("SELECT COUNT(*) FROM activity_reads"), 0, "{what}: the set's history, for the next sync");
        } else {
            assert_eq!(count("SELECT COUNT(*) FROM accounts"), 5, "{what}");
            assert_eq!(legs_by_ids(c), legs_split, "{what}: left as it was");
        }
    }
}

/// A merge that cannot place everything on the kept account changes nothing:
/// a row naming the account it would remove stops it, and the book is as it was.
#[test]
fn a_merge_that_cannot_place_a_row_fails_whole() {
    let f = Fixture::new();
    let t = |key: &str| -> AccountRef { AccountRef::new(Broker::named("wealthsimple"), key) };
    let ty = AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Tfsa, managed: false, joint: false };
    let a = f.book.add_account(f.connection, &[t("ws-1")], &ty, AccountStatus::Closed, Some("Old"), t0()).unwrap();
    let b = f.book.add_account(f.connection, &[t("ws-2")], &ty, AccountStatus::Open, Some("New"), at("2026-01-02T00:00:00Z")).unwrap();
    let c = f.book.conn_for_tests();
    c.execute_batch(&format!("CREATE TABLE elsewhere (account_id TEXT REFERENCES accounts(id)); INSERT INTO elsewhere VALUES ('{a}'), ('{b}');")).unwrap();
    let read = f.book.broker_read(f.connection, "accounts", t0()).unwrap();
    assert!(f.book.store_account_links(f.connection, &[(a, b), (b, a)], &read).is_err());
    let count = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!((count("SELECT COUNT(*) FROM accounts"), count("SELECT COUNT(*) FROM account_links")), (2, 0), "nothing changed");
    assert_eq!((f.book.account_by_ref(&t("ws-1")).unwrap(), f.book.account_by_ref(&t("ws-2")).unwrap()), (Some(a), Some(b)));
}

/// Migration 40: the empty cash and units statements 2.10.0 to 2.10.2 wrote for
/// every closed account go; a closed account's statement from when it was open,
/// which has its rows, stays, and an open account's statements are untouched.
#[test]
fn migration_40_drops_only_a_closed_account_s_statements_of_nothing() {
    let f = Fixture::new();
    let t = |key: &str| -> AccountRef { AccountRef::new(Broker::named("wealthsimple"), key) };
    let ty = AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Tfsa, managed: false, joint: false };
    let closed = f.book.add_account(f.connection, &[t("ws-1")], &ty, AccountStatus::Closed, None, t0()).unwrap();
    let open = f.book.add_account(f.connection, &[t("ws-2")], &ty, AccountStatus::Open, None, t0()).unwrap();
    let cad = |v: &str| std::collections::BTreeMap::from([(bagholder_core::Currency::CAD, bagholder_core::Dec::parse(v).unwrap())]);
    let read = f.book.broker_read(f.connection, "cash", t0()).unwrap();
    // when it was open: a real statement
    f.book.store_cash(closed, at("2026-01-02T00:00:00Z"), &cad("100"), &read).unwrap();
    // since 2.10.0: stated as holding nothing
    f.book.store_cash(closed, at("2026-10-23T00:00:00Z"), &std::collections::BTreeMap::new(), &read).unwrap();
    f.book.store_units(closed, "2026-10-23".parse().unwrap(), &[], &Default::default(), &read, at("2026-10-23T00:00:00Z")).unwrap();
    // an open account's statement of no cash
    f.book.store_cash(open, at("2026-10-23T00:00:00Z"), &std::collections::BTreeMap::new(), &read).unwrap();
    let c = f.book.conn_for_tests();
    let kinds = |a: bagholder_core::AccountId| -> Vec<(String, i64)> {
        c.prepare("SELECT kind, (SELECT COUNT(*) FROM statement_cash WHERE statement_id = s.id) FROM statements s WHERE account_id = ?1 ORDER BY kind, rowid").unwrap().query_map([a.to_string()], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(kinds(closed), vec![("cash".to_string(), 1), ("cash".to_string(), 0), ("units".to_string(), 0)]);
    let open_before = kinds(open);
    c.execute_batch(MIGRATIONS[39].sql).unwrap();
    assert_eq!(kinds(closed), vec![("cash".to_string(), 1)], "the statement with its rows stands");
    assert_eq!(kinds(open), open_before, "an open account's, untouched");
}

/// A key made before keys named their server was made for the one address
/// Bagholder had, kept in `agents.host`: migration 44 gives it that server. One made
/// while no address was set up is left without one.
#[test]
fn migration_44_gives_each_key_the_one_address_it_was_made_for() {
    for host in [Some("agents.example.com"), None] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(bagholder_book::BOOK_FILE);
        let v43: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..43] }));
        {
            let (c, _) = migrate::open(v43, &path, "test", t0()).unwrap();
            c.execute("INSERT INTO agent_keys(id, name, way, hash, made_at) VALUES ('bhk_1', 'Muse', 'api', x'01', '2026-10-20T00:00:00Z')", []).unwrap();
            if let Some(h) = host {
                c.execute("INSERT INTO settings(key, value, source, set_at) VALUES ('agents.host', ?1, 'person', '2026-10-20T00:00:00Z')", [h]).unwrap();
            }
        }
        let (book, _) = Book::open_in(dir.path(), "test", t0()).unwrap();
        let keys = book.agents().keys().unwrap();
        assert_eq!(keys[0].at, host.map(|h| format!("https://{h}")), "{host:?}");
    }
}
