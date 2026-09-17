//! What the market readers are asked for, read off the built model:
//! `model.held_symbols`, `payer_symbols`, `intraday_archive_symbols`,
//! `watch_exposure_key`, and the one-shot journal migration
//! `migrate_legacy_notes`.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::base::Base;
use crate::fifo::Slice;
use crate::trades::{group_id_for_keys, slice_member_key};
use crate::value::field_s;

/// `model.held_symbols`: every held instrument, with what a quote source needs.
pub fn held_symbols(base: &Base) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in &base.positions {
        let sym = field_s(p, "symbol");
        if !seen.insert(sym.clone()) {
            continue;
        }
        out.push(json!({"symbol": sym, "exchange": p.get("exchange").cloned().unwrap_or(Value::Null),
                        "currency": p.get("currency").cloned().unwrap_or(Value::Null), "kind": p.get("kind").cloned().unwrap_or(Value::Null)}));
    }
    out
}

/// `model.payer_symbols`: held positions that have paid a distribution.
pub fn payer_symbols(base: &Base) -> Vec<Value> {
    let payers: HashSet<String> = base.cashflow.iter().filter(|r| field_s(r, "kind") == "Dividend").map(|r| field_s(r, "symbol")).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in &base.positions {
        let sym = field_s(p, "symbol");
        let short = p.get("short").and_then(|v| v.as_bool()).unwrap_or(false);
        if payers.contains(&sym) && !seen.contains(&sym) && !short {
            seen.insert(sym.clone());
            let ex = field_s(p, "exchange");
            out.push(json!({"symbol": sym, "exchange": if ex != "Crypto" { ex } else { String::new() }, "currency": p.get("currency").cloned().unwrap_or(Value::Null)}));
        }
    }
    out
}

/// `model.intraday_archive_symbols`: every symbol traded or held in the past
/// year, with the earliest date its bars are wanted from.
pub fn intraday_archive_symbols(base: &Base) -> Vec<Value> {
    let since = crate::dates::shift_date(&base.today, -365);
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    let charted = |rec: Value| -> Value {
        if field_s(&rec, "kind") == "Options" {
            let under = crate::symbols::underlying_symbol(&field_s(&rec, "symbol"));
            if !under.is_empty() && under != "—" {
                return json!({"symbol": under, "exchange": rec["exchange"], "currency": rec["currency"], "kind": "Shares"});
            }
        }
        rec
    };
    let mut want = |rec: Value, start: String| {
        let key = field_s(&rec, "symbol");
        let replace = match out.get(&key) { None => true, Some(cur) => start < field_s(cur, "start") };
        if replace {
            let mut m = rec.as_object().cloned().unwrap_or_default();
            m.insert("start".into(), json!(start));
            out.insert(key, Value::Object(m));
        }
    };
    let pick = |v: &Value| json!({"symbol": v["symbol"], "exchange": v["exchange"], "currency": v["currency"], "kind": v["kind"]});
    for t in &base.trades {
        if field_s(t, "exitDate") >= since {
            let entry = field_s(t, "entryDate");
            want(charted(pick(t)), if entry > since { entry } else { since.clone() });
        }
    }
    for p in &base.positions {
        let opened = { let o = field_s(p, "opened"); if o.is_empty() { since.clone() } else { o } };
        want(charted(pick(p)), if opened > since { opened } else { since.clone() });
    }
    out.into_values().collect()
}

/// `model.watch_exposure_key`.
pub fn watch_exposure_key(symbol: &str, exchange: &str, currency: &str) -> String {
    format!("share:{}:{}", crate::venues::tmx_symbol(symbol), crate::venues::tmx_form(exchange, currency).unwrap_or(""))
}

/// `model.migrate_legacy_notes`: the old page's note keys (a hash of the slices
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
        let mut direction: Option<String> = None;
        for s in members {
            if direction.as_deref().map(|d| d != s.open_direction).unwrap_or(false) {
                flush(&cur, &mut out);
                cur.clear();
            }
            direction = Some(s.open_direction.clone());
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
