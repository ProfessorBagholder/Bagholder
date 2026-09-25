//! Goldman Sachs's recorded replies, each read through a run of the payers'
//! read: what is written for a real reply, and for each wrong copy that nothing
//! is and which outcome is recorded.

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

use bagholder_sources::payers::goldman;

const GS: &str = "goldman-sachs";
const URL: &str = "https://am.gs.com/services/funds";

fn gswo() -> PayerNeed {
    need("GSWO", "BATS", Currency::USD, "Goldman Sachs ETF Trust - Goldman Sachs MarketBeta Total International Equity ETF")
}

fn replies(funds: &str, fund: &str) -> common::Recorded {
    common::Recorded::new().with_body(URL, "\"operationName\":\"Funds\"", 200, GS, funds).with_body(URL, "\"operationName\":\"Fund\"", 200, GS, fund)
}

#[test]
fn gswo_is_stored_whole_its_repeat_once_with_its_stated_schedule() {
    let ran = run_payer(replies("funds-trimmed.json", "fund-GSWO.json"), &gswo(), GS);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    // eighteen rows with an amount, one of them repeated identically
    assert_eq!((source.as_str(), items.len()), ("goldman-sachs", 17));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 6, 24)), stored(date(2026, 6, 24), Some(date(2026, 6, 24)), Some(date(2026, 6, 30)), "0.3169", Currency::USD, None));
    assert_eq!(on(date(2025, 12, 23)), stored(date(2025, 12, 23), Some(date(2025, 12, 23)), Some(date(2025, 12, 30)), "0.2261", Currency::USD, None));
    assert_eq!(on(date(2022, 6, 24)), stored(date(2022, 6, 24), Some(date(2022, 6, 27)), Some(date(2022, 6, 30)), "0.3054", Currency::USD, None));
    // a date stated with no amount is no distribution
    assert!(items.iter().all(|r| r.ex_date != date(2025, 12, 31)));
    assert_eq!(ran.frequency(), Some((4, "goldman-sachs".to_string())));
}

#[test]
fn a_class_goldman_does_not_list_is_not_carried() {
    let n = need("GLOV", "BATS", Currency::USD, "Goldman Sachs ETF Trust - Goldman Sachs ActiveBeta World Low Vol Plus Equity ETF");
    let ran = run_payer(common::Recorded::new().with_body(URL, "\"operationName\":\"Funds\"", 200, GS, "funds-trimmed.json").with_market_record_unknown(), &n, GS);
    assert_eq!(ran.outcome(), (OutcomeKind::NotCarried, "Goldman Sachs lists no class GLOV".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_schedule_word_goldman_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let m = goldman::class_of(&common::json(GS, "wrong-shape-funds-GSWO-schedule-unknown.json"), "GSWO").unwrap_err();
    assert_eq!(m.path, "data.fundData.funds[1].shareClasses[0]");
    assert!(m.why.contains("\"Fortnightly\""), "{m:?}");
    let ran = run_payer(common::Recorded::new().with_body(URL, "\"operationName\":\"Funds\"", 200, GS, "wrong-shape-funds-GSWO-schedule-unknown.json"), &gswo(), GS);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Mismatch);
    assert!(detail.contains("Fortnightly"), "{detail}");
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_without_its_ex_date_is_a_mismatch_naming_it_writing_nothing() {
    let m = goldman::parse_detail(&common::json(GS, "wrong-shape-fund-GSWO-ex-date-missing.json"), "GSWO", Currency::USD).unwrap_err();
    assert_eq!((m.path.as_str(), m.why.as_str()), ("data.fundsDetail.distributions[0].expirationDate", "absent"));
    let ran = run_payer(replies("funds-trimmed.json", "wrong-shape-fund-GSWO-ex-date-missing.json"), &gswo(), GS);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "data.fundsDetail.distributions[0].expirationDate: absent".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_paid_before_it_goes_ex_is_a_meaning_failure_writing_nothing() {
    let ran = run_payer(replies("funds-trimmed.json", "wrong-meaning-fund-GSWO-paid-before-ex.json"), &gswo(), GS);
    assert_eq!(ran.outcome(), (OutcomeKind::Meaning, "the distribution going ex 2026-06-24 is paid 2026-06-20".to_string()));
    assert!(ran.wrote_nothing());
}
