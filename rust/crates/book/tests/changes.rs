//! The record's change log (migration 028, `docs/plans/stage-3-the-record-by-change.md`,
//! A): written in the writer's own transaction on any connection, never numbered
//! below what was handed out before, and nothing for a write that rolled back.

mod common;

use common::*;

#[test]
fn a_write_is_logged_in_its_own_transaction_and_a_rolled_back_one_leaves_nothing() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let before = f.book.changes_upto().unwrap();
    let failed: bagholder_book::Result<()> = f.book.atomically(|| {
        f.store(&Spelled::v(1), "b1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "10", "-100", "2026-01-02T15:00:00Z")]));
        Err(bagholder_book::BookError::Refused("stopped".into()))
    });
    assert!(failed.is_err());
    assert!(f.book.changes_since(before).unwrap().is_empty(), "a rolled-back write left a change");
    let stored = f.store(&Spelled::v(1), "b1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let changes = f.book.changes_since(before).unwrap();
    assert!(changes.records.contains(&stored.record));
    assert!(changes.upto > before);
}

#[test]
fn a_write_through_another_connection_is_logged() {
    let f = Fixture::new();
    let a = f.account(&["a1"]);
    let before = f.book.changes_upto().unwrap();
    let other = rusqlite::Connection::open(f.dir.path().join(bagholder_book::BOOK_FILE)).unwrap();
    other.execute("UPDATE accounts SET nickname = 'Renamed' WHERE id = ?1", [a.to_string()]).unwrap();
    let changes = f.book.changes_since(before).unwrap();
    assert_eq!(changes.accounts.into_iter().collect::<Vec<_>>(), [a]);
}

/// The sequence is never handed out again: pruned to empty, the next write is
/// still past what was applied (brief 26, 2).
#[test]
fn the_sequence_never_goes_back_after_the_log_is_pruned() {
    let f = Fixture::new();
    f.account(&["a1"]);
    f.store(&Spelled::v(1), "b1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let applied = f.book.changes_upto().unwrap();
    assert!(applied > 0);
    f.book.forget_changes(applied).unwrap();
    assert!(f.book.changes_since(0).unwrap().is_empty(), "pruned");
    let stored = f.store(&Spelled::v(1), "b2", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "5", "-50", "2026-01-03T15:00:00Z")]));
    let changes = f.book.changes_since(applied).unwrap();
    assert!(changes.records.contains(&stored.record), "a write after the prune was numbered at or below what was applied");
}

/// A clear is logged like any write (the build after it drops what it built
/// from), and the sequence goes on past it (brief 27, 6).
#[test]
fn a_clear_is_logged_and_the_sequence_goes_on() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let stored = f.store(&Spelled::v(1), "b1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let before = f.book.changes_upto().unwrap();
    f.book.clear(&bagholder_book::clear::Clearing::all()).unwrap();
    let changes = f.book.changes_since(before).unwrap();
    assert!(changes.records.contains(&stored.record), "the cleared record is not logged: {changes:?}");
    let after = changes.upto;
    // the clear took the connection nothing named any more: a new one, and an account on it
    let connection = f.book.add_connection(&bagholder_core::Broker::named("wealthsimple"), "Wealthsimple", t0()).unwrap();
    let t = bagholder_core::account::AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Tfsa, managed: false, joint: false };
    f.book.add_account(connection, &[bagholder_core::account::AccountRef::new(bagholder_core::Broker::named("wealthsimple"), "a2")], &t, bagholder_core::account::AccountStatus::Open, None, t0()).unwrap();
    assert!(!f.book.changes_since(after).unwrap().accounts.is_empty(), "a write after the clear numbered at or below it");
}

/// A supersede logs both records: a broker record that takes a file row's place
/// is read again with every leg it has (brief 27, 4a).
#[test]
fn a_supersede_logs_both_records() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let from = f.store(&Spelled::v(1), "f1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "10", "-100", "2026-01-02T15:00:00Z")])).record;
    let to = f.store(&Spelled::v(1), "b1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "10", "-100", "2026-01-02T15:00:00Z")])).record;
    let before = f.book.changes_upto().unwrap();
    f.book.supersede(&[from], &[to], "the same fill", t0()).unwrap();
    let changes = f.book.changes_since(before).unwrap();
    assert!(changes.records.contains(&from) && changes.records.contains(&to), "{changes:?}");
}
