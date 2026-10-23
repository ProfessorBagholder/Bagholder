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

use bagholder_sources::payers::harvest;

const HV: &str = "harvest";
const URL: &str = "https://harvestportfolios.com/etf/hhis/";
const PRESS_SHARES: &str = "https://harvestportfolios.com/high-income-shares/press/";
const PRESS_ETFS: &str = "https://harvestportfolios.com/press-releases/";
const SHARES_SEP: &str = "https://harvestportfolios.com/harvest-high-income-shares-etfs-announces-september-2026-distributions/";
const SHARES_OCT: &str = "https://harvestportfolios.com/harvest-high-income-shares-etfs-announces-october-2026-distributions/";
const ETFS_OCT: &str = "https://harvestportfolios.com/harvest-etfs-announces-october-2026-distributions/";

/// The High Income Shares' press page as it stood on 2026-09-24 (October's
/// release came a month later) and the September release it then linked first.
fn shares_in_september(r: common::Recorded) -> common::Recorded {
    r.with_text(PRESS_SHARES, 200, common::harvest_press_before(&["october-2026"])).with(SHARES_SEP, 200, HV, "release-income-shares-2026-09.html")
}

fn hhis() -> PayerNeed {
    need("HHIS", "XTSE", Currency::CAD, "Harvest Portfolios Group Inc. - Harvest Diversified High Income Shares ETF")
}

#[test]
fn hhis_is_stored_whole_with_its_stated_schedule() {
    let ran = run_payer(shares_in_september(common::Recorded::new().with(URL, 200, HV, "page-hhis.html")), &hhis(), HV);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    assert_eq!((source.as_str(), items.len()), ("harvest", 20));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 8, 31)), stored(date(2026, 8, 31), Some(date(2026, 8, 31)), Some(date(2026, 9, 4)), "0.2700", Currency::CAD, None));
    assert!(items.iter().all(|r| r.ex_date >= date(2025, 2, 28)));
    assert_eq!(ran.frequency(), Some((12, "harvest".to_string())));
    // the page's latest was paid 2026-09-04; the September release, out the
    // day before, announces the next, going ex 2026-09-29 and paid 2026-10-06
    assert_eq!(on(date(2026, 9, 29)), stored(date(2026, 9, 29), Some(date(2026, 9, 29)), Some(date(2026, 10, 6)), "0.2700", Currency::CAD, None));
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
    let ran = run_payer(shares_in_september(common::Recorded::new().with(URL, 200, HV, "wrong-meaning-page-hhis-paid-before-ex.html")), &hhis(), HV);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Meaning);
    assert!(detail.contains("paid 2026-08-04"), "{detail}");
    assert!(ran.wrote_nothing());
}

fn listed(symbol: &str, name: &str) -> PayerNeed {
    need(symbol, "XTSE", Currency::CAD, name)
}

#[test]
fn the_month_s_release_is_the_newest_distribution_before_the_page_lists_it() {
    // 2026-10-23: the October release is out, PLTE's page still ends at
    // September's $0.45; the release raises it to $0.50
    let r = common::Recorded::new().with("https://harvestportfolios.com/etf/plte/", 200, HV, "page-plte.html").with(PRESS_SHARES, 200, HV, "press-income-shares.html").with(SHARES_OCT, 200, HV, "release-income-shares-2026-10.html");
    let ran = run_payer(r, &listed("PLTE", "Harvest Portfolios Group Inc. - Harvest Palantir Enhanced High Income Shares ETF"), HV);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (_, items) = ran.declared().unwrap();
    let newest = items.iter().max_by_key(|r| r.ex_date).unwrap();
    assert_eq!(*newest, stored(date(2026, 10, 30), Some(date(2026, 10, 30)), Some(date(2026, 11, 6)), "0.5000", Currency::CAD, None));
    assert!(ran.asked.iter().any(|u| u == SHARES_OCT), "{:?}", ran.asked);
}

#[test]
fn an_etf_s_release_is_read_from_the_etfs_press_page() {
    let r = common::Recorded::new().with("https://harvestportfolios.com/etf/hbix/", 200, HV, "page-hbix.html").with(PRESS_ETFS, 200, HV, "press-etfs.html").with(ETFS_OCT, 200, HV, "release-etfs-2026-10.html");
    let ran = run_payer(r, &listed("HBIX", "Harvest Portfolios Group Inc. - Harvest Bitcoin Enhanced Income ETF"), HV);
    let (_, items) = ran.declared().unwrap();
    let newest = items.iter().max_by_key(|r| r.ex_date).unwrap();
    assert_eq!((newest.ex_date, newest.pay_date, newest.amount.amount), (date(2026, 10, 30), Some(date(2026, 11, 6)), dec("0.1200")));
}

#[test]
fn a_fund_the_month_s_table_does_not_name_keeps_its_page_alone() {
    // HPYB pays twice a month: the release lists it in its own table after the month's
    let r = common::Recorded::new().with("https://harvestportfolios.com/etf/hpyb/", 200, HV, "page-hpyb.html").with(PRESS_ETFS, 200, HV, "press-etfs.html").with(ETFS_OCT, 200, HV, "release-etfs-2026-10.html").with_market_record_unknown();
    let page = answered(harvest::parse_page(&common::read(HV, "page-hpyb.html"), Currency::CAD));
    let ran = run_payer(r, &listed("HPYB", "Harvest Portfolios Group Inc. - Harvest Premium Yield Canadian Bank ETF"), HV);
    let (_, items) = ran.declared().unwrap();
    assert_eq!(items.iter().map(|r| r.ex_date).max(), page.rows.iter().map(|r| r.ex_date).max());
}

#[test]
fn a_release_without_its_days_is_a_mismatch_writing_nothing() {
    let html = common::read(HV, "release-income-shares-2026-10.html").replace("ex-dividend date of", "ex date");
    let Outcome::Mismatch(m) = harvest::announced(&html, "PLTE", Currency::CAD) else { panic!() };
    assert_eq!(m.path, "release");
    assert!(m.why.contains("are not days"), "{m:?}");
}

#[test]
fn the_newest_release_is_the_latest_month_a_press_page_links() {
    let page = common::read(HV, "press-income-shares.html");
    assert_eq!(harvest::newest_release(&page, "harvest-high-income-shares-etfs-announces-").as_deref(), Some(SHARES_OCT));
    let before = common::harvest_press_before(&["october-2026"]);
    assert_eq!(harvest::newest_release(&before, "harvest-high-income-shares-etfs-announces-").as_deref(), Some(SHARES_SEP));
}
