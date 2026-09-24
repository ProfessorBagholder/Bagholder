//! YieldMax's recorded replies, each read through a run of the payers' read:
//! what is written, and for each wrong copy (and for the real page, which
//! carries a record date ten years out) that nothing is and which outcome is
//! recorded.

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

use bagholder_sources::payers::us_pages;

const YM: &str = "yieldmax";
const ETF: &str = "https://yieldmaxetfs.com/wp-json/wp/v2/etf?slug=msty";
const TERMS: &str = "https://yieldmaxetfs.com/wp-json/wp/v2/distribution-frequency";
const PAGE: &str = "https://yieldmaxetfs.com/our-etfs/msty/";

fn msty() -> PayerNeed {
    need("MSTY", "ARCX", Currency::USD, "Tidal Trust II - Yieldmax MSTR Option Income Strategy ETF")
}

fn replies(terms: &str, page: &str) -> common::Recorded {
    common::Recorded::new().with(ETF, 200, YM, "etf-msty.json").with(TERMS, 200, YM, terms).with(PAGE, 200, YM, page)
}

#[test]
fn msty_page_lists_every_row_as_paid_at_the_time() {
    let rows = answered(us_pages::yieldmax_page(&common::read(YM, "page-msty.html")));
    assert_eq!(rows.len(), 96);
    assert_eq!(rows[95], Distribution { ex_date: date(2024, 4, 4), record_date: Some(date(2024, 4, 5)), pay_date: Some(date(2024, 4, 8)), cash: dec("4.1286"), reinvested: None, currency: Currency::USD });
    // the row with a record date ten years past its ex-date, as written
    assert!(rows.contains(&Distribution { ex_date: date(2026, 7, 30), record_date: Some(date(2036, 7, 30)), pay_date: Some(date(2026, 7, 31)), cash: dec("0.2222"), reinvested: None, currency: Currency::USD }));
}

#[test]
fn msty_real_page_is_refused_as_a_whole_for_its_record_date_ten_years_out() {
    let ran = run_payer(replies("distribution-frequency.json", "page-msty.html"), &msty(), YM);
    assert_eq!(ran.outcome(), (OutcomeKind::Meaning, "the distribution going ex 2026-07-30 is on record 2036-07-30".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_fund_the_sites_data_does_not_list_is_not_carried() {
    let n = need("ZZZQX", "ARCX", Currency::USD, "Tidal Trust II - Yieldmax Nothing ETF");
    let recorded = common::Recorded::new().with("https://yieldmaxetfs.com/wp-json/wp/v2/etf?slug=zzzqx", 200, YM, "etf-zzzqx.json").with(TERMS, 200, YM, "distribution-frequency.json");
    let ran = run_payer(recorded, &n, YM);
    assert_eq!(ran.outcome(), (OutcomeKind::NotCarried, "YieldMax lists no fund zzzqx".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_schedule_word_yieldmax_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let m = us_pages::yieldmax_schedule(&common::json(YM, "etf-msty.json"), &common::json(YM, "wrong-shape-distribution-frequency-weekly-renamed.json")).unwrap_err();
    assert!(m.why.contains("\"Fortnightly\""), "{m:?}");
    let recorded = common::Recorded::new().with(ETF, 200, YM, "etf-msty.json").with(TERMS, 200, YM, "wrong-shape-distribution-frequency-weekly-renamed.json");
    let ran = run_payer(recorded, &msty(), YM);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Mismatch);
    assert!(detail.contains("Fortnightly"), "{detail}");
    assert!(ran.wrote_nothing());
}

#[test]
fn a_row_without_its_ex_date_is_a_mismatch_naming_it_writing_nothing() {
    let Outcome::Mismatch(m) = us_pages::yieldmax_page(&common::read(YM, "wrong-shape-page-msty-ex-date-missing.html")) else { panic!() };
    assert_eq!((m.path.as_str(), m.why.as_str()), ("table row 1", "5 cells, not 6"));
    let ran = run_payer(replies("distribution-frequency.json", "wrong-shape-page-msty-ex-date-missing.html"), &msty(), YM);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "table row 1: 5 cells, not 6".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn two_amounts_for_one_ex_date_are_a_meaning_failure_writing_nothing() {
    let rows = answered(us_pages::yieldmax_page(&common::read(YM, "wrong-meaning-page-msty-two-amounts-one-ex-date.html")));
    let err = payers::checked(Record { rows, per_year: Some(52), by_record: vec![] }, t(AT)).unwrap_err();
    assert_eq!(err, "two different distributions go ex 2024-05-06: 2.5239 and 2.5293");
    let ran = run_payer(replies("distribution-frequency.json", "wrong-meaning-page-msty-two-amounts-one-ex-date.html"), &msty(), YM);
    assert_eq!(ran.outcome(), (OutcomeKind::Meaning, err));
    assert!(ran.wrote_nothing());
}
