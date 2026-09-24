//! Nine companies' own dividend declarations on newswire.ca, from recorded
//! real releases (2026-09-24): each company's sentence read for its amount,
//! record date and pay date, its ex-date by the exchange's rule, and its
//! schedule where it states it.

mod common;

use std::collections::BTreeMap;

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::{Currency, Dec, InstrumentId};
use bagholder_sources::contract::Listing;
use bagholder_sources::needs::PayerNeed;
use bagholder_sources::payers::{self, companies, newswire, Distribution};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn company(t: &str) -> &'static companies::Company {
    companies::COMPANIES.iter().find(|c| c.ticker == t).unwrap()
}

fn release(slug: &str) -> newswire::Release {
    newswire::release(&common::read("newswire", &format!("release-{slug}.html"))).unwrap()
}

fn row(cash: &str, currency: Currency, record: Date, pay: Date) -> Distribution {
    Distribution { ex_date: record, record_date: Some(record), pay_date: Some(pay), cash: dec(cash), reinvested: None, currency }
}

#[test]
fn each_companys_sentence_gives_its_amount_record_and_pay_dates() {
    let cases = [
        ("BNS", "scotiabank", row("1.14", Currency::CAD, date(2026, 10, 6), date(2026, 10, 28)), None),
        ("TD", "td-bank-group", row("1.12", Currency::CAD, date(2026, 10, 9), date(2026, 10, 31)), Some(4)),
        ("BCE", "bce-inc.", row("0.4375", Currency::CAD, date(2026, 9, 15), date(2026, 10, 15)), Some(4)),
        ("ENB", "enbridge-inc", row("0.9700", Currency::CAD, date(2026, 8, 14), date(2026, 9, 1)), Some(4)),
        ("T", "telus-corporation", row("0.1875", Currency::CAD, date(2026, 9, 10), date(2026, 10, 1)), Some(4)),
        ("CP", "cpkc", row("0.268", Currency::CAD, date(2026, 9, 25), date(2026, 10, 26)), Some(4)),
        ("ATD", "alimentation-couche-tard-inc", row("0.215", Currency::CAD, date(2026, 9, 11), date(2026, 9, 25)), Some(4)),
        ("ALV", "alvopetro-energy-ltd", row("0.12", Currency::USD, date(2026, 9, 30), date(2026, 10, 15)), Some(4)),
        ("DE", "decisive-dividend-corporation", row("0.045", Currency::CAD, date(2026, 9, 30), date(2026, 10, 15)), Some(12)),
    ];
    for (ticker, slug, expected, schedule) in cases {
        let c = company(ticker);
        let r = release(slug);
        assert_eq!(companies::declared(c, &r).unwrap(), expected, "{ticker}");
        assert_eq!(companies::schedule_in(c, &r), schedule, "{ticker}");
        // the organization's page: the newest release it takes as a declaration
        // is this one, and none it takes is an announcement of something else
        let listed = newswire::listed(&common::read("newswire", &format!("organization-{slug}.html"))).unwrap();
        let taken: Vec<&str> = listed.iter().filter(|l| companies::declares(c, &l.title)).map(|l| l.title.as_str()).collect();
        assert!(!taken.is_empty(), "{ticker}");
        for t in &taken {
            let t = t.to_lowercase();
            assert!(!t.contains("to be announced") && !t.contains("to release") && !t.contains("officer") && !t.contains("voting"), "{ticker}: {t}");
        }
    }
    // Scotiabank states its schedule on its own page
    assert_eq!(companies::schedule_on_page(company("BNS"), &common::read("newswire", "scotiabank-common-share-data.html")), Some(4));
}

#[test]
fn the_ex_date_is_the_record_date_since_settlement_in_one_day() {
    assert_eq!(companies::ex_date(date(2026, 9, 15)), date(2026, 9, 15));
    // before 2024-05-27: the business day before the record date
    assert_eq!(companies::ex_date(date(2024, 5, 24)), date(2024, 5, 23));
    assert_eq!(companies::ex_date(date(2024, 4, 1)), date(2024, 3, 29));
}

#[test]
fn an_amount_in_each_companys_own_writing() {
    assert_eq!(companies::amount(" one dollar and twelve cents ($1.12) per fully paid"), Some((dec("1.12"), Currency::CAD)));
    assert_eq!(companies::amount(" CA 21.5¢ per share"), Some((dec("0.215"), Currency::CAD)));
    assert_eq!(companies::amount(" US$0.12 per common share"), Some((dec("0.12"), Currency::USD)));
    assert_eq!(companies::amount(" 629 of $1.14 per share"), Some((dec("1.14"), Currency::CAD)));
    assert_eq!(companies::long_date("Sept. 30, 2026."), Some(date(2026, 9, 30)));
    assert_eq!(companies::long_date("Smarch 1, 2026"), None);
}

#[test]
fn a_company_is_its_listing_not_its_name() {
    let id = InstrumentId::parse("0192a000-0000-7000-8000-000000000001").unwrap();
    let need = |symbol: &str, mic: &str| PayerNeed { listing: Listing { id, kind: InstrumentKind::Security, currency: Currency::CAD, symbol: symbol.into(), venue_mic: Some(mic.into()), routes: BTreeMap::new() }, name: Some("Bank of Nova Scotia".into()) };
    assert_eq!(payers::adapter_for(&need("BNS.TO", "XTSE")).map(|a| a.source().to_string()).as_deref(), Some("newswire"));
    // the same ticker on another venue is another listing
    assert!(payers::adapter_for(&need("BNS", "XNYS")).is_none());
}

#[test]
fn a_declaring_release_without_its_record_date_is_a_mismatch_naming_it() {
    let r = newswire::release(&common::read("newswire", "wrong-shape-release-bce-no-record-date.html")).unwrap();
    assert!(companies::declared(company("BCE"), &r).unwrap_err().why.contains("record date"));
}
