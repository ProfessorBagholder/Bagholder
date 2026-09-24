//! Harvest's recorded pages, each read through a run of the payers' read: what
//! is written for a real page, and for each wrong copy that nothing is and which
//! outcome is recorded.

#![allow(unused_imports, dead_code)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_book::facts::DeclaredRow;
use bagholder_book::Book;
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::cache::{MarketCache, OutcomeRow};
use bagholder_sources::contract::{DataKind, Listing};
use bagholder_sources::needs::PayerNeed;
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::payers::{self, run, Distribution, Record};
use bagholder_sources::read::Ctx;

const AT: &str = "2026-09-24T04:00:00Z";

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn id() -> InstrumentId {
    InstrumentId::parse("0192a000-0000-7000-8000-000000000001").unwrap()
}

fn need(symbol: &str, mic: &str, currency: Currency, name: &str) -> PayerNeed {
    PayerNeed { listing: Listing { id: id(), kind: InstrumentKind::Security, currency, symbol: symbol.into(), venue_mic: Some(mic.into()), routes: BTreeMap::new() }, name: Some(name.into()) }
}

fn stored(ex: Date, record: Option<Date>, pay: Option<Date>, cash: &str, currency: Currency, reinvested: Option<&str>) -> DeclaredRow {
    DeclaredRow { ex_date: ex, record_date: record, pay_date: pay, amount: Money::new(dec(cash), currency), reinvested: reinvested.map(dec) }
}

/// What one run of the payers' read left behind: the book, the market cache's
/// outcome rows for `source`, and the requests made.
struct Ran {
    _dir: tempfile::TempDir,
    book: Book,
    outcomes: Vec<OutcomeRow>,
    asked: Vec<String>,
}

impl Ran {
    fn declared(&self) -> Option<(String, Vec<DeclaredRow>)> {
        self.book.declared().unwrap().remove(&id()).map(|r| (r.source.to_string(), r.items))
    }

    fn frequency(&self) -> Option<(u32, String)> {
        self.book.frequencies().unwrap().remove(&id()).map(|f| (f.per_year, f.source.to_string()))
    }

    /// The one outcome recorded, its kind and detail.
    fn outcome(&self) -> (OutcomeKind, String) {
        assert_eq!(self.outcomes.len(), 1, "{:?}", self.outcomes);
        let o = &self.outcomes[0];
        assert_eq!((o.kind, o.instrument), (DataKind::Distributions, Some(id())));
        (o.outcome, o.detail.clone())
    }

    /// Nothing written to the book for the payer.
    fn wrote_nothing(&self) -> bool {
        self.declared().is_none() && self.frequency().is_none()
    }
}

fn run_payer(recorded: common::Recorded, need: &PayerNeed, source: &'static str) -> Ran {
    let dir = tempfile::tempdir().unwrap();
    let at = t(AT);
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    common::instrument_in_book(&dir.path().join("book.db"), id(), "security", need.listing.currency.as_str());
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(recorded);
    let net = common::net(&recorded, AT);
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    assert_eq!(payers::adapter_for(need).map(|a| a.source().to_string()).as_deref(), Some(source));
    run::read(&ctx, std::slice::from_ref(need)).unwrap();
    let outcomes = cache.outcomes(&SourceName::named(source)).unwrap();
    let asked = recorded.asked.lock().unwrap().clone();
    Ran { _dir: dir, book, outcomes, asked }
}

fn answered<A: std::fmt::Debug>(o: Outcome<A>) -> A {
    match o {
        Outcome::Answered(a) => a,
        other => panic!("{other:?}"),
    }
}

use bagholder_sources::payers::harvest;

const HV: &str = "harvest";
const URL: &str = "https://harvestportfolios.com/etf/hhis/";

fn hhis() -> PayerNeed {
    need("HHIS", "XTSE", Currency::CAD, "Harvest Portfolios Group Inc. - Harvest Diversified High Income Shares ETF")
}

#[test]
fn hhis_is_stored_whole_with_its_stated_schedule() {
    let ran = run_payer(common::Recorded::new().with(URL, 200, HV, "page-hhis.html"), &hhis(), HV);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    assert_eq!((source.as_str(), items.len()), ("harvest", 19));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 8, 31)), stored(date(2026, 8, 31), Some(date(2026, 8, 31)), Some(date(2026, 9, 4)), "0.2700", Currency::CAD, None));
    assert!(items.iter().all(|r| r.ex_date >= date(2025, 2, 28)));
    assert_eq!(ran.frequency(), Some((12, "harvest".to_string())));
}

#[test]
fn a_schedule_word_harvest_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let Outcome::Mismatch(m) = harvest::parse_page(&common::read(HV, "wrong-shape-page-hhis-schedule-unknown.html"), Currency::CAD) else { panic!() };
    assert_eq!(m.path, "Distribution");
    assert!(m.why.contains("\"Every Full Moon\""), "{m:?}");
    let ran = run_payer(common::Recorded::new().with(URL, 200, HV, "wrong-shape-page-hhis-schedule-unknown.html"), &hhis(), HV);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Mismatch);
    assert!(detail.contains("Every Full Moon"), "{detail}");
    assert!(ran.wrote_nothing());
}

#[test]
fn a_row_without_its_amount_is_a_mismatch_naming_it_writing_nothing() {
    let Outcome::Mismatch(m) = harvest::parse_page(&common::read(HV, "wrong-shape-page-hhis-amount-missing.html"), Currency::CAD) else { panic!() };
    assert_eq!((m.path.as_str(), m.why.as_str()), ("table 0 row 1", "5 cells"));
    let ran = run_payer(common::Recorded::new().with(URL, 200, HV, "wrong-shape-page-hhis-amount-missing.html"), &hhis(), HV);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "table 0 row 1: 5 cells".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_paid_before_it_goes_ex_is_a_meaning_failure_writing_nothing() {
    let ran = run_payer(common::Recorded::new().with(URL, 200, HV, "wrong-meaning-page-hhis-paid-before-ex.html"), &hhis(), HV);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Meaning);
    assert!(detail.contains("paid 2026-08-04"), "{detail}");
    assert!(ran.wrote_nothing());
}
