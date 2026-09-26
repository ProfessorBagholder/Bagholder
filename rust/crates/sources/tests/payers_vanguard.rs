//! Vanguard Canada's own record, from recorded real replies (2026-09-24).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec, InstrumentId};
use bagholder_sources::contract::Listing;
use bagholder_sources::needs::PayerNeed;
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::payers::{self, vanguard_ca, Distribution};

const VG: &str = "vanguard-canada";

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn need(symbol: &str, mic: &str, name: &str) -> PayerNeed {
    let id = InstrumentId::parse("0192a000-0000-7000-8000-000000000001").unwrap();
    PayerNeed { listing: Listing { id, kind: InstrumentKind::Security, currency: Currency::CAD, symbol: symbol.into(), venue_mic: Some(mic.into()), routes: BTreeMap::new() }, name: Some(name.into()) }
}

#[test]
fn the_product_list_names_the_funds_and_the_service_states_ticker_and_schedule() {
    let ids = vanguard_ca::port_ids(&common::read(VG, "product-list.html")).unwrap();
    assert_eq!(ids.len(), 54);
    assert!(ids.contains(&"9692".to_string()));
    let listings = common::json(VG, "listings.json");
    assert_eq!(vanguard_ca::fund_of(&listings, "VEQT").unwrap(), Some(("9692".to_string(), Some(1))));
    assert_eq!(vanguard_ca::fund_of(&listings, "VFV").unwrap(), Some(("9563".to_string(), Some(4))));
    assert_eq!(vanguard_ca::fund_of(&listings, "ZZZQX").unwrap(), None);
    // a fund listed with no schedule: a change of what Vanguard publishes
    let m = vanguard_ca::fund_of(&common::json(VG, "wrong-shape-listings-VEQT-schedule-null.json"), "VEQT").unwrap_err();
    assert!(m.path.ends_with("fundDistributionFrequency"), "{m:?}");
}

#[test]
fn each_distribution_states_its_cash_and_its_reinvested_part() {
    let Outcome::Answered(veqt) = vanguard_ca::parse_distributions(&common::json(VG, "distributions-9692-VEQT.json"), "9692") else { panic!() };
    assert_eq!(veqt.len(), 7);
    assert_eq!(
        veqt[0],
        Distribution { ex_date: date(2025, 12, 30), record_date: Some(date(2025, 12, 30)), pay_date: Some(date(2026, 1, 7)), cash: dec("0.76018"), reinvested: Some(dec("0.24252")), currency: Currency::CAD }
    );
    assert!(matches!(vanguard_ca::parse_distributions(&common::json(VG, "distributions-9692-VEQT.json"), "9563"), Outcome::Meaning(w) if w.contains("9692")));
    let Outcome::Answered(vfv) = vanguard_ca::parse_distributions(&common::json(VG, "distributions-9563-VFV.json"), "9563") else { panic!() };
    assert!(vfv.len() > 40);
}

#[test]
fn a_type_this_reader_does_not_know_is_a_mismatch_naming_it() {
    let v = common::json(VG, "wrong-shape-distributions-unknown-type.json");
    assert!(matches!(vanguard_ca::parse_distributions(&v, "9692"), Outcome::Mismatch(m) if m.why.contains("XYZ")));
}

#[test]
fn a_canadian_vanguard_fund_is_read_whole_and_a_us_one_is_not_this_companys() {
    // VEQT on the TSX is Vanguard Canada's; VTI on the NYSE is not
    assert_eq!(payers::adapter_for(&need("VEQT", "XTSE", "Vanguard All-Equity ETF Portfolio - ETF")).map(|a| a.source().to_string()).as_deref(), Some("vanguard-canada"));
    assert!(payers::adapter_for(&need("VTI", "XNYS", "Vanguard Group, Inc. - Vanguard Morningstar Total Stock Market ETF")).is_none_or(|a| a.source().as_str() != "vanguard-canada"));
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://www.vanguard.ca/en/product", 200, VG, "product-list.html")
            .with_body("https://www.vanguard.ca/gpx/graphql", "\"operationName\":\"Listings\"", 200, VG, "listings.json")
            .with_body("https://www.vanguard.ca/gpx/graphql", "\"operationName\":\"Distributions\"", 200, VG, "distributions-9692-VEQT.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let veqt = need("VEQT", "XTSE", "Vanguard All-Equity ETF Portfolio - ETF");
    let noted = payers::adapter_for(&veqt).unwrap().read(&net, &veqt, "2026-09-24T04:00:00Z".parse().unwrap());
    let Outcome::Answered(record) = noted.outcome else { panic!("{:?}", noted.outcome) };
    assert_eq!((record.per_year, record.rows.len()), (Some(1), 7));
    // a ticker the company does not list is not carried
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://www.vanguard.ca/en/product", 200, VG, "product-list.html")
            .with_body("https://www.vanguard.ca/gpx/graphql", "\"operationName\":\"Listings\"", 200, VG, "listings.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let unknown = need("VZZZ", "XTSE", "Vanguard Something ETF");
    assert_eq!(payers::adapter_for(&unknown).unwrap().read(&net, &unknown, "2026-09-24T04:00:00Z".parse().unwrap()).outcome.kind(), OutcomeKind::NotCarried);
}
