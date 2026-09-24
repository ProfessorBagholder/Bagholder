//! Recorded replies of several sources whose other replies are asserted
//! elsewhere: the wrong-meaning copies of Evolve, Hamilton, iShares Canada, the
//! companies' releases, Purpose and Vanguard Canada, each refused before
//! anything is written; the whole histories read exactly; and the replies an
//! unknown symbol gets (Cboe Canada's, Coinbase's and Yahoo's 404s), each not
//! carried and nothing kept.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, SourceName};
use bagholder_sources::adapters::yahoo;
use bagholder_sources::cache::MarketCache;
use bagholder_sources::contract::{DataKind, Listing};
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::payers::{self, companies, evolve, hamilton, ishares_ca, newswire, purpose, vanguard_ca, Distribution, Record};
use bagholder_sources::quotes;
use bagholder_sources::read::Ctx;

const AT: &str = "2026-09-24T04:00:00Z";

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn answered<A: std::fmt::Debug>(o: Outcome<A>) -> A {
    match o {
        Outcome::Answered(a) => a,
        other => panic!("{other:?}"),
    }
}

fn cad(ex: bagholder_core::jiff::civil::Date, record: bagholder_core::jiff::civil::Date, pay: bagholder_core::jiff::civil::Date, cash: &str, reinvested: Option<&str>) -> Distribution {
    Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash: dec(cash), reinvested: reinvested.map(dec), currency: Currency::CAD }
}

#[test]
fn evolve_a_distribution_paid_before_it_goes_ex_is_refused_as_a_whole() {
    let real = answered(evolve::parse_page(&common::read("evolve", "page-easy.html"), Currency::CAD));
    assert_eq!(payers::checked(real, t(AT)).unwrap().rows.len(), 14);
    let wrong = answered(evolve::parse_page(&common::read("evolve", "wrong-meaning-page-easy-paid-before-ex.html"), Currency::CAD));
    assert_eq!(payers::checked(wrong, t(AT)).unwrap_err(), "the distribution going ex 2026-03-31 is paid 2026-03-08");
}

#[test]
fn hamilton_two_amounts_for_one_ex_date_are_refused_as_a_whole() {
    let real = answered(hamilton::parse_page(&common::read("hamilton", "page-hmax.html"), Currency::CAD));
    assert_eq!(payers::checked(real, t(AT)).unwrap().rows.len(), 43);
    let wrong = answered(hamilton::parse_page(&common::read("hamilton", "wrong-meaning-page-hmax-two-amounts-one-ex-date.html"), Currency::CAD));
    assert_eq!(wrong.rows.len(), 43);
    assert_eq!(payers::checked(wrong, t(AT)).unwrap_err(), "two different distributions go ex 2026-08-31: 0.168 and 0.169");
}

#[test]
fn ishares_canada_parts_that_are_not_the_total_are_a_meaning_failure() {
    let v = ishares_ca::json(common::read("ishares-canada", "wrong-meaning-history-309480-parts-not-the-total.json").as_bytes()).unwrap();
    assert_eq!(ishares_ca::parse_history(&v), Outcome::Meaning("the distribution going ex 2026-09-24 totals 0.122, its parts 0.102 and 0".to_string()));
}

#[test]
fn ishares_canada_histories_are_read_whole_to_their_first_distribution() {
    let xiu = answered(ishares_ca::parse_history(&ishares_ca::json(common::read("ishares-canada", "history-239832.json").as_bytes()).unwrap()));
    assert_eq!(xiu.len(), 118);
    assert_eq!(xiu[0], cad(date(2026, 8, 26), date(2026, 8, 26), date(2026, 8, 31), "0.28700", None));
    assert_eq!(xiu[117], cad(date(1999, 12, 30), date(1999, 12, 30), date(1999, 12, 31), "0.01617", Some("0.01636")));
    let xeqt = answered(ishares_ca::parse_history(&ishares_ca::json(common::read("ishares-canada", "history-309480.json").as_bytes()).unwrap()));
    assert_eq!(xeqt.len(), 29);
    assert_eq!(xeqt[28], cad(date(2019, 9, 24), date(2019, 9, 25), date(2019, 9, 30), "0.08200", None));
}

#[test]
fn a_companys_release_paying_before_its_record_date_is_refused_as_a_whole() {
    let bce = companies::COMPANIES.iter().find(|c| c.ticker == "BCE").unwrap();
    let r = newswire::release(&common::read("newswire", "wrong-meaning-release-bce-paid-before-record.html")).unwrap();
    let row = companies::declared(bce, &r).unwrap().unwrap();
    assert_eq!(row, cad(date(2026, 9, 15), date(2026, 9, 15), date(2026, 9, 1), "0.4375", None));
    assert_eq!(payers::checked(Record { rows: vec![row], per_year: Some(4) }, t(AT)).unwrap_err(), "the distribution going ex 2026-09-15 is paid 2026-09-01");
}

#[test]
fn purpose_a_distribution_paid_before_it_goes_ex_is_refused_as_a_whole() {
    let real = answered(purpose::parse(&purpose::page_data(&common::read("purpose", "page-psa.html")).unwrap(), "PSA"));
    assert_eq!(payers::checked(real, t(AT)).unwrap().rows.len(), 155);
    let wrong = answered(purpose::parse(&purpose::page_data(&common::read("purpose", "wrong-meaning-page-psa-paid-before-ex.html")).unwrap(), "PSA"));
    assert_eq!(payers::checked(wrong, t(AT)).unwrap_err(), "the distribution going ex 2026-08-27 is paid 2026-08-20");
}

#[test]
fn vanguard_canada_amounts_in_two_currencies_are_a_meaning_failure() {
    let v = common::json("vanguard-canada", "wrong-meaning-distributions-9692-VEQT-two-currencies.json");
    assert_eq!(vanguard_ca::parse_distributions(&v, "9692"), Outcome::Meaning("9692 2025-12-30: its amounts are in two currencies".to_string()));
}

#[test]
fn vanguard_canada_vfv_is_read_whole_with_its_reinvested_parts_as_stated() {
    let vfv = answered(vanguard_ca::parse_distributions(&common::json("vanguard-canada", "distributions-9563-VFV.json"), "9563"));
    assert_eq!(vfv.len(), 55);
    assert_eq!(vfv[0], cad(date(2026, 6, 26), date(2026, 6, 26), date(2026, 7, 6), "0.39537", None));
    assert_eq!(vfv[54], cad(date(2012, 12, 27), date(2012, 12, 31), date(2013, 1, 4), "0.149444327", None));
    // the year-end capital gain, stated reinvested at zero, is kept as stated
    let year_end = vfv.iter().find(|r| r.ex_date == date(2025, 12, 30)).unwrap();
    assert_eq!(*year_end, cad(date(2025, 12, 30), date(2025, 12, 30), date(2026, 1, 7), "0.39293", Some("0")));
    assert_eq!(vfv.iter().filter(|r| r.reinvested.is_some()).count(), 11);
}

fn id(n: u8) -> InstrumentId {
    InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap()
}

#[test]
fn an_unknown_symbol_on_cboe_canada_and_on_coinbase_is_not_carried_and_nothing_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let at = t(AT);
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    common::instrument_in_book(&dir.path().join("book.db"), id(1), "security", "CAD");
    common::instrument_in_book(&dir.path().join("book.db"), id(2), "crypto", "CAD");
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://www-api.cboe.com/ca/equities/securities-1/ZZZQX/quote/", 404, "cboe-canada", "quote-ZZZQX-status-404.html")
            .with("https://api.coinbase.com/v2/prices/ZZZQX-CAD/spot", 404, "coinbase", "spot-ZZZQX-CAD-status-404.json"),
    );
    let net = common::net(&recorded, AT);
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let listings = [
        Listing { id: id(1), kind: InstrumentKind::Security, currency: Currency::CAD, symbol: "ZZZQX".into(), venue_mic: Some("NEOE".into()), routes: BTreeMap::new() },
        Listing { id: id(2), kind: InstrumentKind::Crypto, currency: Currency::CAD, symbol: "ZZZQX".into(), venue_mic: None, routes: BTreeMap::new() },
    ];
    quotes::read_quotes(&ctx, &listings).unwrap();
    assert!(cache.quotes().unwrap().is_empty());
    for (source, n, detail) in [("cboe-canada", 1, "ZZZQX: status 404"), ("coinbase", 2, "ZZZQX-CAD: status 404")] {
        let rows = cache.outcomes(&SourceName::named(source)).unwrap();
        assert_eq!(rows.len(), 1, "{source}");
        assert_eq!((rows[0].kind, rows[0].instrument, rows[0].outcome, rows[0].detail.as_str()), (DataKind::Quote, Some(id(n)), OutcomeKind::NotCarried, detail), "{source}");
    }
    assert_eq!(recorded.asked.lock().unwrap().len(), 2);
}

#[test]
fn yahoo_answers_404_for_a_form_it_does_not_list_and_nothing_is_read() {
    let recorded = Arc::new(common::Recorded::new().with("https://query1.finance.yahoo.com/v8/finance/chart/ONE.TO?range=1d&interval=1d", 404, "yahoo", "ONE.TO-status-404.json"));
    let net = common::net(&recorded, AT);
    let noted = yahoo::ask_quote(&net, "ONE.TO", t(AT));
    assert_eq!(noted.outcome.kind(), OutcomeKind::NotCarried);
    assert_eq!(noted.outcome.detail(), "status 404");
    assert_eq!(noted.shape_change, None);
}
