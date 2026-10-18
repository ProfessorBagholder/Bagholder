//! SEDAR+'s answers for every S&P/TSX 60 member, recorded 2026-10-16
//! (`replies/sedar-tsx60/`): each member's name as the TSX directory gives it,
//! asked as `query_of` cuts it, settles to exactly one profile by the name rules,
//! TELUS's two same-name profiles by the tie rule (plan
//! `market-data-keyed-by-listing`, Open questions, 1; brief 41).

use bagholder_market::sedar::{self, ReportingIssuer};
use serde_json::Value;

fn recorded(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/sedar-tsx60").join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_member_settles_to_one_profile_as_sedar_answered() {
    let answers = recorded("sedar-resolve.json");
    let newest = recorded("telus-newest-filings.json");
    let mut settled = 0;
    for a in answers.as_array().unwrap() {
        let name = a["name"].as_str().unwrap();
        assert_eq!(sedar::query_of(name), a["query"].as_str().unwrap(), "{name} is asked as it was recorded");
        let profiles: Vec<ReportingIssuer> = a["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| ReportingIssuer { name: p["name"].as_str().unwrap().into(), profile_no: p["profileNo"].as_str().unwrap().into(), ..Default::default() })
            .collect();
        // the day each tied profile last filed, as SEDAR+ listed it; only a tie asks
        let filed = |profile: &str| -> bagholder_market::disclosures::Fetched<Option<String>> {
            let when = newest[profile]["newest"].as_str().unwrap_or_else(|| panic!("{name}: {profile} was asked its newest filing but none is recorded"));
            Ok(Some(sedar::iso(when)))
        };
        let got = sedar::pick_profile(name, &profiles, "2026-10-16", &filed).unwrap();
        assert!(got.is_ok(), "{name}: {got:?}");
        settled += 1;
        if a["symbol"] == "T" {
            assert_eq!(got.unwrap(), "000011998", "TELUS is the profile that files today, not the one last filing in 1998");
        }
    }
    assert_eq!(settled, 60);
}
