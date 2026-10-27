//! The pull reads only what changed (brief 07 §1, `docs/decisions.md`: calls to
//! Wealthsimple's unofficial API are kept to what is necessary), held by counting
//! what it asks. One account's November, from the owner's own history
//! (`tests/replies/wealthsimple-pull`).

use std::path::{Path, PathBuf};

use bagholder_book::statements::AccountDay;
use bagholder_book::Book;
use bagholder_broker::pull::{pull, pull_restating, Report};
use bagholder_broker::BrokerAdapter;
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
    let mut replay = Replay::read(replies).unwrap();
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
    let mut ws = Wealthsimple::new(Replay::read(&dir()).unwrap());
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
            Step::Holdings => "holdings",
            Step::History { .. } => "history",
        })
        .collect();
    kinds.dedup();
    assert_eq!(kinds, ["accounts", "activity", "recording", "balances", "holdings", "history"]);
    // each counted run goes 1..=of, every account named as the screens name it
    for pick in [0u8, 2] {
        let run: Vec<(String, usize, usize)> = steps
            .iter()
            .filter_map(|s| match (pick, s) {
                (0, Step::Activity { account, n, of }) | (2, Step::History { account, n, of }) => Some((account.clone(), *n, *of)),
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

/// A day an earlier read stored wrong is stated again by the next day's pull:
/// the daily history is read whole, and each day is kept as Wealthsimple states
/// it now. 2.10.2 read an account behind two ids on one of them and stored years
/// of its days at nothing; reading only the days after the last one stored, no
/// pull read them again, and the charts lost the account until its newest days.
#[test]
fn a_day_an_earlier_read_stored_wrong_is_stated_again_by_the_next_day_s_pull() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2026-09-25T20:00:00Z")).unwrap();
    // a day's pull of the recorded replies; the recorded history ends on 2026-09-23
    let day_s_pull = |today: &str| {
        let now = format!("{today}T20:00:00Z");
        let mut ws = Wealthsimple::new(Replay::read(&dir()).unwrap());
        let r = pull(&book, &mut ws, connection(&book, &now), today.parse().unwrap(), at(&now), &mut |_| {}).unwrap();
        assert!(!r.failures.iter().any(|(part, _)| part.starts_with("history")), "{:?}", r.failures);
        r
    };
    day_s_pull("2026-09-25");
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    let stated = book.stated(account).unwrap().days;
    // an earlier build's read of the account: every day from a date on, at nothing
    let from: jiff::civil::Date = "2024-03-01".parse().unwrap();
    let spoiled: Vec<AccountDay> = stated.range(from..).map(|(day, (v, n))| AccountDay { day: *day, net_value: Money::zero(v.currency), net_deposits: Money::zero(n.currency) }).collect();
    let wrong = stated.range(from..).filter(|(_, (v, n))| !(v.amount.is_zero() && n.amount.is_zero())).count();
    assert!(wrong > 500, "the stretch spoiled holds what the account held: {wrong}");
    let read = book.broker_read(connection(&book, "2026-09-25T21:00:00Z"), "history", at("2026-09-25T21:00:00Z")).unwrap();
    book.store_account_days(account, &spoiled, &read).unwrap();
    assert_ne!(book.stated(account).unwrap().days, stated);
    // the next day's pull states every one of them again, and no day is new
    let next = day_s_pull("2026-09-26");
    assert_eq!(book.stated(account).unwrap().days, stated);
    assert_eq!((next.days_restated, next.days_stored), (wrong, 0));
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

/// A row whose record an earlier read removed (2.10.0 to 2.10.3 read accounts
/// that sat apart, then were merged, on one id) is in the book again once
/// Wealthsimple lists it: what the broker lists now is what the book holds.
#[test]
fn a_row_whose_record_an_earlier_read_removed_is_live_again_once_the_broker_lists_it() {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2025-11-19T20:00:00Z")).unwrap();
    let listed = |rows: &mut Vec<Value>| {
        let row = with(row_as(rows, "anon-listed-1", "COMPLETED"), "occurredAt", Value::String("2025-11-19T15:00:00.000000+00:00".into()));
        rows.push(row);
    };
    let (first, _) = once_with(&book, &dir(), "2025-11-19T20:00:00Z", listed);
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    let conn = connection(&book, "2025-11-19T20:00:00Z");
    let r = book.record_by_key(Some(conn), &SourceName::named("wealthsimple"), "anon-listed-1").unwrap().unwrap();
    let held = book.transactions_of(r).unwrap();
    assert!(!held.is_empty());
    book.mark_removed(r, at("2025-11-19T20:30:00Z")).unwrap();
    // the next pull reads from the last full read's day, and the row is listed
    let (second, _) = once_with(&book, &dir(), "2025-11-19T21:00:00Z", listed);
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert_eq!(state(&book, "wealthsimple", "anon-listed-1"), RecordState::Live);
    assert_eq!(book.transactions_of(r).unwrap(), held);
    assert_eq!(second.records_revised, 1, "{second:?}");
    // and once held, the pull after changes nothing
    let (third, _) = once_with(&book, &dir(), "2025-11-19T22:00:00Z", listed);
    assert_eq!((third.records_new, third.records_revised), (0, 0), "{third:?}");
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
        let mut replay = Replay::read(replies.path()).unwrap();
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
    fn positions(&mut self, accounts: &[String], day: jiff::civil::Date) -> bagholder_broker::Answer<std::collections::BTreeMap<String, bagholder_broker::Positions>> {
        self.inner.positions(accounts, day)
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
    let mut ws = Unrecorded { inner: Wealthsimple::new(Replay::read(&dir()).unwrap()), key: key.clone() };
    let first = pull(&book, &mut ws, c, "2025-11-19".parse().unwrap(), at(now), &mut |_| {}).unwrap();
    assert!(first.failures.iter().any(|(p, _)| *p == format!("record:{key}")), "{:?}", first.failures);
    assert!(book.record_by_key(Some(c), &SourceName::named("wealthsimple"), &key).unwrap().is_none());
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    assert_eq!(book.activity_read_at(account).unwrap(), None, "the read is not complete");
    // the next pull asks for the account whole again, and the row is stored
    let mut ws = Wealthsimple::new(Replay::read(&dir()).unwrap());
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
    let mut ws = Wealthsimple::new(Replay::read(replies.path()).unwrap());
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

/// One book account behind two open ids Wealthsimple links as one: every read
/// of what the broker states of it (cash, worth, holdings) asks for both ids,
/// in the day's pull and in the five-minute balances alike, never one of them
/// (2.10.3 read one id per account: its other id's holdings were stated as none).
#[test]
fn an_account_behind_two_open_ids_is_read_on_both_by_the_pull_and_the_balances() {
    let replies = resp_replies_with("open");
    link_resp(replies.path());
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    let mut replay = Replay::read(replies.path()).unwrap();
    resp_rows(&mut replay.rows);
    let mut ws = Wealthsimple::new(replay);
    pull(&book, &mut ws, connection(&book, now), "2025-11-19".parse().unwrap(), at(now), &mut |_| {}).unwrap();
    assert_eq!(resp_account(&book, "anon-resp-1"), resp_account(&book, "anon-resp-2"), "one account");
    let both = |asked: &[String]| ["anon-resp-1", "anon-resp-2"].iter().all(|k| asked.iter().any(|a| a == k));
    assert!(ws.source.asked_balances.iter().any(|b| both(b)), "the pull's balances: {:?}", ws.source.asked_balances);
    for k in ["anon-resp-1", "anon-resp-2"] {
        assert!(ws.source.asked.iter().any(|a| a.starts_with(&format!("positions {k} "))), "the holdings of {k}: {:?}", ws.source.asked);
    }
    let mut ws = Wealthsimple::new(Replay::read(replies.path()).unwrap());
    bagholder_broker::pull::balances(&book, &mut ws, connection(&book, now), at("2025-11-19T20:05:00Z")).unwrap();
    assert!(ws.source.asked_balances.iter().any(|b| both(b)), "the five-minute balances: {:?}", ws.source.asked_balances);
}

/// A statement that an account holds nothing is no read to stand on: the day's
/// pull asks for its holdings again, as of the same day (2.10.2 wrote one onto
/// the accounts it merged, and 2.10.3 took it as read).
#[test]
fn a_statement_of_nothing_held_is_read_again_on_the_day_s_pull() {
    let replies = resp_replies_with("open");
    let home = tempfile::tempdir().unwrap();
    let now = "2025-11-19T20:00:00Z";
    let (book, _) = Book::open_in(home.path(), "test", at(now)).unwrap();
    once_with(&book, replies.path(), now, resp_rows);
    let id = resp_account(&book, "anon-resp-2");
    let c = connection(&book, now);
    let read = book.broker_read(c, "units", at(now)).unwrap();
    book.store_units(id, "2025-11-18".parse().unwrap(), &[], &read, at(now)).unwrap();
    let (_, asked) = once_with(&book, replies.path(), "2025-11-19T21:00:00Z", resp_rows);
    assert!(asked.iter().any(|a| a == "positions anon-resp-2 2025-11-18"), "{asked:?}");
}

// ---------------------------------------------------------------------------
// The shapes the real feed has (brief 76), each over the made-up Wealthsimple
// as the capacity job reads it: the pull runs on `Generated::shaped`, and the
// book is held to what the made-up broker states.
// ---------------------------------------------------------------------------

use bagholder_wealthsimple::generated::{Generated, Shape, Size, SHAPES};

/// Eight accounts: five investment accounts, crypto, spending and a card.
const SHAPED: Size = Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed: 13, deep: 0 };

fn shaped_today() -> jiff::civil::Date {
    "2026-10-08".parse().unwrap()
}

/// A book pulled once from a made-up Wealthsimple with these shapes.
fn shaped(shapes: &[Shape]) -> (tempfile::TempDir, Book, Wealthsimple<Generated>, Report) {
    shaped_by(SHAPED, shaped_today(), shapes)
}

/// A book pulled once, on `today`, from a made-up Wealthsimple of this size with these shapes.
fn shaped_by(size: Size, today: jiff::civil::Date, shapes: &[Shape]) -> (tempfile::TempDir, Book, Wealthsimple<Generated>, Report) {
    shaped_then(size, today, shapes, |_| {})
}

/// The same, the made-up broker's day changed first by `edit`.
fn shaped_then(size: Size, today: jiff::civil::Date, shapes: &[Shape], edit: impl FnOnce(&mut Generated)) -> (tempfile::TempDir, Book, Wealthsimple<Generated>, Report) {
    let home = tempfile::tempdir().unwrap();
    let now = today.to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp().checked_add(jiff::SignedDuration::from_hours(20)).unwrap();
    let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
    let mut g = Generated::shaped(size, today, shapes);
    edit(&mut g);
    let mut ws = Wealthsimple::new(g);
    let report = pull(&book, &mut ws, connection(&book, &now.to_string()), today, now, &mut |_| {}).unwrap();
    (home, book, ws, report)
}

fn book_account(book: &Book, key: &str) -> bagholder_core::AccountId {
    book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), key)).unwrap().unwrap_or_else(|| panic!("no account for {key}"))
}

/// A book account's cash per currency, from its transactions.
fn book_cash(book: &Book, account: bagholder_core::AccountId) -> std::collections::BTreeMap<String, Dec> {
    let mut out: std::collections::BTreeMap<String, Dec> = std::collections::BTreeMap::new();
    for c in book.transactions().unwrap().into_iter().filter(|t| t.account == account).filter_map(|t| t.cash) {
        let e = out.entry(c.currency.to_string()).or_insert(Dec::ZERO);
        *e = e.checked_add(c.amount).unwrap();
    }
    out.retain(|_, v| !v.is_zero());
    out
}

/// A book account's units per symbol (a contract by its terms), from its transactions.
fn book_units(book: &Book, account: bagholder_core::AccountId) -> std::collections::BTreeMap<String, Dec> {
    let mut out: std::collections::BTreeMap<String, Dec> = std::collections::BTreeMap::new();
    for t in book.transactions().unwrap().into_iter().filter(|t| t.account == account) {
        let (Some(i), Some(q)) = (t.instrument, t.quantity) else { continue };
        let symbol = book.current_symbol(i).unwrap().unwrap_or_default();
        let key = match book.option_terms(i).unwrap() {
            Some(o) => format!("{} {} {} {}", book.current_symbol(o.underlying).unwrap().unwrap_or_default(), o.expiry, o.strike.to_text(), if format!("{:?}", o.right).contains("Put") { "P" } else { "C" }),
            None => symbol,
        };
        let e = out.entry(key).or_insert(Dec::ZERO);
        *e = e.checked_add(q).unwrap();
        // an assignment delivers the underlying, its multiplier a contract: out
        // of the account for a call written, into it for a put (the engine's delivery)
        if t.kind == Kind::OptionAssignment {
            let o = book.option_terms(i).unwrap().expect("an assignment's terms");
            let shares = q.abs().checked_mul(o.multiplier.expect("an assigned contract's multiplier")).unwrap();
            let delivered = if format!("{:?}", o.right).contains("Put") { shares } else { shares.neg() };
            let e = out.entry(book.current_symbol(o.underlying).unwrap().unwrap_or_default()).or_insert(Dec::ZERO);
            *e = e.checked_add(delivered).unwrap();
        }
    }
    out.retain(|_, v| !v.is_zero());
    out
}

/// Every account's cash in the book is what the made-up broker states, the
/// accounts it links as one added up.
fn cash_agrees(book: &Book, g: &Generated) {
    let mut stated: std::collections::BTreeMap<bagholder_core::AccountId, std::collections::BTreeMap<String, Dec>> = std::collections::BTreeMap::new();
    for key in g.account_ids() {
        let e = stated.entry(book_account(book, &key)).or_default();
        for (c, v) in g.cash_of(&key) {
            let x = e.entry(c).or_insert(Dec::ZERO);
            *x = x.checked_add(v).unwrap();
        }
    }
    for (account, mut want) in stated {
        want.retain(|_, v| !v.is_zero());
        assert_eq!(book_cash(book, account), want, "account {account}");
    }
}

/// Every account's units in the book are what the made-up broker states, the
/// accounts it links as one added up.
fn units_agree(book: &Book, g: &Generated) {
    let retired = g.retired();
    for key in g.account_ids() {
        if retired.iter().any(|(old, _)| *old == key) {
            continue;
        }
        let account = book_account(book, &key);
        let mut want = g.units_of(&key);
        for (old, new) in &retired {
            if *new == key && book_account(book, old) == account {
                for (k, v) in g.units_of(old) {
                    let e = want.entry(k).or_insert(Dec::ZERO);
                    *e = e.checked_add(v).unwrap();
                }
            }
        }
        want.retain(|_, v| !v.is_zero());
        assert_eq!(book_units(book, account), want, "{key}");
    }
}

#[test]
fn a_move_listed_by_its_arrival_alone_is_booked_leaving_once() {
    let (_h, book, ws, report) = shaped(&[Shape::ArrivalOnly]);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let count = |ty: &str, sub: &str| ws.source.rows().iter().filter(|r| Node::root(r).text("type").ok() == Some(ty) && Node::root(r).text("subType").ok() == Some(sub)).count();
    for ty in ["INTERNAL_TRANSFER", "ASSET_MOVEMENT"] {
        assert!(count(ty, "DESTINATION") > count(ty, "SOURCE"), "some {ty} listed by their arrival alone: {} arrivals, {} leaving", count(ty, "DESTINATION"), count(ty, "SOURCE"));
    }
    // each leaving leg booked once: twice, or none, and the cash or the units would not agree
    cash_agrees(&book, &ws.source);
    units_agree(&book, &ws.source);
}

/// A move listed by its arrival alone on a day another move between the same
/// two accounts is listed whole (an account retired: its CAD by its arrival, its
/// USD on both sides) is booked leaving all the same: the other move's leaving
/// row moves other cash, and records nothing of this one.
#[test]
fn a_move_listed_by_its_arrival_alone_beside_another_listed_whole_is_booked_leaving() {
    for with in [Shape::Closed, Shape::LinkedIds] {
        let (_h, book, ws, report) = shaped(&[Shape::ArrivalOnly, with]);
        assert!(report.failures.is_empty(), "{with:?}: {:?}", report.failures);
        cash_agrees(&book, &ws.source);
        units_agree(&book, &ws.source);
    }
}

#[test]
fn an_account_under_two_ids_wealthsimple_links_is_one_account_open_while_either_is() {
    let (_h, book, ws, report) = shaped(&[Shape::LinkedIds]);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let retired = ws.source.retired();
    let [(old, new)] = &retired[..] else { panic!("one id retired") };
    let account = book_account(&book, old);
    assert_eq!(book_account(&book, new), account, "one book account for both ids");
    assert_eq!(book.account(account).unwrap().status, AccountStatus::Open);
    cash_agrees(&book, &ws.source);
    assert_eq!(book_units(&book, account), ws.source.units_of(new));
}

/// A restating pull states every day again the same day, with no new day due: a
/// run's first whole pull, so a day an earlier build stored wrong is put right as
/// the app starts (the owner, 2026-10-27: "waiting for a whole day to pass for a
/// correction is not acceptable"). A plain pull the same day leaves them as they are.
#[test]
fn a_restating_pull_states_every_day_again_the_same_day() {
    let (_h, book, mut ws, report) = shaped(&[Shape::LinkedIds]);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let [(_, new)] = &ws.source.retired()[..] else { panic!("one id retired") };
    let account = book_account(&book, new);
    let stated = book.stated(account).unwrap().days;
    let spoiled: Vec<AccountDay> = stated.iter().map(|(day, (v, n))| AccountDay { day: *day, net_value: Money::zero(v.currency), net_deposits: Money::zero(n.currency) }).collect();
    let read = book.broker_read(connection(&book, "2026-10-08T21:00:00Z"), "history", at("2026-10-08T21:00:00Z")).unwrap();
    book.store_account_days(account, &spoiled, &read).unwrap();
    let spoilt = book.stated(account).unwrap().days;
    assert_ne!(spoilt, stated);
    // the same day: a plain pull asks no history
    let now = "2026-10-08T22:00:00Z";
    pull(&book, &mut ws, connection(&book, now), shaped_today(), at(now), &mut |_| {}).unwrap();
    assert_eq!(book.stated(account).unwrap().days, spoilt);
    // and a restating one states every day again
    let now = "2026-10-08T23:00:00Z";
    let r = pull_restating(&book, &mut ws, connection(&book, now), shaped_today(), at(now), &mut |_| {}).unwrap();
    assert!(!r.failures.iter().any(|(part, _)| part.starts_with("history")), "{:?}", r.failures);
    assert_eq!(book.stated(account).unwrap().days, stated);
}

/// The days of an account under two ids, the first closed halfway, stored at
/// nothing by an earlier read, are stated again by the next day's pull as both
/// ids state them added up: the closed id is read too, since the days it held
/// the account are its to state.
#[test]
fn an_account_under_two_ids_has_its_days_stated_again_from_both_the_closed_one_too() {
    let (_h, book, mut ws, report) = shaped(&[Shape::LinkedIds]);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let [(old, new)] = &ws.source.retired()[..] else { panic!("one id retired") };
    let (old, new) = (old.clone(), new.clone());
    let account = book_account(&book, &new);
    // what both ids state, added up by day
    let mut both: std::collections::BTreeMap<jiff::civil::Date, (Money, Money)> = std::collections::BTreeMap::new();
    for key in [&old, &new] {
        for d in BrokerAdapter::history(&mut ws, key, None).unwrap() {
            let (v, n) = both.entry(d.day).or_insert((Money::zero(d.net_value.currency), Money::zero(d.net_deposits.currency)));
            *v = Money::new(v.amount.checked_add(d.net_value.amount).unwrap(), v.currency);
            *n = Money::new(n.amount.checked_add(d.net_deposits.amount).unwrap(), n.currency);
        }
    }
    assert_eq!(book.stated(account).unwrap().days, both, "the first pull states both ids");
    let held_by_the_closed_id = BrokerAdapter::history(&mut ws, &old, None).unwrap().iter().filter(|d| !d.net_value.amount.is_zero()).count();
    assert!(held_by_the_closed_id > 30, "the closed id held the account for a while: {held_by_the_closed_id} days");
    // an earlier build's read: every day at nothing
    let spoiled: Vec<AccountDay> = both.iter().map(|(day, (v, n))| AccountDay { day: *day, net_value: Money::zero(v.currency), net_deposits: Money::zero(n.currency) }).collect();
    let read = book.broker_read(connection(&book, "2026-10-08T21:00:00Z"), "history", at("2026-10-08T21:00:00Z")).unwrap();
    book.store_account_days(account, &spoiled, &read).unwrap();
    // the next pull with a day new to state: the made-up broker states its own
    // today, so two days on
    let now = "2026-10-10T20:00:00Z";
    let next = pull(&book, &mut ws, connection(&book, now), "2026-10-10".parse().unwrap(), at(now), &mut |_| {}).unwrap();
    assert!(!next.failures.iter().any(|(part, _)| part.starts_with("history")), "{:?}", next.failures);
    assert_eq!(book.stated(account).unwrap().days, both);
}

#[test]
fn a_closed_account_with_history_is_read_whole_once_and_holds_nothing() {
    let (_h, book, mut ws, report) = shaped(&[Shape::Closed]);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let retired = ws.source.retired();
    let [(closed, _)] = &retired[..] else { panic!("one account closed") };
    let closed = closed.clone();
    let account = book_account(&book, &closed);
    assert_eq!(book.account(account).unwrap().status, AccountStatus::Closed);
    assert!(book_cash(&book, account).is_empty() && book_units(&book, account).is_empty());
    assert!(ws.source.asked_activity.iter().any(|(a, from)| *a == closed && from.is_none()), "read whole");
    cash_agrees(&book, &ws.source);
    // the next pull reads it no more
    ws.source.asked_activity.clear();
    let report = pull(&book, &mut ws, connection(&book, "2026-10-08T21:00:00Z"), shaped_today(), at("2026-10-08T21:00:00Z"), &mut |_| {}).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(!ws.source.asked_activity.iter().any(|(a, _)| *a == closed), "{:?}", ws.source.asked_activity);
}

#[test]
fn a_row_listed_in_a_closed_account_after_its_one_full_read_is_booked_once() {
    let (_h, book, mut ws, report) = shaped(&[Shape::Closed, Shape::LateRow]);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let retired = ws.source.retired();
    let [(closed, to)] = &retired[..] else { panic!("one account closed") };
    let (closed, to) = (closed.clone(), to.clone());
    let kept = book_cash(&book, book_account(&book, &closed));
    assert!(!kept.is_empty(), "a little cash kept in it until the late row");
    ws.source.publish_late();
    ws.source.asked_activity.clear();
    let report = pull(&book, &mut ws, connection(&book, "2026-10-08T21:00:00Z"), shaped_today(), at("2026-10-08T21:00:00Z"), &mut |_| {}).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(!ws.source.asked_activity.iter().any(|(a, from)| *a == closed && from.is_none()), "not read whole again: {:?}", ws.source.asked_activity);
    assert!(ws.source.asked_activity.iter().any(|(a, _)| *a == to));
    assert!(book_cash(&book, book_account(&book, &closed)).is_empty(), "moved out once");
    cash_agrees(&book, &ws.source);
}

/// Every shape at once, as a real book has them, over several seeds and days:
/// the pull fails nothing, and every account's units the book derives agree with
/// the broker. Its cash is not held to the broker's: a conversion whose detail
/// states the side received alone (`Shape::FundingIntent`) books no side paid, and
/// the cash every screen shows is Wealthsimple's (brief 79).
#[test]
fn a_book_of_every_shape_is_pulled_whole_and_agrees_with_the_broker() {
    for (seed, today) in [(13, "2026-10-08"), (1, "2026-10-25"), (5, "2026-10-25"), (9, "2026-10-25")] {
        let (_h, book, ws, report) = shaped_by(Size { seed, ..SHAPED }, today.parse().unwrap(), &SHAPES);
        assert!(report.failures.is_empty(), "seed {seed}: {:?}", report.failures);
        units_agree(&book, &ws.source);
    }
}

/// Calls written on a US share held for them and assigned at expiry
/// (`Shape::Assigned`), with every other shape: the pull fails nothing, the
/// assignment is booked and adds no kind of problem to the book, and every account's
/// units, the shares delivered and the contracts closed among them, agree with the
/// broker (its cash is not held to the broker's, as with every shape at once).
#[test]
fn calls_written_and_assigned_are_pulled_and_the_units_agree_with_the_broker() {
    let shapes: Vec<Shape> = SHAPES.iter().chain(bagholder_wealthsimple::generated::ASSIGNED.iter()).copied().collect();
    let today: jiff::civil::Date = "2026-10-25".parse().unwrap();
    for seed in [13, 1, 5] {
        let (_h, book, ws, report) = shaped_by(Size { seed, ..SHAPED }, today, &shapes);
        assert!(report.failures.is_empty(), "seed {seed}: {:?}", report.failures);
        let assigned = ws.source.rows().iter().filter(|r| Node::root(r).text("type").ok() == Some("OPTIONS_ASSIGN")).count();
        assert_eq!(assigned, 1, "seed {seed}: one assignment listed");
        assert_eq!(book.transactions().unwrap().iter().filter(|t| t.kind == Kind::OptionAssignment).count(), 1, "seed {seed}: the assignment booked");
        let (_h2, without, _, _) = shaped_by(Size { seed, ..SHAPED }, today, &SHAPES);
        // the share bought for the calls may be funded by one more conversion, of a problem the book has without them
        let kinds = |b: &Book| b.problems().unwrap().into_iter().map(|(_, p)| p.code).collect::<std::collections::BTreeSet<_>>();
        assert!(kinds(&book).is_subset(&kinds(&without)), "seed {seed}: the assignment adds a kind of problem: {:?} beside {:?}", kinds(&book), kinds(&without));
        units_agree(&book, &ws.source);
    }
}

// ---------------------------------------------------------------------------
// A book read whole again, and carried by the migrations (brief 76, PR 2): the
// book built from the made-up broker with every shape on is the same book
// whichever way it is reached.
// ---------------------------------------------------------------------------

/// What a book holds, by what names it apart from the book's own ids: each
/// account, by the broker ids behind it, with its cash per currency and units
/// per symbol, and each record's state by its source and key.
#[allow(clippy::type_complexity)]
fn held(book: &Book) -> (std::collections::BTreeMap<String, (std::collections::BTreeMap<String, Dec>, std::collections::BTreeMap<String, Dec>)>, std::collections::BTreeMap<(String, String), String>) {
    let mut accounts = std::collections::BTreeMap::new();
    for a in book.accounts().unwrap() {
        let mut ids: Vec<String> = book.account_refs(a.id).unwrap().into_iter().map(|r| r.value).collect();
        ids.sort();
        accounts.insert(ids.join("+"), (book_cash(book, a.id), book_units(book, a.id)));
    }
    let records = book
        .conn_for_tests()
        .prepare("SELECT source, source_key, state FROM source_records")
        .unwrap()
        .query_map([], |r| Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?), r.get::<_, String>(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (accounts, records)
}

/// A book of every shape read whole again from the same broker, every account's
/// full read cleared as migration 37 cleared them: the same accounts, cash,
/// units and records. 2.10.0's re-read of the owner's book was not the same book.
#[test]
fn a_book_of_every_shape_read_whole_again_is_the_same_book() {
    for (seed, today) in [(13, "2026-10-08"), (5, "2026-10-25")] {
        let today: jiff::civil::Date = today.parse().unwrap();
        let (_h, book, mut ws, report) = shaped_by(Size { seed, ..SHAPED }, today, &SHAPES);
        assert!(report.failures.is_empty(), "seed {seed}: {:?}", report.failures);
        assert!(report.suspect.is_empty(), "seed {seed}: {:?}", report.suspect);
        let before = held(&book);
        book.conn_for_tests().execute("DELETE FROM activity_reads", []).unwrap();
        let now = today.to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp().checked_add(jiff::SignedDuration::from_hours(21)).unwrap();
        let report = pull(&book, &mut ws, connection(&book, &now.to_string()), today, now, &mut |_| {}).unwrap();
        assert!(report.failures.is_empty(), "seed {seed}: {:?}", report.failures);
        // nothing read is held suspect: a guard keeping rows in the book would hide their loss
        assert!(report.suspect.is_empty(), "seed {seed}: {:?}", report.suspect);
        assert!(ws.source.asked_activity.iter().filter(|(_, from)| from.is_none()).count() >= ws.source.account_ids().len() * 2, "seed {seed}: every account read whole twice");
        assert_eq!(held(&book), before, "seed {seed}");
        units_agree(&book, &ws.source);
    }
}

/// A book's rows as a version-36 file holds them: every row of the tables both
/// share copied into a file built from migrations 1 to 36, its change log's
/// triggers set aside while the rows go in (`book/tests/schema.rs`, `as_v36`).
fn as_v36(home: &Path) -> PathBuf {
    use bagholder_book::schema::{MIGRATIONS, SCHEMA};
    use bagholder_sqlite::migrate::{self, Schema};
    let path = home.join("v36.db");
    let v36: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..36] }));
    let (conn, _) = migrate::open(v36, &path, "test", at("2026-01-01T00:00:00Z")).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute("ATTACH DATABASE ?1 AS src", [home.join("book.db").to_string_lossy()]).unwrap();
    let triggers: Vec<(String, String)> = conn.prepare("SELECT name, sql FROM main.sqlite_master WHERE type = 'trigger'").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    for (name, _) in &triggers {
        conn.execute_batch(&format!("DROP TRIGGER main.\"{name}\"")).unwrap();
    }
    let tables: Vec<String> = conn.prepare("SELECT name FROM main.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'schema_migrations'").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    let columns = |schema: &str, t: &str| -> Vec<String> { conn.prepare(&format!("SELECT name FROM pragma_table_info('{t}', '{schema}')")).unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect() };
    for t in &tables {
        let theirs = columns("src", t);
        let both: Vec<String> = columns("main", t).into_iter().filter(|c| theirs.contains(c)).map(|c| format!("\"{c}\"")).collect();
        if both.is_empty() {
            continue;
        }
        conn.execute_batch(&format!("DELETE FROM main.\"{t}\"; INSERT INTO main.\"{t}\" ({0}) SELECT {0} FROM src.\"{t}\"", both.join(", "))).unwrap();
    }
    for (_, sql) in &triggers {
        conn.execute_batch(sql).unwrap();
    }
    drop(conn);
    path
}

/// A book of every shape as a version-36 file holds it (the ids Wealthsimple
/// links on one account, as the import pooled them), carried by the migrations
/// from 37 on: what each migration changes is its own intended effect, named,
/// and the next pull makes it the same book again.
#[test]
fn a_book_of_every_shape_carried_from_version_36_is_the_same_book_after_its_next_pull() {
    let (h, book, mut ws, report) = shaped(&SHAPES);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let before = held(&book);
    drop(book);
    let path = as_v36(h.path());
    let opened = at("2026-10-08T21:00:00Z");
    let mappings: [&dyn bagholder_book::mapping::Mapping; 2] = [&bagholder_wealthsimple::mapping::WealthsimpleMapping, &bagholder_wealthsimple::statement::StatementMapping];
    let (book, done) = Book::open_mapped(&path, "test", opened, &mappings).unwrap();
    assert_eq!(done.from, 36);
    let (accounts, records) = held(&book);
    // migration 37: every account holding several broker ids split to one per id,
    // its legs on the id each names; its records stand as they were
    assert_eq!(records, before.1, "every record as it was");
    let pooled: Vec<&String> = before.0.keys().filter(|k| k.contains('+')).collect();
    assert_eq!(pooled.len(), 1, "the linked set, one account before: {:?}", before.0.keys());
    for (ids, figures) in &before.0 {
        if ids.contains('+') {
            let mut cash: std::collections::BTreeMap<String, Dec> = std::collections::BTreeMap::new();
            let mut units: std::collections::BTreeMap<String, Dec> = std::collections::BTreeMap::new();
            for id in ids.split('+') {
                let (c, u) = accounts.get(id).unwrap_or_else(|| panic!("{id} its own account after migration 37"));
                for (k, v) in c {
                    let e = cash.entry(k.clone()).or_insert(Dec::ZERO);
                    *e = e.checked_add(*v).unwrap();
                }
                for (k, v) in u {
                    let e = units.entry(k.clone()).or_insert(Dec::ZERO);
                    *e = e.checked_add(*v).unwrap();
                }
            }
            cash.retain(|_, v| !v.is_zero());
            units.retain(|_, v| !v.is_zero());
            assert_eq!((&cash, &units), (&figures.0, &figures.1), "{ids}: split, the same cash and units between its ids");
        } else {
            assert_eq!(accounts.get(ids), Some(figures), "{ids}: as it was");
        }
    }
    // migration 39 finds no link to make it one (37 took the split account's
    // links with its history): the next pull states them, and makes it one
    let report = pull(&book, &mut ws, connection(&book, "2026-10-08T22:00:00Z"), shaped_today(), at("2026-10-08T22:00:00Z"), &mut |_| {}).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(report.suspect.is_empty(), "{:?}", report.suspect);
    assert_eq!(report.accounts_merged, 1, "{report:?}");
    assert_eq!(held(&book), before);
}
