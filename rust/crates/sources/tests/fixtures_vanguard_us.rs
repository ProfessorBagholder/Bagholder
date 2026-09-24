//! Vanguard's US recorded replies, each read through a run of the payers' read:
//! what is written for a real reply, and for each wrong copy or refusal that
//! nothing is and which outcome is recorded.

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

use bagholder_sources::payers::vanguard_us;

const VG: &str = "vanguard-us";
const API: &str = "https://advisors.vanguard.com/investments/products/api/funds/";

fn vti() -> PayerNeed {
    need("VTI", "ARCX", Currency::USD, "Vanguard Index Funds - Vanguard Total Stock Market ETF")
}

fn replies(profile: &str, distributions: &str) -> common::Recorded {
    common::Recorded::new()
        .with(&format!("{API}vti/validate"), 200, VG, "validate-VTI.json")
        .with(&format!("{API}0970/profile"), 200, VG, profile)
        .with(&format!("{API}0970/pricing/distributions?hasDistributionYield=true"), 200, VG, distributions)
}

#[test]
fn vti_is_stored_whole_in_dollars_with_its_stated_schedule() {
    let ran = run_payer(replies("profile-0970.json", "distributions-0970.json"), &vti(), VG);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    let (source, items) = ran.declared().unwrap();
    assert_eq!((source.as_str(), items.len()), ("vanguard-us", 39));
    let on = |d: Date| items.iter().find(|r| r.ex_date == d).cloned().unwrap();
    assert_eq!(on(date(2026, 6, 26)), stored(date(2026, 6, 26), Some(date(2026, 6, 26)), Some(date(2026, 6, 30)), "1.0437", Currency::USD, None));
    assert_eq!(on(date(2016, 12, 20)), stored(date(2016, 12, 20), Some(date(2016, 12, 22)), Some(date(2016, 12, 27)), "0.727", Currency::USD, None));
    assert!(items.iter().all(|r| r.ex_date >= date(2016, 12, 20)));
    assert_eq!(ran.frequency(), Some((4, "vanguard-us".to_string())));
}

#[test]
fn a_ticker_vanguard_answers_404_for_is_not_carried_and_writes_nothing() {
    let n = need("ZZZQX", "ARCX", Currency::USD, "Vanguard Index Funds - Vanguard Nothing ETF");
    let ran = run_payer(common::Recorded::new().with(&format!("{API}zzzqx/validate"), 404, VG, "validate-ZZZQX-status-404.html"), &n, VG);
    assert_eq!(ran.outcome(), (OutcomeKind::NotCarried, "status 404".to_string()));
    assert!(ran.wrote_nothing());
    assert_eq!(ran.asked, vec![format!("{API}zzzqx/validate")]);
}

#[test]
fn a_schedule_word_vanguard_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let m = vanguard_us::schedule(&common::json(VG, "wrong-shape-profile-0970-schedule-unknown.json")).unwrap_err();
    assert_eq!(m.path, "");
    assert!(m.why.contains("\"Fortnightly\""), "{m:?}");
    let recorded = common::Recorded::new()
        .with(&format!("{API}vti/validate"), 200, VG, "validate-VTI.json")
        .with(&format!("{API}0970/profile"), 200, VG, "wrong-shape-profile-0970-schedule-unknown.json");
    let ran = run_payer(recorded, &vti(), VG);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "the reply: \"Fortnightly\" is not a schedule this reader knows".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_without_its_ex_date_is_a_mismatch_naming_it_writing_nothing() {
    let m = vanguard_us::parse_distributions(&common::json(VG, "wrong-shape-distributions-0970-ex-date-missing.json")).unwrap_err();
    assert_eq!((m.path.as_str(), m.why.as_str()), ("[0].exDividendDate", "absent"));
    let ran = run_payer(replies("profile-0970.json", "wrong-shape-distributions-0970-ex-date-missing.json"), &vti(), VG);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "[0].exDividendDate: absent".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_distribution_paid_before_it_goes_ex_is_a_meaning_failure_writing_nothing() {
    let ran = run_payer(replies("profile-0970.json", "wrong-meaning-distributions-0970-paid-before-ex.json"), &vti(), VG);
    assert_eq!(ran.outcome(), (OutcomeKind::Meaning, "the distribution going ex 2026-06-26 is paid 2026-06-20".to_string()));
    assert!(ran.wrote_nothing());
}
