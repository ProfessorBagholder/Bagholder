//! Sector and country exposure: `exposure.norm_sector` and
//! `exposure_slices`.
//!
//! A share carries one sector and one country; a fund is looked through to
//! what it holds. Whatever no record covers is named rather than hidden.

use serde_json::Value;
use std::collections::HashMap;

use crate::activity::Kind;
use crate::value::{num, FSum};
use crate::wire::{ExposureSlice, Position};

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

/// What a security is exposed to: weights by sector and by country, in the
/// order the record gives them (which decides a tie for the dominant sector).
#[derive(Clone, Debug, Default)]
pub struct Exposure {
    pub sectors: Vec<(String, f64)>,
    pub countries: Vec<(String, f64)>,
}

impl<'de> serde::Deserialize<'de> for Exposure {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Exposure, D::Error> {
        let v = Value::deserialize(d)?;
        let weights = |key: &str| match v.get(key) {
            Some(Value::Object(m)) => m.iter().map(|(name, w)| (name.clone(), num(Some(w), 0.0))).collect(),
            _ => vec![],
        };
        Ok(Exposure { sectors: weights("sectors"), countries: weights("countries") })
    }
}

/// Exposure records by key: a security's id, `share:<TICKER>:<venue form>` for a
/// listing looked up by ticker, `fund:…` for a fund's own record.
pub type Exposures = HashMap<String, Exposure>;

/// The record an option's exposure is read from: its underlying's share record,
/// on the venue its currency suggests first.
pub fn underlying_exposure<'a>(exposures: &'a Exposures, underlying: &str, currency: &str) -> Option<&'a Exposure> {
    let under = underlying.to_uppercase();
    let us = format!("{}{}::US", crate::venues::SHARE_KEY, under);
    let ca = format!("{}{}:", crate::venues::SHARE_KEY, under);
    let (first, second) = if currency.to_uppercase() == "USD" { (us, ca) } else { (ca, us) };
    exposures.get(&first).or_else(|| exposures.get(&second))
}

/// The open long positions in scope spread by sector and by country, largest
/// first, `Not classified` last.
pub fn exposure_slices(positions: &[&Position], exposures: &Exposures, cad: &dyn Fn(f64, &str) -> f64) -> (Vec<ExposureSlice>, Vec<ExposureSlice>) {
    #[derive(Default)]
    struct Spread {
        total: HashMap<String, f64>,
        order: Vec<String>,
        unclassified: f64,
    }
    impl Spread {
        fn add(&mut self, name: String, amount: f64) {
            if !self.total.contains_key(&name) {
                self.order.push(name.clone());
            }
            *self.total.entry(name).or_insert(0.0) += amount;
        }
        fn rows(self, all: f64) -> Vec<ExposureSlice> {
            let mut out: Vec<(String, f64)> = self.order.into_iter().filter_map(|n| self.total.get(&n).copied().filter(|v| *v > 0.0).map(|v| (n, v))).collect();
            out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if self.unclassified > 0.005 {
                out.push((UNCLASSIFIED.to_string(), self.unclassified));
            }
            out.into_iter().map(|(name, value)| ExposureSlice { name, value, share: if all != 0.0 { value / all } else { 0.0 } }).collect()
        }
    }
    let (mut sectors, mut countries) = (Spread::default(), Spread::default());
    let mut total = 0.0_f64;
    let nothing = Exposure::default();
    // a coin is its own sector and no country's
    let coin = Exposure { sectors: vec![("Digital assets".to_string(), 1.0)], countries: vec![] };

    for p in positions {
        // the same positions and values as Allocation: every one worth something
        let v = cad(p.mv, &p.currency);
        if v <= 0.0 {
            continue;
        }
        total += v;
        let record = match p.kind {
            Kind::Crypto => &coin,
            // a contract is its underlying's exposure, under the share's record
            Kind::Options => underlying_exposure(exposures, &p.underlying, &p.currency).unwrap_or(&nothing),
            _ => exposures.get(&p.security_id).unwrap_or(&nothing),
        };
        for (name, w) in &record.sectors {
            // a record read before an alias was known folds here
            let known = norm_sector(name);
            sectors.add(if known.is_empty() { name.clone() } else { known }, v * w);
        }
        for (name, w) in &record.countries {
            countries.add(name.clone(), v * w);
        }
        let covered = |weights: &[(String, f64)]| f64::min(1.0, weights.iter().map(|(_, w)| *w).fsum());
        sectors.unclassified += v * f64::max(0.0, 1.0 - covered(&record.sectors));
        countries.unclassified += v * f64::max(0.0, 1.0 - covered(&record.countries));
    }
    (sectors.rows(total), countries.rows(total))
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

/// A regex word character over Unicode text: a Unicode letter or digit, or `_`.
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
