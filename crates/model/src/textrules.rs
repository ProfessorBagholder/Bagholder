//! Python's own readings of text, where the Rust standard library reads the
//! same text differently: `float()`, `str.splitlines()`, and the `csv`
//! module's reader. Every figure imported or published passes through one of
//! these, so they are CPython's rules exactly, not approximations of them.

use serde_json::{json, Map, Value};

/// `csv.field_size_limit()`'s default.
const FIELD_LIMIT: usize = 131072;

/// The zero of every run of Unicode decimal digits, generated from Python's
/// `unicodedata` (unicode 15.0.0): Python's `float()` reads any of them as the
/// digit it is.
const DIGIT_ZEROS: [u32; 68] = [0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0, 0xff10, 0x104a0, 0x10d30, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0, 0x11650, 0x116c0, 0x11730, 0x118e0, 0x11950, 0x11c50, 0x11d50, 0x11da0, 0x11f50, 0x16a60, 0x16ac0, 0x16b50, 0x1d7ce, 0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e950, 0x1fbf0];

/// Python's whitespace for `str.strip()` and `float()`.
pub fn is_space_char(c: char) -> bool {
    matches!(c as u32, 0x9 | 0xa | 0xb | 0xc | 0xd | 0x1c | 0x1d | 0x1e | 0x1f | 0x20 | 0x85 | 0xa0 | 0x1680 | 0x2000 | 0x2001 | 0x2002 | 0x2003 | 0x2004 | 0x2005 | 0x2006 | 0x2007 | 0x2008 | 0x2009 | 0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000)
}

pub fn ascii_digits(text: &str) -> String {
    text.chars()
        .map(|c| {
            let cp = c as u32;
            match DIGIT_ZEROS.iter().find(|z| cp >= **z && cp < **z + 10) {
                Some(z) => char::from(b'0' + (cp - z) as u8),
                None => c,
            }
        })
        .collect()
}

/// `float()` on a string, as Python reads one: surrounding whitespace, a sign,
/// `inf`/`infinity`/`nan` in any case, underscores between digits, and any
/// script's decimal digits.
pub fn parse_float(text: &str) -> Option<f64> {
    let folded = ascii_digits(text);
    let t = folded.trim_matches(is_space_char);
    if t.is_empty() {
        return None;
    }
    let b = t.as_bytes();
    let mut clean = String::with_capacity(t.len());
    for (i, ch) in t.char_indices() {
        if ch == '_' {
            let before = i > 0 && b[i - 1].is_ascii_digit();
            let after = i + 1 < b.len() && b[i + 1].is_ascii_digit();
            if !(before && after) {
                return None;
            }
            continue;
        }
        clean.push(ch);
    }
    let body = clean.trim_start_matches(['+', '-']);
    let lower = body.to_ascii_lowercase();
    if clean.len() - body.len() > 1 {
        return None;
    }
    let neg = clean.starts_with('-');
    let v = match lower.as_str() {
        "inf" | "infinity" => f64::INFINITY,
        "nan" => f64::NAN,
        _ => {
            // Rust also takes these spellings of its own; Python does not
            if !body.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
                return None;
            }
            return clean.parse::<f64>().ok();
        }
    };
    Some(if neg { -v } else { v })
}


/// `float()` on a JSON value's text, None where Python raises.
pub fn float_value(v: &Value) -> Option<f64> {
    match v {
        Value::String(t) => parse_float(t),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// `str.splitlines()`: every line boundary Python knows.
pub fn splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = text.char_indices().peekable();
    while let Some((i, ch)) = it.next() {
        let boundary = matches!(ch, '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if boundary {
            out.push(&text[start..i]);
            let mut next = i + ch.len_utf8();
            if ch == '\r' {
                if let Some((_, '\n')) = it.peek() {
                    it.next();
                    next += 1;
                }
            }
            start = next;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// `csv.reader(io.StringIO(text))` over the excel dialect, as CPython's
/// reader walks it: the text is read a line at a time on `\n`; a quoted field
/// may run across lines and ends at its closing quote, whatever follows it on
/// the line joining the field; a `\r` outside quotes must end its line, and
/// Python refuses the file where it does not; a line with nothing on it is an
/// empty row.
pub fn csv_rows(text: &str) -> Result<Vec<Vec<String>>, String> {
    #[derive(PartialEq)]
    enum St { StartRecord, StartField, InField, InQuoted, QuoteInQuoted, EatCrnl }
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut st = St::StartRecord;

    for line in text.split_inclusive('\n') {
        for c in line.chars() {
            if field.chars().count() >= FIELD_LIMIT && matches!(st, St::InField | St::InQuoted) {
                return Err("field larger than field limit (131072)".into());
            }
            // one character, as `parse_process_char` takes it
            loop {
                match st {
                    St::StartRecord => {
                        if c == '\n' || c == '\r' {
                            st = St::EatCrnl;
                            break;
                        }
                        st = St::StartField;
                        continue;
                    }
                    St::StartField => {
                        match c {
                            '\n' | '\r' => {
                                fields.push(std::mem::take(&mut field));
                                st = St::EatCrnl;
                            }
                            '"' => st = St::InQuoted,
                            ',' => fields.push(std::mem::take(&mut field)),
                            _ => {
                                field.push(c);
                                st = St::InField;
                            }
                        }
                        break;
                    }
                    St::InField => {
                        match c {
                            '\n' | '\r' => {
                                fields.push(std::mem::take(&mut field));
                                st = St::EatCrnl;
                            }
                            ',' => {
                                fields.push(std::mem::take(&mut field));
                                st = St::StartField;
                            }
                            _ => field.push(c),
                        }
                        break;
                    }
                    St::InQuoted => {
                        if c == '"' { st = St::QuoteInQuoted } else { field.push(c) }
                        break;
                    }
                    St::QuoteInQuoted => {
                        match c {
                            '"' => {
                                field.push('"');
                                st = St::InQuoted;
                            }
                            ',' => {
                                fields.push(std::mem::take(&mut field));
                                st = St::StartField;
                            }
                            '\n' | '\r' => {
                                fields.push(std::mem::take(&mut field));
                                st = St::EatCrnl;
                            }
                            _ => {
                                field.push(c);
                                st = St::InField;
                            }
                        }
                        break;
                    }
                    St::EatCrnl => {
                        if c == '\n' || c == '\r' {
                            break;
                        }
                        return Err("new-line character seen in unquoted field".into());
                    }
                }
            }
        }
        // the end of a line
        match st {
            St::InQuoted | St::QuoteInQuoted if st == St::InQuoted => {
                // the field runs on to the next line
                continue;
            }
            St::QuoteInQuoted => {
                fields.push(std::mem::take(&mut field));
            }
            St::StartRecord => {}
            St::InField | St::StartField => {
                fields.push(std::mem::take(&mut field));
            }
            St::EatCrnl | St::InQuoted => {}
        }
        records.push(std::mem::take(&mut fields));
        st = St::StartRecord;
    }
    // text that ends inside a quoted field: what was read of it is the field
    if st == St::InQuoted {
        fields.push(std::mem::take(&mut field));
        records.push(std::mem::take(&mut fields));
    }

    Ok(records)
}

/// `csv.DictReader(io.StringIO(text))`: the first row names the fields; a
/// line with nothing on it is no row, and a short row's missing fields are
/// None.
pub fn csv_records(text: &str) -> Result<Vec<Map<String, Value>>, String> {
    let mut it = csv_rows(text)?.into_iter();
    let names = loop {
        match it.next() {
            Some(h) => break h,
            None => return Ok(vec![]),
        }
    };
    Ok(it
        .filter(|r| !r.is_empty())
        .map(|r| {
            let mut m = Map::new();
            for (i, name) in names.iter().enumerate() {
                m.insert(name.clone(), r.get(i).map(|v| json!(v)).unwrap_or(Value::Null));
            }
            m
        })
        .collect())
}


/// `str.strip()`: Python's whitespace from both ends.
pub fn trim_space(text: &str) -> &str {
    text.trim_matches(is_space_char)
}

/// `int()` on digits a pattern's `\d` matched, in whatever script they are.
pub fn parse_int(text: &str) -> Option<i64> {
    ascii_digits(trim_space(text)).parse().ok()
}

/// `uuid.uuid4()`, from the system's own randomness.
pub fn uuid4() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if read.is_err() {
        // no /dev/urandom: the clock and the process, which is unique enough
        // for an id no two rows of one import share
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = n ^ ((std::process::id() as u128) << 64);
        b.copy_from_slice(&seed.to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{:02x}", x)).collect();
    format!("{}-{}-{}-{}-{}", h[0..4].concat(), h[4..6].concat(), h[6..8].concat(), h[8..10].concat(), h[10..16].concat())
}
