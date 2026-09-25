//! What the market readers are asked for, read off the built model:
//! `held_symbols`, `payer_symbols`, `intraday_archive_symbols`,
//! `watch_exposure_key`, and the one-shot journal migration
//! `migrate_legacy_notes`.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::activity::Kind;
use crate::context::MarketBase as Base;
use crate::fifo::Slice;
use crate::input::Listing;
use crate::trades::{group_id_for_keys, slice_member_key};
use crate::value::field_s;

fn listing(symbol: &str, exchange: &str, currency: &str, kind: &str) -> Listing {
    Listing { symbol: symbol.into(), exchange: exchange.into(), currency: currency.into(), kind: kind.into(), quote_key: None, yahoo: None, start: None }
}

/// Every held instrument, with what a quote source needs.
pub fn held_symbols(base: &Base) -> Vec<Listing> {
    let mut seen = HashSet::new();
    base.positions.iter().filter(|p| seen.insert(p.symbol.as_str())).map(|p| listing(&p.symbol, &p.exchange, &p.currency, p.kind.as_str())).collect()
}

/// Every symbol traded or held in the past year, with the earliest date its bars
/// are wanted from. A contract is charted as what it is written on.
pub fn intraday_archive_symbols(base: &Base) -> Vec<Listing> {
    let since = crate::dates::shift_date(&base.today, -365);
    let mut out: BTreeMap<String, Listing> = BTreeMap::new();
    let mut want = |symbol: &str, exchange: &str, currency: &str, kind: Kind, start: &str| {
        let under = crate::symbols::underlying_symbol(symbol);
        let charted = if kind == Kind::Options && !under.is_empty() && under != "—" { listing(&under, exchange, currency, "Shares") } else { listing(symbol, exchange, currency, kind.as_str()) };
        let start = if start > since.as_str() { start } else { since.as_str() };
        if out.get(&charted.symbol).map_or(true, |known| start < known.start.as_deref().unwrap_or("")) {
            out.insert(charted.symbol.clone(), Listing { start: Some(start.to_string()), ..charted });
        }
    };
    for t in base.traded.iter().filter(|t| t.exit_date.is_empty() || t.exit_date >= since) {
        want(&t.symbol, &t.exchange, &t.currency, t.kind, &t.entry_date);
    }
    for p in base.positions.iter() {
        want(&p.symbol, &p.exchange, &p.currency, p.kind, if p.opened.is_empty() { &since } else { &p.opened });
    }
    out.into_values().collect()
}

/// `watch_exposure_key`.
pub fn watch_exposure_key(symbol: &str, exchange: &str, currency: &str) -> String {
    format!("share:{}:{}", crate::venues::tmx_symbol(symbol), crate::venues::tmx_form(exchange, currency).unwrap_or(""))
}

/// `migrate_legacy_notes`: the old page's note keys (a hash of the slices
/// in a lane group) mapped onto round-trip ids, so an existing journal is not
/// lost.
pub fn migrate_legacy_notes(closed: &[Slice], saved_groups: &[Value], notes: &Map<String, Value>) -> Map<String, Value> {
    if notes.is_empty() {
        return Map::new();
    }
    let by_key: HashMap<String, &Slice> = closed.iter().map(|s| (slice_member_key(s), s)).collect();
    let mut used: HashSet<String> = HashSet::new();
    let mut out: Map<String, Value> = Map::new();
    for rec in saved_groups {
        let members: Vec<&Slice> = rec.get("members").and_then(|m| m.as_array()).cloned().unwrap_or_default().iter()
            .filter_map(|k| by_key.get(&crate::value::s(Some(k))).copied()).collect();
        if members.is_empty() {
            continue;
        }
        for m in &members {
            used.insert(slice_member_key(m));
        }
        let gid = field_s(rec, "id");
        if let Some(n) = notes.get(&gid) {
            out.insert(gid, n.clone());
        }
    }
    let mut lane_order: Vec<(String, String, String)> = Vec::new();
    let mut lanes: HashMap<(String, String, String), Vec<&Slice>> = HashMap::new();
    for s in closed {
        if used.contains(&slice_member_key(s)) {
            continue;
        }
        let k = (s.account_id.clone(), s.symbol.clone(), s.currency.clone());
        if !lanes.contains_key(&k) {
            lane_order.push(k.clone());
        }
        lanes.entry(k).or_default().push(s);
    }
    for k in lane_order {
        let mut members = lanes.remove(&k).unwrap();
        members.sort_by(|a, b| (&a.exit_date, &a.entry_date, slice_member_key(a)).cmp(&(&b.exit_date, &b.entry_date, slice_member_key(b))));
        let flush = |cur: &Vec<&Slice>, out: &mut Map<String, Value>| {
            if cur.is_empty() {
                return;
            }
            let gid = group_id_for_keys(&cur.iter().map(|s| slice_member_key(s)).collect::<Vec<_>>());
            if let Some(n) = notes.get(&gid) {
                // the round trip most of the slices belong to; the first to reach the count wins a tie
                let mut order: Vec<Option<String>> = Vec::new();
                let mut counts: HashMap<Option<String>, usize> = HashMap::new();
                for s in cur {
                    if !counts.contains_key(&s.rt) {
                        order.push(s.rt.clone());
                    }
                    *counts.entry(s.rt.clone()).or_insert(0) += 1;
                }
                let mut best: Option<String> = None;
                let mut best_n = 0;
                for r in order {
                    let c = counts[&r];
                    if c > best_n {
                        best_n = c;
                        best = r;
                    }
                }
                if let Some(b) = best.filter(|b| !b.is_empty()) {
                    out.insert(b, n.clone());
                }
            }
        };
        let mut cur: Vec<&Slice> = Vec::new();
        let mut direction: Option<crate::activity::Direction> = None;
        for s in members {
            if direction.map(|d| d != s.open_direction).unwrap_or(false) {
                flush(&cur, &mut out);
                cur.clear();
            }
            direction = Some(s.open_direction);
            cur.push(s);
        }
        flush(&cur, &mut out);
    }
    let mut journal = Map::new();
    for (k, v) in out {
        let tags: Vec<String> = field_s(&v, "tag").split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        journal.insert(k, json!({"thesis": field_s(&v, "thesis"), "tags": tags, "grade": field_s(&v, "grade")}));
    }
    journal
}
