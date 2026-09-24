//! BMO's recorded replies, each read through a run of the payers' read: what is
//! written for a real reply, and for each wrong copy that nothing is and which
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

use bagholder_sources::payers::bmo;

const BMO: &str = "bmo";
const URL: &str = "https://df.bmogam.com/api/graphql/etf-funds-production";

fn zea() -> PayerNeed {
    need("ZEA", "XTSE", Currency::CAD, "BMO Asset Management Inc. - BMO MSCI EAFE Index ETF")
}

fn with(name: &str) -> common::Recorded {
    common::Recorded::new().with_body(URL, "\"entityId\":\"ZEA-a\"", 200, BMO, name)
}

#[test]
fn zea_is_stored_whole_with_its_stated_schedule_and_its_parts() {
    let ran = run_payer(with("fund-ZEA.json"), &zea(), BMO);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    assert_eq!((source.as_str(), items.len()), ("bmo", 50));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 6, 29)), stored(date(2026, 6, 29), Some(date(2026, 6, 29)), Some(date(2026, 7, 3)), "0.156", Currency::CAD, None));
    // a year-end distribution with a reinvested part: both parts as stated
    assert_eq!(on(date(2025, 12, 30)), stored(date(2025, 12, 30), Some(date(2025, 12, 30)), Some(date(2026, 1, 5)), "0.15", Currency::CAD, Some("0.12")));
    assert_eq!(on(date(2014, 3, 26)), stored(date(2014, 3, 26), Some(date(2014, 3, 28)), Some(date(2014, 4, 4)), "0.09", Currency::CAD, None));
    assert_eq!(ran.frequency(), Some((4, "bmo".to_string())));
}

#[test]
fn zfl_states_a_monthly_schedule_and_every_distribution() {
    let zfl = answered(bmo::parse(&common::json(BMO, "fund-ZFL.json"), "ZFL"));
    assert_eq!((zfl.per_year, zfl.rows.len()), (Some(12), 190));
    assert_eq!(zfl.rows[0], Distribution { ex_date: date(2026, 8, 28), record_date: Some(date(2026, 8, 28)), pay_date: Some(date(2026, 9, 2)), cash: dec("0.028"), reinvested: None, currency: Currency::CAD });
    assert_eq!(zfl.rows[189], Distribution { ex_date: date(2010, 9, 27), record_date: Some(date(2010, 9, 29)), pay_date: Some(date(2010, 10, 7)), cash: dec("0.214169"), reinvested: None, currency: Currency::CAD });
}

#[test]
fn an_entity_the_service_does_not_know_is_not_carried_and_writes_nothing() {
    let n = need("ZZZQX", "XTSE", Currency::CAD, "BMO Asset Management Inc. - BMO Nothing ETF");
    let ran = run_payer(common::Recorded::new().with_body(URL, "\"entityId\":\"ZZZQX-a\"", 200, BMO, "fund-ZZZQX.json"), &n, BMO);
    assert_eq!(ran.outcome(), (OutcomeKind::NotCarried, "BMO's service does not know ZZZQX".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_schedule_word_bmo_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let v = common::json(BMO, "wrong-shape-fund-ZEA-schedule-unknown.json");
    let Outcome::Mismatch(m) = bmo::parse(&v, "ZEA") else { panic!() };
    assert_eq!(m.path, "data.webProfiles[0].webProfileSalesOptions[0].salesOption.series");
    assert!(m.why.contains("\"Fortnightly\""), "{m:?}");
    let ran = run_payer(with("wrong-shape-fund-ZEA-schedule-unknown.json"), &zea(), BMO);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Mismatch);
    assert!(detail.contains("Fortnightly"), "{detail}");
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_without_its_ex_date_is_a_mismatch_naming_it_writing_nothing() {
    let v = common::json(BMO, "wrong-shape-fund-ZEA-ex-date-missing.json");
    let Outcome::Mismatch(m) = bmo::parse(&v, "ZEA") else { panic!() };
    assert_eq!((m.path.as_str(), m.why.as_str()), ("data.monthlyDistributionBreakdowns[0].exDate", "absent"));
    let ran = run_payer(with("wrong-shape-fund-ZEA-ex-date-missing.json"), &zea(), BMO);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "data.monthlyDistributionBreakdowns[0].exDate: absent".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn parts_that_are_not_the_total_are_a_meaning_failure_writing_nothing() {
    let ran = run_payer(with("wrong-meaning-fund-ZEA-parts-not-the-total.json"), &zea(), BMO);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Meaning);
    assert!(detail.contains("totals"), "{detail}");
    assert!(ran.wrote_nothing());
}
