//! The security records Wealthsimple returns, and the exchange each symbol is
//! really listed on.
//!
//! An option's record points at its underlying, and a Canadian name often
//! comes back quoted on Alpha, which is a venue rather than the listing: both
//! are followed to the record the page should name.

use serde_json::Value;
use std::collections::HashMap;

use crate::value::{field_s, s as vs};

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

/// `exchange_label`: the name the page prints, from the exchange field
/// when there is one and the MIC when there is not.
pub fn exchange_label(sec: Option<&Value>) -> String {
    let sec = match sec { Some(s) => s, None => return String::new() };
    let raw = field_s(sec, "primaryExchange").trim().to_string();
    let up = raw.to_uppercase();
    if let Some((_, v)) = EXCH_ALIAS.iter().find(|(k, _)| *k == up) {
        return (*v).to_string();
    }
    if !raw.is_empty() {
        return raw;
    }
    let mic = field_s(sec, "primaryMic").to_uppercase();
    MIC_MAP.iter().find(|(k, _)| *k == mic).map(|(_, v)| (*v).to_string()).unwrap_or_default()
}

/// `listing_ticker`: `SHOP.TO` -> `SHOP`.
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

/// `is_alpha_venue`: Alpha is where a trade printed, not where the name
/// is listed.
pub fn is_alpha_venue(sec: Option<&Value>) -> bool {
    let sec = match sec { Some(s) => s, None => return false };
    let exch = field_s(sec, "primaryExchange").to_uppercase();
    let mic = field_s(sec, "primaryMic").to_uppercase();
    exch == "ALPHA EXCHANGE" || exch == "ALPHA" || mic == "XATS"
}

/// `Securities`.
pub struct Securities {
    pub by_id: HashMap<String, Value>,
    /// Insertion order, because `preferred` takes the first record that fits.
    order: Vec<String>,
}

impl Securities {
    pub fn new(rows: &[Value]) -> Self {
        let mut by_id = HashMap::new();
        let mut order = Vec::new();
        for r in rows {
            let id = field_s(r, "id");
            if !id.is_empty() {
                if !by_id.contains_key(&id) {
                    order.push(id.clone());
                }
                by_id.insert(id, r.clone());
            }
        }
        Securities { by_id, order }
    }

    /// `Securities.preferred`: an Alpha quote is swapped for the same name's
    /// real listing when the book carries one.
    fn preferred<'a>(&'a self, sec: Option<&'a Value>) -> Option<&'a Value> {
        let sec = sec?;
        if !is_alpha_venue(Some(sec)) {
            return Some(sec);
        }
        let sym = listing_ticker(&field_s(sec, "symbol"));
        let ccy = field_s(sec, "currency");
        if sym.is_empty() {
            return Some(sec);
        }
        let sec_id = field_s(sec, "id");
        for oid in &self.order {
            let other = &self.by_id[oid];
            if *oid == sec_id || !field_s(other, "underlyingId").is_empty() {
                continue;
            }
            if listing_ticker(&field_s(other, "symbol")) != sym {
                continue;
            }
            let occ = field_s(other, "currency");
            if !ccy.is_empty() && !occ.is_empty() && occ != ccy {
                continue;
            }
            if is_alpha_venue(Some(other)) || exchange_label(Some(other)).is_empty() {
                continue;
            }
            return Some(other);
        }
        Some(sec)
    }

    /// `Securities.listing`: the record the page should name -- an option's
    /// underlying, then the preferred venue.
    pub fn listing(&self, security_id: &str) -> Option<&Value> {
        let mut sec = self.by_id.get(security_id);
        if let Some(s) = sec {
            let uid = field_s(s, "underlyingId");
            if !uid.is_empty() {
                if let Some(under) = self.by_id.get(&uid) {
                    sec = Some(under);
                }
            }
        }
        self.preferred(sec)
    }

    pub fn exchange(&self, security_id: &str) -> String {
        exchange_label(self.listing(security_id))
    }

    pub fn name(&self, security_id: &str, fallback: &str) -> String {
        let n = self.listing(security_id).map(|s| field_s(s, "name")).unwrap_or_default();
        if n.is_empty() { fallback.to_string() } else { n }
    }

    /// The underlying a record points at, for the assignment rows.
    pub fn underlying_id(&self, security_id: &str) -> Option<String> {
        let sec = self.by_id.get(security_id)?;
        let uid = vs(sec.get("underlyingId"));
        if uid.is_empty() { None } else { Some(uid) }
    }

    /// `Securities.cash_currencies`: the cash rows Wealthsimple lists as if
    /// they were securities.
    pub fn cash_currencies(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for sid in &self.order {
            let sec = &self.by_id[sid];
            let sym = field_s(sec, "symbol").to_uppercase();
            if sym == "CAD" || sym == "USD" || sid.starts_with("sec-c-") {
                let ccy = field_s(sec, "currency").to_uppercase();
                out.insert(sid.clone(), if ccy.is_empty() { sym } else { ccy });
            }
        }
        out
    }

    pub fn known_exchanges(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .order
            .iter()
            .map(|sid| self.exchange(&field_s(&self.by_id[sid], "id")))
            .filter(|e| !e.is_empty())
            .collect();
        out.sort();
        out.dedup();
        out
    }
}
