//! The zone a page states (`docs/plans/stage-3c-switch.md`, §2, "The zone").

use bagholder_book::{Book, BookError};

fn t(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

#[test]
fn the_latest_zone_a_page_states_is_kept_and_a_name_with_no_rules_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(dir.path(), "test", t("2026-09-25T12:00:00Z")).unwrap();
    assert!(book.zone().unwrap().is_none(), "no page has stated one");
    // every zone the database holds can be stated and read back
    for name in jiff::tz::TimeZoneDatabase::bundled().available() {
        let name = name.as_str();
        book.state_zone(name, t("2026-09-25T12:00:00Z")).unwrap();
        assert_eq!(book.zone().unwrap().unwrap().name, name);
    }
    assert!(book.state_zone("Pacific/Auckland", t("2026-09-25T12:01:00Z")).unwrap());
    assert!(!book.state_zone("Pacific/Auckland", t("2026-09-25T12:02:00Z")).unwrap(), "the same zone again changes nothing");
    let held = book.zone().unwrap().unwrap();
    assert_eq!((held.name.as_str(), held.stated_at), ("Pacific/Auckland", t("2026-09-25T12:01:00Z")));
    assert!(matches!(book.state_zone("Mars/Olympus", t("2026-09-25T12:03:00Z")), Err(BookError::Refused(_))));
    // kept across opening the book again
    drop(book);
    let (book, _) = Book::open_in(dir.path(), "test", t("2026-09-25T13:00:00Z")).unwrap();
    assert_eq!(book.zone().unwrap().unwrap().name, "Pacific/Auckland");
}
