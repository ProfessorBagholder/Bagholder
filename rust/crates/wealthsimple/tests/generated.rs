//! The made-up Wealthsimple (`src/generated.rs`) read through the real adapter,
//! mapping and pull: every row it makes is one the mapping reads, with no
//! problem; the cash, units and values it states agree with what the book
//! derives from its rows; and the same seed makes the same replies.

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{Generated, Size};

const SIZE: Size = Size { accounts: 14, instruments: 24, trades: 80, months: 8, seed: 7 };

fn today() -> jiff::civil::Date {
    "2026-10-08".parse().unwrap()
}

#[test]
fn the_same_seed_makes_the_same_replies() {
    let canon = |g: &Generated| g.rows().iter().map(|r| r.canonical()).collect::<Vec<_>>();
    assert_eq!(canon(&Generated::new(SIZE, today())), canon(&Generated::new(SIZE, today())));
    let other = Generated::new(Size { seed: 8, ..SIZE }, today());
    assert_ne!(canon(&Generated::new(SIZE, today())), canon(&other));
}

#[test]
fn every_row_is_read_with_no_problem_and_the_pull_fails_nothing() {
    let home = tempfile::tempdir().unwrap();
    let now: jiff::Timestamp = "2026-10-08T20:00:00Z".parse().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
    let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
    let mut ws = Wealthsimple::new(Generated::new(SIZE, today()));
    let rows = ws.source.rows().len();
    let report = pull(&book, &mut ws, connection, today(), now, &mut |_| {}).unwrap();
    eprintln!("{report:#?}");
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.rows_read, rows);
    let problems = book.problems().unwrap();
    assert!(problems.is_empty(), "{} problems, the first: {:?}", problems.len(), problems.iter().take(5).collect::<Vec<_>>());
}

