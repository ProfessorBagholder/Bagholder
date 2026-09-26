//! The market instruments the watchlist can follow beside listings: indices,
//! futures, rates and currency pairs, each with the code Yahoo's chart
//! endpoint quotes it under.
//!
//! Nothing here is traded from the app. The directory exists so the ⌘K list
//! finds `WTI`, `NDX` or `VIX` and the watchlist can quote them.

use serde_json::{json, Value};

pub use bagholder_core::directory::{find, kind_label, label, Entry as Instrument, INSTRUMENTS, KIND_LABEL};
use bagholder_core::directory::RATE_FROM_PRICE;

/// `implied_rate`: the rate such a contract is pricing -- the
/// contract's own definition, not a reading of it.
pub fn implied_rate(symbol: &str, price: Option<f64>) -> Option<f64> {
    let sym = symbol.trim().to_uppercase();
    if !RATE_FROM_PRICE.contains(&sym.as_str()) {
        return None;
    }
    let p = price?;
    Some(((100.0 - p) * 1e4).round() / 1e4)
}



pub fn rows() -> Vec<Value> {
    INSTRUMENTS
        .iter()
        .map(|r| {
            json!({
                "symbol": r.symbol, "name": r.name, "kind": r.kind, "exchange": r.exchange,
                "currency": r.currency, "yahoo": r.yahoo, "aliases": r.aliases,
            })
        })
        .collect()
}


/// `search`: an exact symbol or alias first, then one starting
/// with the text, then one with a word starting with it.
///
/// A single letter matches only an exact symbol, so `V` finds Visa's listings
/// and not every index with a V in it.
pub fn search(text: &str) -> Vec<crate::wire::SymbolMatch> {
    let q = text.trim().to_uppercase();
    if q.is_empty() {
        return vec![];
    }
    let mut out: Vec<(u8, crate::wire::SymbolMatch)> = Vec::new();
    for r in INSTRUMENTS {
        let mut names: Vec<String> = vec![r.symbol.to_string()];
        names.extend(r.aliases.iter().map(|a| a.to_uppercase()));
        let upper_name = r.name.to_uppercase();

        let mut words: Vec<String> = Vec::new();
        for x in names.iter().cloned().chain(std::iter::once(upper_name.clone())) {
            for w in x.replace('/', " ").split_whitespace() {
                words.push(w.to_string());
            }
        }

        let rank: i8 = if names.contains(&q) {
            0
        } else if q.chars().count() < 2 {
            -1
        } else if names.iter().chain(std::iter::once(&upper_name)).any(|x| x.starts_with(&q)) {
            1
        } else if words.iter().any(|w| w.starts_with(&q)) {
            2
        } else {
            -1
        };
        if rank >= 0 {
            out.push((
                rank as u8,
                crate::wire::SymbolMatch {
                    symbol: r.symbol.to_string(), name: r.name.to_string(), exchange: r.exchange.to_string(),
                    currency: r.currency.to_string(), kind: Some(r.kind.to_string()), rank: Some(rank as f64),
                },
            ));
        }
    }
    // a stable sort on the rank alone
    out.sort_by_key(|(rank, _)| *rank);
    out.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `crates/model/tests/instruments.rs` covers `search`'s ranking by symbol;
    /// this is the one thing it does not check, the numeric rank itself.
    #[test]
    fn test_search_carries_the_rank_an_exact_match_beats_a_prefix_beats_a_word() {
        assert_eq!(search("VIX")[0].rank, Some(0.0));
        assert_eq!(search("VOLAT")[0].rank, Some(1.0), "an alias VIX has no symbol prefix for, but starts with it");
    }
}
