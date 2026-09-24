//! Fidelity Canada's own data, from recorded real replies (2026-09-24).

mod common;

use bagholder_sources::outcome::Outcome;
use bagholder_sources::payers::fidelity;

#[test]
fn fidelity_states_the_schedule_and_says_when_a_series_has_paid_nothing() {
    let funds = common::json("fidelity", "funds-trimmed.json");
    assert_eq!(fidelity::schedule_of(&funds, "FBTC").unwrap(), Some(Some(1)));
    assert_eq!(fidelity::schedule_of(&funds, "FCAB").unwrap(), Some(Some(12)));
    assert_eq!(fidelity::schedule_of(&funds, "ZZZQX").unwrap(), None);
    // FBTC has paid nothing in cash: the service answers an empty body
    assert!(matches!(fidelity::parse_history(common::read("fidelity", "history-FBTC.json").as_bytes(), "FBTC"), Outcome::Answered(())));
    // a list with one date a row and no ex-date is refused, named
    assert!(matches!(fidelity::parse_history(common::read("fidelity", "history-FCAB.json").as_bytes(), "FCAB"), Outcome::Mismatch(m) if m.why.contains("no ex-date")));
}
