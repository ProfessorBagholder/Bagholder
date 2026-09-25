//! Defiance's recorded page, read through a run of the payers' read: what is
//! written for the real page, and for each wrong copy that nothing is and which
//! outcome is recorded. Defiance's page states no schedule (the company states
//! it only in a PDF this reader does not read), so no schedule word can be
//! unknown to it and none is stored.

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
    DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: ex, record_date: record, pay_date: pay, amount: Money::new(dec(cash), currency), reinvested: reinvested.map(dec) }
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

use bagholder_sources::payers::us_pages;

const DF: &str = "defiance";
const PAGE: &str = "https://www.defianceetfs.com/qtum/";

fn qtum() -> PayerNeed {
    need("QTUM", "XNAS", Currency::USD, "ETF Series Solutions - Defiance Quantum ETF")
}

fn usd(ex: Date, record: Date, pay: Date, cash: &str) -> Distribution {
    Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash: dec(cash), reinvested: None, currency: Currency::USD }
}

#[test]
fn qtum_lists_every_distribution_and_a_scheduled_date_is_none() {
    let rows = answered(us_pages::defiance_page(&common::read(DF, "page-qtum.html")));
    assert_eq!(rows.len(), 27);
    assert_eq!(rows[0], usd(date(2026, 9, 23), date(2026, 9, 23), date(2026, 9, 24), "0.464627200"));
    assert_eq!(rows[26], usd(date(2019, 6, 24), date(2019, 6, 25), date(2019, 6, 26), "0.08246653"));
    // the record date a day before its ex-date, as written
    assert!(rows.contains(&usd(date(2022, 6, 22), date(2022, 6, 21), date(2022, 6, 24), "0.19326214")));
}

#[test]
fn qtum_is_stored_whole_with_no_schedule() {
    let ran = run_payer(common::Recorded::new().with(PAGE, 200, DF, "page-qtum.html"), &qtum(), DF);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    assert_eq!((source.as_str(), items.len()), ("defiance", 27));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 9, 23)), stored(date(2026, 9, 23), Some(date(2026, 9, 23)), Some(date(2026, 9, 24)), "0.464627200", Currency::USD, None));
    assert!(items.iter().all(|r| r.ex_date != date(2026, 12, 23)));
    assert_eq!(ran.frequency(), None);
}

#[test]
fn a_row_without_its_record_date_is_a_mismatch_naming_it_writing_nothing() {
    let Outcome::Mismatch(m) = us_pages::defiance_page(&common::read(DF, "wrong-shape-page-qtum-record-date-missing.html")) else { panic!() };
    assert_eq!((m.path.as_str(), m.why.as_str()), ("table row 2", "4 cells, not 5"));
    let ran = run_payer(common::Recorded::new().with(PAGE, 200, DF, "wrong-shape-page-qtum-record-date-missing.html"), &qtum(), DF);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "table row 2: 4 cells, not 5".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_paid_before_it_goes_ex_is_a_meaning_failure_writing_nothing() {
    let ran = run_payer(common::Recorded::new().with(PAGE, 200, DF, "wrong-meaning-page-qtum-paid-before-ex.html"), &qtum(), DF);
    assert_eq!(ran.outcome(), (OutcomeKind::Meaning, "the distribution going ex 2026-09-23 is paid 2026-09-20".to_string()));
    assert!(ran.wrote_nothing());
}
