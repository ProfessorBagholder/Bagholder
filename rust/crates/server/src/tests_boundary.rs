//! The figure path imports none of the earlier crates (`docs/plans/stage-3c-switch.md`,
//! §8): what builds, sends and keeps current the figures document, and what writes
//! the book, uses the book, the engine, the sources and the broker adapters. The
//! market's context is the one door, `wire/context.rs`, until stage 5 moves it.

use std::path::PathBuf;

/// The earlier crates, by the names code uses them under.
const EARLIER: [&str; 4] = ["bagholder_model", "bagholder_store", "bagholder_ws", "bagholder_market"];

/// The figure path: what builds and sends the figures document, keeps the engine
/// current, and writes the book.
const FIGURE_PATH: [&str; 13] = [
    "wire/mod.rs", "wire/build.rs", "wire/figures.rs", "wire/filters.rs",
    "figures.rs", "engine_inputs.rs", "due.rs", "read_sources.rs", "broker_reads.rs",
    "entries.rs", "csv_import.rs", "events.rs", "http/model.rs",
];

/// Each earlier crate a file's code names, outside its tests.
fn violations(text: &str) -> Vec<&'static str> {
    let code = text.split("#[cfg(test)]").next().unwrap_or("");
    EARLIER.iter().copied().filter(|c| code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(c))).collect()
}

#[test]
fn nothing_on_the_figure_path_uses_the_earlier_crates() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for f in FIGURE_PATH {
        let text = std::fs::read_to_string(src.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(violations(&text), Vec::<&str>::new(), "{f} uses an earlier crate");
    }
    // every file of the wire is on it, but the context's door
    for entry in std::fs::read_dir(src.join("wire")).unwrap() {
        let name = format!("wire/{}", entry.unwrap().file_name().to_string_lossy());
        assert!(FIGURE_PATH.contains(&name.as_str()) || name == "wire/context.rs", "{name} is in the wire and not on the figure path's list");
    }
}

#[test]
fn the_check_finds_an_earlier_crate_named_in_code_and_not_in_a_comment_or_a_test() {
    assert_eq!(violations("use bagholder_model::base::Base;\nfn f() {}"), vec!["bagholder_model"]);
    assert_eq!(violations("fn f() { bagholder_store::tables::accounts(c); bagholder_ws::x(); }"), vec!["bagholder_store", "bagholder_ws"]);
    assert!(violations("// once read bagholder_model's view\nfn f() {}").is_empty());
    assert!(violations("fn f() {}\n#[cfg(test)]\nmod tests { use bagholder_market::x; }").is_empty());
}

/// What reaches Wealthsimple's order mutations in a file's code, outside its tests: the
/// operations named, or the client's one-shot `mutate` called.
fn mutations(text: &str) -> Vec<String> {
    let code = text.split("#[cfg(test)]").next().unwrap_or("");
    let mut out = Vec::new();
    for l in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for op in ["SoOrdersOrderCreate", "SoOrdersOrderCancel", "SoOrdersOrderModify", ".mutate("] {
            if l.contains(op) {
                out.push(op.to_string());
            }
        }
    }
    out
}

fn server_sources() -> Vec<(String, String)> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut dirs = vec![src.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.file_name().unwrap().to_string_lossy().starts_with("tests") {
                out.push((p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out
}

#[test]
fn no_order_mutation_reaches_wealthsimple_but_through_the_gate() {
    for (f, text) in server_sources() {
        let found = mutations(&text);
        match f.as_str() {
            "orders/gate.rs" => assert!(!found.is_empty(), "the gate sends them"),
            // the read path names them only to refuse them
            "orders/tools.rs" => assert!(!found.contains(&".mutate(".to_string())),
            _ => assert_eq!(found, Vec::<String>::new(), "{f} reaches an order mutation outside the gate"),
        }
    }
    assert_eq!(mutations("fn f() { c.mutate(s, \"SoOrdersOrderCancel\", &v) }"), vec!["SoOrdersOrderCancel".to_string(), ".mutate(".to_string()]);
    assert!(mutations("// SoOrdersOrderCreate\nfn f() {}\n#[cfg(test)]\nmod t { fn g() { x.mutate(1) } }").is_empty());
}

/// A list the order code acts on is never cut at a count: `LIMIT` only in the page's own
/// paging of the book's orders and brackets.
fn limits(text: &str) -> usize {
    text.split("#[cfg(test)]").next().unwrap_or("").lines().filter(|l| !l.trim_start().starts_with("//") && l.contains(" LIMIT ")).count()
}

#[test]
fn no_list_the_order_code_acts_on_is_cut_at_a_count() {
    for (f, text) in server_sources().into_iter().filter(|(f, _)| f.starts_with("orders/")) {
        assert_eq!(limits(&text), 0, "{f} cuts a list at a count");
    }
    let book = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../book/src/orders.rs")).unwrap();
    // `orders_before` and `brackets_where`'s paged arm: the page's paging, nothing else
    assert_eq!(limits(&book), 2, "a LIMIT in the book's orders beyond the page's own paging");
    assert_eq!(limits("fn f() { q(\"SELECT * FROM orders LIMIT 200\") }"), 1);
}

#[test]
fn the_order_path_reads_nothing_leniently_and_nothing_from_the_earlier_store() {
    for (f, text) in server_sources().into_iter().filter(|(f, _)| f.starts_with("orders/") || f == "http/orders.rs") {
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        for word in ["lenient", "page_num", "bagholder_store"] {
            assert!(!code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(word)), "{f} uses {word}");
        }
    }
}
