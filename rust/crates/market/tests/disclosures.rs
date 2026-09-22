//! Filing sources: matching, categories and routing.
use bagholder_market::disclosures as d;
use bagholder_store::feeds::{FiledDocument, Regulator};

fn doc(source: Regulator, form: &str, category: &str) -> FiledDocument {
    FiledDocument {
        id: String::new(),
        source,
        category: category.into(),
        profile_no: String::new(),
        issuer: String::new(),
        form: form.into(),
        title: String::new(),
        date: String::new(),
        date_text: String::new(),
        size: String::new(),
        url: String::new(),
    }
}

#[test]
fn test_stale_category_is_corrected() {
    assert_eq!(d::categorize(&doc(Regulator::Sec, "F-X", "Offerings")), d::OTHER);
    assert_eq!(d::categorize(&doc(Regulator::Sec, "F-1", "Other")), d::OFFERINGS);
}
