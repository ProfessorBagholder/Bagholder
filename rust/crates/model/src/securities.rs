//! The security records Wealthsimple returns, and the exchange each symbol is
//! really listed on.
//!
//! An option's record points at its underlying, and a Canadian name often
//! comes back quoted on Alpha, which is a venue rather than the listing: both
//! are followed to the record the page should name.

use serde::Deserialize;
use std::collections::HashMap;

use crate::lenient;

const EXCH_ALIAS: [(&str, &str); 10] = [
    ("TSXV", "TSX-V"),
    ("TSX-V", "TSX-V"),
    ("TSX VENTURE", "TSX-V"),
    ("CDNX", "TSX-V"),
    ("VENTURE", "TSX-V"),
    ("TORONTO", "TSX"),
    ("TSX", "TSX"),
    ("CBOE CANADA", "Cboe Canada"),
    ("CBOE CA", "Cboe Canada"),
    ("NEO", "Cboe Canada"),
];

const MIC_MAP: [(&str, &str); 8] = [
    ("XTSV", "TSX-V"),
    ("XTSX", "TSX"),
    ("XNAS", "NASDAQ"),
    ("XNYS", "NYSE"),
    ("XASE", "NYSE American"),
    ("ARCX", "NYSE Arca"),
    ("XCNQ", "CSE"),
    ("NEOE", "Cboe Canada"),
];

/// One security as Wealthsimple describes it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Security {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub primary_exchange: String,
    #[serde(deserialize_with = "lenient::text")]
    pub primary_mic: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    /// An option's record points at what it is an option on.
    #[serde(deserialize_with = "lenient::text")]
    pub underlying_id: String,
}

impl Security {
    /// The name the page prints for its exchange, from the exchange field when
    /// there is one and the MIC when there is not.
    pub fn exchange_label(&self) -> String {
        let raw = self.primary_exchange.trim();
        let up = raw.to_uppercase();
        if let Some((_, v)) = EXCH_ALIAS.iter().find(|(k, _)| *k == up) {
            return (*v).to_string();
        }
        if !raw.is_empty() {
            return raw.to_string();
        }
        let mic = self.primary_mic.to_uppercase();
        MIC_MAP.iter().find(|(k, _)| *k == mic).map(|(_, v)| (*v).to_string()).unwrap_or_default()
    }

    /// Alpha is where a trade printed, not where the name is listed.
    pub fn is_alpha_venue(&self) -> bool {
        let exchange = self.primary_exchange.to_uppercase();
        exchange == "ALPHA EXCHANGE" || exchange == "ALPHA" || self.primary_mic.to_uppercase() == "XATS"
    }
}

/// `SHOP.TO` -> `SHOP`.
pub fn listing_ticker(sym: &str) -> String {
    let s = sym.trim();
    if let Some(i) = s.rfind('.') {
        let suffix = s[i + 1..].to_uppercase();
        if !s[..i].is_empty() && matches!(suffix.as_str(), "TO" | "V" | "CN" | "NE") {
            return s[..i].to_string();
        }
    }
    s.to_string()
}

/// The securities the book knows, by id.
#[derive(Default)]
pub struct Securities {
    by_id: HashMap<String, Security>,
    /// Insertion order, because `preferred` takes the first record that fits.
    order: Vec<String>,
}

impl Securities {
    pub fn new(rows: &[Security]) -> Self {
        let mut out = Securities::default();
        for r in rows.iter().filter(|r| !r.id.is_empty()) {
            if !out.by_id.contains_key(&r.id) {
                out.order.push(r.id.clone());
            }
            out.by_id.insert(r.id.clone(), r.clone());
        }
        out
    }

    /// An Alpha quote is swapped for the same name's real listing when the book
    /// carries one.
    fn preferred<'a>(&'a self, sec: &'a Security) -> &'a Security {
        if !sec.is_alpha_venue() {
            return sec;
        }
        let sym = listing_ticker(&sec.symbol);
        if sym.is_empty() {
            return sec;
        }
        self.order
            .iter()
            .map(|id| &self.by_id[id])
            .filter(|other| other.id != sec.id && other.underlying_id.is_empty() && listing_ticker(&other.symbol) == sym)
            .filter(|other| sec.currency.is_empty() || other.currency.is_empty() || other.currency == sec.currency)
            .find(|other| !other.is_alpha_venue() && !other.exchange_label().is_empty())
            .unwrap_or(sec)
    }

    /// The record the page should name: an option's underlying, then the
    /// preferred venue.
    pub fn listing(&self, security_id: &str) -> Option<&Security> {
        let sec = self.by_id.get(security_id)?;
        let named = if sec.underlying_id.is_empty() { sec } else { self.by_id.get(&sec.underlying_id).unwrap_or(sec) };
        Some(self.preferred(named))
    }

    pub fn exchange(&self, security_id: &str) -> String {
        self.listing(security_id).map(Security::exchange_label).unwrap_or_default()
    }

    pub fn name(&self, security_id: &str, fallback: &str) -> String {
        match self.listing(security_id) {
            Some(sec) if !sec.name.is_empty() => sec.name.clone(),
            _ => fallback.to_string(),
        }
    }

    /// The underlying a record points at, for the assignment rows.
    pub fn underlying_id(&self, security_id: &str) -> Option<String> {
        self.by_id.get(security_id).map(|s| s.underlying_id.clone()).filter(|id| !id.is_empty())
    }

    /// The cash rows Wealthsimple lists as if they were securities: id -> currency.
    pub fn cash_currencies(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for id in &self.order {
            let sec = &self.by_id[id];
            let sym = sec.symbol.to_uppercase();
            if sym == "CAD" || sym == "USD" || id.starts_with("sec-c-") {
                let ccy = sec.currency.to_uppercase();
                out.insert(id.clone(), if ccy.is_empty() { sym } else { ccy });
            }
        }
        out
    }

    pub fn known_exchanges(&self) -> Vec<String> {
        let mut out: Vec<String> = self.order.iter().map(|id| self.exchange(id)).filter(|e| !e.is_empty()).collect();
        out.sort();
        out.dedup();
        out
    }
}
