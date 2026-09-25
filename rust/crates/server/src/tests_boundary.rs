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
