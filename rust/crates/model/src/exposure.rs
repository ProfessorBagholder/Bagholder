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

/// The one set of sectors every name is folded onto (`SPEC.md` portfolio-19).
pub const SECTORS: [&str; 12] = [
    "Information Technology",
    "Financials",
    "Health Care",
    "Consumer Discretionary",
    "Consumer Staples",
    "Industrials",
    "Energy",
    "Materials",
    "Utilities",
    "Real Estate",
    "Communication Services",
    "Digital assets",
];

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
    // TMX Money's name for it
    ("media & telecommunications", "Communication Services"),
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

/// Names a source states that span two sectors of the set, so they place
/// nothing by themselves: Bloomberg's "Consumer, Non-cyclical" holds health care
/// beside staples (Evolve's healthcare fund states it for itself and for every
/// holding: Eli Lilly, Pfizer, Johnson & Johnson, Intuitive Surgical, Thermo
/// Fisher; `sources/tests/replies/evolve/breakdown-life.html`). A fund that
/// states one is placed by its holdings, and a holding by its own listing (#542).
const SPANNING: &[&str] = &["consumer, non-cyclical", "consumer non-cyclical"];

/// Whether a name spans two sectors of the set (`SPANNING`).
pub fn spans_sectors(name: &str) -> bool {
    let key = name.trim().to_lowercase();
    SPANNING.contains(&key.as_str())
}

/// `exposure.norm_sector`: the sector under the name the Portfolio uses, empty
/// for none (cash, other, blank) and for a name spanning two (`spans_sectors`).
pub fn norm_sector(name: &str) -> String {
    let key = name.trim().to_lowercase();
    if key.is_empty() {
        return String::new();
    }
    if let Some((_, v)) = SECTOR_ALIAS.iter().find(|(k, _)| *k == key) {
        return (*v).to_string();
    }
    // one of the set under its own name; any other name is no sector (a fund a
    // fund of funds holds, a word no alias folds): its weight is not classified
    SECTORS.iter().find(|s| s.to_lowercase() == key).map_or_else(String::new, |s| (*s).to_string())
}

/// Whether a name is one a source states for what is no sector (cash, other,
/// blank): its weight is not classified, and it is no unknown name.
pub fn stated_non_sector(name: &str) -> bool {
    let key = name.trim().to_lowercase();
    key.is_empty() || SECTOR_ALIAS.iter().any(|(k, v)| *k == key && v.is_empty())
}

/// Each industry that sits wholly inside one sector, with that sector: the GICS
/// industries (MSCI's GICS structure, effective 2023-03-17), and the names TMX
/// Money and Nasdaq give the industries of the listings recorded in
/// `sources/tests/replies`. An industry two sectors share (TMX's "Agriculture",
/// Nasdaq's "Telecommunications Equipment", which it gives a carrier) is none.
const INDUSTRY_SECTOR: &[(&str, &str)] = &[
    // GICS
    ("energy equipment & services", "Energy"),
    ("oil, gas & consumable fuels", "Energy"),
    ("chemicals", "Materials"),
    ("construction materials", "Materials"),
    ("containers & packaging", "Materials"),
    ("metals & mining", "Materials"),
    ("paper & forest products", "Materials"),
    ("aerospace & defense", "Industrials"),
    ("building products", "Industrials"),
    ("construction & engineering", "Industrials"),
    ("electrical equipment", "Industrials"),
    ("industrial conglomerates", "Industrials"),
    ("machinery", "Industrials"),
    ("trading companies & distributors", "Industrials"),
    ("commercial services & supplies", "Industrials"),
    ("professional services", "Industrials"),
    ("air freight & logistics", "Industrials"),
    ("passenger airlines", "Industrials"),
    ("marine transportation", "Industrials"),
    ("ground transportation", "Industrials"),
    ("transportation infrastructure", "Industrials"),
    ("automobile components", "Consumer Discretionary"),
    ("automobiles", "Consumer Discretionary"),
    ("household durables", "Consumer Discretionary"),
    ("leisure products", "Consumer Discretionary"),
    ("textiles, apparel & luxury goods", "Consumer Discretionary"),
    ("hotels, restaurants & leisure", "Consumer Discretionary"),
    ("diversified consumer services", "Consumer Discretionary"),
    ("distributors", "Consumer Discretionary"),
    ("broadline retail", "Consumer Discretionary"),
    ("specialty retail", "Consumer Discretionary"),
    ("consumer staples distribution & retail", "Consumer Staples"),
    ("beverages", "Consumer Staples"),
    ("food products", "Consumer Staples"),
    ("tobacco", "Consumer Staples"),
    ("household products", "Consumer Staples"),
    ("personal care products", "Consumer Staples"),
    ("health care equipment & supplies", "Health Care"),
    ("health care providers & services", "Health Care"),
    ("health care technology", "Health Care"),
    ("biotechnology", "Health Care"),
    ("pharmaceuticals", "Health Care"),
    ("life sciences tools & services", "Health Care"),
    ("banks", "Financials"),
    ("financial services", "Financials"),
    ("consumer finance", "Financials"),
    ("capital markets", "Financials"),
    ("mortgage real estate investment trusts (reits)", "Financials"),
    ("insurance", "Financials"),
    ("it services", "Information Technology"),
    ("software", "Information Technology"),
    ("communications equipment", "Information Technology"),
    ("technology hardware, storage & peripherals", "Information Technology"),
    ("electronic equipment, instruments & components", "Information Technology"),
    ("semiconductors & semiconductor equipment", "Information Technology"),
    ("diversified telecommunication services", "Communication Services"),
    ("wireless telecommunication services", "Communication Services"),
    ("media", "Communication Services"),
    ("entertainment", "Communication Services"),
    ("interactive media & services", "Communication Services"),
    ("electric utilities", "Utilities"),
    ("gas utilities", "Utilities"),
    ("multi-utilities", "Utilities"),
    ("water utilities", "Utilities"),
    ("independent power and renewable electricity producers", "Utilities"),
    ("diversified reits", "Real Estate"),
    ("industrial reits", "Real Estate"),
    ("hotel & resort reits", "Real Estate"),
    ("office reits", "Real Estate"),
    ("health care reits", "Real Estate"),
    ("residential reits", "Real Estate"),
    ("retail reits", "Real Estate"),
    ("specialized reits", "Real Estate"),
    ("real estate management & development", "Real Estate"),
    // TMX Money's names
    ("hardware & equipment", "Information Technology"),
    ("software & it services", "Information Technology"),
    ("retail - discretionary", "Consumer Discretionary"),
    ("retailers - staples", "Consumer Staples"),
    ("transportation", "Industrials"),
    ("oil & gas storage/transport", "Energy"),
    ("regulated utilities", "Utilities"),
    ("reits", "Real Estate"),
    ("banking", "Financials"),
    ("telecommunications", "Communication Services"),
    // Nasdaq's names
    ("computer manufacturing", "Information Technology"),
    ("catalog/specialty distribution", "Consumer Discretionary"),
    ("aerospace", "Industrials"),
    ("construction/ag equipment/trucks", "Industrials"),
    ("major banks", "Financials"),
    ("major chemicals", "Materials"),
    ("electric utilities: central", "Utilities"),
    ("package goods/cosmetics", "Consumer Staples"),
    ("real estate investment trusts", "Real Estate"),
    ("medical specialities", "Health Care"),
    ("integrated oil companies", "Energy"),
];

/// The sector an industry sits wholly inside, empty for one it does not (or
/// one no table names): what a share is classified by when its source states
/// its industry and no sector the set holds.
pub fn sector_of_industry(industry: &str) -> String {
    let key = industry.trim().to_lowercase();
    INDUSTRY_SECTOR.iter().find(|(k, _)| *k == key).map_or_else(String::new, |(_, s)| (*s).to_string())
}

/// What a security is exposed to: weights by sector and by country, in the
/// order the record gives them (which decides a tie for the dominant sector).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exposure {
    pub sectors: Vec<(String, f64)>,
    pub countries: Vec<(String, f64)>,
}

impl<'de> serde::Deserialize<'de> for Exposure {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Exposure, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(Exposure::from_weights(v.get("sectors").unwrap_or(&Value::Null), v.get("countries").unwrap_or(&Value::Null)))
    }
}

impl Exposure {
    /// From the two weight maps as the record keeps them; what is not a map is no weights.
    pub fn from_weights(sectors: &Value, countries: &Value) -> Exposure {
        let weights = |v: &Value| match v {
            Value::Object(m) => m.iter().map(|(name, w)| (name.clone(), num(Some(w), 0.0))).collect(),
            _ => vec![],
        };
        Exposure { sectors: weights(sectors), countries: weights(countries) }
    }
}

/// Exposure records by key: a security's id, `share:<ListingKey>` for a listing
/// classified as a share, `fund:<ListingKey>` for a fund's own record.
pub type Exposures = HashMap<String, Exposure>;

/// The record an option's exposure is read from: its underlying's share record,
/// on the underlying's own venue, which the contract's record names; none where
/// it names none (a currency never stands in for a venue).
pub fn underlying_exposure<'a>(exposures: &'a Exposures, underlying: &str, exchange: &str) -> Option<&'a Exposure> {
    exposures.get(&crate::venues::share_key_of(underlying, exchange)?)
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
            Kind::Options => underlying_exposure(exposures, &p.underlying, &p.exchange).unwrap_or(&nothing),
            _ => exposures.get(&p.security_id).unwrap_or(&nothing),
        };
        // a record read before an alias was known folds here; a name the set does
        // not hold (one kept by an earlier build) is no sector, and its weight is
        // not classified
        let mut sector_weights: Vec<f64> = Vec::new();
        for (name, w) in &record.sectors {
            let known = norm_sector(name);
            if !known.is_empty() {
                sectors.add(known, v * w);
                sector_weights.push(*w);
            }
        }
        for (name, w) in &record.countries {
            countries.add(name.clone(), v * w);
        }
        let covered = |weights: &[(String, f64)]| f64::min(1.0, weights.iter().map(|(_, w)| *w).fsum());
        sectors.unclassified += v * f64::max(0.0, 1.0 - f64::min(1.0, sector_weights.iter().copied().fsum()));
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
