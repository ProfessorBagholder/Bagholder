//! What 2.10.9 booked from the statements, taken back (`docs/decisions.md`,
//! 2026-10-26). Two books written by those versions' own code, from one book
//! 2.10.8 read (`tests/books/README.md`): 2.10.8 reading it again, and 2.10.9
//! reading it again. Read once more by this version, the second is the first:
//! the same records live and superseded, with the same latest payloads, and the
//! same transactions; what 2.10.9 wrote is removed.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use bagholder_book::Book;
use bagholder_broker::pull::{pull, Report};
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{Generated, Shape, Size, SHAPES, STATED_OTHERWISE};

/// The reason 2.10.9 gave a feed record a statement fill took the place of.
const RESTATED_IN_2_10_9: &str = "the broker's statement states the fill otherwise";

/// One of the books in `tests/books`, in a home of its own.
fn book_of(name: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let gz = std::fs::File::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/books").join(format!("{name}.db.gz"))).unwrap();
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut bytes).unwrap();
    std::fs::write(home.path().join(bagholder_book::BOOK_FILE), bytes).unwrap();
    home
}

/// A read of the made-up broker the books were written from, `hour` hours into its day.
fn read(home: &std::path::Path, hour: i64) -> (Book, Report) {
    let today: jiff::civil::Date = "2026-10-25".parse().unwrap();
    let now = today.to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp().checked_add(jiff::SignedDuration::from_hours(hour)).unwrap();
    let shapes: Vec<Shape> = SHAPES.iter().chain(STATED_OTHERWISE.iter()).copied().collect();
    let size = Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed: 13, deep: 0 };
    let (book, _) = Book::open_in(home, "test", now).unwrap();
    let connection = book.connections().unwrap().into_iter().find(|c| c.broker == Broker::named("wealthsimple")).unwrap().id;
    let mut ws = Wealthsimple::new(Generated::shaped(size, today, &shapes));
    let report = pull(&book, &mut ws, connection, today, now, &mut |_| {}).unwrap();
    (book, report)
}

/// Each record by its source and key: its state and latest payload.
fn records(book: &Book) -> BTreeMap<(String, String), (String, String)> {
    let c = book.conn_for_tests();
    let mut q = c
        .prepare("SELECT r.source, r.source_key, r.state, (SELECT v.payload FROM record_revisions v WHERE v.record_id = r.id ORDER BY v.revision DESC LIMIT 1) FROM source_records r")
        .unwrap();
    let rows = q.query_map([], |r| Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?)))).unwrap();
    rows.map(Result::unwrap).collect()
}

/// Each transaction, by its record's source and key and its leg, as it stands.
fn transactions(book: &Book) -> BTreeSet<String> {
    let keys: BTreeMap<bagholder_core::RecordId, (String, String)> = {
        let c = book.conn_for_tests();
        let mut q = c.prepare("SELECT id, source, source_key FROM source_records").unwrap();
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).unwrap();
        rows.map(Result::unwrap).map(|(id, s, k)| (bagholder_core::RecordId::parse(&id).unwrap(), (s, k))).collect()
    };
    book.transactions()
        .unwrap()
        .into_iter()
        .map(|t| {
            let (source, key) = &keys[&t.id.record];
            format!("{source} {key} {} {} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {} {:?} {:?}", t.id.leg, t.account, t.instrument, t.kind, t.effect, t.quantity, t.cash, t.price, t.fee, t.trade_date, t.settle_date, t.fx_rate)
        })
        .collect()
}

#[test]
fn a_book_2_10_9_read_is_the_book_2_10_8_read_once_this_version_reads_it() {
    let (eight, nine) = (book_of("read-twice-by-2.10.8"), book_of("read-by-2.10.8-then-2.10.9"));
    // what 2.10.9 wrote is in its book: fills in the feed's place, and one the feed has none of
    {
        let (book, _) = (Book::open_in(nine.path(), "test", "2026-10-25T20:00:00Z".parse().unwrap()).unwrap().0, ());
        assert!(book.first_superseded_for(RESTATED_IN_2_10_9).unwrap().is_some(), "2.10.9 took no fill in the feed's place");
        assert_ne!(records(&book), records(&Book::open_in(eight.path(), "test", "2026-10-25T20:00:00Z".parse().unwrap()).unwrap().0));
    }
    let (after_eight, _) = read(eight.path(), 20);
    let (after_nine, report) = read(nine.path(), 20);
    assert!(report.statements.withdrawn > 0, "{:?}", report.statements);
    assert!(after_nine.first_superseded_for(RESTATED_IN_2_10_9).unwrap().is_none());

    let (a, b) = (records(&after_eight), records(&after_nine));
    // the records live or superseded in either: the same, with the same latest payloads
    let standing = |m: &BTreeMap<(String, String), (String, String)>| -> BTreeMap<(String, String), (String, String)> { m.iter().filter(|(_, (state, _))| state != "removed").map(|(k, v)| (k.clone(), v.clone())).collect() };
    let (sa, sb) = (standing(&a), standing(&b));
    let only = |x: &BTreeMap<(String, String), (String, String)>, y: &BTreeMap<(String, String), (String, String)>| -> Vec<String> { x.iter().filter(|(k, v)| y.get(*k) != Some(v)).map(|(k, (s, p))| format!("{k:?} {s} {p}")).collect() };
    assert_eq!(only(&sb, &sa), Vec::<String>::new(), "standing after 2.10.9's book is read, not after 2.10.8's");
    assert_eq!(only(&sa, &sb), Vec::<String>::new(), "standing after 2.10.8's book is read, not after 2.10.9's");
    // a record removed in 2.10.8's book is removed in 2.10.9's; any other removed there is a statement's
    for (k, (state, _)) in &a {
        if state == "removed" {
            assert_eq!(b.get(k).map(|(s, _)| s.as_str()), Some("removed"), "{k:?}");
        }
    }
    for (k, (state, _)) in &b {
        if state == "removed" && !a.contains_key(k) {
            assert_eq!(k.0, "wealthsimple-statement", "{k:?}");
        }
    }
    assert_eq!(transactions(&after_nine), transactions(&after_eight));
}
