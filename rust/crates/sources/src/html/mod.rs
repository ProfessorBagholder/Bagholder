//! Reading HTML (moved from `bagholder-market`, one copy): every `<table>` on a
//! page as rows of cell texts ([`html_tables`]), and character references
//! resolved by the WHATWG rules ([`unescape`]).

pub mod entities;
mod tables;

pub use tables::html_tables;

/// HTML5 character references resolved by the WHATWG rules, semicolon optional,
/// longest known prefix, since a headline carries whatever the wire put in it.
///
/// A reference is `&` then a decimal or hexadecimal code point, or a name of
/// up to 32 characters, each with the semicolon optional. A name that is not
/// in the table is retried against its longest prefix that is, which is how
/// `&notit;` reads as `\u{ac}it;` -- the legacy names are recognised without
/// their semicolon.
pub fn unescape(t: &str) -> String {
    if !t.contains('&') {
        return t.to_string();
    }
    let mut out = String::with_capacity(t.len());
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            let ch = t[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        match charref(t, i + 1) {
            Some((text, next)) => {
                out.push_str(&text);
                i = next;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// One `&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\f <&#;]{1,32};?)`, replaced.
fn charref(t: &str, at: usize) -> Option<(String, usize)> {
    let b = t.as_bytes();
    if at < b.len() && b[at] == b'#' {
        let hex = at + 1 < b.len() && (b[at + 1] | 32) == b'x';
        let start = if hex { at + 2 } else { at + 1 };
        let mut end = start;
        while end < b.len() && (if hex { b[end].is_ascii_hexdigit() } else { b[end].is_ascii_digit() }) {
            end += 1;
        }
        if end == start {
            return None;
        }
        let num = u32::from_str_radix(&t[start..end], if hex { 16 } else { 10 }).unwrap_or(0x10_ffff + 1);
        let next = if end < b.len() && b[end] == b';' { end + 1 } else { end };
        return Some((codepoint(num), next));
    }
    // a name: up to 32 characters, none of them tab, newline, form feed,
    // space, `<`, `&`, `#` or `;`, and then an optional semicolon
    let mut end = at;
    while end < b.len() && end - at < 32 && !matches!(b[end], b'\t' | b'\n' | 0x0c | b' ' | b'<' | b'&' | b'#' | b';') {
        end += 1;
    }
    if end == at {
        return None;
    }
    let with_semi = end < b.len() && b[end] == b';';
    let stop = if with_semi { end + 1 } else { end };
    let name = &t[at..stop];
    if let Some(v) = lookup(name) {
        return Some((v.to_string(), stop));
    }
    // the longest prefix of the name that is in the table, the rest kept
    let mut cut = name.len();
    while cut > 1 {
        cut -= 1;
        if !name.is_char_boundary(cut) {
            continue;
        }
        if let Some(v) = lookup(&name[..cut]) {
            return Some((format!("{}{}", v, &name[cut..]), stop));
        }
    }
    None
}

fn lookup(name: &str) -> Option<&'static str> {
    entities::HTML5
        .binary_search_by(|(k, _)| (*k).cmp(name))
        .ok()
        .map(|i| entities::HTML5[i].1)
}

/// What a numeric reference to a code point that is not one resolves to, per
/// the HTML5 replacement table.
fn codepoint(n: u32) -> String {
    const INVALID: [(u32, char); 27] = [
        (0x00, '\u{fffd}'), (0x0d, '\r'), (0x80, '\u{20ac}'), (0x81, '\u{81}'), (0x82, '\u{201a}'),
        (0x83, '\u{192}'), (0x84, '\u{201e}'), (0x85, '\u{2026}'), (0x86, '\u{2020}'), (0x87, '\u{2021}'),
        (0x88, '\u{2c6}'), (0x89, '\u{2030}'), (0x8a, '\u{160}'), (0x8b, '\u{2039}'), (0x8c, '\u{152}'),
        (0x8d, '\u{8d}'), (0x8e, '\u{17d}'), (0x8f, '\u{8f}'), (0x90, '\u{90}'), (0x91, '\u{2018}'),
        (0x92, '\u{2019}'), (0x93, '\u{201c}'), (0x94, '\u{201d}'), (0x95, '\u{2022}'), (0x96, '\u{2013}'),
        (0x97, '\u{2014}'), (0x98, '\u{2dc}'),
    ];
    const INVALID2: [(u32, char); 5] = [
        (0x99, '\u{2122}'), (0x9a, '\u{161}'), (0x9b, '\u{203a}'), (0x9c, '\u{153}'), (0x9d, '\u{9d}'),
    ];
    const INVALID3: [(u32, char); 2] = [(0x9e, '\u{17e}'), (0x9f, '\u{178}')];
    for (k, v) in INVALID.iter().chain(INVALID2.iter()).chain(INVALID3.iter()) {
        if *k == n {
            return v.to_string();
        }
    }
    if (0xd800..=0xdfff).contains(&n) || n > 0x10_ffff {
        return "\u{fffd}".to_string();
    }
    if is_invalid_codepoint(n) {
        return String::new();
    }
    char::from_u32(n).map(|c| c.to_string()).unwrap_or_default()
}

/// The code points a document may not carry --
/// the C0 and C1 controls that are not whitespace, and the non-characters.
/// They are dropped rather than replaced.
fn is_invalid_codepoint(n: u32) -> bool {
    (0x1..=0x8).contains(&n)
        || n == 0xb
        || (0xe..=0x1f).contains(&n)
        || (0x7f..=0x9f).contains(&n)
        || (0xfdd0..=0xfdef).contains(&n)
        || matches!(n & 0xffff, 0xfffe | 0xffff)
}

