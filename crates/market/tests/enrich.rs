//! Subject extraction from PDF metadata, text
//! extraction, and the graceful behaviour when the local model is absent. The
//! model is stood in for through `localmodel::hooks`; no network, no subprocess.

use bagholder_market::{enrich, localmodel};

fn no_pdf() {
    // never shell out to a PDF engine during tests
    std::env::set_var("BAGHOLDER_NO_PDF", "1");
}

fn pdf_with_title(title: &[u8]) -> Vec<u8> {
    let mut v = b"%PDF-1.7\n1 0 obj<< /Title (".to_vec();
    v.extend_from_slice(title);
    v.extend_from_slice(b") >>\nendobj\n%%EOF");
    v
}

fn set_chat(f: impl Fn(&str, i64) -> String + 'static) {
    localmodel::hooks::CHAT.with(|h| *h.borrow_mut() = Some(Box::new(f)));
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|c| *c as char).collect()
}

// --- SubjectTest

#[test]
fn test_a_word_authored_title_reduces_to_its_subject() {
    let data = pdf_with_title(b"Microsoft Word - CHARBONE - Closing 2nd Drawdown PR_FINAL_EN_2026-09-04_v6");
    assert_eq!(enrich::extract_pdf_subject(&data), "CHARBONE - Closing 2nd Drawdown");
}

#[test]
fn test_language_version_and_date_tails_are_stripped() {
    assert_eq!(enrich::clean_subject("Q3 2026 Results PR EN v3"), "Q3 2026 Results");
    assert_eq!(enrich::clean_subject("Prospectus Supplement No 3 FINAL"), "Prospectus Supplement No 3");
}

#[test]
fn test_a_generic_or_empty_title_yields_no_subject() {
    assert_eq!(enrich::extract_pdf_subject(&pdf_with_title(b"News release")), "");
    assert_eq!(enrich::extract_pdf_subject(&pdf_with_title(b"Document")), "");
    assert_eq!(enrich::extract_pdf_subject(b"%PDF-1.7 no title here"), "");
}

#[test]
fn test_a_utf16_hex_title_is_decoded() {
    let hex: String = "Financing Update".chars().map(|c| format!("{:04x}", c as u32)).collect();
    let mut data = b"%PDF-1.7\n1 0 obj<< /Title <feff".to_vec();
    data.extend_from_slice(hex.as_bytes());
    data.extend_from_slice(b"> >>\nendobj");
    assert_eq!(enrich::extract_pdf_subject(&data), "Financing Update");
}

#[test]
fn test_a_title_of_bytes_that_merely_decoded_is_no_title() {
    let binary: &[u8] = b"\\022\x8a\xf0,0\x91\x9f\xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0\\027\xb3c\xab\\)Q\xb1\xba \x9cO\xb4.\xb4\xf9 xBtu\x83\xe5\\f!";
    assert_eq!(enrich::extract_pdf_subject(&pdf_with_title(binary)), "");
    assert!(!enrich::readable(&latin1(binary)));
}

#[test]
fn test_what_reads_as_a_title_and_what_does_not() {
    for good in ["CHARBONE - Closing 2nd Drawdown", "D\u{e9}claration de placement avec dispense 45-106F1",
                 "Q3 2026 Interim Financial Statements", "Form 45-106F1 Report of Exempt Distribution"] {
        assert!(enrich::readable(good), "{}", good);
    }
    for bad in ["2026-09-04", "\u{b1}\u{ba}\u{b4}\u{ab}\u{b9}\u{b2}", "", "   ", "\x0c\x12 Report", "45-106"] {
        assert!(!enrich::readable(bad), "{:?}", bad);
    }
}

#[test]
fn test_an_unreadable_title_falls_back_to_the_model() {
    no_pdf();
    set_chat(|prompt, _| if prompt.contains("Title:") { "Report of exempt distribution in Canada".into() } else { "It reports a distribution.".into() });
    enrich::hooks::DOCUMENT_TEXT.with(|h| *h.borrow_mut() = Some(Box::new(|_, _| "A report of exempt distribution.".to_string())));
    let data = pdf_with_title(b"\x8a\xf0,0\x91\x9f\xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0");
    let out = enrich::enrich_document("sedar", &data, "application/pdf");
    assert_eq!(out["subject"], "Report of exempt distribution in Canada");
}

#[test]
fn test_non_pdf_bytes_have_no_pdf_subject() {
    assert_eq!(enrich::extract_pdf_subject(b"<html>...</html>"), "");
}

// --- TextTest

#[test]
fn test_html_text_drops_scripts_and_tags() {
    let html = b"<html><head><style>.x{}</style></head><body><h1>Results</h1><script>x()</script><p>Net income up 20%</p></body></html>";
    assert_eq!(enrich::html_text(html), "Results Net income up 20%");
}

#[test]
fn test_pdf_text_is_empty_when_no_engine() {
    no_pdf();
    assert_eq!(enrich::pdf_text(b"%PDF-1.7 ..."), "");
}

#[test]
fn test_document_text_routes_by_type() {
    assert_eq!(enrich::document_text(b"<p>hello there</p>", "text/html"), "hello there");
}

// --- SummaryTest

#[test]
fn test_no_model_means_no_summary() {
    set_chat(|_, _| String::new());
    assert_eq!(enrich::summarize("Some filing text"), "");
}

#[test]
fn test_summary_of_empty_text_is_empty() {
    set_chat(|_, _| panic!("must not ask the model"));
    assert_eq!(enrich::summarize(""), "");
}

#[test]
fn test_summary_is_kept_to_one_sentence() {
    set_chat(|_, _| "It announces a private placement. Extra sentence.".into());
    assert_eq!(enrich::summarize("text"), "It announces a private placement.");
}

#[test]
fn test_summary_strips_chat_template_tokens() {
    set_chat(|_, _| "It announces a private placement.<|eot_id|>".into());
    assert_eq!(enrich::summarize("text"), "It announces a private placement.");
}

#[test]
fn test_title_strips_a_chatty_preamble_and_markdown() {
    set_chat(|_, _| "Sure, here is the title: **Closing of $1.5M Drawdown**".into());
    assert_eq!(enrich::title_from_model("some filing text"), "Closing of $1.5M Drawdown");
}

#[test]
fn test_title_rejects_a_bare_form_code_or_echo() {
    set_chat(|_, _| "Schedule 13G".into());
    assert_eq!(enrich::title_from_model("text"), "");
    set_chat(|_, _| "Here is a title for the filing".into());
    assert_eq!(enrich::title_from_model("text"), "");
}

#[test]
fn test_no_model_means_no_title() {
    set_chat(|_, _| String::new());
    assert_eq!(enrich::title_from_model("text"), "");
}

#[test]
fn test_enrich_document_titles_from_the_model_when_there_is_no_pdf_subject() {
    set_chat(|prompt, _| if prompt.contains("Title:") { "Q2 2026 MD&A and interim financial statements".into() } else { "It reports Q2 2026 results.".into() });
    let info = enrich::enrich_document("SEC", b"<html><body>Management discussion...</body></html>", "text/html");
    assert_eq!(info["subject"], "Q2 2026 MD&A and interim financial statements");
    assert_eq!(info["summary"], "It reports Q2 2026 results.");
}

#[test]
fn test_enrich_document_gives_subject_without_a_model() {
    no_pdf();
    set_chat(|_, _| String::new());
    let info = enrich::enrich_document("SEDAR+", &pdf_with_title(b"Microsoft Word - Acme Announces Buyback EN"), "application/pdf");
    assert_eq!(info["subject"], "Acme Announces Buyback");
    assert_eq!(info["summary"], "");
}

// --- SentenceTest

#[test]
fn test_a_company_suffix_does_not_end_the_sentence() {
    for opening in ["Quantum eMotion Corp.", "Aegis Critical Energy Defence Corp.", "Shopify Inc.", "High Tide Ltd.", "Brookfield Co.", "Barrick PLC"] {
        let line = format!("{} announces a commercial order for seven units in the United States.", opening);
        assert_eq!(enrich::first_sentence(&line), line, "{}", opening);
    }
}

#[test]
fn test_an_initial_or_an_abbreviation_does_not_end_it_either() {
    for line in ["U.S. regulators approved the base shelf prospectus.",
                 "Dr. Chen was appointed chief scientist of the company.",
                 "No. 4 of the schedule lists the securities offered.",
                 "J. Smith resigned from the board of directors."] {
        assert_eq!(enrich::first_sentence(line), line, "{}", line);
    }
}

#[test]
fn test_a_real_second_sentence_is_dropped() {
    assert_eq!(enrich::first_sentence("The company files Q1 statements. It also names a director."), "The company files Q1 statements.");
    assert_eq!(enrich::first_sentence("Is the prospectus final? The company says yes."), "Is the prospectus final?");
}

#[test]
fn test_a_stop_followed_by_more_of_the_same_sentence_is_not_an_ending() {
    let line = "The filing lists exhibits 1.2 and 3. and describes the securities offered.";
    assert_eq!(enrich::first_sentence(line), line);
}

#[test]
fn test_an_answer_with_no_stop_at_all_survives_whole() {
    assert_eq!(enrich::first_sentence("Quantum eMotion Corp files its interim statements"), "Quantum eMotion Corp files its interim statements");
    assert_eq!(enrich::first_sentence(""), "");
}

#[test]
fn test_the_summary_keeps_the_whole_sentence_rather_than_the_name_alone() {
    let said = "Quantum eMotion Corp. announces its participation as a sponsor of the AI for Good Global Summit.";
    set_chat(move |_, _| said.to_string());
    assert_eq!(enrich::summarize("the filing's text"), said);
}

#[test]
fn test_a_bare_name_is_still_no_summary() {
    set_chat(|_, _| "Quantum eMotion Corp.".into());
    assert_eq!(enrich::summarize("the filing's text"), "");
}
