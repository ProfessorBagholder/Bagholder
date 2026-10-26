//! The made-up Wealthsimple (`src/generated.rs`) read through the real adapter,
//! mapping and pull: every row it makes is one the mapping reads, with no
//! problem; the cash, units and values it states agree with what the book
//! derives from its rows; and the same seed makes the same replies.

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::Wealthsimple;
#[cfg(feature = "demo")]
use bagholder_wealthsimple::generated::{AccountRecord, ListingRecord, ListingSort, Record};
use bagholder_wealthsimple::generated::{Generated, Size};

const SIZE: Size = Size { accounts: 14, instruments: 24, trades: 80, months: 8, seed: 7, deep: 0 };

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

/// A day's rows are timed in the order they are made, in a book of more rows
/// than an hour has seconds: timed by a count over the whole book, the day the
/// count passed the hour timed a sale before the buy it sold, and that holding
/// was never flat again (#618).
#[test]
fn a_days_rows_are_timed_in_the_order_they_are_made() {
    use bagholder_sources::reply::Node;
    let g = Generated::new(Size { accounts: 29, instruments: 157, trades: 646, months: 36, seed: 1, deep: 0 }, today());
    // an expiry is timed at the close, whenever it is made
    let at: Vec<&str> = g.rows().iter().map(Node::root).filter(|r| r.text("type") != Ok("OPTIONS_EXPIRY")).map(|r| r.text("occurredAt").unwrap()).collect();
    assert!(at.len() > 3_600, "{} rows", at.len());
    for w in at.windows(2) {
        assert!(w[0][..10] != w[1][..10] || w[0] <= w[1], "{} is made after {}", w[1], w[0]);
    }
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


/// A made-up Wealthsimple stated whole (`Generated::from_records`): every row
/// the records state, at its own cash and units, read with no problem; a put
/// stated as one; a contract sold to open held short; each day's value as stated.
/// Built only with the `demo` feature (`cargo test --features demo`), as it is.
#[cfg(feature = "demo")]
#[test]
fn a_book_stated_whole_is_pulled_as_stated() {
    let day = |s: &str| s.parse::<jiff::civil::Date>().unwrap();
    let accounts = vec![AccountRecord {
        id: "acct-a".into(),
        nickname: "Margin".into(),
        ty: "SELF_DIRECTED_NON_REGISTERED_MARGIN".into(),
        currency: "CAD".into(),
        opened: day("2026-01-02"),
        days: vec![(day("2026-01-02"), 0, 0), (day("2026-10-07"), 1_234_500, 1_000_000)],
        buying_power: Some(4_125_080),
    }];
    let share = |id: &str, symbol: &str, cur: &str, venue: (&str, &str), price| ListingRecord { id: id.into(), symbol: symbol.into(), name: format!("{symbol} Made Up"), currency: cur.into(), sort: ListingSort::Share { exchange: venue.0.into(), mic: venue.1.into() }, price };
    let contract = |id: &str, put, price| ListingRecord { id: id.into(), symbol: "ZQU".into(), name: String::new(), currency: "USD".into(), sort: ListingSort::Contract { underlying: "sec-zqu".into(), expiry: day("2026-12-18"), strike: 5_000, put }, price };
    let listings = vec![share("sec-zqu", "ZQU", "USD", ("NYSE", "XNYS"), 5_150), share("sec-zqc", "ZQC", "CAD", ("TSX", "XTSE"), 2_000), contract("sec-zqu-put", true, 120), contract("sec-zqu-call", false, 90)];
    let (a, rec) = ("acct-a".to_string(), |d: &str, r| (day(d), r));
    let records = vec![
        rec("2026-02-02", Record::Deposit { account: a.clone(), cents: 2_000_000 }),
        rec("2026-02-03", Record::Buy { account: a.clone(), listing: "sec-zqc".into(), units: 100, cents: 190_000 }),
        rec("2026-02-04", Record::Buy { account: a.clone(), listing: "sec-zqu".into(), units: 100, cents: 500_000 }),
        rec("2026-03-02", Record::Dividend { account: a.clone(), listing: "sec-zqc".into(), units: 100, cents: 1_550 }),
        rec("2026-04-01", Record::Buy { account: a.clone(), listing: "sec-zqu-put".into(), units: 2, cents: 30_000 }),
        rec("2026-05-01", Record::Sell { account: a.clone(), listing: "sec-zqu-put".into(), units: 2, cents: 24_000 }),
        rec("2026-09-01", Record::Sell { account: a.clone(), listing: "sec-zqu-call".into(), units: 1, cents: 9_000 }),
        rec("2026-09-01", Record::InterestCharge { account: a.clone(), currency: "USD".into(), cents: 6_410 }),
    ];
    let home = tempfile::tempdir().unwrap();
    let now: jiff::Timestamp = "2026-10-08T20:00:00Z".parse().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
    let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
    let mut ws = Wealthsimple::new(Generated::from_records(today(), &accounts, &listings, &records));
    assert_eq!(ws.source.rows().len(), records.len(), "one row a record, nothing funded besides");
    let report = pull(&book, &mut ws, connection, today(), now, &mut |_| {}).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(book.problems().unwrap().is_empty(), "{:?}", book.problems().unwrap());
    let cash: Vec<String> = book.transactions().unwrap().iter().filter_map(|t| t.cash).map(|c| format!("{} {}", c.amount.to_text(), c.currency)).collect();
    assert_eq!(cash, ["20000 CAD", "-1900 CAD", "-5000 USD", "15.5 CAD", "-300 USD", "240 USD", "90 USD", "-64.1 USD"]);
    let paid_on: Vec<String> = book.transactions().unwrap().iter().filter_map(|t| t.paid_on).map(|q| q.to_text()).collect();
    assert_eq!(paid_on, ["100"]);
    let rights: Vec<String> = book.instruments().unwrap().iter().filter_map(|i| book.option_terms(i.id).unwrap()).map(|t| format!("{:?}", t.right)).collect();
    assert_eq!(rights.len(), 2);
    assert!(rights.iter().any(|r| r.contains("Put")) && rights.iter().any(|r| r.contains("Call")), "{rights:?}");
    let account = book.accounts().unwrap()[0].id;
    assert_eq!(book.last_account_day(account).unwrap(), Some(day("2026-10-07")));
}
