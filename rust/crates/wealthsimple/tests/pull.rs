//! The pull reads only what changed (brief 07 §1, `docs/decisions.md`: calls to
//! Wealthsimple's unofficial API are kept to what is necessary), held by counting
//! what it asks. One account's November, from the owner's own history
//! (`tests/replies/wealthsimple-pull`).

use std::path::{Path, PathBuf};

use bagholder_book::mapping::{Draft, MapContext, Mapped, Mapping};
use bagholder_book::records::Incoming;
use bagholder_book::Book;
use bagholder_broker::pull::{pull, Report};
use bagholder_core::account::{AccountRef, AccountStatus};
use bagholder_core::json::Value;
use bagholder_core::record::RecordState;
use bagholder_core::transaction::Kind;
use bagholder_core::{Broker, Currency, Dec, Leg, Money, SourceName};
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_sources::reply::Node;
use bagholder_wealthsimple::replay::Replay;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-pull")
}

fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

fn connection(book: &Book, now: &str) -> bagholder_core::ConnectionId {
    match book.connections().unwrap().into_iter().find(|c| c.broker == Broker::named("wealthsimple")) {
        Some(c) => c.id,
        None => book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", at(now)).unwrap(),
    }
}

/// One pull of the replies in `replies` into `book`: what it did, and what it asked.
fn once(book: &Book, replies: &Path, now: &str) -> (Report, Vec<String>) {
    once_with(book, replies, now, |_| {})
}

/// One pull, the feed's rows edited first.
fn once_with(book: &Book, replies: &Path, now: &str, edit: impl FnOnce(&mut Vec<Value>)) -> (Report, Vec<String>) {
    let connection = connection(book, now);
    let mut replay = Replay::read(replies).unwrap();
    edit(&mut replay.rows);
    let mut ws = Wealthsimple::new(replay);
    let r = pull(book, &mut ws, connection, "2025-11-19".parse().unwrap(), at(now)).unwrap();
    (r, ws.source.asked.clone())
}

#[test]
fn a_pull_with_nothing_new_asks_only_the_accounts_their_activity_and_their_cash() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    let (first, _) = once(&book, &dir(), "2025-11-19T20:00:00Z");
    // one month of rows names only some of what the account holds: the rest
    // of its positions are named, not dropped
    assert_eq!(first.failures.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["units:anon-tfsa-1"]);
    assert_eq!(first.records_new, 53);
    // the same day again: nothing new to read, nothing stated to read again
    let (second, asked) = once(&book, &dir(), "2025-11-19T21:00:00Z");
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert_eq!((second.records_new, second.records_revised), (0, 0));
    assert_eq!(asked, vec!["accounts", "activity anon-tfsa-1", "balances"]);
}

#[test]
fn a_pull_with_one_new_trade_asks_only_what_the_trade_needs_besides() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    once(&book, &dir(), "2025-11-19T20:00:00Z");
    // the same replies, and one trade Wealthsimple has posted since
    let later = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let to = name.strip_suffix(".later").map(str::to_string).unwrap_or(name);
        std::fs::copy(&p, later.path().join(to)).unwrap();
    }
    let (third, asked) = once(&book, later.path(), "2025-11-19T22:00:00Z");
    assert!(third.failures.is_empty(), "{:?}", third.failures);
    assert_eq!(third.records_new, 1);
    assert_eq!(asked, vec!["accounts", "activity anon-tfsa-1", "securities 1", "balances"]);
}

/// One of the fixture's rows, under another id, with another status.
fn row_as(rows: &[Value], key: &str, status: &str) -> Value {
    let mut v = rows.iter().find(|r| Node::root(r).text("type").ok() == Some("DIY_BUY")).unwrap().clone();
    if let Value::Object(o) = &mut v {
        o.insert("canonicalId".into(), Value::String(key.into()));
        o.insert("unifiedStatus".into(), Value::String(status.into()));
        o.insert("occurredAt".into(), Value::String("2025-11-18T15:00:00.000000+00:00".into()));
    }
    v
}

fn state(book: &Book, source: &'static str, key: &str) -> RecordState {
    let r = book.record_by_key(Some(connection(book, "2025-11-19T20:00:00Z")), &SourceName::named(source), key).unwrap().unwrap();
    book.record(r).unwrap().state
}

#[test]
fn a_pending_row_posted_again_under_another_id_leaves_the_book() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    let (first, _) = once_with(&book, &dir(), "2025-11-19T20:00:00Z", |rows| {
        let pending = row_as(rows, "anon-pending-1", "PENDING");
        rows.push(pending);
    });
    assert_eq!((first.records_new, first.removed), (54, 0));
    // final now: Wealthsimple lists it under another id, and the pending one no longer
    let (second, _) = once_with(&book, &dir(), "2025-11-19T21:00:00Z", |rows| {
        let settled = row_as(rows, "anon-settled-1", "COMPLETED");
        rows.push(settled);
    });
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert_eq!((second.records_new, second.removed), (1, 1));
    assert_eq!(state(&book, "wealthsimple", "anon-pending-1"), RecordState::Removed);
    assert_eq!(state(&book, "wealthsimple", "anon-settled-1"), RecordState::Live);
    // nothing left pending: the next pull reads from its last full read, and removes nothing
    let (third, _) = once_with(&book, &dir(), "2025-11-19T22:00:00Z", |rows| {
        let settled = row_as(rows, "anon-settled-1", "COMPLETED");
        rows.push(settled);
    });
    assert_eq!((third.records_new, third.removed), (0, 0));
}

/// An imported record: one cash leg, known by the broker's id of its row.
struct Imported;

impl Mapping for Imported {
    fn source(&self) -> SourceName {
        SourceName::named("bagholder-import")
    }
    fn version(&self) -> u32 {
        1
    }
    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let v = bagholder_core::json::parse(payload).unwrap();
        let (day, cash) = (Node::root(&v).text("day").unwrap().to_string(), Node::root(&v).text("cash").unwrap().to_string());
        Mapped {
            legs: vec![Draft {
                leg: Leg::named("cash"),
                account: AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1"),
                occurred_at: None,
                trade_date: day.parse().unwrap(),
                settle_date: None,
                kind: Kind::Deposit,
                effect: None,
                instrument: None,
                quantity: None,
                price: None,
                cash: Some(Money::new(Dec::parse(&cash).unwrap(), Currency::parse("CAD").unwrap())),
                fee: None,
                fx_rate: None,
            }],
            problems: vec![],
            adjustments: vec![],
        }
    }
}

#[test]
fn an_imported_row_the_broker_no_longer_lists_leaves_the_book_on_the_first_full_read() {
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let c = connection(&book, now);
    book.add_account(c, &[AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")], &bagholder_book::import::wealthsimple_account_type("TFSA"), AccountStatus::Open, None, at(now)).unwrap();
    let listed = Node::root(&Replay::read(&dir()).unwrap().rows[0]).text("canonicalId").unwrap().to_string();
    for (key, payload) in [("old-listed", r#"{"cash":"10","day":"2025-11-03"}"#), ("old-pending", r#"{"cash":"20","day":"2025-11-12"}"#)] {
        let broker_id = if key == "old-listed" { listed.clone() } else { "credit-transaction-anon-1".to_string() };
        book.store(&Imported, &Incoming { connection: Some(c), source_key: key, payload, refs: vec![("broker-record:wealthsimple".into(), broker_id)] }, at(now)).unwrap();
    }
    let (first, _) = once(&book, &dir(), now);
    assert_eq!((first.superseded, first.removed), (1, 1));
    assert_eq!(state(&book, "bagholder-import", "old-listed"), RecordState::Superseded);
    assert_eq!(state(&book, "bagholder-import", "old-pending"), RecordState::Removed);
}
