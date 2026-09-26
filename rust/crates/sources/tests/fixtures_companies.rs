//! A company's declarations read through a run of the payers' read: one from
//! before T+1 is dated on the exchange's sessions, read from XIC's closes, and
//! nothing is stored while those sessions cannot be read.

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
use bagholder_sources::cache::MarketCache;
use bagholder_sources::contract::{Benchmark, DataKind, Listing};
use bagholder_sources::needs::PayerNeed;
use bagholder_sources::outcome::OutcomeKind;
use bagholder_sources::payers::run;
use bagholder_sources::read::Ctx;

const AT: &str = "2026-09-24T04:00:00Z";
const NEWSWIRE: &str = "https://www.newswire.ca";
const YAHOO: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";

fn id() -> InstrumentId {
    InstrumentId::parse("0192a000-0000-7000-8000-000000000001").unwrap()
}

fn bce() -> PayerNeed {
    PayerNeed { listing: Listing { id: id(), kind: InstrumentKind::Security, currency: Currency::CAD, symbol: "BCE".into(), venue_mic: Some("XTSE".into()), routes: BTreeMap::new() }, name: Some("BCE Inc.".into()) }
}

fn stored(ex: Date, record: Date, pay: Date) -> DeclaredRow {
    DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: ex, record_date: Some(record), pay_date: Some(pay), amount: Money::new(Dec::parse("0.4375").unwrap(), Currency::CAD), reinvested: None }
}

/// BCE's organization page, its second quarter's release as captured, and its
/// first quarter's edited to declare a dividend of record 2023-10-10.
fn bce_pages() -> common::Recorded {
    common::Recorded::new()
        .with(&format!("{NEWSWIRE}/news/bce-inc./"), 200, "newswire", "organization-bce-inc..html")
        .with(&format!("{NEWSWIRE}/news-releases/bce-reports-second-quarter-2026-results-831758874.html"), 200, "newswire", "release-bce-inc..html")
        .with(&format!("{NEWSWIRE}/news-releases/bce-reports-first-quarter-2026-results-811877023.html"), 200, "newswire", "edited-release-bce-record-2023-10-10.html")
}

/// XIC's closes over the fortnight before that record date, under the URLs the
/// reader asks.
const XIC_DAYS: &str = "XIC.TO?period1=1695686400&period2=1696982400&interval=1d&events=div%7Csplit";
const XIC_SPLITS: &str = "XIC.TO?period1=1696982400&period2=1790294400&interval=1mo&events=split";

struct Ran {
    _dir: tempfile::TempDir,
    book: Book,
    cache: MarketCache,
    asked: Vec<String>,
}

fn run_bce(recorded: common::Recorded) -> Ran {
    let dir = tempfile::tempdir().unwrap();
    let at: Timestamp = AT.parse().unwrap();
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    common::instrument_in_book(&dir.path().join("book.db"), id(), "security", "CAD");
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(recorded);
    let net = common::net(&recorded, AT);
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    run::read(&ctx, &[bce()]).unwrap();
    let asked = recorded.asked.lock().unwrap().clone();
    Ran { _dir: dir, book, cache, asked }
}

#[test]
fn a_declaration_before_t_plus_one_goes_ex_a_session_before_its_record_date() {
    let ran = run_bce(bce_pages().with(&format!("{YAHOO}{XIC_SPLITS}"), 200, "yahoo", "XIC.TO-splits-2023-10-11-2026-09-24.json").with(&format!("{YAHOO}{XIC_DAYS}"), 200, "yahoo", "XIC.TO-2023-09-26-2023-10-10.json"));
    let read = ran.book.declared().unwrap().remove(&id()).expect("stored");
    assert_eq!(read.source.to_string(), "newswire");
    // 2023-10-09 was Thanksgiving: the session before the 10th is the 6th
    assert_eq!(read.items, vec![stored(date(2023, 10, 6), date(2023, 10, 10), date(2023, 11, 1)), stored(date(2026, 9, 15), date(2026, 9, 15), date(2026, 10, 15))]);
    // the sessions read are kept as XIC's closes, the holiday not among them
    let days = ran.cache.benchmark_days(Benchmark::Tsx).unwrap();
    assert!(days.contains(&date(2023, 10, 6)) && days.contains(&date(2023, 10, 10)) && !days.contains(&date(2023, 10, 9)), "{days:?}");
    assert!(ran.asked.iter().any(|u| u.ends_with(XIC_DAYS)));
}

#[test]
fn nothing_is_stored_while_the_exchanges_sessions_cannot_be_read() {
    let ran = run_bce(bce_pages().with(&format!("{YAHOO}{XIC_SPLITS}"), 500, "yahoo", "XIC.TO-splits-2023-10-11-2026-09-24.json"));
    assert!(ran.book.declared().unwrap().remove(&id()).is_none());
    assert!(ran.book.frequencies().unwrap().remove(&id()).is_none());
    // the company's own answer is recorded as what it was; the payer stays due
    let outcomes = ran.cache.outcomes(&SourceName::named("newswire")).unwrap();
    assert_eq!(outcomes.iter().map(|o| (o.kind, o.outcome)).collect::<Vec<_>>(), vec![(DataKind::Distributions, OutcomeKind::Answered)]);
    assert!(ran.cache.reads(&id().to_string(), DataKind::Distributions).unwrap().is_empty());
}
