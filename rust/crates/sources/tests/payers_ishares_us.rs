//! iShares' US record, from recorded real pages (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::payers::{ishares_us, Distribution};

#[test]
fn ishares_us_names_the_page_and_the_page_states_schedule_and_history() {
    let path = ishares_us::page_of(&common::json("ishares-us", "screener-trimmed.json"), "IEFA").unwrap().unwrap();
    assert!(path.starts_with("/us/products/244049/"), "{path}");
    assert_eq!(ishares_us::page_of(&common::json("ishares-us", "screener-trimmed.json"), "ZZZQX").unwrap(), None);
    let page = common::read("ishares-us", "page-iefa.html");
    assert_eq!(ishares_us::schedule_word(&page).as_deref(), Some("Semi-Annual"));
    let rows = ishares_us::parse_component(&ishares_us::component(&page).unwrap()).unwrap();
    assert_eq!(rows.len(), 29);
    assert_eq!(rows[0], Distribution { ex_date: date(2026, 6, 15), record_date: Some(date(2026, 6, 15)), pay_date: Some(date(2026, 6, 18)), cash: Dec::parse("1.57806").unwrap(), reinvested: None, currency: Currency::USD });
    let eemv = ishares_us::parse_component(&ishares_us::component(&common::read("ishares-us", "page-eemv.html")).unwrap()).unwrap();
    assert_eq!(eemv.len(), 30);
}
