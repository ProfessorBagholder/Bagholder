//! Port of tests/test_pdftext.py. The Rust engine has no provisioning step and no
//! injectable extractor; what carries over is the routing of bytes.
use bagholder_market::pdftext;

#[test]
fn test_non_pdf_bytes_yield_no_text() {
    assert_eq!(pdftext::text(b"<html>not a pdf</html>"), "");
}

#[test]
fn test_a_broken_extractor_is_swallowed() {
    // a malformed PDF: whichever engine reads it, the answer is "" and never a panic
    assert_eq!(pdftext::text(b"%PDF-1.7 ..."), "");
}

#[test]
fn test_ensure_is_a_noop_when_disabled() {
    assert!(!pdftext::pending());
}
