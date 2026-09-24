//! None of the old app's guesses is in the new build (`docs/old-app-mistakes.md`,
//! every entry marked "scan"): a quantity worked out from cash, a split ratio
//! read from fill prices, a renamed ticker found by its symbol, a contract taken
//! to be 100 shares, option terms read from a symbol's text, a tolerance for
//! coins sold beyond what was held, a residue dropped, a fallback rate, a
//! position marked at a fill, a payout frequency worked out from payments, rows
//! relabelled or read leniently, floating-point money. Each has a case or a test
//! holding what replaced it; this reads the new crates' sources for the old names
//! and shapes, a tripwire beside those.

use std::path::Path;

fn sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push((p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    // the new build's crates; the old ones (`model`, `store`, `market`, `ws`,
    // `server`) go at the switch
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut out = Vec::new();
    for c in ["core", "book", "engine", "sources"] {
        let before = out.len();
        walk(&crates.join(c).join("src"), &mut out);
        assert!(out.len() > before, "no sources read for {c}");
    }
    out
}

/// Each line of code in the new crates, comments stripped, with where it is.
fn code_lines() -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for (path, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("").to_string();
            out.push((path.clone(), n + 1, code));
        }
    }
    out
}

#[test]
fn the_old_guesses_are_not_in_the_engine() {
    // the old engine's own names for them, and the constants they used
    let banned = [
        ("infer_zero_qty_option_fills", "a contract count worked out from cash"),
        ("is_clean_option_qty", "a contract count worked out from cash"),
        ("split_markers", "a split ratio read from fill prices"),
        ("ticker_was_replaced", "a renamed holding found by its symbol"),
        ("replacement_index", "a renamed holding found by its symbol"),
        ("option_multiplier", "a contract taken to be 100 shares"),
        ("from_int(100)", "a contract taken to be 100 shares"),
        ("FX_FALLBACK", "a made-up rate"),
        ("1.35", "a made-up rate"),
        ("fn dust", "a tolerance for selling beyond what was held"),
        ("0.01 *", "a tolerance for selling beyond what was held"),
        ("last_fill", "a position marked at the person's own fill"),
        ("LastFill", "a position marked at the person's own fill"),
        ("fold_option_rolls", "separate same-day orders folded into a roll"),
        ("< 1.0)", "a residue under a dollar dropped"),
        ("option_symbol", "option terms read from a symbol's text"),
        ("payments_per_year", "a payout frequency worked out from payments"),
        ("relabel", "broker rows rewritten"),
        ("lenient", "a bad row read as something rather than refused"),
    ];
    let mut found = Vec::new();
    for (path, n, code) in code_lines() {
        for (pattern, what) in banned {
            if code.contains(pattern) {
                found.push(format!("{path}:{n}: {what} ({pattern})"));
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn money_is_never_a_float() {
    // a float only for the statistics (ratios, returns), where `Dec::to_f64`
    // hands one over, and in reading the old database, which stored floats
    let allowed = ["engine/src/stat/", "core/src/dec.rs", "book/src/import/old.rs"];
    let found: Vec<String> = code_lines()
        .into_iter()
        .filter(|(_, _, code)| code.contains("f64") || code.contains("f32"))
        .filter(|(path, _, _)| !allowed.iter().any(|a| path.replace('\\', "/").contains(a)))
        .map(|(path, n, _)| format!("{path}:{n}"))
        .collect();
    assert!(found.is_empty(), "a float outside the statistics:\n{}", found.join("\n"));
}
