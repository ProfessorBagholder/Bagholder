//! Every <table> on a page as rows of cell texts, read the way Python's
//! `html.parser.HTMLParser` reads a page: tags matched case-insensitively,
//! attribute values in quotes skipped over when finding a tag's end, comments
//! and declarations passed by, script and style bodies taken as raw text, and
//! character references resolved in the text.

use regex::Regex;
use std::sync::OnceLock;

fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// The end of a start tag from `<`, as `locatestarttagend_tolerant` finds it:
/// the first `>` outside a quoted attribute value.
fn tag_end(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => {
                // a quote only opens a value right after `=`, possibly spaced
                let mut j = i;
                while j > from && (b[j - 1] as char).is_ascii_whitespace() {
                    j -= 1;
                }
                if j > from && b[j - 1] == b'=' {
                    quote = Some(c);
                }
            }
            None if c == b'>' => return Some(i),
            None => {}
        }
        i += 1;
    }
    None
}

enum Event {
    Start(String),
    End(String),
    Data(String),
}

fn events(html: &str) -> Vec<Event> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    let mut raw_until: Option<String> = None;
    let flush = |out: &mut Vec<Event>, from: usize, to: usize| {
        if to > from {
            out.push(Event::Data(crate::news::unescape(&html[from..to])));
        }
    };
    while i < b.len() {
        if let Some(close) = raw_until.as_ref() {
            // script and style: text until their own end tag
            let lower = html[i..].to_ascii_lowercase();
            match lower.find(&format!("</{}", close)) {
                Some(p) => {
                    out.push(Event::Data(html[i..i + p].to_string()));
                    i += p;
                    text_start = i;
                    raw_until = None;
                    continue;
                }
                None => return out,
            }
        }
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if is_letter(next) {
            let end = match tag_end(b, i + 1) { Some(e) => e, None => break };
            flush(&mut out, text_start, i);
            let inner = &html[i + 1..end];
            let name_len = inner.bytes().take_while(|c| !c.is_ascii_whitespace() && *c != b'/' && *c != b'>').count();
            let name = inner[..name_len].to_ascii_lowercase();
            let self_closing = inner.trim_end().ends_with('/');
            out.push(Event::Start(name.clone()));
            if self_closing {
                out.push(Event::End(name.clone()));
            } else if name == "script" || name == "style" {
                raw_until = Some(name);
            }
            i = end + 1;
            text_start = i;
            continue;
        }
        if next == b'/' && is_letter(b.get(i + 2).copied().unwrap_or(0)) {
            let end = match html[i..].find('>') { Some(p) => i + p, None => break };
            flush(&mut out, text_start, i);
            let inner = &html[i + 2..end];
            let name_len = inner.bytes().take_while(|c| !c.is_ascii_whitespace() && *c != b'/').count();
            out.push(Event::End(inner[..name_len].to_ascii_lowercase()));
            i = end + 1;
            text_start = i;
            continue;
        }
        if html[i..].starts_with("<!--") {
            let end = match html[i + 4..].find("-->") { Some(p) => i + 4 + p + 3, None => break };
            flush(&mut out, text_start, i);
            i = end;
            text_start = i;
            continue;
        }
        if next == b'!' || next == b'?' {
            let end = match html[i..].find('>') { Some(p) => i + p + 1, None => break };
            flush(&mut out, text_start, i);
            i = end;
            text_start = i;
            continue;
        }
        i += 1;
    }
    // what follows the last tag is held back, as the parser holds it without a
    // close(): no table ends there
    out
}

/// `exposure.html_tables`.
pub fn html_tables(html: &str) -> Vec<Vec<Vec<String>>> {
    static WS: OnceLock<Regex> = OnceLock::new();
    let ws = WS.get_or_init(|| Regex::new(r"\s+").unwrap());
    let mut tables: Vec<Vec<Vec<String>>> = Vec::new();
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut row: Option<Vec<String>> = None;
    let mut cell: Option<String> = None;
    for ev in events(html) {
        match ev {
            Event::Start(t) => match t.as_str() {
                "table" => table = Some(Vec::new()),
                "tr" if table.is_some() => row = Some(Vec::new()),
                "td" | "th" if row.is_some() => cell = Some(String::new()),
                "br" => {
                    if let Some(c) = cell.as_mut() {
                        c.push(' ');
                    }
                }
                _ => {}
            },
            Event::End(t) => match t.as_str() {
                "td" | "th" if cell.is_some() && row.is_some() => {
                    let text = cell.take().unwrap();
                    let collapsed = ws.replace_all(&text, " ");
                    row.as_mut().unwrap().push(bagholder_model::pytext::py_strip(&collapsed).to_string());
                }
                "tr" if row.is_some() && table.is_some() => {
                    let r = row.take().unwrap();
                    if !r.is_empty() {
                        table.as_mut().unwrap().push(r);
                    }
                }
                "table" if table.is_some() => tables.push(table.take().unwrap()),
                _ => {}
            },
            Event::Data(d) => {
                if let Some(c) = cell.as_mut() {
                    c.push_str(&d);
                }
            }
        }
    }
    tables
}
