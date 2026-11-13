//! The figures every reading of the broker comes to, pinned (brief 77, §3): for
//! the made-up Wealthsimple with every shape on, at twelve seeds, each account's
//! cash in each currency, its units of each symbol and the units of each it bought
//! and sold, as the pull and the engine make them for a person's book. A change
//! to the pull or a mapping that moves a figure moves it in
//! `tests/golden/figures.json` in the same PR, so that file's diff is every
//! figure it moves. `BAGHOLDER_BLESS=1` writes the file; a missing file fails.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bagholder_book::mapping::Mapping;
use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{Generated, Shape, Size, FILL_FORMS, SHAPES};
use serde_json::{json, Map, Value};

/// Every shape the made-up broker has from recorded books.
fn every_shape() -> Vec<Shape> {
    SHAPES.iter().chain(FILL_FORMS.iter()).copied().collect()
}

/// The day the made-up broker is read on, and the moment.
fn read_on() -> (bagholder_core::jiff::civil::Date, bagholder_core::jiff::Timestamp) {
    let today: bagholder_core::jiff::civil::Date = "2026-10-25".parse().unwrap();
    (today, today.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).unwrap().timestamp().checked_add(bagholder_core::jiff::SignedDuration::from_hours(20)).unwrap())
}

/// One seed's book, pulled whole and opened as the app opens it: its figures.
/// `carried`: the broker read first by the mapping a version back, which read a
/// conversion's side paid as unstated (`tests_common::PaidUnread`), as a
/// person's book was read before rule set 16; the opening then derives it again.
fn figures_through(seed: u64, carried: bool) -> Value {
    let (today, now) = read_on();
    let home = tempfile::tempdir().unwrap();
    {
        let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
        // the zone the page states, as a person's first visit keeps it
        book.state_zone("America/Toronto", now).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        let size = Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed, deep: 0 };
        let ws = Wealthsimple::new(Generated::shaped(size, today, &every_shape()));
        if carried { pull(&book, &mut crate::tests_common::ReadBefore(ws, &crate::tests_common::PaidUnread), connection, today, now, &mut |_| {}) } else { pull(&book, &mut { ws }, connection, today, now, &mut |_| {}) }.unwrap();
    }
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
        json!({"accounts": accounts})
    })
    .unwrap();
    value
}

/// One seed's figures: a book the version before read, once opened, is the one
/// the version makes.
fn figures(seed: u64) -> Value {
    let direct = figures_through(seed, false);
    let carried = figures_through(seed, true);
    assert_eq!(carried, direct, "seed {seed}: the book the version before read, once opened, is not the one the version makes");
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

/// What a book holds, as text, but each instrument's fund type, each record's
/// and transaction's version, the change log (`ledger_changes`,
/// `successions_due`), which says what was read again, and a name's row numbers
/// of its first and last sightings, which every derivation writes afresh (the
/// sightings themselves are compared); those apart.
struct Held {
    rest: String,
    funds: Vec<(String, Option<i64>)>,
    versions: std::collections::BTreeSet<i64>,
    logged: std::collections::BTreeSet<String>,
}

fn held(home: &std::path::Path, after: i64) -> Held {
    let c = rusqlite::Connection::open(home.join(bagholder_book::BOOK_FILE)).unwrap();
    let tables: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('ledger_changes', 'successions_due') ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    let mut rest = String::new();
    for t in tables {
        let mut st = c.prepare(&format!("SELECT * FROM {t} ORDER BY 1, 2")).unwrap();
        let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let cells: Vec<String> = names
                .iter()
                .enumerate()
                .filter(|(_, n)| !matches!((t.as_str(), n.as_str()), ("instruments", "fund") | ("source_records", "derived_version") | ("transactions", "mapping_version") | ("instrument_names", "first_sighting" | "last_sighting")))
                .map(|(i, n)| match r.get_ref(i).unwrap() {
                    rusqlite::types::ValueRef::Text(t) => format!("{n}={}", String::from_utf8_lossy(t)),
                    v => format!("{n}={v:?}"),
                })
                .collect();
            rest.push_str(&format!("{t}: {}\n", cells.join(" | ")));
        }
    }
    let funds = c.prepare("SELECT kind, fund FROM instruments ORDER BY id").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    let mut versions: std::collections::BTreeSet<i64> = c.prepare("SELECT derived_version FROM source_records WHERE state = 'live'").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    versions.extend(c.prepare("SELECT mapping_version FROM transactions").unwrap().query_map([], |r| r.get::<_, i64>(0)).unwrap().map(Result::unwrap));
    let logged = c.prepare("SELECT DISTINCT kind FROM ledger_changes WHERE seq > ?").unwrap().query_map([after], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    Held { rest, funds, versions, logged }
}

/// The re-derivation version 19 starts on a book the version before read
/// (brief 82): no transaction moves, and nothing else the book holds but what
/// the version states anew, each instrument's fund type; each record's version;
/// and the change log's note of what was read again. The made-up broker states
/// each share an equity, so a share comes to be no fund, and an option or a
/// coin stays unsaid.
#[test]
fn deriving_a_book_the_version_before_read_moves_only_each_instrument_s_fund_type() {
    let _g = crate::tests_common::guard();
    let version = bagholder_wealthsimple::mapping::WealthsimpleMapping.version() as i64;
    for seed in 1..=12u64 {
        let (today, now) = read_on();
        let home = tempfile::tempdir().unwrap();
        let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
        book.state_zone("America/Toronto", now).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        let size = Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed, deep: 0 };
        let ws = Wealthsimple::new(Generated::shaped(size, today, &every_shape()));
        pull(&book, &mut crate::tests_common::ReadBefore(ws, &crate::tests_common::FundUnstated), connection, today, now, &mut |_| {}).unwrap();
        let last: i64 = rusqlite::Connection::open(home.path().join(bagholder_book::BOOK_FILE)).unwrap().query_row("SELECT COALESCE(MAX(seq), 0) FROM ledger_changes", [], |r| r.get(0)).unwrap();
        let before = held(home.path(), last);
        let changes = book.rederive(&bagholder_wealthsimple::mapping::WealthsimpleMapping, now).unwrap();
        let after = held(home.path(), last);
        assert!(changes.is_empty(), "seed {seed}: transactions moved: {changes:?}");
        let moved = |a: &str, b: &str| a.lines().filter(|l| !b.lines().any(|m| m == *l)).map(str::to_string).collect::<Vec<_>>();
        assert!(before.rest == after.rest, "seed {seed}: something but the fund types and the versions moved: was {:#?} now {:#?}", moved(&before.rest, &after.rest), moved(&after.rest, &before.rest));
        assert_eq!((before.versions, after.versions), ([version - 1].into(), [version].into()), "seed {seed}");
        assert!(after.logged.iter().all(|k| k == "record" || k == "instrument"), "seed {seed}: {:?}", after.logged);
        assert!(before.funds.iter().all(|(_, f)| f.is_none()), "seed {seed}: the version before stated a fund type");
        for (kind, fund) in &after.funds {
            assert_eq!(*fund, (kind == "security").then_some(0), "seed {seed}: a {kind}'s fund type");
        }
        assert!(after.funds.iter().any(|(k, _)| k == "security") && after.funds.iter().any(|(k, _)| k != "security"), "seed {seed}: {:?}", after.funds);
    }
}
