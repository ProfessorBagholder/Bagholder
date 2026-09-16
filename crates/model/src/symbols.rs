//! Reading an option symbol: Wealthsimple posts them in more than one shape
//! ("ZZZ 21AUG26 10.00 CALL", "ZZZ 210826C10"), and the model has to agree
//! with itself about which rows are options and what they are written on.

use crate::value::{compact, fold_spaces_upper};

/// `model.is_option_symbol`.
pub fn is_option_symbol(symbol: &str) -> bool {
    let u = fold_spaces_upper(symbol);
    if u.is_empty() { return false; }
    if has_word(&u, "PUT") || has_word(&u, "CALL") { return true; }
    if ends_with_space_cp(&u) { return true; }
    occ_underlying(&u).is_some()
}

/// A bare `PUT`/`CALL` word, not a substring of a longer token.
fn has_word(u: &str, word: &str) -> bool {
    u.split(|c: char| !(c.is_ascii_alphanumeric()))
        .any(|t| t == word)
}

/// The `\s[CP]$` tail of the short form.
fn ends_with_space_cp(u: &str) -> bool {
    let b = u.as_bytes();
    b.len() >= 2 && (b[b.len() - 1] == b'C' || b[b.len() - 1] == b'P') && b[b.len() - 2] == b' '
}

/// `^([A-Z][A-Z0-9.]{0,9}) \d{6}[CP]\d+` -- the OCC-style form. Returns the
/// underlying when it matches.
fn occ_underlying(u: &str) -> Option<&str> {
    let (head, rest) = u.split_once(' ')?;
    if !valid_root(head) { return None; }
    let b = rest.as_bytes();
    if b.len() < 8 { return None; }
    if !b[..6].iter().all(|c| c.is_ascii_digit()) { return None; }
    if b[6] != b'C' && b[6] != b'P' { return None; }
    if !b[7].is_ascii_digit() { return None; }
    Some(head)
}

/// `^([A-Z][A-Z0-9.]{0,9}) \d{1,2}[A-Z]{3}\d{2}\b` -- the "21AUG26" form.
fn dated_underlying(u: &str) -> Option<&str> {
    let (head, rest) = u.split_once(' ')?;
    if !valid_root(head) { return None; }
    let tok = rest.split(|c: char| c == ' ').next()?;
    let b = tok.as_bytes();
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 2 || b.len() < digits + 5 { return None; }
    if !b[digits..digits + 3].iter().all(|c| c.is_ascii_uppercase()) { return None; }
    if !b[digits + 3..digits + 5].iter().all(|c| c.is_ascii_digit()) { return None; }
    Some(head)
}

fn valid_root(head: &str) -> bool {
    let b = head.as_bytes();
    if b.is_empty() || b.len() > 10 { return false; }
    if !b[0].is_ascii_uppercase() { return false; }
    b[1..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'.')
}

/// `model.underlying_symbol`: the name the contract is written on, or the
/// symbol itself when it is not an option. An em dash for an empty symbol,
/// as the page prints it.
pub fn underlying_symbol(symbol: &str) -> String {
    let t = symbol.trim();
    if t.is_empty() { return "—".into(); }
    let u = fold_spaces_upper(t);
    if has_word(&u, "PUT") || has_word(&u, "CALL") || ends_with_space_cp(&u) {
        let first = u.split(' ').next().unwrap_or("");
        return if first.is_empty() { t.to_string() } else { first.to_string() };
    }
    if let Some(root) = occ_underlying(&u) { return root.to_string(); }
    if let Some(root) = dated_underlying(&u) { return root.to_string(); }
    t.to_string()
}

/// `model.option_multiplier`: a contract is a hundred shares, anything else
/// is one unit.
pub fn option_multiplier(symbol: &str) -> f64 {
    if is_option_symbol(symbol) { 100.0 } else { 1.0 }
}

/// `model.option_right`: a symbol is a put only when it says so; everything
/// else reads as a call, matching the Python fallback.
pub fn option_right(symbol: &str) -> &'static str {
    let u = fold_spaces_upper(symbol);
    if u.ends_with(" PUT") || u.ends_with(" P") { return "PUT"; }
    // ` \d{6}P\d+`
    for (i, _) in u.match_indices(' ') {
        let rest = &u.as_bytes()[i + 1..];
        if rest.len() >= 8
            && rest[..6].iter().all(|c| c.is_ascii_digit())
            && rest[6] == b'P'
            && rest[7].is_ascii_digit()
        {
            return "PUT";
        }
    }
    "CALL"
}

/// `model.is_multileg`.
pub fn is_multileg_raw(raw_type: &str) -> bool {
    compact(raw_type).contains("MULTILEG")
}
