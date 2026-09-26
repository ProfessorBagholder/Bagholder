//! Vanguard's US record, from recorded real replies (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::payers::{vanguard_us, Distribution};

#[test]
fn vanguard_us_states_the_id_the_schedule_and_each_distribution_in_dollars() {
    assert_eq!(vanguard_us::port_id(&common::json("vanguard-us", "validate-VTI.json")).unwrap(), "0970");
    assert_eq!(vanguard_us::schedule(&common::json("vanguard-us", "profile-0970.json")).unwrap(), Some(4));
    let rows = vanguard_us::parse_distributions(&common::json("vanguard-us", "distributions-0970.json")).unwrap();
    assert!(rows.len() >= 38);
    let latest = rows.iter().max_by_key(|r| r.ex_date).unwrap();
    assert_eq!(*latest, Distribution { ex_date: date(2026, 6, 26), record_date: Some(date(2026, 6, 26)), pay_date: Some(date(2026, 6, 30)), cash: Dec::parse("1.0437").unwrap(), reinvested: None, currency: Currency::USD });
}
