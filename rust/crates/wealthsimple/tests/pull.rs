//! The pull reads only what changed (brief 07 §1, `docs/decisions.md`: calls to
//! Wealthsimple's unofficial API are kept to what is necessary), held by counting
//! what it asks. One account's November, from the owner's own history
//! (`tests/replies/wealthsimple-pull`).

use std::path::{Path, PathBuf};

use bagholder_book::Book;
use bagholder_broker::pull::{pull, Report};
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::replay::Replay;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-pull")
}

fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

/// One pull of the replies in `replies` into `book`: what it did, and what it asked.
fn once(book: &Book, replies: &Path, now: &str) -> (Report, Vec<String>) {
    let connection = match book.connections().unwrap().into_iter().find(|c| c.broker == Broker::named("wealthsimple")) {
        Some(c) => c.id,
        None => book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", at(now)).unwrap(),
    };
    let mut ws = Wealthsimple::new(Replay::read(replies).unwrap());
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
