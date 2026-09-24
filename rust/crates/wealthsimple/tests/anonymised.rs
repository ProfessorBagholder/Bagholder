//! No recorded reply carries a value that identifies the owner
//! (`docs/plans/stage-3b-wealthsimple.md`, "Anonymised, not scrubbed of
//! figures"): every identifier and personal text is a stand-in.

use bagholder_core::json;
use bagholder_wealthsimple::anonymise::{leaks, Anonymiser};

#[test]
fn every_recorded_reply_holds_only_stand_ins() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let v = json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let left = leaks(&v);
            assert!(left.is_empty(), "{}: identifying values at {left:?}", path.display());
            seen += 1;
        }
    }
    assert!(seen > 0, "no recorded replies");
}

#[test]
fn one_value_is_one_stand_in_wherever_it_appears() {
    let v = json::parse(r#"{"accounts":[{"id":"tfsa-abc123"}],"rows":[{"accountId":"tfsa-abc123","securityId":"sec-s-1","eTransferEmail":"a@b.c"},{"accountId":"rrsp-xyz"}]}"#).unwrap();
    let out = Anonymiser::default().value(&v).canonical();
    assert_eq!(out, r#"{"accounts":[{"id":"anon-tfsa-1"}],"rows":[{"accountId":"anon-tfsa-1","eTransferEmail":"anon-personal-1","securityId":"sec-s-1"},{"accountId":"anon-rrsp-1"}]}"#);
}

#[test]
fn a_leak_is_named_by_its_path() {
    let v = json::parse(r#"{"rows":[{"accountId":"tfsa-abc123"}]}"#).unwrap();
    assert_eq!(leaks(&v), vec!["rows[0].accountId".to_string()]);
}
