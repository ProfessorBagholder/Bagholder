//! A source file as the scans that hold a rule over the server's code read it
//! (brief 25): every item under `#[cfg(test)]` taken out wherever it stands (a
//! module, a function, an import, a field), not the text after the first one,
//! which left most of a file whose first test-only item is near its top unscanned.
//! Lines are kept, so a scan's line numbers are the file's.

/// `text` with its comments and string literals blanked, lengths and lines kept:
/// a brace, a quote or an attribute inside one is not code.
pub(crate) fn blanked(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut [u8], from: usize, to: usize| {
        for c in &mut out[from..to.min(b.len())] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| i + p);
            blank(&mut out, i, end);
            i = end;
        } else if b[i..].starts_with(b"/*") {
            let end = b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len(), |p| i + 2 + p + 2);
            blank(&mut out, i, end);
            i = end;
        } else if b[i] == b'r' && (i == 0 || !is_ident(b[i - 1])) && raw_start(&b[i + 1..]).is_some() {
            // a raw string, `r"…"` or `r#"…"#`: no escapes, closed by the same hashes
            let hashes = raw_start(&b[i + 1..]).unwrap();
            let body = i + 2 + hashes;
            let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
            let end = b[body..].windows(close.len()).position(|w| w == close.as_slice()).map_or(b.len(), |p| body + p);
            blank(&mut out, body, end);
            i = end + close.len();
        } else if b[i] == b'"' {
            let mut k = i + 1;
            while k < b.len() && b[k] != b'"' {
                k += if b[k] == b'\\' { 2 } else { 1 };
            }
            blank(&mut out, i + 1, k);
            i = k + 1;
        } else if b[i] == b'\'' && i + 2 < b.len() && (b[i + 2] == b'\'' || b[i + 1] == b'\\') {
            // a char literal (`'{'` is not a brace); a lifetime has no closing quote
            let from = if b[i + 1] == b'\\' { i + 3 } else { i + 1 };
            let end = b[from..].iter().position(|&c| c == b'\'').map_or(b.len(), |p| from + p);
            blank(&mut out, i + 1, end);
            i = end + 1;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("blanking keeps the text UTF-8")
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The number of hashes of a raw string that starts here (after its `r`).
fn raw_start(s: &[u8]) -> Option<usize> {
    let hashes = s.iter().take_while(|&&c| c == b'#').count();
    (s.get(hashes) == Some(&b'"')).then_some(hashes)
}

/// `text` with every item under `#[cfg(test)]` blanked, lines kept; the rest as
/// written, strings and comments included.
pub(crate) fn production(text: &str) -> String {
    let code = blanked(text);
    let c = code.as_bytes();
    let mut out = text.as_bytes().to_vec();
    let mut from = 0;
    while let Some(p) = code[from..].find("#[cfg(test)]").map(|p| from + p) {
        let end = item_end(c, p);
        for b in &mut out[p..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        from = end;
    }
    String::from_utf8(out).expect("blanking keeps the text UTF-8")
}

/// The end of the item whose attributes start at `at`: past its other
/// attributes, then to the `;` or `,` that ends it, or the brace that closes its
/// body, at its own depth; or up to the bracket that closes what it sits in.
fn item_end(c: &[u8], at: usize) -> usize {
    let mut k = at;
    loop {
        while k < c.len() && c[k].is_ascii_whitespace() {
            k += 1;
        }
        if !c[k..].starts_with(b"#[") {
            break;
        }
        k = closing(c, k + 1);
    }
    let mut depth = 0usize;
    while k < c.len() {
        match c[k] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return k,
            b'}' if depth == 1 => {
                // a body closed; an item written as a value takes its `;` too
                let next = c[k + 1..].iter().position(|b| !b.is_ascii_whitespace()).map(|p| k + 1 + p);
                return match next {
                    Some(n) if c[n] == b';' || c[n] == b',' => n + 1,
                    _ => k + 1,
                };
            }
            b')' | b']' | b'}' => depth -= 1,
            b';' | b',' if depth == 0 => return k + 1,
            _ => {}
        }
        k += 1;
    }
    c.len()
}

/// The index just past the bracket that closes the one opening at `open`.
fn closing(c: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (k, &ch) in c.iter().enumerate().skip(open) {
        match ch {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    c.len()
}

#[test]
fn production_code_keeps_everything_but_the_test_only_items_wherever_they_stand() {
    let text = "use a::b;\n#[cfg(test)]\nuse a::fake;\nstruct S {\n    x: u8,\n    #[cfg(test)]\n    seen: Vec<(u8, u8)>,\n    y: u8\n}\n#[cfg(test)]\n#[derive(Debug)]\nfn helper() -> Result<(), E> { market(b, c) }\nfn real() { market(b, c) }\n#[cfg(test)]\nstatic N: S = S { x: 1 };\n#[cfg(test)]\nmod tests { fn g() { brokers(b) } }\nfn after() { let s = \"#[cfg(test)] }\"; brokers(b) }\n";
    let p = production(text);
    assert_eq!(p.lines().count(), text.lines().count(), "lines kept");
    for kept in ["use a::b;", "x: u8,", "y: u8", "fn real() { market(b, c) }", "fn after() { let s = \"#[cfg(test)] }\"; brokers(b) }"] {
        assert!(p.contains(kept), "{kept} was taken out:\n{p}");
    }
    for gone in ["fake", "seen", "helper", "derive", "static N", "mod tests", "fn g"] {
        assert!(!p.contains(gone), "{gone} was kept:\n{p}");
    }
    // a field last in its struct, with no comma, ends at the struct's brace
    let last = production("struct S { a: u8, #[cfg(test)] b: u8 }\nfn f() {}");
    assert!(last.contains("a: u8,") && !last.contains("b: u8") && last.ends_with("}\nfn f() {}"), "{last}");
    // a brace in a comment, a char or a raw string is not code
    let tricky = "#[cfg(test)]\nfn t() { let _ = '}'; /* } */ let _ = r#\"}\"#; // }\n}\nfn kept() {}";
    assert!(production(tricky).contains("fn kept() {}") && !production(tricky).contains("fn t"));
}

/// Every scan of the server's sources reads them through `production`: none cuts
/// a file at its first `#[cfg(test)]` (the gap brief 25 found).
#[test]
fn no_scan_of_the_sources_cuts_a_file_at_its_first_test_item() {
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let cut = regex::Regex::new(r##"\.(split|split_once|find)\("#\[cfg\(test\)\]"##).unwrap();
    let mut found = Vec::new();
    let mut dirs = vec![src.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().map(Result::unwrap) {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("tests_source.rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                found.extend(text.lines().enumerate().filter(|(_, l)| cut.is_match(l)).map(|(i, _)| format!("{}:{}", p.display(), i + 1)));
            }
        }
    }
    assert!(found.is_empty(), "a scan cuts a file at its first #[cfg(test)]; read it through tests_source::production: {found:?}");
}
