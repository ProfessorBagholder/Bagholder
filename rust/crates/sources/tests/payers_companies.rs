//! Nine companies' own dividend declarations on newswire.ca, from recorded
//! real releases (2026-09-24): each company's sentence read for its amount,
//! record date and pay date, its ex-date by the exchange's rule, and its
//! schedule where it states it.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::{date, Date, Weekday};
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
        assert_eq!(companies::declared(c, &r).map(|d| d.with_ex(d.record_date)).unwrap(), expected, "{ticker}");
        assert_eq!(companies::schedule_in(c, &r), schedule, "{ticker}");
        // the organization's page: the newest release it takes as a declaration
        // is this one, and none it takes is an announcement of something else
        let listed = newswire::listed(&common::read("newswire", &format!("organization-{slug}.html"))).unwrap();
        let taken: Vec<&str> = listed.iter().filter(|l| companies::declares(c, &l.title)).map(|l| l.title.as_str()).collect();
        assert!(!taken.is_empty(), "{ticker}");
        for t in &taken {
            let t = t.to_lowercase();
            assert!(!t.contains("to be announced") && !t.contains("to release") && !t.contains("officer") && !t.contains("voting") && !t.contains("preferred share conversions"), "{ticker}: {t}");
        }
    }
    // Scotiabank states its schedule on its own page
    assert_eq!(companies::schedule_on_page(company("BNS"), &common::read("newswire", "scotiabank-common-share-data.html")), Some(4));
}

#[test]
fn the_ex_date_is_the_exchanges_rule_counted_on_its_sessions() {
    // the TSX's sessions: weekdays less its holidays (2017-09-04 Labour Day,
    // 2023-10-09 Thanksgiving, 2024-05-20 Victoria Day)
    let holidays = [date(2017, 9, 4), date(2023, 10, 9), date(2024, 5, 20)];
    let mut sessions = BTreeSet::new();
    for (from, to) in [(date(2017, 8, 21), date(2017, 9, 15)), (date(2023, 9, 25), date(2023, 10, 20)), (date(2024, 5, 10), date(2024, 6, 7))] {
        let mut d = from;
        while d <= to {
            if !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) && !holidays.contains(&d) {
                sessions.insert(d);
            }
            d = d.tomorrow().unwrap();
        }
    }
    let ex = |y, m, d| companies::ex_date(date(y, m, d), &sessions);
    // T+1: the record date itself, from record date 2024-05-28 (TMX's notice)
    assert_eq!(ex(2026, 9, 15), Some(date(2026, 9, 15)));
    assert_eq!(ex(2024, 5, 28), Some(date(2024, 5, 28)));
    // record 2024-05-27 went ex on 2024-05-24, 2024-05-24 on 2024-05-23 (the notice)
    assert_eq!(ex(2024, 5, 27), Some(date(2024, 5, 24)));
    assert_eq!(ex(2024, 5, 24), Some(date(2024, 5, 23)));
    // one session before, over a holiday: 2023-10-10's was 2023-10-06
    assert_eq!(ex(2023, 10, 10), Some(date(2023, 10, 6)));
    // T+2 from record date 2017-09-07 (TSX notice 2017-018), two sessions before until then
    assert_eq!(ex(2017, 9, 7), Some(date(2017, 9, 6)));
    assert_eq!(ex(2017, 9, 6), Some(date(2017, 9, 1)));
    assert_eq!(ex(2017, 9, 5), Some(date(2017, 8, 31)));
    // no sessions held before it: none
    assert_eq!(ex(2010, 3, 15), None);
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
    // the same ticker on another venue is another listing, which no company
    // reader serves: its market's record
    assert_eq!(payers::adapter_for(&need("BNS", "XNYS")).map(|a| a.source().to_string()).as_deref(), Some("yahoo"));
}

#[test]
fn a_declaring_release_without_its_record_date_is_a_mismatch_naming_it() {
    let r = newswire::release(&common::read("newswire", "wrong-shape-release-bce-no-record-date.html")).unwrap();
    assert!(companies::declared(company("BCE"), &r).unwrap_err().why.contains("record date"));
}

#[test]
fn a_company_whose_statement_of_its_schedule_is_gone_is_a_mismatch_naming_it() {
    let td = company("TD");
    let mut r = release("td-bank-group");
    assert_eq!(companies::schedule_in(td, &r), Some(4));
    // reworded: the phrase it states its schedule with is gone
    r.body = r.body.replace("declared for the quarter ending", "declared for the period ending");
    assert_eq!(companies::schedule_in(td, &r), None);
    assert!(companies::schedule_missing(td).why.contains("declared for the quarter ending"));
    // Scotiabank states it on its own page
    assert_eq!(companies::schedule_on_page(company("BNS"), "<p>nothing about it</p>"), None);
    assert!(companies::schedule_missing(company("BNS")).why.contains("no longer states"));
}
