//! The notices in the book: carried once from the earlier store with every value,
//! the earlier store only read, and cleared with what the person follows.

mod common;

use bagholder_book::clear::Clearing;
use bagholder_book::notices::{CarriedNotices, SEEN, SETTINGS};
use common::*;

/// An earlier store as its schema keeps the notices, with rows in each.
fn old_store(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("bagholder.db");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE told (scope TEXT NOT NULL, event TEXT NOT NULL, at TEXT NOT NULL, PRIMARY KEY (scope, event));
         CREATE TABLE notifications (id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, kind TEXT NOT NULL, key TEXT NOT NULL UNIQUE, title TEXT NOT NULL, body TEXT, extra TEXT, seen_at TEXT, read_at TEXT);
         INSERT INTO notifications (id, at, kind, key, title, body, extra, seen_at, read_at) VALUES
            (3, '2026-09-01T10:00:00Z', 'fills', 'order:1:filled', 'Order filled · QNC', 'Bought 5 at 1.75', '{\"symbol\":\"QNC\"}', '2026-09-01T10:00:00Z', NULL),
            (7, '2026-09-02T11:00:00Z', 'releases', 'release:QNC:abc', 'Release · QNC', NULL, NULL, NULL, '2026-09-03T09:00:00Z');
         -- a row deleted after: the next id follows the earlier store's count
         UPDATE sqlite_sequence SET seq = 9 WHERE name = 'notifications';
         INSERT INTO told VALUES ('news:QNC@TSX', 'a release', '2026-09-02T11:00:00Z'), ('filings:QNC:SEDAR+', 'x|y', '2026-09-02T12:00:00Z');
         INSERT INTO meta VALUES ('notify_settings', '{\"fills\":true}'), ('notify_seen:news:QNC@TSX', '2026-09-02|a'), ('notify_seen:cleared', ''), ('bars_source:QNC', 'tmx');",
    )
    .unwrap();
    path
}

fn rows(book: &bagholder_book::Book, sql: &str) -> Vec<Vec<Option<String>>> {
    let c = book.notices();
    let mut stmt = c.prepare(sql).unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |r| (0..n).map(|i| r.get_ref(i).map(|v| match v {
        rusqlite::types::ValueRef::Null => None,
        rusqlite::types::ValueRef::Integer(i) => Some(i.to_string()),
        rusqlite::types::ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        other => Some(format!("{other:?}")),
    })).collect::<rusqlite::Result<Vec<_>>>()).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

#[test]
fn the_earlier_stores_notices_are_carried_once_with_every_value() {
    let f = Fixture::new();
    let old = old_store(f.dir.path());
    let before = std::fs::read(&old).unwrap();
    let carried = f.book.carry_notices(&old, t0()).unwrap();
    assert_eq!(carried, Some(CarriedNotices { notifications: 2, told: 2, settings: 2 }));
    let o = rusqlite::Connection::open(&old).unwrap();
    let old_rows = |sql: &str| -> Vec<Vec<Option<String>>> {
        let mut stmt = o.prepare(sql).unwrap();
        let n = stmt.column_count();
        stmt.query_map([], |r| (0..n).map(|i| r.get_ref(i).map(|v| match v {
            rusqlite::types::ValueRef::Null => None,
            rusqlite::types::ValueRef::Integer(i) => Some(i.to_string()),
            rusqlite::types::ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
            other => Some(format!("{other:?}")),
        })).collect::<rusqlite::Result<Vec<_>>>()).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    };
    for sql in ["SELECT * FROM notifications ORDER BY id", "SELECT * FROM told ORDER BY scope, event"] {
        assert_eq!(rows(&f.book, sql), old_rows(sql), "{sql}");
    }
    assert_eq!(f.book.setting(SETTINGS).unwrap().as_deref(), Some("{\"fills\":true}"));
    assert_eq!(f.book.setting(&format!("{SEEN}news:QNC@TSX")).unwrap().as_deref(), Some("2026-09-02|a"));
    assert_eq!(f.book.setting(&format!("{SEEN}cleared")).unwrap(), None, "an emptied mark is nothing to carry");
    assert_eq!(f.book.setting("bars_source:QNC").unwrap(), None, "a reader's key is the market cache's");
    // the next notification is numbered after the earlier store's last
    f.book.notices().execute("INSERT INTO notifications (at, kind, key, title) VALUES ('x', 'test', 'k', 't')", []).unwrap();
    assert_eq!(f.book.notices().last_insert_rowid(), 10);
    // a second start carries nothing, and the earlier store was only read
    assert_eq!(f.book.carry_notices(&old, t0()).unwrap(), None);
    drop(o);
    assert_eq!(std::fs::read(&old).unwrap(), before);
}

#[test]
fn an_earlier_store_without_notices_carries_nothing_and_is_marked() {
    let f = Fixture::new();
    let old = f.dir.path().join("bagholder.db");
    rusqlite::Connection::open(&old).unwrap().execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    assert_eq!(f.book.carry_notices(&old, t0()).unwrap(), Some(CarriedNotices::default()));
    assert!(f.book.notices_carried().unwrap());
}

#[test]
fn clearing_what_the_person_follows_empties_the_notices_and_keeps_the_carry_mark() {
    let f = Fixture::new();
    let old = old_store(f.dir.path());
    f.book.carry_notices(&old, t0()).unwrap();
    f.book.clear(&Clearing { broker: true, entries: true, journal: true, market: true, orders: true, following: false }).unwrap();
    assert_eq!(rows(&f.book, "SELECT COUNT(*) FROM notifications"), vec![vec![Some("2".to_string())]], "only Settings clears them");
    f.book.clear(&Clearing { following: true, ..Clearing::default() }).unwrap();
    for t in ["notifications", "told"] {
        assert_eq!(rows(&f.book, &format!("SELECT COUNT(*) FROM {t}")), vec![vec![Some("0".to_string())]], "{t}");
    }
    assert_eq!(f.book.setting(SETTINGS).unwrap(), None);
    assert_eq!(f.book.setting(&format!("{SEEN}news:QNC@TSX")).unwrap(), None);
    assert!(f.book.notices_carried().unwrap(), "cleared, not carried again at the next start");
}
