//! The notices in the book: cleared with what the person follows, and only then.

mod common;

use bagholder_book::clear::Clearing;
use bagholder_book::notices::{SEEN, SETTINGS};
use common::*;

fn count(book: &bagholder_book::Book, table: &str) -> i64 {
    book.notices().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn clearing_what_the_person_follows_empties_the_notices() {
    let f = Fixture::new();
    f.book
        .notices()
        .execute_batch(
            "INSERT INTO notifications (at, kind, key, title, body, extra, seen_at, read_at) VALUES
                ('2026-09-01T10:00:00Z', 'fills', 'order:1:filled', 'Order filled · QNC', 'Bought 5 at 1.75', '{\"symbol\":\"QNC\"}', '2026-09-01T10:00:00Z', NULL),
                ('2026-09-02T11:00:00Z', 'releases', 'release:QNC:abc', 'Release · QNC', NULL, NULL, NULL, '2026-09-03T09:00:00Z');
             INSERT INTO told VALUES ('news:QNC@TSX', 'a release', '2026-09-02T11:00:00Z');",
        )
        .unwrap();
    f.book.set_setting(SETTINGS, Some("{\"fills\":true}"), t0()).unwrap();
    f.book.set_setting(&format!("{SEEN}news:QNC@TSX"), Some("2026-09-02|a"), t0()).unwrap();
    f.book.clear(&Clearing { broker: true, entries: true, journal: true, market: true, orders: true, following: false, agents: false }).unwrap();
    assert_eq!(count(&f.book, "notifications"), 2, "only Settings clears them");
    f.book.clear(&Clearing { following: true, ..Clearing::default() }).unwrap();
    for t in ["notifications", "told"] {
        assert_eq!(count(&f.book, t), 0, "{t}");
    }
    assert_eq!(f.book.setting(SETTINGS).unwrap(), None);
    assert_eq!(f.book.setting(&format!("{SEEN}news:QNC@TSX")).unwrap(), None);
}
