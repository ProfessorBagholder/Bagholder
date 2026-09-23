//! None of the old engine's guesses is in this one (`docs/plans/stage-2-engine.md`,
//! acceptance): a quantity worked out from cash, a split ratio read from fill
//! prices, a renamed ticker found by its symbol, a contract taken to be 100
//! shares, a tolerance for coins sold beyond what was held, a residue dropped, a
//! fallback rate, a position marked at a fill. Each has a case holding what
//! replaced it (`tests/cases`); this reads the sources for the old names and
//! shapes, a tripwire beside those cases.

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
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
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
    ];
    let mut found = Vec::new();
    for (path, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for (pattern, what) in banned {
                if code.contains(pattern) {
                    found.push(format!("{path}:{}: {what} ({pattern})", n + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
