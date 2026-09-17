//! Sector and country exposure: `exposure.norm_sector` and
//! `model.exposure_slices`.
//!
//! A share carries one sector and one country; a fund is looked through to
//! what it holds. Whatever no record covers is named rather than hidden.

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::value::{field_s, get, num};

pub const UNCLASSIFIED: &str = "Not classified";

const SECTOR_ALIAS: &[(&str, &str)] = &[
    ("technology", "Information Technology"),
    ("information technology", "Information Technology"),
    ("tech", "Information Technology"),
    ("financial", "Financials"),
    ("financials", "Financials"),
    ("financial services", "Financials"),
    ("finance", "Financials"),
    ("banks", "Financials"),
    ("health care", "Health Care"),
    ("healthcare", "Health Care"),
    ("consumer discretionary", "Consumer Discretionary"),
    ("consumer cyclicals", "Consumer Discretionary"),
    ("consumer, cyclical", "Consumer Discretionary"),
    ("consumer cyclical", "Consumer Discretionary"),
    ("consumer staples", "Consumer Staples"),
    ("consumer non-cyclicals", "Consumer Staples"),
    ("consumer, non-cyclical", "Consumer Staples"),
    ("consumer non-cyclical", "Consumer Staples"),
    ("consumer defensive", "Consumer Staples"),
    ("industrials", "Industrials"),
    ("industrial", "Industrials"),
    ("energy", "Energy"),
    ("materials", "Materials"),
    ("basic materials", "Materials"),
    ("utilities", "Utilities"),
    ("real estate", "Real Estate"),
    ("realestate", "Real Estate"),
    ("communication services", "Communication Services"),
    ("communications", "Communication Services"),
    ("communication", "Communication Services"),
    ("media", "Communication Services"),
    ("telecommunications services", "Communication Services"),
    ("telecommunications", "Communication Services"),
    ("telecommunication services", "Communication Services"),
    ("bitcoin holding", "Digital assets"),
    ("digital assets", "Digital assets"),
    ("cryptocurrency", "Digital assets"),
    ("crypto", "Digital assets"),
    ("cash and/or derivatives", ""),
    ("cash", ""),
    ("other", ""),
    ("miscellaneous", ""),
    ("-", ""),
    ("n/a", ""),
];

/// `exposure.norm_sector`: the sector under the name the Portfolio uses, empty
/// for none (cash, other, blank).
pub fn norm_sector(name: &str) -> String {
    let key = name.trim().to_lowercase();
    if key.is_empty() {
        return String::new();
    }
    if let Some((_, v)) = SECTOR_ALIAS.iter().find(|(k, _)| *k == key) {
        return (*v).to_string();
    }
    name.trim().to_string()
}

fn weight_map(v: Option<&Value>) -> Vec<(String, f64)> {
    match v {
        Some(Value::Object(m)) => m.iter().map(|(k, x)| (k.clone(), num(Some(x), 0.0))).collect(),
        _ => vec![],
    }
}

/// `model.exposure_slices`: the open long positions in scope spread by sector
/// and by country, largest first, `Not classified` last.
pub fn exposure_slices(
    positions: &[Value],
    exposures: &serde_json::Map<String, Value>,
    cad: &dyn Fn(f64, &str) -> f64,
) -> (Vec<Value>, Vec<Value>) {
    let mut sec_tot: HashMap<String, f64> = HashMap::new();
    let mut cty_tot: HashMap<String, f64> = HashMap::new();
    let mut sec_order: Vec<String> = Vec::new();
    let mut cty_order: Vec<String> = Vec::new();
    let mut sec_unc = 0.0_f64;
    let mut cty_unc = 0.0_f64;
    let mut total = 0.0_f64;

    for p in positions {
        // the same positions and values as Allocation: every one worth something
        let v = cad(num(get(p, "mv"), 0.0), &field_s(p, "currency"));
        if v <= 0.0 {
            continue;
        }
        total += v;
        let kind = field_s(p, "kind");
        let mut rec = exposures.get(&field_s(p, "securityId")).cloned().unwrap_or(Value::Null);

        if kind == "Options" {
            // a contract is its underlying's exposure, under the share's record
            let under = field_s(p, "underlying").to_uppercase();
            // exposure.share_exposure's keys: the ticker, then the venue form
            let us = format!("share:{}::US", under);
            let ca = format!("share:{}:", under);
            let (first, second) = if field_s(p, "currency").to_uppercase() == "USD" { (us, ca) } else { (ca, us) };
            rec = exposures
                .get(&first)
                .or_else(|| exposures.get(&second))
                .cloned()
                .unwrap_or(Value::Null);
        }

        let (s_map, c_map) = if kind == "Crypto" {
            // a coin is its own sector and no country's
            (vec![("Digital assets".to_string(), 1.0)], vec![])
        } else {
            (weight_map(rec.get("sectors")), weight_map(rec.get("countries")))
        };

        let s_sum: f64 = s_map.iter().map(|(_, w)| *w).fold(0.0, |a, b| a + b);
        let c_sum: f64 = c_map.iter().map(|(_, w)| *w).fold(0.0, |a, b| a + b);
        for (n, w) in &s_map {
            // a record read before an alias was known folds here
            let name = { let x = norm_sector(n); if x.is_empty() { n.clone() } else { x } };
            if !sec_tot.contains_key(&name) {
                sec_order.push(name.clone());
            }
            *sec_tot.entry(name).or_insert(0.0) += v * w;
        }
        for (n, w) in &c_map {
            if !cty_tot.contains_key(n) {
                cty_order.push(n.clone());
            }
            *cty_tot.entry(n.clone()).or_insert(0.0) += v * w;
        }
        sec_unc += v * f64::max(0.0, 1.0 - f64::min(1.0, s_sum));
        cty_unc += v * f64::max(0.0, 1.0 - f64::min(1.0, c_sum));
    }

    let rows = |tot: &HashMap<String, f64>, order: &[String], unc: f64| -> Vec<Value> {
        let mut out: Vec<(String, f64)> = order
            .iter()
            .filter_map(|n| tot.get(n).filter(|v| **v > 0.0).map(|v| (n.clone(), *v)))
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if unc > 0.005 {
            out.push((UNCLASSIFIED.to_string(), unc));
        }
        out.into_iter()
            .map(|(name, value)| json!({"name": name, "value": value, "share": if total != 0.0 { value / total } else { 0.0 }}))
            .collect()
    };
    (rows(&sec_tot, &sec_order, sec_unc), rows(&cty_tot, &cty_order, cty_unc))
}

/// `exposure._ISSUERS`: the fund families named at the start of a fund's name.
const ISSUERS: [(&str, &[&str]); 7] = [
    ("vanguard", &["vanguard"]),
    ("ishares", &["ishares"]),
    ("harvest", &["harvest"]),
    ("ninepoint", &["ninepoint"]),
    ("evolve", &["evolve"]),
    ("bmo", &["bmo"]),
    ("globalx", &["global x", "horizons"]),
];

/// Python's `\w` for a `str` pattern: a Unicode letter or digit, or `_`.
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `text[at..at+len]` stands between word boundaries.
fn bounded(text: &str, at: usize, len: usize) -> bool {
    let before = text[..at].chars().next_back().map(word_char).unwrap_or(false);
    let after = text[at + len..].chars().next().map(word_char).unwrap_or(false);
    let first = text[at..].chars().next().map(word_char).unwrap_or(false);
    let last = text[..at + len].chars().next_back().map(word_char).unwrap_or(false);
    (before != first) && (after != last)
}

/// `exposure.issuer_of`: the fund family a name starts with, or "".
pub fn issuer_of(name: &str) -> &'static str {
    let n = name.trim().to_lowercase();
    for (key, marks) in ISSUERS {
        for m in marks {
            if n.starts_with(m) && bounded(&n, 0, m.len()) {
                return key;
            }
        }
    }
    ""
}

/// `exposure.is_fund`: `\b(ETF|Index|Fund|Portfolio|Trust)\b` anywhere in the
/// name, in any case, or a fund family's name at its start.
pub fn is_fund(name: &str) -> bool {
    // lower-casing can change a string's length; the search runs on an ASCII
    // fold so the boundaries are read on the name itself
    let folded: String = name.chars().map(|c| if c.is_ascii() { c.to_ascii_lowercase() } else { c }).collect();
    for word in ["etf", "index", "fund", "portfolio", "trust"] {
        let mut from = 0;
        while let Some(pos) = folded[from..].find(word) {
            let at = from + pos;
            if bounded(&folded, at, word.len()) {
                return true;
            }
            from = at + 1;
            while !folded.is_char_boundary(from) {
                from += 1;
            }
        }
    }
    !issuer_of(name).is_empty()
}
