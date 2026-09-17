//! Port of tests/test_forms.py.
use bagholder_market::{enrich, forms};
use serde_json::json;

const F1: &str = "Form 45-106F1 Report of Exempt Distribution ITEM 1 - REPORT TYPE New report Amended report If amended, provide filing date of report that is being amended. (YYYY-MM-DD) ITEM 2 - PARTY CERTIFYING THE REPORT Indicate the party certifying the report (select only one). For guidance regarding whether an issuer is an investment fund, refer to section 1.1 of National Instrument 81-106 Investment Fund Continuous Disclosure. ITEM 3 - ISSUER NAME Charbone Corporation ITEM 7 - INFORMATION ABOUT THE DISTRIBUTION b) Distribution dates State the distribution start and end dates. Start date 2026 YYYY 09 08 MM DD End date 2026 YYYY 09 08 MM DD c) Detailed purchaser information Complete Schedule 1 of this form for each purchaser. Province or country Exemption relied on Number of unique purchasers Total amount (Canadian $) Gibraltar NI 45-106 2.10 [Minimum amount investment] 1 1,500,000.0000 Total dollar amount of securities distributed $1,500,000.0000 Total number of unique 1";

const RELEASE: &str = "CHARBONE Corporation announces the closing of a second drawdown of $1.5M with RiverFort Global Opportunities PCC Ltd. The proceeds will accelerate the growth of its industrial gas platform across North America, the company said on Tuesday. Management will host a call to discuss the transaction.";

#[test]
fn test_a_regulators_fill_in_form_is_told_from_something_written() {
    assert!(forms::is_form(F1));
    assert!(!forms::is_form(RELEASE));
    assert!(!forms::is_form(""));
}

#[test]
fn test_the_exempt_distribution_report_is_read_value_by_value() {
    let out = forms::read(F1);
    assert_eq!(out["subject"], "Exempt distribution of $1,500,000");
    assert_eq!(out["summary"], "$1,500,000 distributed from 1 purchaser on 8 September 2026, under NI 45-106 2.10 (minimum amount investment).");
}

#[test]
fn test_a_value_the_form_does_not_carry_is_left_out_rather_than_filled_in() {
    let thin = "Form 45-106F1 Report of Exempt Distribution Total dollar amount of securities distributed $250,000.0000";
    assert_eq!(forms::read(thin)["summary"], "$250,000 distributed.");
    assert_eq!(forms::read("Form 45-106F1 Report of Exempt Distribution and nothing else"), json!({}));
}

#[test]
fn test_several_purchasers_read_as_several() {
    let many = F1.replace("Total number of unique 1", "Total number of unique 14");
    assert!(forms::read(&many)["summary"].as_str().unwrap().contains("from 14 purchasers"));
}

#[test]
fn test_nothing_else_is_claimed() {
    assert_eq!(forms::read(RELEASE), json!({}));
}

#[test]
fn test_a_hedged_line_is_thrown_away() {
    for guess in ["The company announces the completion of a new report, likely a Form 45-106F1.",
                  "This appears to be a report of exempt distribution.",
                  "The filing may be related to a private placement.",
                  "It is not clear what the document reports."] {
        assert!(enrich::hedged(guess), "{}", guess);
    }
    assert!(!enrich::hedged("The company closed a $1.5M drawdown with RiverFort."));
}

#[test]
fn test_a_form_this_app_cannot_read_is_not_summarized_at_all() {
    // document_text of text/plain-ish bytes: fed as HTML so the text is the form itself; a form
    // never reaches the model (no model is mocked here, so reaching it would not return this shape)
    let unknown = "Form 51-999F9 Something New (YYYY-MM-DD) refer to Part B of the Instructions. Complete Schedule 2 for each holder. Select only one. If applicable, provide the filing date.";
    let text = enrich::document_text(unknown.as_bytes(), "text/html");
    assert!(forms::is_form(&text), "{}", text);
    let out = enrich::enrich_document("sedar", unknown.as_bytes(), "text/html");
    assert_eq!(out, json!({"subject": "", "summary": "", "final": true}));
}

#[test]
fn test_a_form_this_app_reads_never_sees_a_model() {
    let out = enrich::enrich_document("sedar", F1.as_bytes(), "text/html");
    assert_eq!(out["subject"], "Exempt distribution of $1,500,000");
    assert!(out["summary"].as_str().unwrap().contains("1 purchaser"));
}
