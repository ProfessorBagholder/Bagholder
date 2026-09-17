//! Filings whose facts sit on the page in a fixed shape, read exactly rather
//! than summarized.
//!
//! A regulator's fill-in form is mostly its own instructions, and a model
//! handed that text summarizes the instructions. So a form this module knows
//! is read here, value by value, and never reaches a model; a form it does not
//! know is not summarized at all. Everything here is the document's own text;
//! a value that is not found is left out of the sentence rather than filled in.

use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;

use bagholder_model::textrules::{parse_float, parse_int};

fn marks() -> &'static [Regex; 8] {
    static M: OnceLock<[Regex; 8]> = OnceLock::new();
    M.get_or_init(|| {
        [
            Regex::new(r"(?i)\(YYYY\s*-\s*MM\s*-\s*DD\)").unwrap(),
            Regex::new(r"(?i)\brefer to (?:part|section|item)\b").unwrap(),
            Regex::new(r"(?i)\bselect (?:only )?one\b").unwrap(),
            Regex::new(r"(?i)\bcomplete (?:schedule|item|part)\b").unwrap(),
            Regex::new(r"(?i)\bif applicable\b").unwrap(),
            Regex::new(r"(?i)\bcheck (?:the )?box\b").unwrap(),
            Regex::new(r"(?i)\bdo not complete\b").unwrap(),
            Regex::new(r"\bof the [Ii]nstructions\b").unwrap(),
        ]
    })
}

/// Three of the eight, so prose that happens to say "if applicable" is not a
/// form.
pub const FORM_MARK_MIN: usize = 3;

/// `forms.is_form`: a regulator's fill-in form rather than something written.
pub fn is_form(text: &str) -> bool {
    marks().iter().filter(|m| m.is_match(text)).count() >= FORM_MARK_MIN
}

fn num(s: &str) -> Option<f64> {
    parse_float(&s.replace(',', ""))
}

/// `"{:,.Nf}".format(n)`: fixed decimals, thousands separated.
fn grouped(n: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, n);
    let (sign, body) = if let Some(rest) = s.strip_prefix('-') { ("-", rest) } else { ("", s.as_str()) };
    let (int, frac) = match body.split_once('.') { Some((a, b)) => (a, Some(b)), None => (body, None) };
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    match frac { Some(f) => format!("{}{}.{}", sign, out, f), None => format!("{}{}", sign, out) }
}

/// `forms._money`: a dollar amount without cents it does not have.
fn money(n: Option<f64>) -> String {
    match n {
        None => String::new(),
        Some(v) => format!("${}", if v.fract() == 0.0 || v >= 1000.0 { grouped(v, 0) } else { grouped(v, 2) }),
    }
}

/// `forms._date`: a form's date boxes, `Start date 2026 YYYY 09 08 MM DD`.
fn date(text: &str, label: &str) -> String {
    let r = Regex::new(&format!(r"(?i){}\s*(\d{{4}})\s*YYYY\s*(\d{{1,2}})\s*(\d{{1,2}})\s*MM", label)).unwrap();
    match r.captures(text) {
        Some(m) => format!("{}-{:02}-{:02}", &m[1], parse_int(&m[2]).unwrap_or(0), parse_int(&m[3]).unwrap_or(0)),
        None => String::new(),
    }
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// `forms._day`: `2026-09-08` as `8 September 2026`. A month of 0 reads as
/// December, since Python's `months[-1]` is the last one.
fn day(iso: &str) -> String {
    let parts: Vec<Option<i64>> = iso.split('-').map(parse_int).collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_none()) {
        return String::new();
    }
    let (y, m, d) = (parts[0].unwrap(), parts[1].unwrap(), parts[2].unwrap());
    let idx = m - 1;
    let name = if (0..12).contains(&idx) { MONTHS[idx as usize] } else if (-12..0).contains(&idx) { MONTHS[(12 + idx) as usize] } else { return String::new() };
    format!("{} {} {}", d, name, y)
}

/// `forms.read_45_106f1`: Form 45-106F1, Report of Exempt Distribution -- what
/// was raised, from how many purchasers, on what date, under which exemption.
pub fn read_45_106f1(text: &str) -> Value {
    static AMOUNT: OnceLock<Regex> = OnceLock::new();
    static BUYERS: OnceLock<Regex> = OnceLock::new();
    static EXEMPTION: OnceLock<Regex> = OnceLock::new();
    let amount = AMOUNT
        .get_or_init(|| Regex::new(r"(?i)Total dollar amount of securities distributed\s*\$?\s*([\d,]+(?:\.\d+)?)").unwrap())
        .captures(text)
        .and_then(|m| num(&m[1]));
    let buyers = BUYERS
        .get_or_init(|| Regex::new(r"(?i)Total number of unique\s*(?:purchasers)?\s*(\d[\d,]*)").unwrap())
        .captures(text)
        .and_then(|m| num(&m[1]));
    let when = { let s = date(text, "Start date"); if s.is_empty() { date(text, "End date") } else { s } };
    let exemption = EXEMPTION
        .get_or_init(|| Regex::new(r"NI\s*45-106\s*([\d.]+)\s*\[([^\]]{3,60})\]").unwrap())
        .captures(text)
        .map(|m| format!("NI 45-106 {} ({})", &m[1], bagholder_model::textrules::trim_space(&m[2]).to_lowercase()))
        .unwrap_or_default();
    if amount.is_none() && buyers.is_none() {
        return json!({});
    }
    let mut parts: Vec<String> = Vec::new();
    if amount.is_some() {
        parts.push(format!("{} distributed", money(amount)));
    }
    if let Some(b) = buyers {
        parts.push(format!("{} purchaser{}", b.trunc() as i64, if b == 1.0 { "" } else { "s" }));
    }
    let mut head = if parts.len() == 2 { parts.join(" from ") } else { parts[0].clone() };
    if !when.is_empty() {
        head.push_str(&format!(" on {}", day(&when)));
    }
    if !exemption.is_empty() {
        head.push_str(&format!(", under {}", exemption));
    }
    // the title says what the document is about, not what it is
    let subject = if amount.is_some() { format!("Exempt distribution of {}", money(amount)) } else { String::new() };
    json!({"subject": subject, "summary": format!("{}.", head)})
}

/// `forms.read`: the document read exactly where this module knows its form,
/// {} otherwise. A form is claimed by the words on its own first page.
pub fn read(text: &str) -> Value {
    static CLAIM: OnceLock<Regex> = OnceLock::new();
    let head: String = text.chars().take(4000).collect();
    if CLAIM.get_or_init(|| Regex::new(r"(?i)Form\s*45-106F1|Report of Exempt Distribution").unwrap()).is_match(&head) {
        let out = read_45_106f1(text);
        if out.get("summary").and_then(|s| s.as_str()).map(|s| !s.is_empty()).unwrap_or(false) {
            return out;
        }
    }
    json!({})
}
