//! No path from the history to an amount (brief 79 §1,
//! `docs/plans/every-amount-as-wealthsimple-states-it.md` §5, test 2): the one
//! function that builds the holdings, `bagholder_engine::stated::holdings`, reads
//! what the broker states, the instruments' terms and the market alone. This
//! reads `engine/src/stated.rs`, comments aside, for every name of the history:
//! a record, a transaction, a lot, a round trip, the matcher's result or the
//! journal. One of them in its code is a way for the journal to become an amount.

use std::path::Path;

/// The names of the history, as the engine's types and fields call them.
const HISTORY: [&str; 22] = [
    "Matched", "matched", "Lot", "Lots", "lots", "Trip", "TripKey", "trips", "trip_of", "Transaction", "Transactions", "transactions", "Record", "Records", "records", "SourceRecord", "Identity", "identity", "journal", "JournalSubject", "Continued", "Reach",
];

/// Each word of the file's code, comments stripped, with its line.
fn code_words(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or_default();
        for w in code.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()) {
            out.push((n + 1, w.to_string()));
        }
    }
    out
}

fn named_history(text: &str) -> Vec<String> {
    code_words(text).into_iter().filter(|(_, w)| HISTORY.contains(&w.as_str())).map(|(n, w)| format!("stated.rs:{n} {w}")).collect()
}

#[test]
fn the_holdings_are_built_from_no_part_of_the_history() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/stated.rs")).unwrap();
    assert!(text.contains("pub fn holdings<'a>(of: &Sources, stated: impl IntoIterator<Item = Stated<'a>>, as_of: Option<Date>) -> Vec<Holding>"), "the scan reads the function that builds the holdings");
    assert!(code_words(&text).len() > 500, "the scan reads the file's code");
    assert_eq!(named_history(&text), Vec::<String>::new(), "the holdings name a part of the history: an amount can come from the journal");
}

/// The scan finds the history named, a parameter of `holdings` or a field of
/// what it reads.
#[test]
fn the_scan_finds_the_history_named() {
    assert_eq!(named_history("pub fn holdings(of: &Sources, matched: &Matched) {}"), ["stated.rs:1 matched", "stated.rs:1 Matched"]);
    assert_eq!(named_history("    let q = inputs.ledger.transactions.len();"), ["stated.rs:1 transactions"]);
    assert_eq!(named_history("// the journal's lots, said in a comment"), Vec::<String>::new());
}
