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
    let (first, asked) = once(&book, &dir(), "2025-11-19T20:00:00Z");
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    assert_eq!(first.records_new, 53);
    // one month of rows names only some of what the account holds: the rest
    // are read from their securities' records, in one batch, and stated whole
    assert_eq!(asked.iter().filter(|a| a.starts_with("securities")).collect::<Vec<_>>(), vec!["securities 13", "securities 7"]);
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    let (_, units) = book.stated(account).unwrap().units.expect("the units stated");
    let positions = bagholder_core::json::parse(&std::fs::read_to_string(dir().join("positions@anon-tfsa-1@2025-11-18.json")).unwrap()).unwrap();
    let held = Node::root(&positions).obj("data").unwrap().list("accounts").unwrap()[0].obj("financials").unwrap().obj("current").unwrap().obj("positionsAsOfDate").unwrap().list("edges").unwrap().iter().filter(|e| !e.obj("node").unwrap().obj("security").unwrap().text("id").unwrap().starts_with("sec-c-")).count();
    assert_eq!(units.len(), held);
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

/// The accounts list's edges in an accounts reply.
fn edges(v: &mut Value) -> &mut Vec<Value> {
    let Value::Object(root) = v else { panic!() };
    let Some(Value::Object(data)) = root.get_mut("data") else { panic!() };
    let Some(Value::Object(identity)) = data.get_mut("identity") else { panic!() };
    let Some(Value::Object(list)) = identity.get_mut("accounts") else { panic!() };
    let Some(Value::Array(edges)) = list.get_mut("edges") else { panic!() };
    edges
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
    assert_eq!((first.records_new, first.removed.len()), (54, 0));
    // final now: Wealthsimple lists it under another id, and the pending one no longer
    let (second, _) = once_with(&book, &dir(), "2025-11-19T21:00:00Z", |rows| {
        let settled = row_as(rows, "anon-settled-1", "COMPLETED");
        rows.push(settled);
    });
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert_eq!((second.records_new, second.removed.len()), (1, 1));
    assert_eq!(second.removed.iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>(), vec!["anon-pending-1"]);
    assert_eq!(state(&book, "wealthsimple", "anon-pending-1"), RecordState::Removed);
    assert_eq!(state(&book, "wealthsimple", "anon-settled-1"), RecordState::Live);
    // nothing left pending: the next pull reads from its last full read, and removes nothing
    let (third, _) = once_with(&book, &dir(), "2025-11-19T22:00:00Z", |rows| {
        let settled = row_as(rows, "anon-settled-1", "COMPLETED");
        rows.push(settled);
    });
    assert_eq!((third.records_new, third.removed.len()), (0, 0));
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
                paid_on: None,
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
    assert_eq!((first.superseded, first.removed.len()), (1, 1));
    assert_eq!(state(&book, "bagholder-import", "old-listed"), RecordState::Superseded);
    assert_eq!(state(&book, "bagholder-import", "old-pending"), RecordState::Removed);
}

#[test]
fn a_credit_card_s_cash_is_what_is_owed_on_it() {
    // the accounts list with a card beside the account (its node edited from
    // the account's), and the card's own reply as Wealthsimple sent it
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    let replies = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        if !p.to_string_lossy().ends_with(".later") {
            std::fs::copy(&p, replies.path().join(p.file_name().unwrap())).unwrap();
        }
    }
    let path = replies.path().join("edited-accounts-one.json");
    let mut accounts = bagholder_core::json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut card = edges(&mut accounts)[0].clone();
    if let Value::Object(e) = &mut card {
        if let Some(Value::Object(n)) = e.get_mut("node") {
            n.insert("id".into(), Value::String("anon-ca-4".into()));
            n.insert("unifiedAccountType".into(), Value::String("CREDIT_CARD".into()));
            n.insert("nickname".into(), Value::Null);
        }
    }
    edges(&mut accounts).push(card);
    std::fs::write(&path, accounts.canonical()).unwrap();
    std::fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple/credit-card-account-1.json"), replies.path().join("credit-card-account-1.json")).unwrap();
    let (report, asked) = once(&book, replies.path(), "2025-11-19T20:00:00Z");
    assert!(asked.contains(&"card anon-ca-4".to_string()), "{asked:?}");
    assert!(!report.failures.iter().any(|(p, _)| p.starts_with("cash")), "{:?}", report.failures);
    let card = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-ca-4")).unwrap().unwrap();
    let (_, stated) = book.stated(card).unwrap().cash.expect("the card's cash stated");
    assert_eq!(stated.get(&Currency::parse("CAD").unwrap()), Some(&Dec::parse("-5140.28").unwrap()));
}

fn with(mut row: Value, key: &str, value: Value) -> Value {
    if let Value::Object(o) = &mut row {
        o.insert(key.into(), value);
    }
    row
}

#[test]
fn a_row_the_adapter_cannot_read_is_kept_with_why_and_the_rest_of_its_account_is_read() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    let (first, _) = once_with(&book, &dir(), "2025-11-19T20:00:00Z", |rows| {
        let unlisted = with(row_as(rows, "anon-odd-status", "COMPLETED"), "unifiedStatus", Value::String("ON_HOLD".into()));
        let no_time = with(row_as(rows, "anon-no-time", "COMPLETED"), "occurredAt", Value::String("yesterday".into()));
        rows.push(unlisted);
        rows.push(no_time);
    });
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    assert_eq!(first.records_new, 55);
    for key in ["anon-odd-status", "anon-no-time"] {
        let r = book.record_by_key(Some(connection(&book, "2025-11-19T20:00:00Z")), &SourceName::named("wealthsimple"), key).unwrap().unwrap();
        let codes: Vec<String> = book.problems_of(r).unwrap().into_iter().map(|p| p.code).collect();
        assert_eq!(codes, vec!["unreadable".to_string()], "{key}");
        assert!(book.transactions_of(r).unwrap().is_empty());
    }
    // the account's read is complete: the rest of its rows are on the record
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    assert!(book.activity_read_at(account).unwrap().is_some());
    // and the two are read again: the next pull reads the whole account, as
    // one of them states no day
    let (_, asked) = once(&book, &dir(), "2025-11-19T21:00:00Z");
    assert!(asked.contains(&"activity anon-tfsa-1".to_string()));
}

#[test]
fn a_row_with_no_id_to_keep_it_by_leaves_its_account_s_read_incomplete_named() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    let (first, _) = once_with(&book, &dir(), "2025-11-19T20:00:00Z", |rows| {
        let mut keyless = row_as(rows, "anon-x", "COMPLETED");
        if let Value::Object(o) = &mut keyless {
            o.remove("canonicalId");
        }
        rows.push(keyless);
    });
    assert_eq!(first.failures.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["activity:anon-tfsa-1"]);
    assert!(first.failures[0].1.to_string().contains("no id"), "{}", first.failures[0].1);
    // the rows it could keep are kept; the read is not a full one
    assert_eq!(first.records_new, 53);
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    assert!(book.activity_read_at(account).unwrap().is_none());
}

#[test]
fn a_read_that_would_remove_more_than_a_few_final_rows_removes_nothing_and_is_suspect() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    // four final rows on the last day read, then a read that no longer lists them
    let extra = |rows: &mut Vec<Value>| {
        for i in 0..4 {
            let r = row_as(rows, &format!("anon-final-{i}"), "COMPLETED");
            rows.push(with(r, "occurredAt", Value::String("2025-11-19T15:00:00.000000+00:00".into())));
        }
    };
    once_with(&book, &dir(), "2025-11-19T20:00:00Z", extra);
    let (second, _) = once(&book, &dir(), "2025-11-19T21:00:00Z");
    assert!(second.removed.is_empty(), "{:?}", second.removed);
    assert_eq!(second.suspect.len(), 1);
    assert!(second.suspect[0].1.contains("4 final rows"), "{:?}", second.suspect);
    assert_eq!(state(&book, "wealthsimple", "anon-final-0"), RecordState::Live);
}

#[test]
fn a_read_that_would_remove_a_final_row_older_than_the_rows_read_again_removes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    // a pending row two days before the full read, and a final row the day
    // before it; the next read starts at the pending row's day to read it
    // again, and lists neither
    once_with(&book, &dir(), "2025-11-19T20:00:00Z", |rows| {
        let pending = with(row_as(rows, "anon-pending-2", "PENDING"), "occurredAt", Value::String("2025-11-17T15:00:00.000000+00:00".into()));
        let done = with(row_as(rows, "anon-final-9", "COMPLETED"), "occurredAt", Value::String("2025-11-18T15:00:00.000000+00:00".into()));
        rows.push(pending);
        rows.push(done);
    });
    let (second, _) = once(&book, &dir(), "2025-11-20T21:00:00Z");
    assert!(second.removed.is_empty(), "{:?}", second.removed);
    assert!(second.suspect.iter().any(|(_, w)| w.contains("anon-final-9") && w.contains("older than the rows read again")), "{:?}", second.suspect);
    assert_eq!(state(&book, "wealthsimple", "anon-pending-2"), RecordState::Live);
}
