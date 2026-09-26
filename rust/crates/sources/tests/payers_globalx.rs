//! Global X's own releases on newswire.ca, from recorded real pages (2026-09-24).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec, InstrumentId};
use bagholder_sources::contract::Listing;
use bagholder_sources::needs::PayerNeed;
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::payers::{self, globalx, newswire, Distribution};

const NW: &str = "newswire";

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn the_organization_lists_its_releases_and_a_release_its_tables_by_heading() {
    let listed = newswire::listed(&common::read(NW, "organization-global-x.html")).unwrap();
    assert!(listed.len() >= 20);
    let distributions: Vec<_> = listed.iter().filter(|l| globalx::announces_distributions(&l.title)).collect();
    assert!(distributions.len() >= 8, "{}", distributions.len());
    assert!(distributions[0].path.ends_with("883969767.html"));
    let sep = newswire::release(&common::read(NW, "release-global-x-2026-09.html")).unwrap();
    assert_eq!(sep.at, "2026-09-22T20:00:00Z".parse().unwrap());
    let headings: Vec<&str> = sep.tables.iter().map(|(h, _)| h.as_str()).collect();
    assert_eq!(headings.iter().filter_map(|h| globalx::schedule(h)).collect::<Vec<_>>(), vec![(4, false), (4, true), (12, false), (24, false)]);
}

#[test]
fn a_funds_rows_come_with_the_schedule_of_the_table_it_is_in() {
    let sep = newswire::release(&common::read(NW, "release-global-x-2026-09.html")).unwrap();
    let agcc = globalx::rows_for(&sep, "AGCC").unwrap();
    assert_eq!(agcc, vec![(Distribution { ex_date: date(2026, 9, 29), record_date: Some(date(2026, 9, 29)), pay_date: Some(date(2026, 10, 7)), cash: dec("0.21500"), reinvested: None, currency: Currency::CAD }, 12)]);
    // a second class shares the first's dates
    let divy_u = globalx::rows_for(&sep, "DIVY.U").unwrap();
    assert_eq!(divy_u.len(), 1);
    assert_eq!((divy_u[0].0.ex_date, divy_u[0].0.currency, divy_u[0].1), (date(2026, 9, 29), Currency::USD, 4));
    // an accumulating class's distributions are paid in units
    let cash_l = globalx::rows_for(&sep, "CASH.L").unwrap();
    assert_eq!((cash_l[0].0.cash, cash_l[0].0.reinvested, cash_l[0].1), (Dec::ZERO, Some(dec("0.27300")), 4));
    // twice a month: both the month's distributions
    assert_eq!(globalx::rows_for(&sep, "BCCC").unwrap().len(), 2);
}

#[test]
fn a_read_takes_every_distribution_release_listed() {
    let listed = newswire::listed(&common::read(NW, "organization-global-x.html")).unwrap();
    let mut recorded = common::Recorded::new().with("https://www.newswire.ca/news/global-x-investments-canada-inc/", 200, NW, "organization-global-x.html");
    // the two newest releases are recorded; the rest answer with the older of them,
    // which is enough to show every listed release is read
    for (i, l) in listed.iter().filter(|l| globalx::announces_distributions(&l.title)).enumerate() {
        let name = if i == 0 { "release-global-x-2026-09.html" } else { "release-global-x-2026-08.html" };
        recorded = recorded.with(&format!("https://www.newswire.ca{}", l.path), 200, NW, name);
    }
    let recorded = Arc::new(recorded);
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let id = InstrumentId::parse("0192a000-0000-7000-8000-000000000001").unwrap();
    let need = PayerNeed { listing: Listing { id, kind: InstrumentKind::Security, currency: Currency::CAD, symbol: "AGCC".into(), venue_mic: Some("XTSE".into()), routes: BTreeMap::new() }, name: Some("Global X Silver Covered Call ETF (the “ETF”)".into()) };
    let noted = payers::adapter_for(&need).unwrap().read(&net, &need, "2026-09-24T04:00:00Z".parse().unwrap());
    let Outcome::Answered(record) = noted.outcome else { panic!("{:?}", noted.outcome) };
    assert_eq!(record.per_year, Some(12));
    assert!(record.rows.iter().any(|r| r.ex_date == date(2026, 9, 29)));
    assert!(record.rows.iter().any(|r| r.ex_date.month() == 8));
    // the record folds the repeated August release into one row each
    let checked = payers::checked(record, "2026-09-24T04:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(checked.rows.len(), 2);
    // a ticker no release lists is not carried
    let unknown = PayerNeed { name: Some("Horizons Something ETF".into()), listing: Listing { symbol: "ZZZQX".into(), ..need.listing.clone() } };
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    assert_eq!(payers::adapter_for(&unknown).unwrap().read(&net, &unknown, "2026-09-24T04:00:00Z".parse().unwrap()).outcome.kind(), OutcomeKind::NotCarried);
}
