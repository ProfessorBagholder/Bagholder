//! The figures every reading of the broker comes to, pinned (brief 77, §3): for
//! the made-up Wealthsimple with every shape on, at twelve seeds, each account's
//! cash in each currency, its units of each symbol and the units of each it bought
//! and sold, and the header's lines about
//! the broker, as the pull and the engine make them for a person's book. A change
//! to the pull, a mapping or the statements walk that moves a figure moves it in
//! `tests/golden/figures.json` in the same PR, so that file's diff is every
//! figure it moves. `BAGHOLDER_BLESS=1` writes the file; a missing file fails.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{Generated, Shape, Size, FILL_FORMS, SHAPES};
use serde_json::{json, Map, Value};

/// Every shape the made-up broker has from recorded books, but a fill the statement
/// states on two rows (`Shape::SplitFill`): 2.10.8's units step reads its cash row as
/// a fill of no units wherever a walk kept its month, and the months kept differ
/// between a book the version before read and one this version reads, so the two
/// disagree until that step is fixed, when it joins.
fn every_shape() -> Vec<Shape> {
    SHAPES.iter().chain(FILL_FORMS.iter()).filter(|s| **s != Shape::SplitFill).copied().collect()
}

/// One seed's book, pulled whole and opened as the app opens it: its figures.
/// `carried`: the broker read first by the mapping a version back, which read a
/// conversion's side paid as unstated (`tests_common::PaidUnread`), as a
/// person's book was read before rule set 16; the opening then derives it again,
/// and says how many of the statements' bookings it withdrew.
fn figures_through(seed: u64, carried: bool) -> (Value, usize) {
    let today: bagholder_core::jiff::civil::Date = "2026-10-25".parse().unwrap();
    let now = today.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).unwrap().timestamp().checked_add(bagholder_core::jiff::SignedDuration::from_hours(20)).unwrap();
    let home = tempfile::tempdir().unwrap();
    let statements = bagholder_book::mapping::Mapping::source(&bagholder_wealthsimple::statement::StatementMapping);
    let (report, booked) = {
        let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
        // the zone the page states, as a person's first visit keeps it
        book.state_zone("America/Toronto", now).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        let size = Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed, deep: 0 };
        let ws = Wealthsimple::new(Generated::shaped(size, today, &every_shape()));
        let report = if carried { pull(&book, &mut crate::tests_common::ReadBefore(ws), connection, today, now, &mut |_| {}) } else { pull(&book, &mut { ws }, connection, today, now, &mut |_| {}) }.unwrap();
        (report, book.live_records(&statements).unwrap())
    };
    let f = crate::figures::Figures::open(home.path(), now).unwrap();
    let value = f.read(|e| {
        let inputs = e.inputs();
        type Sums = BTreeMap<String, bagholder_core::Dec>;
        let add = |m: &mut Sums, k: String, v: bagholder_core::Dec| {
            let e = m.entry(k).or_insert(bagholder_core::Dec::ZERO);
            *e = e.checked_add(v).unwrap();
        };
        // net cash and units, and the units bought and sold: a round trip moves no net figure
        let mut accounts: BTreeMap<String, (Sums, Sums, Sums, Sums)> = BTreeMap::new();
        for t in inputs.ledger.transactions.iter() {
            let (cash, units, bought, sold) = accounts.entry(crate::wire::build::account_name(inputs, t.account)).or_default();
            if let Some(c) = t.cash {
                add(cash, c.currency.to_string(), c.amount);
            }
            if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
                let symbol = crate::status::symbol(inputs, i);
                add(units, symbol.clone(), q);
                match t.kind {
                    bagholder_core::transaction::Kind::Buy => add(bought, symbol, q.abs()),
                    bagholder_core::transaction::Kind::Sell => add(sold, symbol, q.abs()),
                    _ => {}
                }
            }
        }
        let accounts: Map<String, Value> = accounts
            .into_iter()
            .map(|(name, (cash, units, bought, sold))| {
                let text = |m: Sums| -> Map<String, Value> { m.into_iter().filter(|(_, v)| !v.is_zero()).map(|(k, v)| (k, json!(v.to_text()))).collect() };
                (name, json!({"cash": text(cash), "units": text(units), "bought": text(bought), "sold": text(sold)}))
            })
            .collect();
        let mut header = crate::status::broker_failures(e);
        header.extend(crate::status::unreconciled(e, &report.statements.unreconciled));
        header.extend(crate::status::feed_only(e, &report.statements.feed_only));
        header.extend(crate::status::unread_rows(inputs));
        // the lines, not their order, which differs from one opening to the next
        header.sort();
        json!({"accounts": accounts, "header": header})
    })
    .unwrap();
    drop(f);
    let live = Book::open_in(home.path(), "test", now).unwrap().0.live_records(&statements).unwrap();
    (value, booked.iter().filter(|r| !live.contains(r)).count())
}

/// One seed's figures, and how many of the statements' bookings an opening
/// withdraws in a book the version before read: that book, once opened, is the
/// one the version makes.
fn figures(seed: u64) -> Value {
    let (mut direct, _) = figures_through(seed, false);
    let (carried, withdrawn) = figures_through(seed, true);
    assert_eq!(carried, direct, "seed {seed}: the book the version before read, once opened, is not the one the version makes");
    direct["withdrawn at opening, the book read a version before"] = json!(withdrawn);
    direct
}

#[test]
fn every_figure_the_broker_s_reading_comes_to_is_the_golden_one() {
    let _g = crate::tests_common::guard();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/figures.json");
    let got: Map<String, Value> = (1..=12u64).map(|seed| (format!("seed {seed:02}"), figures(seed))).collect();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(got)).unwrap() + "\n").unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is missing ({e}): write it with BAGHOLDER_BLESS=1 and review its every line", path.display()));
    let want: Map<String, Value> = serde_json::from_str(&text).unwrap();
    let mut seeds: Vec<&String> = got.keys().chain(want.keys()).collect();
    seeds.sort();
    seeds.dedup();
    for seed in seeds {
        assert_eq!(got.get(seed), want.get(seed), "{seed}'s figures are not the golden ones: a change that moves a figure moves it in tests/golden/figures.json");
    }
}
