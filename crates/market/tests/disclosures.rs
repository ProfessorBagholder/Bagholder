//! Filing sources: matching, categories and routing.
use bagholder_market::disclosures as d;
use serde_json::json;

#[test]
fn test_stale_category_is_corrected() {
    let row = json!({"source": "SEC", "type": "F-X", "category": "Offerings"});
    assert_eq!(d::categorize(&row), json!(d::OTHER));
    assert_eq!(d::categorize(&json!({"source": "SEC", "type": "F-1", "category": "Other"})), json!(d::OFFERINGS));
}

#[test]
fn test_unknown_source_keeps_stored() {
    assert_eq!(d::categorize(&json!({"source": "???", "type": "X", "category": "Financials"})), json!("Financials"));
}
