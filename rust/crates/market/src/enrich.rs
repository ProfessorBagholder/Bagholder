//! Reading a filing: its subject, and a concise summary, both optional and
//! local.
//!
//! - Subject: the document's own title, which SEDAR+ hides behind a generic
//!   file name, pulled from the PDF's metadata.
//! - Summary: one plain sentence of what the filing announces, from a language
//!   model running locally, so nothing leaves the machine.
//!
//! Everything here degrades to "" rather than failing.

use regex::bytes::Regex as BytesRegex;
use regex::Regex;
use std::sync::OnceLock;

use crate::disclosures::Enrichment;
use bagholder_sources::html::unescape;
use bagholder_model::unichars::{is_alpha, is_digit, is_space, is_upper};
use bagholder_model::textrules::trim_space;

/// Characters of the filing fed to the model.
pub const MAX_TEXT: usize = 3000;
/// How long a document read waits for a model that is starting.
pub const SUMMARY_WAIT_SEC: f64 = 40.0;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).unwrap())
        }
    };
}

fn ws_collapse(s: &str) -> String {
    re_ws().replace_all(s, " ").into_owned()
}

re!(re_ws, r"\s+");
re!(re_tags, r"<[^>]+>");

/// Words split on runs of Unicode whitespace, empty pieces dropped.
fn split_space(s: &str) -> Vec<&str> {
    s.split(is_space).filter(|w| !w.is_empty()).collect()
}

// --- subject: the document's own title -----------------------------------------

fn title_lit() -> &'static BytesRegex {
    static R: OnceLock<BytesRegex> = OnceLock::new();
    R.get_or_init(|| BytesRegex::new(r"(?-u)/Title\s*\(((?:[^()\\]|\\.)*)\)").unwrap())
}

fn title_hex() -> &'static BytesRegex {
    static R: OnceLock<BytesRegex> = OnceLock::new();
    R.get_or_init(|| BytesRegex::new(r"(?-u)/Title\s*<([0-9A-Fa-f]+)>").unwrap())
}

/// `bytes.decode("utf-16-be"/"utf-16-le", "replace")`.
fn utf16(bytes: &[u8], big: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
        .collect();
    let mut s: String = char::decode_utf16(units.iter().copied()).map(|r| r.unwrap_or('\u{fffd}')).collect();
    if bytes.len() % 2 == 1 {
        s.push('\u{fffd}');
    }
    s
}

/// The cleaned /Title from a PDF's metadata.
pub fn extract_pdf_subject(data: &[u8]) -> String {
    let m = title_lit().captures(data).or_else(|| title_hex().captures(data));
    let m = match m { Some(m) => m, None => return String::new() };
    let mut raw: Vec<u8> = m[1].to_vec();
    if title_hex().is_match(data) && raw.iter().all(|c| c.is_ascii_hexdigit()) && raw.len() % 2 == 0 {
        let hex = String::from_utf8_lossy(&raw).into_owned();
        raw = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)).collect();
    }
    let s = if raw.starts_with(&[0xfe, 0xff]) {
        utf16(&raw[2..], true)
    } else if raw.starts_with(&[0xff, 0xfe]) {
        utf16(&raw[2..], false)
    } else {
        raw.iter().map(|b| *b as char).collect()
    };
    clean_subject(&s)
}

/// Whether a string reads as a title, or is bytes that
/// merely decoded into characters. A title is letters, digits and ordinary
/// punctuation, and mostly letters.
pub fn readable(s: &str) -> bool {
    let text = trim_space(s);
    let ctrl = |c: char| matches!(c as u32, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f | 0xfffd);
    if text.is_empty() || text.chars().any(ctrl) {
        return false;
    }
    let body: Vec<char> = text.chars().filter(|c| !is_space(*c)).collect();
    if body.is_empty() {
        return false;
    }
    let punct = |c: char| " -\u{2010}\u{2013}\u{2014}'\u{2019}&(),.:;/%+#?!\"\u{201c}\u{201d}".contains(c);
    let sane = body.iter().filter(|c| is_alpha(**c) || is_digit(**c) || punct(**c)).count();
    let letters = body.iter().filter(|c| is_alpha(**c)).count();
    sane as f64 >= body.len() as f64 * 0.9 && letters as f64 >= body.len() as f64 * 0.4
}

re!(re_tooling, r"(?i)^\s*(Microsoft Word|Microsoft PowerPoint|Adobe \w+|Acrobat)\s*-\s*");
re!(re_extension, r"(?i)\.(pdf|docx?|pptx?|rtf|txt)\s*$");
re!(re_date, r"\d{4}[-_]\d{2}[-_]\d{2}");
re!(re_lang_tail, r"(?i)[_\-\s]*(FINAL|DRAFT|REVISED|v\d+|EN|FR|ENG?|FRE?|English|French)\b");
re!(re_markers, r"(?i)\b(PR|FINAL|NR|DRAFT|REVISED|v\d+)\b");
re!(re_three_letters, r"[A-Za-z]{3,}");

pub fn clean_subject(s: &str) -> String {
    let s = re_tooling().replacen(s, 1, "").into_owned();
    let s = re_extension().replacen(&s, 1, "").into_owned();
    // underscores first, so the word boundaries below actually fire
    let s = s.replace('_', " ");
    let s = re_date().replace_all(&s, " ").into_owned();
    let s = re_lang_tail().replace_all(&s, " ").into_owned();
    let s = re_markers().replace_all(&s, " ").into_owned();
    let s = ws_collapse(&s);
    let s = s.trim_matches([' ', '-', '–', '—', '·']).to_string();
    // a bare file-code with no letters, or a title that is only the generic
    // name, is no subject
    if !re_three_letters().is_match(&s) {
        return String::new();
    }
    if ["news release", "press release", "document"].contains(&s.to_lowercase().as_str()) {
        return String::new();
    }
    if readable(&s) { s } else { String::new() }
}

// --- text, for the model ---------------------------------------------------------

re!(re_sec_header, r"(?i)^\s*[\w.\-]{1,12}\s+\d+\s+\S+\.(?:htm|html|txt|xml)\s+");
re!(re_sec_exlabel, r"(?i)^\s*(?:form\s+\S+\s+)?(?:exhibit\s+[\d.]+\s+){1,3}");
re!(re_script, r"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>|<head[^>]*>.*?</head>");

fn strip_sec_header(text: &str) -> String {
    let t = re_sec_header().replacen(text, 1, "").into_owned();
    let t = re_sec_exlabel().replacen(&t, 1, "").into_owned();
    trim_space(&t).to_string()
}

/// Readable text from a SEC filing's HTML.
pub fn html_text(data: &[u8]) -> String {
    let s = String::from_utf8_lossy(data).into_owned();
    // `<(script|style|head)[^>]*>.*?</\1>`: the regex crate has no back
    // references, and the three spelled out match the same
    let s = re_script().replace_all(&s, " ").into_owned();
    let s = re_tags().replace_all(&s, " ").into_owned();
    strip_sec_header(trim_space(&ws_collapse(&unescape(&s))))
}

pub fn pdf_text(data: &[u8]) -> String {
    trim_space(&ws_collapse(&crate::pdftext::text(data))).to_string()
}

pub mod hooks {
    use std::cell::RefCell;
    pub type DocumentText = Box<dyn Fn(&[u8], &str) -> String>;
    thread_local! {
        /// A stand-in for reading a document's text, per thread.
        pub static DOCUMENT_TEXT: RefCell<Option<DocumentText>> = RefCell::new(None);
    }
}

pub fn document_text(data: &[u8], content_type: &str) -> String {
    if let Some(r) = hooks::DOCUMENT_TEXT.with(|h| h.borrow().as_ref().map(|f| f(data, content_type))) {
        return r;
    }
    if content_type.to_lowercase().contains("pdf") || data.starts_with(b"%PDF-") {
        return pdf_text(data);
    }
    html_text(data)
}

// --- summary: a local model, optional -------------------------------------------

const PROMPT: &str = "Below is the text of a company regulatory filing. In ONE short sentence, at most 20 words, say what it contains or announces \u{2014} name the actual documents, events, or figures, not the company. If it is a cover form listing exhibits, name those exhibits. Do not restate the form type or begin with 'This filing'.\n\nFILING TEXT:\n%s\n\nSUMMARY (one sentence):";
const TITLE_PROMPT: &str = "Give a short, specific title for this company filing: a noun phrase of at most 8 words naming what it is \u{2014} the documents, event, or figures it contains. Not a form code, not the company name alone, no quotes, no preamble.\n\nFILING TEXT:\n%s\n\nTitle:";

pub fn summary_available() -> bool {
    crate::localmodel::available()
}

/// As `summary_available`, from what is already known: no probe, nothing started.
pub fn summary_ready() -> bool {
    crate::localmodel::is_ready()
}

/// Start bringing a model up, if one is not up or coming already.
pub fn summary_ensure() {
    crate::localmodel::ensure()
}

pub fn wait_for_summary(seconds: f64) -> bool {
    crate::localmodel::wait_ready(seconds)
}

pub fn summary_status() -> &'static str {
    if crate::pdftext::pending() {
        return "downloading";
    }
    crate::localmodel::status()
}

/// Full stops that end an abbreviation rather than a sentence.
const ABBREV: [&str; 27] = ["corp", "inc", "ltd", "co", "llc", "llp", "plc", "lp", "sa", "nv", "ag", "cie", "pte",
    "jr", "sr", "mr", "mrs", "ms", "dr", "prof", "st", "no", "nos", "vs", "etc", "approx", "al"];

/// `(?:[a-z]\.)*[a-z]` in full.
fn is_initial(word: &str) -> bool {
    let b = word.as_bytes();
    if b.is_empty() || b.len() % 2 == 0 {
        return false;
    }
    b.iter().enumerate().all(|(i, c)| if i % 2 == 0 { c.is_ascii_lowercase() } else { *c == b'.' })
}

/// The first sentence of the model's answer, which
/// is not the text up to its first full stop. A stop ends a sentence only when
/// the word before it is not an abbreviation or an initial and what follows
/// begins a new one.
pub fn first_sentence(out: &str) -> String {
    let out = trim_space(out);
    let chars: Vec<(usize, char)> = out.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !matches!(c, '.' | '!' | '?') {
            i += 1;
            continue;
        }
        // `[.!?]+(?=\s|$)`: the whole run, then whitespace or the end
        let mut j = i;
        while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?') {
            j += 1;
        }
        let end = if j < chars.len() { chars[j].0 } else { out.len() };
        let followed = j >= chars.len() || is_space(chars[j].1);
        if !followed {
            i = j;
            continue;
        }
        let run = &out[start..end];
        let before = &out[..start];
        let last = before.rsplit(|ch: char| is_space(ch) || matches!(ch, '(' | '[' | '"' | '\'')).next().unwrap_or("");
        let word = last.to_lowercase();
        let word = word.trim_matches(['"', '\'', '(', '[']);
        let abbreviation = ABBREV.contains(&word) || is_initial(word);
        if run == "." && abbreviation {
            i = j;
            continue;
        }
        let rest = out[end..].trim_start_matches(is_space);
        if let Some(first) = rest.chars().next() {
            if !(is_upper(first) || is_digit(first) || "\"\u{201c}(".contains(first)) {
                i = j;
                continue;
            }
        }
        return trim_space(&out[..end]).to_string();
    }
    out.to_string()
}

re!(re_hedge, r"(?i)\b(likely|probably|presumably|apparently|possibly|perhaps|seems?\s+to|appears?\s+to|may\s+be|might\s+be|could\s+be|suggests?\s+that|unclear|not\s+specified|unspecified|i\s+think|it\s+is\s+not\s+clear)\b");
re!(re_lower_word, r"\b[a-z]{3,}\b");

/// Whether a model's line is a guess rather than a reading.
pub fn hedged(out: &str) -> bool {
    re_hedge().is_match(out)
}

/// The model's answer made a summary, or "" for a
/// bare name, a guess or no statement at all.
pub fn summary_from(answer: &str) -> String {
    let out = first_sentence(&strip_preamble(answer));
    if split_space(&out).len() < 4 || !re_lower_word().is_match(&out) {
        return String::new();
    }
    if hedged(&out) {
        return String::new();
    }
    out.chars().take(240).collect()
}

/// One-sentence summary of a filing's text, or "".
pub fn summarize(text: &str) -> String {
    let text = trim_space(text);
    if text.is_empty() {
        return String::new();
    }
    let clipped: String = text.chars().take(MAX_TEXT).collect();
    summary_from(&crate::localmodel::chat(&PROMPT.replacen("%s", &clipped, 1), 90))
}

re!(re_special_tokens, r"<\|[^>]*\|>");
re!(re_bullets, r"^[*#>\-\s]+");
re!(re_preamble, r"(?i)^\s*(sure[,!.]?\s+)?(here(?:'?s| is| are)\b[^:]*:?\s*)");
re!(re_label, r"(?i)^\s*(title|summary|answer)\s*[:\-]\s*");
// "This Form 8-K reports on ...", "This filing contains ...": the form is the
// row's own column, so a sentence that starts by naming it again starts later.
re!(re_restates_form, r"(?i)^\s*this\s+(?:form\s+\S+|filing|document|report|prospectus|news\s+release)\s+(?:reports\s+on|report\s+on|contains|includes|covers|presents|outlines|summari[sz]es|relates\s+to|describes|announces|is|provides|details|discloses|sets\s+out)\s+(?:that\s+)?(?:the\s+)?");

/// The same sentence with a capital at the front.
fn upper_first(text: &str) -> String {
    let mut c = text.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Drop the chatty preamble a small model prepends.
pub fn strip_preamble(out: &str) -> String {
    let out = re_special_tokens().replace_all(out, " ").into_owned();
    let out = trim_space(&ws_collapse(&out)).to_string();
    let mut out = re_bullets().replacen(&out, 1, "").into_owned();
    for _ in 0..2 {
        out = re_preamble().replacen(&out, 1, "").into_owned();
        out = re_label().replacen(&out, 1, "").into_owned();
        out = re_bullets().replacen(&out, 1, "").into_owned();
    }
    let shorter = re_restates_form().replacen(&out, 1, "").into_owned();
    if !shorter.trim().is_empty() {
        out = upper_first(shorter.trim());
    }
    trim_space(trim_space(&out).trim_matches('"').trim_matches('*')).to_string()
}

re!(re_ex_digit, r"\bex-?\d");
re!(re_five_digits, r"\d{5,}");

/// A title that is really a file name, exhibit label
/// or document id.
pub fn is_junk_title(s: &str) -> bool {
    let low = s.to_lowercase();
    if low.is_empty() {
        return true;
    }
    if !readable(s) {
        return true;
    }
    low.contains(".htm") || low.contains(".xml") || low.contains(".pdf") || low.contains("exhibit") || re_ex_digit().is_match(&low) || re_five_digits().is_match(&low)
}

/// A title read from the model's answer.
pub fn title_from(answer: &str) -> String {
    let out = strip_preamble(answer);
    let out = trim_space(out.trim_end_matches(['.', ':'])).to_string();
    let out = split_space(&out).into_iter().take(9).collect::<Vec<_>>().join(" ");
    let low = out.to_lowercase();
    if split_space(&out).len() < 3 || low.contains("title") || low.starts_with("here") || is_junk_title(&out) || hedged(&out) {
        // a preamble echo, a form or file header, a bare form code, or a guess
        return String::new();
    }
    out.chars().take(90).collect()
}

/// A short title for a filing from the local
/// model, or "".
pub fn title_from_model(text: &str) -> String {
    let text = trim_space(text);
    if text.is_empty() {
        return String::new();
    }
    let clipped: String = text.chars().take(MAX_TEXT).collect();
    title_from(&crate::localmodel::chat(&TITLE_PROMPT.replacen("%s", &clipped, 1), 40))
}

/// A title and a one-sentence summary for one
/// document, both from its readable text. `final` says the document has been
/// read for good -- a regulator's form, read exactly or not at all -- and no
/// model will add to it.
pub fn enrich_document(source: &str, data: &[u8], content_type: &str) -> Enrichment {
    enrich_document_of("", source, data, content_type)
}

/// The same, told which form the document is, so a current report is named by
/// its own items and the model is asked for the sentence alone.
pub fn enrich_document_of(code: &str, _source: &str, data: &[u8], content_type: &str) -> Enrichment {
    let is_pdf = data.starts_with(b"%PDF-");
    let mut subject = if is_pdf { extract_pdf_subject(data) } else { String::new() };
    if is_junk_title(&subject) {
        subject = String::new();
    }
    let text = document_text(data, content_type);
    if let Some(exact) = crate::forms::read(&text) {
        return Enrichment {
            subject: if exact.subject.is_empty() { subject } else { exact.subject },
            summary: exact.summary,
            final_: true,
        };
    }
    if crate::forms::is_form(&text) {
        return Enrichment { subject, summary: String::new(), final_: true };
    }
    if let Some(named) = crate::formnames::items_title(code, &text) {
        subject = named;   // the report's own items, better than any sentence about them
    } else if subject.is_empty() {
        if let Some(named) = crate::formnames::any_title(code) {
            subject = named;
        }
    }
    let summary = summarize(&text);
    if subject.is_empty() {
        subject = title_from_model(&text);   // a form the regulator does not name: the model reads one
    }
    Enrichment { subject, summary, final_: false }
}
