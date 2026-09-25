//! Payers' own publications, read from recorded real replies, checked, and
//! stored; the exchange-side record for the two companies whose publication
//! cannot be read; and when a payer is read.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_book::facts::{DeclaredReadRow, DeclaredRow, StatedFrequency};
use bagholder_book::Book;
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::cache::MarketCache;
use bagholder_sources::contract::{DataKind, Listing};
use bagholder_sources::needs::PayerNeed;
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::payers::{self, ninepoint, run, Distribution, Record};
use bagholder_sources::read::Ctx;

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn eastern() -> TimeZone {
    TimeZone::get("America/Toronto").unwrap()
}

fn id(n: u8) -> InstrumentId {
    InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap()
}

fn payer(n: u8, symbol: &str, mic: &str, currency: Currency, name: &str) -> PayerNeed {
    PayerNeed { listing: Listing { id: id(n), kind: InstrumentKind::Security, currency, symbol: symbol.into(), venue_mic: Some(mic.into()), routes: BTreeMap::new() }, name: Some(name.into()) }
}

const NP: &str = "ninepoint";

#[test]
fn a_payer_has_its_companys_reader_else_its_markets_record() {
    let source = |p: &PayerNeed| payers::adapter_for(p).map(|a| a.source().to_string());
    // its company's reader, by the brand its name carries
    assert_eq!(source(&payer(1, "CCHI", "XTSE", Currency::CAD, "Ninepoint Partners LP - Cameco Highshares ETF")).as_deref(), Some("ninepoint"));
    // no company reader serves it: its market's record, whatever it is called
    for (symbol, mic, currency, want) in [("ZZZ", "XTSE", Currency::CAD, "tmx"), ("ZZZ", "XTSX", Currency::CAD, "tmx"), ("ZZZ", "XNAS", Currency::USD, "yahoo"), ("ZZZ", "BATS", Currency::USD, "yahoo")] {
        let p = payer(2, symbol, mic, currency, "An Issuer No Reader Knows - Income Fund");
        assert_eq!(source(&p).as_deref(), Some(want), "{mic}");
        assert!(run::unread(std::slice::from_ref(&p)).is_empty());
    }
    // a listing on a venue no record covers waits on it, named
    let unknown = payer(4, "ZZZ", "XXXX", Currency::CAD, "An Issuer No Reader Knows - Income Fund");
    assert_eq!(source(&unknown), None);
    assert_eq!(run::unread(std::slice::from_ref(&unknown)).len(), 1);
}

#[test]
fn a_ninepoint_page_states_its_schedule_and_every_distribution() {
    let v = common::json(NP, "funds-trimmed.json");
    assert_eq!(ninepoint::page_of(&v, "CCHI").unwrap().as_deref(), Some("/funds/ninepoint-cameco-highshares-etf/"));
    assert_eq!(ninepoint::page_of(&v, "ZZZQX").unwrap(), None);
    let Outcome::Answered(cchi) = ninepoint::parse_page(&common::read(NP, "page-cchi.html")) else { panic!() };
    assert_eq!(cchi.per_year, Some(24));
    assert_eq!(cchi.rows.len(), 14);
    assert_eq!(cchi.rows[0], Distribution { ex_date: date(2026, 9, 15), record_date: Some(date(2026, 9, 15)), pay_date: Some(date(2026, 9, 21)), cash: dec("0.13500"), reinvested: None, currency: Currency::CAD });
    let Outcome::Answered(sxhi) = ninepoint::parse_page(&common::read(NP, "page-sxhi.html")) else { panic!() };
    assert_eq!(sxhi.rows.len(), 4);
    // a money market fund's page shows only its latest payment, with no ex-date: a mismatch, named
    assert!(matches!(ninepoint::parse_page(&common::read(NP, "page-nsav-seriesETF.html")), Outcome::Mismatch(m) if m.why.contains("no table")));
    assert!(matches!(ninepoint::parse_page(&common::read(NP, "wrong-shape-page-cchi-schedule-unknown.html")), Outcome::Mismatch(m) if m.why.contains("Fortnightly")));
    assert!(matches!(ninepoint::parse_page(&common::read(NP, "wrong-shape-page-cchi-amount-not-a-number.html")), Outcome::Mismatch(m) if m.why.contains("thirteen")));
    // two different rows for one ex-date: read, then refused as a whole before anything is written
    let Outcome::Answered(two) = ninepoint::parse_page(&common::read(NP, "wrong-meaning-page-cchi-two-rows-one-ex-date.html")) else { panic!() };
    assert!(payers::checked(two, t("2026-09-24T04:00:00Z")).unwrap_err().contains("2026-09-15"));
}

fn row(ex: Date, cash: &str) -> Distribution {
    Distribution { ex_date: ex, record_date: Some(ex), pay_date: ex.tomorrow().ok(), cash: dec(cash), reinvested: None, currency: Currency::CAD }
}

#[test]
fn a_record_is_checked_before_anything_is_written() {
    let now = t("2026-09-24T04:00:00Z");
    // an identical repeat is one row
    let r = payers::checked(Record { rows: vec![row(date(2026, 9, 15), "0.1"), row(date(2026, 9, 15), "0.1")], per_year: Some(12), by_record: vec![] }, now).unwrap();
    assert_eq!(r.rows.len(), 1);
    // a record date ten years ahead (a typo in the publication) is past the fund's life
    let mut far = row(date(2036, 7, 30), "0.1");
    far.record_date = Some(date(2036, 7, 30));
    assert!(payers::checked(Record { rows: vec![far], per_year: None, by_record: vec![] }, now).unwrap_err().contains("past the fund's life"));
    // paid before it goes ex
    let mut early = row(date(2026, 9, 15), "0.1");
    early.pay_date = Some(date(2026, 9, 1));
    assert!(payers::checked(Record { rows: vec![early], per_year: None, by_record: vec![] }, now).is_err());
}

fn read(at: &str, items: &[Date]) -> DeclaredReadRow {
    DeclaredReadRow {
        read_at: t(at),
        source: SourceName::named("ninepoint"),
        items: items.iter().map(|d| DeclaredRow { ex_date: *d, record_date: None, pay_date: None, amount: Money::new(dec("0.1"), Currency::CAD), reinvested: None }).collect(),
    }
}

fn freq(n: u32) -> StatedFrequency {
    StatedFrequency { per_year: n, source: SourceName::named("ninepoint"), stated_at: None }
}

#[test]
fn a_payer_is_read_when_it_first_appears_and_when_its_next_distribution_is_due() {
    let z = eastern();
    assert!(run::due(None, None, t("2026-09-24T12:00:00Z"), &z));
    // monthly, latest ex 2026-09-15: the next about 2026-10-15, looked for from 2026-10-08
    let monthly = read("2026-09-20T12:00:00Z", &[date(2026, 8, 15), date(2026, 9, 15)]);
    assert!(!run::due(Some(&monthly), Some(&freq(12)), t("2026-10-07T12:00:00Z"), &z));
    assert!(run::due(Some(&monthly), Some(&freq(12)), t("2026-10-08T12:00:00Z"), &z));
    // read today already: not again today
    let today = read("2026-10-08T11:00:00Z", &[date(2026, 9, 15)]);
    assert!(!run::due(Some(&today), Some(&freq(12)), t("2026-10-08T20:00:00Z"), &z));
    // a distribution declared ahead moves the window with it
    let ahead = read("2026-10-08T11:00:00Z", &[date(2026, 9, 15), date(2026, 10, 15)]);
    assert!(!run::due(Some(&ahead), Some(&freq(12)), t("2026-10-20T12:00:00Z"), &z));
    // no schedule stated: once a week
    let unstated = read("2026-09-20T12:00:00Z", &[]);
    assert!(!run::due(Some(&unstated), None, t("2026-09-26T12:00:00Z"), &z));
    assert!(run::due(Some(&unstated), None, t("2026-09-27T12:00:00Z"), &z));
}

const TMX: &str = "https://app-money.tmx.com/graphql";

#[test]
fn a_run_stores_each_payers_record_under_its_source_and_its_schedule_where_stated() {
    // a payer its company's reader serves, and two no company reader serves (their
    // names carry no company any reader knows): those have the market's record
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    for (n, c) in [(1, "CAD"), (2, "CAD"), (3, "USD")] {
        common::instrument_in_book(&dir.path().join("book.db"), id(n), "security", c);
    }
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://www.ninepoint.com/api/funds/getallfundswithpriceandperformance/en?showAllSeries=true", 200, NP, "funds-trimmed.json")
            .with("https://www.ninepoint.com/funds/ninepoint-cameco-highshares-etf/?series=ETF", 200, NP, "page-cchi.html")
            .with_body(TMX, "getQuoteBySymbol", 200, "tmx", "quote-QCN.json")
            .with_body(TMX, "getDividendsForSymbol", 200, "tmx", "dividends-QCN.json")
            .with("https://query1.finance.yahoo.com/v8/finance/chart/WQTM?period1=946857600&period2=1790294400&interval=1d&events=div%7Csplit", 200, "yahoo", "WQTM-2025-10-01-2026-09-24.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let needs = [
        payer(1, "CCHI", "XTSE", Currency::CAD, "Ninepoint Partners LP - Cameco Highshares ETF"),
        payer(2, "QCN", "XTSE", Currency::CAD, "An Issuer No Reader Knows - Canadian Equity Index ETF"),
        payer(3, "WQTM", "BATS", Currency::USD, "Another Issuer No Reader Knows - Quantum Computing Fund"),
    ];
    run::read(&ctx, &needs).unwrap();
    let declared = book.declared().unwrap();
    let freq = book.frequencies().unwrap();
    // Ninepoint's own page, its schedule its own word
    assert_eq!(declared[&id(1)].source.as_str(), "ninepoint");
    assert_eq!(declared[&id(1)].items.len(), 14);
    assert_eq!((freq[&id(1)].per_year, freq[&id(1)].source.as_str()), (24, "ninepoint"));
    // a Canadian listing no company reader serves: the exchange's record and the
    // schedule it states, marked as TMX's
    assert_eq!(declared[&id(2)].source.as_str(), "tmx");
    let units = declared[&id(2)].items.iter().find(|r| r.ex_date == date(2023, 12, 28)).unwrap();
    assert_eq!((units.amount.amount, units.reinvested), (Dec::ZERO, Some(dec("0.35705"))));
    assert_eq!((freq[&id(2)].per_year, freq[&id(2)].source.as_str()), (4, "tmx"));
    // a US listing no company reader serves: Yahoo's record (nothing paid yet), and
    // no schedule, since Yahoo states none
    assert_eq!(declared[&id(3)].source.as_str(), "yahoo");
    assert!(declared[&id(3)].items.is_empty());
    assert!(!freq.contains_key(&id(3)));
    for s in ["ninepoint", "tmx", "yahoo"] {
        let outcomes = cache.outcomes(&SourceName::named(s)).unwrap();
        assert!(outcomes.iter().any(|o| o.kind == DataKind::Distributions && o.outcome == OutcomeKind::Answered), "{s}");
    }
    // the same moment again: nothing is due
    let asked = recorded.asked.lock().unwrap().len();
    run::read(&ctx, &needs).unwrap();
    assert_eq!(recorded.asked.lock().unwrap().len(), asked);
}

#[test]
fn a_payer_its_companys_publication_does_not_carry_has_the_markets_record() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    common::instrument_in_book(&dir.path().join("book.db"), id(1), "security", "CAD");
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://www.ninepoint.com/api/funds/getallfundswithpriceandperformance/en?showAllSeries=true", 200, NP, "funds-trimmed.json")
            .with_body(TMX, "getQuoteBySymbol", 200, "tmx", "quote-QCN.json")
            .with_body(TMX, "getDividendsForSymbol", 200, "tmx", "dividends-QCN.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    // the name carries a company a reader knows, whose own list does not hold the ticker
    run::read(&ctx, &[payer(1, "QCN", "XTSE", Currency::CAD, "Ninepoint Partners LP - A Fund Its List Does Not Hold")]).unwrap();
    let company = cache.outcomes(&SourceName::named("ninepoint")).unwrap();
    assert_eq!(company.iter().map(|o| o.outcome).collect::<Vec<_>>(), vec![OutcomeKind::NotCarried]);
    assert_eq!(book.declared().unwrap()[&id(1)].source.as_str(), "tmx");
    assert_eq!(book.frequencies().unwrap()[&id(1)].per_year, 4);
}

#[test]
fn a_payers_failed_read_is_recorded_as_it_failed_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    common::instrument_in_book(&dir.path().join("book.db"), id(1), "security", "CAD");
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(common::Recorded::new().with("https://harvestportfolios.com/etf/hhis/", 500, "harvest", "page-hhis.html"));
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    run::read(&ctx, &[payer(1, "HHIS", "XTSE", Currency::CAD, "Harvest Portfolios Group Inc. - Harvest Diversified High Income Shares ETF")]).unwrap();
    assert!(book.declared().unwrap().is_empty());
    let outcomes = cache.outcomes(&SourceName::named("harvest")).unwrap();
    assert_eq!(outcomes.len(), 1);
    // a company reader that failed is not passed over for the market's record
    assert_eq!(recorded.asked.lock().unwrap().len(), 1);
    assert_eq!(outcomes[0].outcome, OutcomeKind::Unreachable);
    assert!(outcomes[0].detail.contains("status 500"), "{}", outcomes[0].detail);
    // Harvest answers 403 to a request without a User-Agent
    assert!(recorded.headers.lock().unwrap()[0].iter().any(|(k, v)| k == "User-Agent" && v.starts_with("Bagholder/")));
    // the failure waits out its source's rest (a minute for a host with no pace
    // of its own), then the payer is asked again
    let need = [payer(1, "HHIS", "XTSE", Currency::CAD, "Harvest Portfolios Group Inc. - Harvest Diversified High Income Shares ETF")];
    run::read(&Ctx { now: t("2026-09-24T04:00:30Z"), ..ctx }, &need).unwrap();
    assert_eq!(recorded.asked.lock().unwrap().len(), 1);
    run::read(&Ctx { now: t("2026-09-24T04:01:00Z"), ..ctx }, &need).unwrap();
    assert_eq!(recorded.asked.lock().unwrap().len(), 2);
}

#[test]
fn a_cash_row_and_a_units_row_for_one_ex_date_are_one_distribution() {
    let ex = date(2026, 9, 29);
    let cash = Distribution { ex_date: ex, record_date: Some(ex), pay_date: Some(date(2026, 10, 7)), cash: dec("0.10"), reinvested: None, currency: Currency::CAD };
    let units = Distribution { cash: Dec::ZERO, reinvested: Some(dec("0.25")), ..cash };
    let r = payers::checked(Record { rows: vec![units, cash], per_year: Some(4), by_record: vec![] }, t("2026-10-01T00:00:00Z")).unwrap();
    assert_eq!(r.rows, vec![Distribution { reinvested: Some(dec("0.25")), ..cash }]);
    // two different cash amounts for one ex-date are still a conflict
    let other = Distribution { cash: dec("0.11"), ..cash };
    assert!(payers::checked(Record { rows: vec![cash, other], per_year: Some(4), by_record: vec![] }, t("2026-10-01T00:00:00Z")).unwrap_err().contains("two different"));
}

#[test]
fn a_payer_is_next_due_at_the_first_instant_its_rule_holds() {
    // every kind of record, in every zone: the instant `next_due` names is the
    // first at which `due` holds, the second before it not
    let reads = [
        read("2026-09-20T12:00:00Z", &[]),
        read("2026-09-20T12:00:00Z", &[date(2026, 9, 15)]),
        read("2026-10-08T11:00:00Z", &[date(2026, 9, 15), date(2026, 10, 15)]),
    ];
    let schedules = [None, Some(freq(12)), Some(freq(4)), Some(freq(52)), Some(freq(1))];
    let db = bagholder_core::jiff::tz::TimeZoneDatabase::bundled();
    let now = t("2026-09-21T00:00:00Z");
    for name in db.available() {
        let z = db.get(name.as_str()).unwrap();
        for r in &reads {
            for f in &schedules {
                let at = run::next_due(Some(r), f.as_ref(), now, &z);
                assert!(run::due(Some(r), f.as_ref(), at, &z), "{} {:?} {:?}: due at {at}", name.as_str(), r.read_at, f.as_ref().map(|f| f.per_year));
                let before = at - bagholder_core::jiff::SignedDuration::from_secs(1);
                assert!(!run::due(Some(r), f.as_ref(), before, &z), "{} {:?} {:?}: already due at {before}", name.as_str(), r.read_at, f.as_ref().map(|f| f.per_year));
            }
        }
    }
    // never read: due now
    assert_eq!(run::next_due(None, None, now, &eastern()), now);
}
