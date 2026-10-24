//! The pull reads only what changed (brief 07 §1, `docs/decisions.md`: calls to
//! Wealthsimple's unofficial API are kept to what is necessary), held by counting
//! what it asks. One account's November, from the owner's own history
//! (`tests/replies/wealthsimple-pull`).

use std::path::{Path, PathBuf};

use bagholder_book::Book;
use bagholder_broker::pull::{pull, Report};
use bagholder_core::account::{AccountRef, AccountStatus};
use bagholder_core::json::Value;
use bagholder_core::record::RecordState;
use bagholder_core::transaction::Kind;
use bagholder_core::{Broker, Currency, Dec, Money, SourceName};
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
    let mut replay = Replay::read(replies).unwrap().taken_before_statements();
    edit(&mut replay.rows);
    let mut ws = Wealthsimple::new(replay);
    let r = pull(book, &mut ws, connection, "2025-11-19".parse().unwrap(), at(now), &mut |_| {}).unwrap();
    (r, ws.source.asked.clone())
}

#[test]
fn a_pull_says_each_step_as_it_goes_every_account_by_name_and_counted() {
    use bagholder_broker::Step;
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let connection = connection(&book, now);
    let mut ws = Wealthsimple::new(Replay::read(&dir()).unwrap().taken_before_statements());
    let mut steps: Vec<Step> = Vec::new();
    let r = pull(&book, &mut ws, connection, "2025-11-19".parse().unwrap(), at(now), &mut |s| steps.push(s)).unwrap();
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    // the kinds of step, each run of one kind once, in the order the pull does them
    let mut kinds: Vec<&str> = steps
        .iter()
        .map(|s| match s {
            Step::Accounts => "accounts",
            Step::Activity { .. } => "activity",
            Step::Recording { .. } => "recording",
            Step::Balances => "balances",
            Step::Holdings { .. } => "holdings",
            Step::History { .. } => "history",
            Step::Statements => "statements",
        })
        .collect();
    kinds.dedup();
    // the recorded month's activity begins after the newest month a statement
    // covers: no statement is read for it
    assert_eq!(kinds, ["accounts", "activity", "recording", "balances", "holdings", "history"]);
    // each counted run goes 1..=of, every account named as the screens name it
    for pick in [0u8, 1, 2] {
        let run: Vec<(String, usize, usize)> = steps
            .iter()
            .filter_map(|s| match (pick, s) {
                (0, Step::Activity { account, n, of }) | (1, Step::Holdings { account, n, of }) | (2, Step::History { account, n, of }) => Some((account.clone(), *n, *of)),
                _ => None,
            })
            .collect();
        assert!(!run.is_empty());
        let of = run[0].2;
        assert_eq!(run.iter().map(|x| x.1).collect::<Vec<_>>(), (1..=of).collect::<Vec<_>>());
        assert!(run.iter().all(|x| x.2 == of && !x.0.trim().is_empty()));
    }
    // recording counts every row stored, from none
    let rec: Vec<(usize, usize)> = steps.iter().filter_map(|s| match s { Step::Recording { done, of } => Some((*done, *of)), _ => None }).collect();
    assert_eq!(rec.first().map(|x| x.0), Some(0));
    assert!(rec.windows(2).all(|w| w[1].0 == w[0].0 + 1 && w[1].1 == w[0].1));
    assert_eq!(rec.len(), rec[0].1, "one step per row recorded");
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
    // and the net worth feature's outside accounts, one read a pull (plan debts-held-outside-wealthsimple)
    assert_eq!(asked, vec!["accounts", "activity anon-tfsa-1", "balances", "net worth"]);
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
    assert_eq!(asked, vec!["accounts", "activity anon-tfsa-1", "securities 1", "balances", "net worth"]);
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

/// A capture whose reply lacks the list its operation carries does not read:
/// the replay says which file, never a pull that found nothing there.
#[test]
fn a_capture_reply_missing_its_list_is_refused_with_its_file() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("positions@acct@2026-09-16.json"), r#"{"data": {"identity": {}}}"#).unwrap();
    let err = Replay::read(d.path()).err().expect("a reply without its accounts list is refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("positions@acct@2026-09-16.json"), "{err}");
}

/// The recorded replies with an accounts list of two RESPs: anon-resp-1 a
/// managed one, closed, and anon-resp-2 a self-directed one, open.
fn resp_replies() -> tempfile::TempDir {
    resp_replies_with("closed")
}

/// As `resp_replies`, the managed account's status `managed`.
fn resp_replies_with(managed: &str) -> tempfile::TempDir {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple");
    let replies = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(&fixtures).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, replies.path().join(p.file_name().unwrap())).unwrap();
    }
    let mut accounts = bagholder_core::json::parse(&std::fs::read_to_string(dir().join("edited-accounts-one.json")).unwrap()).unwrap();
    let template = edges(&mut accounts)[0].clone();
    edges(&mut accounts).clear();
    for (id, ty, status) in [("anon-resp-1", "MANAGED_RESP_FAMILY", managed), ("anon-resp-2", "SELF_DIRECTED_RESP_FAMILY", "open")] {
        let mut a = template.clone();
        if let Value::Object(e) = &mut a {
            if let Some(Value::Object(n)) = e.get_mut("node") {
                n.insert("id".into(), Value::String(id.into()));
                n.insert("unifiedAccountType".into(), Value::String(ty.into()));
                n.insert("status".into(), Value::String(status.into()));
                n.insert("nickname".into(), Value::Null);
                n.insert("custodianAccounts".into(), Value::Array(vec![]));
                n.insert("accountFeatures".into(), Value::Array(vec![]));
            }
        }
        edges(&mut accounts).push(a);
    }
    std::fs::write(replies.path().join("edited-accounts-resp.json"), accounts.canonical()).unwrap();
    replies
}

/// The move made up, in cash, every figure here made up.
const MOVED: &str = "4321.10";

/// The feed as Wealthsimple states the owner's move (2025-06): the managed
/// account sold to cash on the 13th, its cash arriving in the self-directed
/// one on the 16th, and no row leaving the managed account.
fn resp_rows(rows: &mut Vec<Value>) {
    rows.retain(|r| Node::root(r).text("canonicalId").ok() != Some("anon-asset-3"));
    let set = |r: &mut Value, fields: &[(&str, &str)]| {
        if let Value::Object(o) = r {
            for (k, v) in fields {
                o.insert(k.to_string(), Value::String(v.to_string()));
            }
        }
    };
    let arrival = rows.iter_mut().find(|r| Node::root(r).text("canonicalId").ok() == Some("anon-asset-6")).unwrap();
    set(arrival, &[("amount", MOVED)]);
    let mut sale = rows.iter().find(|r| Node::root(r).text("type").ok() == Some("MANAGED_BUY")).unwrap().clone();
    set(&mut sale, &[("type", "MANAGED_SELL"), ("canonicalId", "anon-made-up-sale-1"), ("externalCanonicalId", "anon-made-up-order-1"), ("occurredAt", "2025-06-13T15:00:00.000000+00:00"), ("amount", MOVED)]);
    rows.push(sale);
}

fn resp_account(book: &Book, key: &str) -> bagholder_core::AccountId {
    book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), key)).unwrap().unwrap()
}

/// Each account's cash, every currency added up, over all the book counts.
fn cash_by_account(book: &Book) -> std::collections::BTreeMap<bagholder_core::AccountId, Dec> {
    let mut out = std::collections::BTreeMap::new();
    for t in book.transactions_as_named().unwrap() {
        if let Some(c) = t.cash {
            let e = out.entry(t.account).or_insert(Dec::ZERO);
            *e = e.checked_add(c.amount).unwrap();
        }
    }
    out
}

/// One move between the two RESPs, as a closed account's feed gains it after
/// its one full read: its leaving row's id, its arrival's, the day it moved,
/// the edit giving the feed before the move is listed (`before`) and once
/// both rows are (`after`).
struct LateMove {
    out: &'static str,
    inn: &'static str,
    day: &'static str,
    pulls: [&'static str; 3],
    before: fn(&mut Vec<Value>),
    after: fn(&mut Vec<Value>),
}

/// The units the made-up move of holdings carries.
const UNITS: &str = "10";

/// The move in cash: the June move, every figure made up.
fn late_cash_move() -> LateMove {
    fn before(rows: &mut Vec<Value>) {
        resp_rows(rows);
        rows.retain(|r| Node::root(r).text("canonicalId").ok() != Some("anon-asset-6"));
    }
    fn after(rows: &mut Vec<Value>) {
        resp_rows(rows);
        let mut leaving = Replay::read(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple")).unwrap().rows.into_iter().find(|r| Node::root(r).text("canonicalId").ok() == Some("anon-asset-3")).unwrap();
        if let Value::Object(o) = &mut leaving {
            o.insert("amount".into(), Value::String(MOVED.into()));
        }
        rows.push(leaving);
    }
    LateMove {
        out: "anon-asset-3",
        inn: "anon-asset-6",
        day: "2025-06-16",
        pulls: ["2025-06-15T20:00:00Z", "2025-06-17T20:00:00Z", "2025-06-18T20:00:00Z"],
        before,
        after,
    }
}

/// The move of holdings: the July move, its positions made to show ten units
/// of the fund leaving the closed account and arriving in the other, no cash.
fn late_units_move(replies: &Path) -> LateMove {
    let held = std::fs::read_to_string(replies.join("positions@anon-resp-2@2025-07-03.json")).unwrap();
    std::fs::write(replies.join("positions@anon-resp-1@2025-07-03.json"), held.replace("\"anon-resp-2\"", "\"anon-resp-1\"").replace("\"199.2845\"", &format!("\"{UNITS}\""))).unwrap();
    std::fs::write(replies.join("positions@anon-resp-2@2025-07-05.json"), held.replace("\"199.2845\"", "\"209.2845\"")).unwrap();
    fn before(rows: &mut Vec<Value>) {
        rows.retain(|r| !matches!(Node::root(r).text("canonicalId").ok(), Some("anon-asset-1" | "anon-asset-5")));
    }
    LateMove {
        out: "anon-asset-1",
        inn: "anon-asset-5",
        day: "2025-07-04",
        pulls: ["2025-07-03T20:00:00Z", "2025-07-05T20:00:00Z", "2025-07-06T20:00:00Z"],
        before,
        after: |_| {},
    }
}

/// What the closed account's moves out on the move's day are, each with its record.
fn leaving(book: &Book, day: &str) -> Vec<bagholder_core::transaction::Transaction> {
    let managed = resp_account(book, "anon-resp-1");
    book.transactions_as_named().unwrap().into_iter().filter(|t| t.account == managed && t.kind == Kind::TransferOut && t.trade_date == day.parse().unwrap()).collect()
}

/// An open account is read whole the first time, then from its last full
/// read on every pull; a closed one is read whole once and not again (owner,
/// 2026-10-24: "why would closed accounts be re-read at all, ever?").
#[test]
fn an_open_account_is_read_from_its_last_full_read_and_a_closed_one_once() {
    let replies = resp_replies();
    let home = tempfile::tempdir().unwrap();
    let pulls = ["2025-11-19T20:00:00Z", "2025-11-20T20:00:00Z", "2025-11-21T20:00:00Z"];
    let (book, _) = Book::open_in(home.path(), "test", at(pulls[0])).unwrap();
    let mut asked = Vec::new();
    for now in pulls {
        let mut replay = Replay::read(replies.path()).unwrap().taken_before_statements();
        resp_rows(&mut replay.rows);
        let mut ws = Wealthsimple::new(replay);
        pull(&book, &mut ws, connection(&book, now), "2025-11-19".parse().unwrap(), at(now), &mut |_| {}).unwrap();
        asked.push(ws.source.asked_from.clone());
    }
    let day = |s: &str| Some(s.parse::<jiff::civil::Date>().unwrap());
    let from = |key: &str| -> Vec<Option<Option<jiff::civil::Date>>> { asked.iter().map(|a| a.iter().find(|(k, _)| k == key).map(|(_, d)| *d)).collect() };
    assert_eq!(from("anon-resp-2"), vec![Some(None), Some(day("2025-11-19")), Some(day("2025-11-20"))], "the open account, on every pull");
    assert_eq!(from("anon-resp-1"), vec![Some(None), None, None], "the closed account, whole once");
    let managed = resp_account(&book, "anon-resp-1");
    assert_eq!(book.account(managed).unwrap().status, AccountStatus::Closed);
}

/// The leaving row of a move out of a closed account, listed only after that
/// account was read in full: the closed account is not read again; the row,
/// read for the arrival's record over the move's days, is stored from that
/// read, and the move is booked once in each account from its own row,
/// whatever the pulls after. In cash, and in units.
#[test]
fn a_move_out_of_a_closed_account_is_booked_once_from_its_own_row_read_beside_the_arrival() {
    for units in [false, true] {
        let replies = resp_replies();
        let m = if units { late_units_move(replies.path()) } else { late_cash_move() };
        let home = tempfile::tempdir().unwrap();
        let (book, _) = Book::open_in(home.path(), "test", at(m.pulls[0])).unwrap();
        once_with(&book, replies.path(), m.pulls[0], m.before);
        assert!(leaving(&book, m.day).is_empty());
        let cash_before = cash_by_account(&book);
        let (report, _) = once_with(&book, replies.path(), m.pulls[1], m.after);
        assert!(report.failures.iter().all(|(p, _)| !p.starts_with("record") && !p.starts_with("activity")), "{:?}", report.failures);
        for _ in 0..2 {
            let c = connection(&book, m.pulls[0]);
            let source = SourceName::named("wealthsimple");
            let own = book.record_by_key(Some(c), &source, m.out).unwrap().expect("the closed account's row of it leaving, stored from the read beside");
            let arrival = book.record_by_key(Some(c), &source, m.inn).unwrap().unwrap();
            let out = leaving(&book, m.day);
            assert_eq!(out.len(), 1, "{out:?}");
            assert_eq!(out[0].id.record, own, "booked from its own row");
            let in_ = book.transactions_of(arrival).unwrap();
            assert_eq!(in_.len(), 1, "the arrival books its own account alone: {in_:?}");
            assert_eq!(in_[0].kind, Kind::TransferIn);
            assert_ne!(in_[0].account, out[0].account);
            match out[0].instrument {
                Some(i) => {
                    assert_eq!(out[0].quantity, Some(Dec::parse(&format!("-{UNITS}")).unwrap()));
                    assert_eq!((in_[0].instrument, in_[0].quantity), (Some(i), Some(Dec::parse(UNITS).unwrap())), "the same units arrive");
                }
                None => {
                    assert_eq!(out[0].cash, Some(Money::new(Dec::parse(&format!("-{MOVED}")).unwrap(), Currency::CAD)));
                    assert_eq!(in_[0].cash, Some(Money::new(Dec::parse(MOVED).unwrap(), Currency::CAD)));
                    let mut cash = cash_before.clone();
                    for t in [&out[0], &in_[0]] {
                        let e = cash.entry(t.account).or_insert(Dec::ZERO);
                        *e = e.checked_add(t.cash.unwrap().amount).unwrap();
                    }
                    assert_eq!(cash_by_account(&book), cash, "each account's cash moved once");
                }
            }
            // and the pull after changes nothing
            once_with(&book, replies.path(), m.pulls[2], m.after);
        }
    }
}

/// A closed account the book holds cash in (the sale's cash, its leaving row
/// not listed): its statement of nothing is stored by the sync and by the
/// five-minute balances pass, neither asking Wealthsimple for its balance, so
/// the difference waits after the sync and is confirmed by the pass.
#[test]
fn a_closed_account_with_cash_left_in_the_book_is_said_after_the_sync_and_one_balances_pass() {
    let replies = resp_replies();
    let home = tempfile::tempdir().unwrap();
    let (sync, pass) = ("2025-11-19T20:00:00Z", "2025-11-19T20:05:00Z");
    let (book, _) = Book::open_in(home.path(), "test", at(sync)).unwrap();
    let mut replay = Replay::read(replies.path()).unwrap().taken_before_statements();
    resp_rows(&mut replay.rows);
    let mut ws = Wealthsimple::new(replay);
    let c = connection(&book, sync);
    pull(&book, &mut ws, c, "2025-11-19".parse().unwrap(), at(sync), &mut |_| {}).unwrap();
    let managed = resp_account(&book, "anon-resp-1");
    assert!(cash_by_account(&book).get(&managed).is_some_and(|c| !c.is_zero()), "the book holds the sale's cash there");
    let stated = book.stated(managed).unwrap();
    let (when, cash) = stated.cash.expect("its cash stated by the sync");
    assert!(cash.is_empty(), "as nothing");
    assert_eq!(stated.cash_read, Some(Default::default()), "nothing, as of the read");
    assert_eq!(stated.units.map(|(_, l)| l.len()), Some(0), "and no units");
    assert_eq!(Some(when), stated.activity_read_at, "stated with the read: a difference waits");
    bagholder_broker::pull::balances(&book, &mut ws, c, at(pass)).unwrap();
    let stated = book.stated(managed).unwrap();
    let (when, cash) = stated.cash.expect("stated again by the pass");
    assert!(cash.is_empty());
    assert!(stated.activity_read_at.is_some_and(|read| when > read), "stated after the read: the difference is confirmed");
    assert!(ws.source.asked_balances.iter().flatten().all(|k| k != "anon-resp-1"), "{:?}", ws.source.asked_balances);
}

/// The broker of the recorded replies, but one row's record cannot be put
/// together: what a failed read for it answers.
struct Unrecorded {
    inner: Wealthsimple<Replay>,
    key: String,
}

impl bagholder_broker::BrokerAdapter for Unrecorded {
    fn broker(&self) -> Broker {
        self.inner.broker()
    }
    fn mapping(&self) -> &dyn bagholder_book::mapping::Mapping {
        self.inner.mapping()
    }
    fn accounts(&mut self) -> bagholder_broker::Answer<Vec<bagholder_broker::AccountStated>> {
        self.inner.accounts()
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> bagholder_broker::Answer<bagholder_broker::Activity> {
        self.inner.activity(account, from)
    }
    fn prepare(&mut self, rows: &[&bagholder_broker::Row]) {
        self.inner.prepare(rows)
    }
    fn record(&mut self, row: &bagholder_broker::Row, book: &mut dyn bagholder_broker::BookMoves) -> bagholder_broker::Answer<Value> {
        if row.key == self.key {
            return Err(bagholder_broker::Failure::Refused("not answered".into()));
        }
        self.inner.record(row, book)
    }
    fn holds(&self, payload: &Value, row: &bagholder_broker::Row) -> bool {
        self.inner.holds(payload, row)
    }
    fn reads_positions(&self, payload: &Value) -> bool {
        self.inner.reads_positions(payload)
    }
    fn unsettled(&self, payload: &Value) -> Option<(String, Option<jiff::civil::Date>)> {
        self.inner.unsettled(payload)
    }
    fn placed(&self, payload: &Value) -> Option<(String, jiff::civil::Date)> {
        self.inner.placed(payload)
    }
    fn day(&self, at: jiff::Timestamp) -> jiff::civil::Date {
        self.inner.day(at)
    }
    fn cash(&mut self, accounts: &[String]) -> bagholder_broker::Answer<std::collections::BTreeMap<String, std::collections::BTreeMap<Currency, Dec>>> {
        self.inner.cash(accounts)
    }
    fn buying_power(&mut self, accounts: &[String]) -> bagholder_broker::Answer<std::collections::BTreeMap<String, Result<Dec, String>>> {
        self.inner.buying_power(accounts)
    }
    fn units(&mut self, account: &str, day: jiff::civil::Date) -> bagholder_broker::Answer<Vec<bagholder_broker::Units>> {
        self.inner.units(account, day)
    }
    fn instruments(&mut self, refs: &[bagholder_core::instrument::Reference], day: jiff::civil::Date) -> Vec<(bagholder_core::instrument::Reference, bagholder_broker::Answer<bagholder_book::mapping::InstrumentDraft>)> {
        self.inner.instruments(refs, day)
    }
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> bagholder_broker::Answer<Vec<bagholder_broker::DayValue>> {
        self.inner.history(account, from)
    }
}

/// A row whose record cannot be put together stores nothing, and its account's read is not complete: the next pull asks
/// from before the row again, and it is listed again.
#[test]
fn a_row_whose_record_cannot_be_put_together_leaves_its_account_s_read_incomplete_and_is_listed_again() {
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let c = connection(&book, now);
    let key = Node::root(&Replay::read(&dir()).unwrap().rows[0]).text("canonicalId").unwrap().to_string();
    let mut ws = Unrecorded { inner: Wealthsimple::new(Replay::read(&dir()).unwrap().taken_before_statements()), key: key.clone() };
    let first = pull(&book, &mut ws, c, "2025-11-19".parse().unwrap(), at(now), &mut |_| {}).unwrap();
    assert!(first.failures.iter().any(|(p, _)| *p == format!("record:{key}")), "{:?}", first.failures);
    assert!(book.record_by_key(Some(c), &SourceName::named("wealthsimple"), &key).unwrap().is_none());
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    assert_eq!(book.activity_read_at(account).unwrap(), None, "the read is not complete");
    // the next pull asks for the account whole again, and the row is stored
    let mut ws = Wealthsimple::new(Replay::read(&dir()).unwrap().taken_before_statements());
    pull(&book, &mut ws, c, "2025-11-19".parse().unwrap(), at("2025-11-20T20:00:00Z"), &mut |_| {}).unwrap();
    assert_eq!(ws.source.asked_from, vec![("anon-tfsa-1".to_string(), None)], "asked whole again");
    assert!(book.record_by_key(Some(c), &SourceName::named("wealthsimple"), &key).unwrap().is_some(), "listed again, and stored");
    assert!(book.activity_read_at(account).unwrap().is_some());
}

/// The pull's replies with the account made a margin account, and, with
/// `rates`, Wealthsimple's statement of its margin interest rates (every
/// figure made up).
fn margin_replies(rates: Option<&str>) -> tempfile::TempDir {
    let replies = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        if !p.to_string_lossy().ends_with(".later") {
            std::fs::copy(&p, replies.path().join(p.file_name().unwrap())).unwrap();
        }
    }
    let path = replies.path().join("edited-accounts-one.json");
    std::fs::write(&path, std::fs::read_to_string(&path).unwrap().replace("\"SELF_DIRECTED_TFSA\"", "\"SELF_DIRECTED_NON_REGISTERED_MARGIN\"")).unwrap();
    if let Some(applied) = rates {
        std::fs::write(replies.path().join("FetchAccountAppliedRates-1.json"), format!(r#"{{"data":{{"account":{{"id":"anon-tfsa-1","appliedRates":{applied},"__typename":"Account"}}}}}}"#)).unwrap();
    }
    replies
}

/// A margin account's interest rates are read with the day's pull and kept as
/// stated; the balances read between pulls, and a pull of the activity alone,
/// do not ask them again (a rate changes seldom).
#[test]
fn a_margin_account_s_rates_are_read_with_the_day_s_pull_alone() {
    let replies = margin_replies(Some(r#"{"__typename":"AppliedMarginInterestRates","cadPositionRate":"0.0445","usdPositionRate":"0.0695"}"#));
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let (report, asked) = once(&book, replies.path(), now);
    assert!(!report.failures.iter().any(|(p, _)| p.starts_with("margin rate")), "{:?}", report.failures);
    assert_eq!(asked.iter().filter(|a| a.starts_with("margin rates")).collect::<Vec<_>>(), ["margin rates anon-tfsa-1"]);
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    let (_, rates) = book.stated(account).unwrap().margin_rates.expect("the rates stated");
    assert_eq!(rates, [(Currency::CAD, Dec::parse("0.0445").unwrap()), (Currency::USD, Dec::parse("0.0695").unwrap())].into());
    let c = connection(&book, now);
    let mut ws = Wealthsimple::new(Replay::read(replies.path()).unwrap().taken_before_statements());
    bagholder_broker::pull::balances(&book, &mut ws, c, at("2025-11-19T20:05:00Z")).unwrap();
    bagholder_broker::pull::pull_activity(&book, &mut ws, c, "2025-11-19".parse().unwrap(), at("2025-11-19T20:10:00Z"), &mut |_| {}).unwrap();
    assert!(!ws.source.asked.iter().any(|a| a.starts_with("margin rates")), "{:?}", ws.source.asked);
}

/// A rate read that fails is its own failed part, named by the account, and
/// stores nothing; a rate that is not a yearly fraction is refused, never
/// converted.
#[test]
fn a_margin_rate_not_read_is_said_by_its_account_and_stores_nothing() {
    for (rates, why) in [(None, "no margin rates"), (Some(r#"{"__typename":"AppliedMarginInterestRates","cadPositionRate":"4.45","usdPositionRate":null}"#), "not a yearly rate")] {
        let replies = margin_replies(rates);
        let home = tempfile::tempdir().unwrap();
        let now = "2025-11-19T20:00:00Z";
        let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
        let (report, _) = once(&book, replies.path(), now);
        let failed: Vec<&(String, bagholder_broker::Failure)> = report.failures.iter().filter(|(p, _)| p.starts_with("margin rate")).collect();
        assert_eq!(failed.len(), 1, "{:?}", report.failures);
        assert_eq!(failed[0].0, "margin rate:anon-tfsa-1");
        assert!(format!("{:?}", failed[0].1).contains(why), "{:?}", failed[0].1);
        let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
        assert_eq!(book.stated(account).unwrap().margin_rates, None);
    }
}

/// Link the two accounts of `replies`' accounts list to each other, as
/// Wealthsimple states an account given a new id beside its old one.
fn link_resp(replies: &Path) {
    let path = replies.join("edited-accounts-resp.json");
    let mut accounts = bagholder_core::json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let ids: Vec<String> = edges(&mut accounts).iter().map(|e| Node::root(e).obj("node").unwrap().text("id").unwrap().to_string()).collect();
    for (e, other) in edges(&mut accounts).iter_mut().zip(ids.iter().rev()) {
        if let Value::Object(e) = e {
            if let Some(Value::Object(n)) = e.get_mut("node") {
                n.insert("linkedAccount".into(), bagholder_core::json::parse(&format!("{{\"id\": \"{other}\"}}")).unwrap());
            }
        }
    }
    std::fs::write(&path, accounts.canonical()).unwrap();
}

/// One book account per set of ids Wealthsimple links as one (`linked`): two
/// ids read apart are two accounts, and the pull in which Wealthsimple first
/// states their link makes them one, every record and transaction in place,
/// open while either id is.
#[test]
fn accounts_wealthsimple_links_are_one_account_from_the_pull_that_states_the_link() {
    let replies = resp_replies();
    let home = tempfile::tempdir().unwrap();
    let pulls = ["2025-11-19T20:00:00Z", "2025-11-20T20:00:00Z", "2025-11-21T20:00:00Z"];
    let (book, _) = Book::open_in(home.path(), "test", at(pulls[0])).unwrap();
    let placed = |book: &Book| -> std::collections::BTreeSet<(String, String, String)> {
        let c = book.conn_for_tests();
        c.prepare("SELECT r.source_key, t.leg, t.kind FROM transactions t JOIN source_records r ON r.id = t.record_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    once_with(&book, replies.path(), pulls[0], resp_rows);
    let (old, new) = (resp_account(&book, "anon-resp-1"), resp_account(&book, "anon-resp-2"));
    assert_ne!(old, new, "two ids read apart are two accounts");
    let before = placed(&book);
    link_resp(replies.path());
    let (report, _) = once_with(&book, replies.path(), pulls[1], resp_rows);
    assert_eq!(report.accounts_merged, 1);
    let one = resp_account(&book, "anon-resp-1");
    assert_eq!(resp_account(&book, "anon-resp-2"), one, "both ids on one account");
    assert_eq!(one, old, "the oldest account keeps its id");
    assert_eq!(book.account(one).unwrap().status, AccountStatus::Open, "open while either id is");
    let c = book.conn_for_tests();
    let count = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM accounts"), 1);
    assert_eq!(count(&format!("SELECT COUNT(*) FROM transactions WHERE account_id <> '{one}'")), 0);
    assert_eq!(placed(&book), before, "every record's legs, as before");
    // the next pull keeps them one
    once_with(&book, replies.path(), pulls[2], resp_rows);
    assert_eq!(count("SELECT COUNT(*) FROM accounts"), 1);
}
