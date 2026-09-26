//! iShares' US recorded replies, each read through a run of the payers' read:
//! what is written for a real page, and for each wrong copy that nothing is and
//! which outcome is recorded.

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

use bagholder_sources::payers::ishares_us;

const IS: &str = "ishares-us";
const SCREENER: &str = "https://www.ishares.com/us/product-screener/product-screener-v3.jsn?dcrPath=/templatedata/config/product-screener-v3/data/en/us-ishares/ishares-product-screener-backend-config&siteEntryPassthrough=true";
const PAGE: &str = "https://www.ishares.com/us/products/244049/ishares-core-msci-eafe-etf";

fn iefa() -> PayerNeed {
    need("IEFA", "BATS", Currency::USD, "iShares Trust - iShares Core MSCI EAFE ETF")
}

fn replies(page: &str) -> common::Recorded {
    common::Recorded::new().with(SCREENER, 200, IS, "screener-trimmed.json").with(PAGE, 200, IS, page)
}

fn usd(ex: Date, record: Date, pay: Date, cash: &str) -> Distribution {
    Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash: dec(cash), reinvested: None, currency: Currency::USD }
}

#[test]
fn iefa_is_stored_whole_with_its_stated_schedule() {
    let ran = run_payer(replies("page-iefa.html"), &iefa(), IS);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    assert_eq!((source.as_str(), items.len()), ("ishares-us", 29));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 6, 15)), stored(date(2026, 6, 15), Some(date(2026, 6, 15)), Some(date(2026, 6, 18)), "1.57806", Currency::USD, None));
    assert_eq!(on(date(2012, 12, 18)), stored(date(2012, 12, 18), Some(date(2012, 12, 20)), Some(date(2012, 12, 27)), "0.170617", Currency::USD, None));
    assert_eq!(ran.frequency(), Some((2, "ishares-us".to_string())));
}

#[test]
fn eemv_states_every_distribution_from_its_first() {
    let page = common::read(IS, "page-eemv.html");
    assert_eq!(ishares_us::schedule_word(&page).as_deref(), Some("Semi-Annual"));
    let rows = ishares_us::parse_component(&ishares_us::component(&page).unwrap()).unwrap();
    assert_eq!(rows.len(), 30);
    assert_eq!(rows[0], usd(date(2026, 6, 15), date(2026, 6, 15), date(2026, 6, 18), "0.647284"));
    assert_eq!(rows[29], usd(date(2011, 12, 20), date(2011, 12, 22), date(2011, 12, 29), "0.71317"));
}

#[test]
fn a_fund_the_screener_does_not_list_is_not_carried() {
    let n = need("ZZZQX", "BATS", Currency::USD, "iShares Trust - iShares Nothing ETF");
    let ran = run_payer(common::Recorded::new().with(SCREENER, 200, IS, "screener-trimmed.json").with_market_record_unknown(), &n, IS);
    assert_eq!(ran.outcome(), (OutcomeKind::NotCarried, "iShares lists no US fund ZZZQX".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_schedule_word_ishares_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let page = common::read(IS, "wrong-shape-page-iefa-schedule-unknown.html");
    assert_eq!(ishares_us::schedule_word(&page).as_deref(), Some("Fortnightly"));
    assert_eq!(ishares_us::per_year("Fortnightly"), None);
    let ran = run_payer(replies("wrong-shape-page-iefa-schedule-unknown.html"), &iefa(), IS);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "Distribution Frequency: \"Fortnightly\" is not a schedule this reader knows".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_component_without_its_ex_dates_is_a_mismatch_naming_them_writing_nothing() {
    let m = ishares_us::parse_component(&ishares_us::component(&common::read(IS, "wrong-shape-page-iefa-ex-date-missing.html")).unwrap()).unwrap_err();
    assert_eq!((m.path.as_str(), m.why.as_str()), ("", "no column exDate"));
    let ran = run_payer(replies("wrong-shape-page-iefa-ex-date-missing.html"), &iefa(), IS);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "the reply: no column exDate".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_paid_before_it_goes_ex_is_a_meaning_failure_writing_nothing() {
    let ran = run_payer(replies("wrong-meaning-page-iefa-paid-before-ex.html"), &iefa(), IS);
    assert_eq!(ran.outcome(), (OutcomeKind::Meaning, "the distribution going ex 2026-06-15 is paid 2026-06-10".to_string()));
    assert!(ran.wrote_nothing());
}
