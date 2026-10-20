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

/// An imported record: one cash leg (a deposit into the account, unless the
/// payload names its account and kind), known by the broker's id of its row.
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
        // the account and kind where the payload names them
        let account = Node::root(&v).text("account").unwrap_or("anon-tfsa-1").to_string();
        let kind = match Node::root(&v).text("kind").ok() {
            Some("TransferOut") => Kind::TransferOut,
            _ => Kind::Deposit,
        };
        Mapped {
            legs: vec![Draft {
                leg: Leg::named("cash"),
                account: AccountRef::new(Broker::named("wealthsimple"), account),
                occurred_at: None,
                trade_date: day.parse().unwrap(),
                settle_date: None,
                kind,
                effect: None,
                instrument: None,
                quantity: None,
                price: None,
                cash: Some(Money::new(Dec::parse(&cash).unwrap(), Currency::parse("CAD").unwrap())),
                fee: None,
                fx_rate: None,
                paid_on: None,
                value: None,
            }],
            problems: vec![],
            adjustments: vec![],
            hold: None,
            orders: vec![]
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

/// The managed account's moves of the sale and the move over those days, and
/// the self-directed account's arrivals: the cash nets to nothing in the one,
/// and arrived once in the other.
fn assert_moved_once(book: &Book) {
    let (managed, self_directed) = (resp_account(book, "anon-resp-1"), resp_account(book, "anon-resp-2"));
    let txs = book.transactions_as_named().unwrap();
    let days = |t: &&bagholder_core::transaction::Transaction| t.trade_date >= "2025-06-13".parse().unwrap() && t.trade_date <= "2025-06-16".parse().unwrap();
    let cad = Currency::parse("CAD").unwrap();
    let moved: Vec<_> = txs.iter().filter(|t| t.account == managed && matches!(t.kind, Kind::Sell | Kind::TransferOut)).filter(days).collect();
    assert_eq!(moved.len(), 2, "{moved:?}");
    assert_eq!(moved.iter().filter(|t| t.kind == Kind::TransferOut).count(), 1, "booked leaving once");
    let net = moved.iter().filter_map(|t| t.cash).filter(|c| c.currency == cad).fold(Dec::ZERO, |a, c| a.checked_add(c.amount).unwrap());
    assert_eq!(net, Dec::ZERO, "the sale's cash left the managed account");
    let arrived: Vec<_> = txs.iter().filter(|t| t.account == self_directed && t.kind == Kind::TransferIn).filter(days).collect();
    assert_eq!(arrived.len(), 1, "{arrived:?}");
    assert_eq!(arrived[0].cash, Some(Money::new(Dec::parse(MOVED).unwrap(), cad)));
}

/// A managed account sold to cash on one day and closed, its cash moved to a
/// self-directed account three days later, Wealthsimple stating the move
/// arriving and nothing in the managed account's feed leaving it (the owner's
/// shape, 2025-06; every figure here made up): after the pull, the managed
/// account's cash from the sale has left it, and the self-directed one holds it.
#[test]
fn a_move_whose_leaving_row_the_broker_never_lists_leaves_the_other_account_after_the_pull() {
    let replies = resp_replies();
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let (report, _) = once_with(&book, replies.path(), now, resp_rows);
    assert!(!report.failures.iter().any(|(p, _)| p.starts_with("activity")), "{:?}", report.failures);
    assert_moved_once(&book);
}

/// The earlier app's import of the move leaving the managed account, the only
/// record of it before: the leg Wealthsimple's arrival books in its place
/// replaces it, so the cash leaves once.
#[test]
fn a_move_leaving_booked_from_its_arrival_supersedes_the_import_of_it() {
    let replies = resp_replies();
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let c = connection(&book, now);
    book.add_account(c, &[AccountRef::new(Broker::named("wealthsimple"), "anon-resp-1")], &bagholder_book::import::wealthsimple_account_type("MANAGED_RESP_FAMILY"), AccountStatus::Closed, None, at(now)).unwrap();
    let payload = format!(r#"{{"account":"anon-resp-1","kind":"TransferOut","cash":"-{MOVED}","day":"2025-06-16"}}"#);
    book.store(&Imported, &Incoming { connection: Some(c), source_key: "old-leaving", payload: &payload, refs: vec![("broker-record:wealthsimple".into(), "anon-made-up-leaving-1".into())] }, at(now)).unwrap();
    let (report, _) = once_with(&book, replies.path(), now, resp_rows);
    assert!(!report.failures.iter().any(|(p, _)| p.starts_with("activity") || p.starts_with("record")), "{:?}", report.failures);
    assert_eq!(state(&book, "bagholder-import", "old-leaving"), RecordState::Superseded);
    assert_moved_once(&book);
}

/// A record of the move stored before the other account's feed was read with
/// it books no leaving: the next pull reads that feed over the move's days,
/// adds it, and books the leaving; the pull after reads nothing more for it.
#[test]
fn a_move_arriving_stored_before_its_other_account_s_feed_was_read_with_it_is_read_again_once() {
    let replies = resp_replies();
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    once_with(&book, replies.path(), "2025-11-19T20:00:00Z", resp_rows);
    // the record as the earlier assembly stored it: no other account's feed
    let c = connection(&book, "2025-11-19T20:00:00Z");
    let source = SourceName::named("wealthsimple");
    let r = book.record_by_key(Some(c), &source, "anon-asset-6").unwrap().unwrap();
    let (_, _, payload) = book.revisions(r).unwrap().pop().unwrap();
    let mut v = bagholder_core::json::parse(&payload).unwrap();
    if let Value::Object(o) = &mut v {
        assert!(o.remove("opposing").is_some(), "read with it now");
    }
    let old = v.canonical();
    book.store(&bagholder_wealthsimple::mapping::WealthsimpleMapping, &Incoming { connection: Some(c), source_key: "anon-asset-6", payload: &old, refs: vec![] }, at("2025-11-19T20:30:00Z")).unwrap();
    let managed = resp_account(&book, "anon-resp-1");
    let leaving = |book: &Book| book.transactions_of(r).unwrap().into_iter().filter(|t| t.account == managed).count();
    assert_eq!(leaving(&book), 0, "the earlier assembly books no leaving");
    // the next pull: the managed account, closed and read in full, read again over the move's days
    let (second, asked) = once_with(&book, replies.path(), "2025-11-20T20:00:00Z", resp_rows);
    assert!(second.failures.iter().all(|(p, _)| !p.starts_with("record")), "{:?}", second.failures);
    assert!(asked.contains(&"activity anon-resp-1".to_string()), "{asked:?}");
    assert_eq!(leaving(&book), 1);
    assert_moved_once(&book);
    // and the pull after: nothing more read for it
    let (_, asked) = once_with(&book, replies.path(), "2025-11-21T20:00:00Z", resp_rows);
    assert!(!asked.contains(&"activity anon-resp-1".to_string()), "{asked:?}");
    assert_eq!(leaving(&book), 1);
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

/// A move still under way: the first pull, the day it arrives, reads its
/// arrival alone, and books it leaving the other account from it; the next
/// reads the other account's own row of it leaving, which then books it, and
/// the arrival books it no more, in the same write. The cash leaves once
/// throughout, and the same cash either side of the second pull.
#[test]
fn a_move_whose_leaving_row_arrives_a_pull_later_is_booked_leaving_from_that_row_alone() {
    let replies = resp_replies_with("open");
    let home = tempfile::tempdir().unwrap();
    let first = "2025-06-16T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(first)).unwrap();
    // the first pull: Wealthsimple lists the arrival, not yet the leaving row
    once_with(&book, replies.path(), first, resp_rows);
    let c = connection(&book, first);
    let source = SourceName::named("wealthsimple");
    let arrival = book.record_by_key(Some(c), &source, "anon-asset-6").unwrap().unwrap();
    let managed = resp_account(&book, "anon-resp-1");
    let inferred = |book: &Book| book.transactions_of(arrival).unwrap().into_iter().filter(|t| t.account == managed).count();
    assert_eq!(inferred(&book), 1, "booked leaving from its arrival");
    assert!(book.record_by_key(Some(c), &source, "anon-asset-3").unwrap().is_none());
    assert_moved_once(&book);
    let before = cash_by_account(&book);
    // the next pull: the leaving row listed
    let (report, asked) = once_with(&book, replies.path(), "2025-06-17T20:00:00Z", |rows| {
        resp_rows(rows);
        let mut leaving = Replay::read(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple")).unwrap().rows.into_iter().find(|r| Node::root(r).text("canonicalId").ok() == Some("anon-asset-3")).unwrap();
        if let Value::Object(o) = &mut leaving {
            o.insert("amount".into(), Value::String(MOVED.into()));
        }
        rows.push(leaving);
    });
    assert!(report.failures.iter().all(|(p, _)| !p.starts_with("record") && !p.starts_with("activity")), "{:?}", report.failures);
    assert!(asked.contains(&"activity anon-resp-1".to_string()), "{asked:?}");
    let own = book.record_by_key(Some(c), &source, "anon-asset-3").unwrap().expect("the leaving row's record");
    let out: Vec<_> = book.transactions_of(own).unwrap().into_iter().filter(|t| t.account == managed && t.kind == Kind::TransferOut).collect();
    assert_eq!(out.len(), 1, "booked leaving from its own row");
    assert_eq!(out[0].cash, Some(Money::new(Dec::parse(&format!("-{MOVED}")).unwrap(), Currency::parse("CAD").unwrap())));
    assert_eq!(inferred(&book), 0, "and from the arrival no more");
    assert_moved_once(&book);
    assert_eq!(cash_by_account(&book), before, "the same cash either side");
}
