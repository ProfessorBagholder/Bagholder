//! Fidelity Canada's recorded replies, each read through a run of the payers'
//! read: what is written for a real reply, and for each wrong copy that nothing
//! is and which outcome is recorded. Fidelity's history service lists no
//! ex-dates, so no distribution is ever read from it; there is no meaning check
//! a copy could trip (nothing it states is checked against anything else).

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

use bagholder_sources::payers::fidelity;

const FD: &str = "fidelity-canada";
const DIR: &str = "fidelity";
const FUNDS: &str = "https://www.fidelity.ca/content/fidelity-data/pnp-cached-en.json";
const HISTORY: &str = "https://fidcaapi.fidelity.ca/FidcaAPI/api/fund/distributions/history/EN/";

fn series(symbol: &str) -> PayerNeed {
    need(symbol, "XTSE", Currency::CAD, "Fidelity Investments Canada ULC - Fidelity Fund")
}

fn replies(funds: &str, symbol: &str, history: &str) -> common::Recorded {
    common::Recorded::new().with(FUNDS, 200, DIR, funds).with(&format!("{HISTORY}{symbol}"), 200, DIR, history)
}

#[test]
fn a_series_that_has_paid_nothing_is_stored_empty_with_its_schedule() {
    let ran = run_payer(replies("funds-trimmed.json", "FBTC", "history-FBTC.json"), &series("FBTC"), FD);
    assert_eq!(ran.outcome().0, OutcomeKind::Answered);
    assert_eq!(ran.declared(), Some(("fidelity-canada".to_string(), vec![])));
    assert_eq!(ran.frequency(), Some((1, "fidelity-canada".to_string())));
}

#[test]
fn a_history_with_no_ex_dates_is_a_mismatch_writing_nothing() {
    let ran = run_payer(replies("funds-trimmed.json", "FCAB", "history-FCAB.json"), &series("FCAB"), FD);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "distributions: FCAB's distributions carry one date each and no ex-date".to_string()));
    assert!(ran.wrote_nothing());
}

#[test]
fn a_series_fidelity_does_not_list_is_not_carried() {
    let ran = run_payer(common::Recorded::new().with(FUNDS, 200, DIR, "funds-trimmed.json"), &series("ZZZQX"), FD);
    assert_eq!(ran.outcome(), (OutcomeKind::NotCarried, "Fidelity lists no series ZZZQX".to_string()));
    assert!(ran.wrote_nothing());
    assert_eq!(ran.asked, vec![FUNDS.to_string()]);
}

#[test]
fn a_schedule_word_fidelity_does_not_know_is_a_mismatch_naming_it_writing_nothing() {
    let m = fidelity::schedule_of(&common::json(DIR, "wrong-shape-funds-FCAB-schedule-unknown.json"), "FCAB").unwrap_err();
    assert_eq!(m.path, "funds[0].series[0]");
    assert!(m.why.contains("\"Fortnightly\""), "{m:?}");
    // FBTC's series is untouched in the copy
    assert_eq!(fidelity::schedule_of(&common::json(DIR, "wrong-shape-funds-FCAB-schedule-unknown.json"), "FBTC").unwrap(), Some(Some(1)));
    let ran = run_payer(common::Recorded::new().with(FUNDS, 200, DIR, "wrong-shape-funds-FCAB-schedule-unknown.json"), &series("FCAB"), FD);
    let (kind, detail) = ran.outcome();
    assert_eq!(kind, OutcomeKind::Mismatch);
    assert!(detail.contains("Fortnightly"), "{detail}");
    assert!(ran.wrote_nothing());
}

#[test]
fn a_series_without_its_schedule_is_a_mismatch_naming_it_writing_nothing() {
    let m = fidelity::schedule_of(&common::json(DIR, "wrong-shape-funds-FCAB-schedule-missing.json"), "FCAB").unwrap_err();
    assert_eq!((m.path.as_str(), m.why.as_str()), ("funds[0].series[0].distribution_frequency", "absent"));
    let ran = run_payer(common::Recorded::new().with(FUNDS, 200, DIR, "wrong-shape-funds-FCAB-schedule-missing.json"), &series("FCAB"), FD);
    assert_eq!(ran.outcome(), (OutcomeKind::Mismatch, "funds[0].series[0].distribution_frequency: absent".to_string()));
    assert!(ran.wrote_nothing());
}
