//! No failure is discarded outside tests (`docs/plans/stage-5-interface-and-running.md`,
//! B). The compiler holds three forms, through the workspace's clippy lints
//! (`Cargo.toml`, `clippy.toml`): a `Result` dropped with `let _ =`, turned into
//! nothing with `.ok();`, and read as a default with `unwrap_or_default()`; a site
//! that may drop one says why in an `#[expect(.., reason = "..")]` beside it. Clippy
//! has no lint for the other two forms, so they are scanned for here: an
//! `if let Ok(..)` on an I/O call with no `else`, and a `_ = ..;` statement.
//! The same lints refuse an `unsafe` block with no `SAFETY:` comment saying why it
//! is sound, and a `dbg!`, `todo!` or `unimplemented!` left in.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The crates' folder.
fn crates() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Each refusal clippy makes on the fixture crate built with `feature`, by the
/// lint that made it, in the order the fixture holds them.
fn refused_by_clippy(feature: &str) -> Vec<String> {
    // the fixture crate, built with its violations, under the workspace's lints; a
    // target folder of its own, so the suite's own build is not waited on
    let target = tempfile::tempdir().unwrap();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(crates().join(".."))
        .args(["clippy", "-q", "-p", "bagholder-lint-fixture", "--features", feature, "--message-format=json", "--target-dir"])
        .arg(target.path())
        .output()
        .expect("cargo runs");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "clippy passed the fixture's {feature}: the lints are off\n{}", String::from_utf8_lossy(&out.stderr));
    said.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-message" && m["message"]["level"] == "error")
        .filter_map(|m| m["message"]["code"]["code"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn test_clippy_refuses_each_form_of_a_discarded_failure() {
    assert_eq!(refused_by_clippy("violations"), ["clippy::let_underscore_must_use", "clippy::unused_result_ok", "clippy::disallowed_methods"]);
}

#[test]
fn test_clippy_refuses_unsafe_without_its_reason_and_code_left_unfinished() {
    assert_eq!(
        refused_by_clippy("unfinished"),
        ["clippy::undocumented_unsafe_blocks", "clippy::dbg_macro", "clippy::todo", "clippy::unimplemented"]
    );
}

/// An `if let Ok` whose scrutinee is one of these reaches the disk, the store, a
/// process or a socket: an error there is a failure, and dropping it with no `else`
/// hides it.
const IO_CALLS: [&str; 8] = [".get()", ".open()", "connect(", "std::fs::", "fs::", "try_wait()", "File::", "read_dir("];

/// Places the two scanned forms stand, by file, with why each may: the scan
/// fails when a file has more or fewer than listed.
const LISTED: [(&str, usize, &str); 2] = [
    ("server/src/http/mod.rs", 1, "`tokio::select!`'s arm (`_ = async { .. } => Ok(())`), not an assignment: the drain's deadline after a stop"),
    ("server/src/main.rs", 2, "`tokio::select!`'s arms (`_ = ctrl_c() => {}`), not an assignment: the signal that stops the app"),
];

/// `text`'s production code with its comments and string literals blanked,
/// lengths and lines kept.
fn code(text: &str) -> String {
    crate::tests_source::blanked(&crate::tests_source::production(text))
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

/// From the app being made to the server listening, nothing stops the start (#516,
/// brief 50, 6): a service manager would only start it again into the same failure,
/// with no page and no update button. Whatever step is added there, a failure in it
/// is said and the start goes on, so the code there has no early return, no `?`,
/// no exit, and nothing that panics on a failure.
#[test]
fn test_nothing_between_the_app_made_and_the_server_listening_stops_the_start() {
    let main = code(&std::fs::read_to_string(crates().join("server/src/main.rs")).unwrap());
    let serve = &main[main.find("fn serve() -> i32").expect("serve")..];
    let from = serve.find("app::App::new(").expect("the app made in serve");
    let to = serve.find("TcpListener::bind(").expect("the server bound in serve");
    assert!(from < to, "the app is made before the server listens");
    let between = &serve[from..to];
    assert_eq!(stops(between), Vec::<&str>::new(), "a step between the app made and the server listening can stop the start");
    // the scan finds each way a start can stop
    for planted in ["return 1;", "x?;", "std::process::exit(1);", "f.unwrap();", "f.expect(\"x\");", "panic!(\"x\");", "unreachable!();"] {
        assert!(!stops(planted).is_empty(), "{planted}");
    }
    assert!(stops("let x = a.cache(); if let Err(e) = x { failed(e) }").is_empty());
}

/// Each way code can stop a start, as found in `text`.
fn stops(text: &str) -> Vec<&'static str> {
    let ways: [(&str, &str); 7] = [("return", "return "), ("?", "?;"), ("?", "?)"), ("exit", "process::exit("), ("unwrap", ".unwrap()"), ("expect", ".expect("), ("panic", "panic!(")];
    let mut found: Vec<&'static str> = ways.iter().filter(|(_, w)| text.contains(w)).map(|(n, _)| *n).collect();
    if text.contains("unreachable!(") || text.contains("todo!(") || text.contains("unimplemented!(") {
        found.push("panic");
    }
    found.dedup();
    found
}
