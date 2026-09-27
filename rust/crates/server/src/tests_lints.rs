//! No failure is discarded outside tests (`docs/plans/stage-5-interface-and-running.md`,
//! B). The compiler holds three forms, through the workspace's clippy lints
//! (`Cargo.toml`, `clippy.toml`): a `Result` dropped with `let _ =`, turned into
//! nothing with `.ok();`, and read as a default with `unwrap_or_default()`; a site
//! that may drop one says why in an `#[expect(.., reason = "..")]` beside it. Clippy
//! has no lint for the other two forms, so they are scanned for here: an
//! `if let Ok(..)` on an I/O call with no `else`, and a `_ = ..;` statement.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The crates' folder.
fn crates() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn test_clippy_refuses_each_form_of_a_discarded_failure() {
    // the fixture crate, built with its violations, under the workspace's lints; a
    // target folder of its own, so the suite's own build is not waited on
    let target = tempfile::tempdir().unwrap();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(crates().join(".."))
        .args(["clippy", "-q", "-p", "bagholder-lint-fixture", "--features", "violations", "--message-format=json", "--target-dir"])
        .arg(target.path())
        .output()
        .expect("cargo runs");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "clippy passed the fixture's violations: the lints are off\n{}", String::from_utf8_lossy(&out.stderr));
    // each refusal, by the lint that made it
    let refused: Vec<String> = said
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-message" && m["message"]["level"] == "error")
        .filter_map(|m| m["message"]["code"]["code"].as_str().map(str::to_string))
        .collect();
    assert_eq!(refused, ["clippy::let_underscore_must_use", "clippy::unused_result_ok", "clippy::disallowed_methods"], "{said}");
}

/// An `if let Ok` whose scrutinee is one of these reaches the disk, the store, a
/// process or a socket: an error there is a failure, and dropping it with no `else`
/// hides it.
const IO_CALLS: [&str; 8] = [".get()", ".open()", "connect(", "std::fs::", "fs::", "try_wait()", "File::", "read_dir("];

/// Places the two scanned forms stand, by file, with why each may: the scan
/// fails when a file has more or fewer than listed.
const LISTED: [(&str, usize, &str); 1] = [
    ("server/src/main.rs", 2, "`tokio::select!`'s arms (`_ = ctrl_c() => {}`), not an assignment: the signal that stops the app"),
];

/// `text` with its comments and string literals blanked, lengths kept, cut at
/// its tests.
fn code(text: &str) -> String {
    let text = text.split("#[cfg(test)]").next().unwrap_or("");
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                let n = if b[i] == b'\\' { 2 } else { 1 };
                for k in i..(i + n).min(b.len()) {
                    if b[k] != b'\n' {
                        out[k] = b' ';
                    }
                }
                i += n;
            }
            i += 1;
        } else if b[i] == b'\'' && i + 2 < b.len() && (b[i + 2] == b'\'' || b[i + 1] == b'\\') {
            // a char literal: `'{'` must not count as a brace
            let from = if b[i + 1] == b'\\' { i + 3 } else { i + 1 };
            let end = b[from..].iter().position(|&c| c == b'\'').map_or(b.len(), |p| from + p);
            for k in i + 1..end {
                out[k] = b' ';
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("blanking keeps the text UTF-8")
}

/// The index just past the bracket that closes the one opening at `open`.
fn closing(s: &[u8], open: usize) -> usize {
    let (o, c) = (s[open], if s[open] == b'(' { b')' } else { b'}' });
    let mut depth = 0;
    for (k, &ch) in s.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return k + 1;
            }
        }
    }
    s.len()
}

/// Each line of `text`'s code holding a scanned form.
fn scanned(text: &str) -> Vec<usize> {
    let s = code(text);
    let b = s.as_bytes();
    let line = |at: usize| s[..at].matches('\n').count() + 1;
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(p) = s[from..].find("if let Ok(") {
        let at = from + p;
        from = at + 1;
        let pattern_end = closing(b, at + "if let Ok".len());
        let Some(brace) = s[pattern_end..].find('{').map(|q| pattern_end + q) else { break };
        let scrutinee = &s[pattern_end..brace];
        let after = s[closing(b, brace)..].trim_start();
        if IO_CALLS.iter().any(|c| scrutinee.contains(c)) && !after.starts_with("else") {
            found.push(line(at));
        }
    }
    for (i, l) in s.lines().enumerate() {
        if l.trim_start().starts_with("_ = ") {
            found.push(i + 1);
        }
    }
    found.sort();
    found
}

#[test]
fn test_the_scan_finds_each_form_and_not_what_handles_its_failure() {
    // an I/O `if let Ok` with no `else`, on each kind of call
    for io in ["pool.get()", "app.open()", "Ws::connect(&u, d)", "std::fs::metadata(p)", "p.try_wait()", "std::fs::read_dir(d)"] {
        assert_eq!(scanned(&format!("fn f() {{\n    if let Ok(x) = {io} {{\n        use_it(x);\n    }}\n}}")), vec![2], "{io}");
    }
    assert_eq!(scanned("fn f() {\n    _ = tx.send(1);\n}"), vec![2]);
    // handled: an `else`, a parsed reply's optional field, a comment, a string, a test
    assert!(scanned("fn f() { if let Ok(c) = pool.get() { a(c); } else { b(); } }").is_empty());
    assert!(scanned("fn f() { if let Ok(id) = node.text(\"id\") { a(id); } }").is_empty());
    assert!(scanned("// if let Ok(c) = pool.get() { a(c); }\nfn f() {}").is_empty());
    assert!(scanned("fn f() { let s = \"if let Ok(c) = pool.get() { }\"; }").is_empty());
    assert!(scanned("fn f() {}\n#[cfg(test)]\nmod tests { fn g() { if let Ok(c) = pool.get() { a(c); } } }").is_empty());
    // a nested block and a brace in a char literal do not end the scan early
    assert!(scanned("fn f() { if let Ok(c) = app.open() { if x { y('}'); } } else { z(); } }").is_empty());
}

/// Every `.rs` file of the crates outside tests, by its path under the crates' folder.
fn sources(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if p.is_dir() {
            if name != "tests" && name != "target" && name != "lint-fixture" {
                sources(&p, root, out);
            }
        } else if name.ends_with(".rs") && !name.starts_with("tests") {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, std::fs::read_to_string(&p).unwrap()));
        }
    }
}

#[test]
fn test_no_io_failure_is_dropped_by_an_if_let_or_an_underscore() {
    let root = crates();
    let mut files = Vec::new();
    sources(&root, &root, &mut files);
    let mut found: Vec<(String, usize)> = files.iter().map(|(f, t)| (f.clone(), scanned(t).len())).filter(|(_, n)| *n > 0).collect();
    found.sort();
    let want: Vec<(String, usize)> = LISTED.iter().map(|(f, n, _)| (f.to_string(), *n)).collect();
    assert_eq!(found, want, "an I/O failure dropped by `if let Ok` with no `else`, or a `_ = ..;`: handle it, or argue for it in LISTED");
}
