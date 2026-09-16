//! Round trips: `model.build_trades` and `model.collapse_trade`.
//!
//! A trade is a position going from flat to open and back to flat. Partial
//! exits are legs of the same trade, and the id is stable from the first fill
//! so a journal entry survives later exits.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

use crate::clock::when_parts;
use crate::dates::days_between;
use crate::fifo::{trade_side, Slice};
use crate::securities::Securities;
use crate::symbols::{option_multiplier, underlying_symbol};
use crate::value::{field_num, field_s, fmt8, num_repr};

pub type Journal = Map<String, Value>;

/// `model.slice_member_key`: what a saved group names its members by.
pub fn slice_member_key(t: &Slice) -> String {
    if !t.buy_activity_id.is_empty() && !t.sell_activity_id.is_empty() {
        return [t.buy_activity_id.clone(), t.sell_activity_id.clone(), fmt8(t.quantity)].join("|");
    }
    t.id.clone()
}

/// `model.group_id_for_keys`: the legacy ledger.html group id, FNV-1a over the
/// sorted member keys.
pub fn group_id_for_keys(keys: &[String]) -> String {
    let mut sorted: Vec<&String> = keys.iter().collect();
    sorted.sort();
    let joined = sorted.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
    let mut h: u32 = 2166136261;
    for ch in joined.chars() {
        h ^= ch as u32;
        h = h.wrapping_mul(16777619);
    }
    format!("g_{:x}_{}", h, keys.len())
}

fn slim_slice(s: &Slice) -> Value {
    json!({
        "key": slice_member_key(s),
        "qty": s.quantity,
        "entry": s.entry_price,
        "exit": s.exit_price,
        "entryDate": s.entry_date,
        "exitDate": s.exit_date,
        "pnl": s.pnl,
        "pnlCad": s.pnl_cad,
        "fees": s.commission,
        "buyActivityId": s.buy_activity_id,
        "sellActivityId": s.sell_activity_id,
        "flags": s.flags,
    })
}

/// `model._fill_row`: one broker fill as the page prints it.
pub fn fill_row_public(a: &Value) -> Value { fill_row(a) }

fn fill_row(a: &Value) -> Value {
    let occurred = { let o = field_s(a, "occurredAt"); if o.is_empty() { field_s(a, "transactionDate") } else { o } };
    let (day, clock) = when_parts(&occurred);
    let side = trade_side(a);
    let qty = field_num(a, "quantity").abs();
    json!({
        "id": field_s(a, "id"),
        "when": occurred,
        "date": if day.is_empty() { field_s(a, "transactionDate") } else { day },
        "time": clock,
        "side": side,
        "sub": field_s(a, "activitySubType"),
        "qty": if side == "SELL" { -qty } else { qty },
        "price": field_num(a, "unitPrice"),
        "amount": field_num(a, "netCashAmount"),
        "fees": field_num(a, "commission"),
        "currency": field_s(a, "currency"),
        "flags": a.get("flags").cloned().unwrap_or_else(|| json!([])),
    })
}

/// `model.collapse_trade`: one group of slices as a single round trip.
#[allow(clippy::too_many_arguments)]
pub fn collapse_trade(
    gid: &str,
    slices: &[Slice],
    locked: bool,
    status: &str,
    acts_by_id: &HashMap<String, Value>,
    securities: &Securities,
    journal: &Journal,
) -> Value {
    let mut slices: Vec<Slice> = slices.to_vec();
    slices.sort_by(|a, b| {
        (a.exit_date.clone(), a.entry_date.clone(), slice_member_key(a))
            .cmp(&(b.exit_date.clone(), b.entry_date.clone(), slice_member_key(b)))
    });
    let t0 = slices[0].clone();

    let qty: f64 = slices.iter().map(|s| s.quantity).sum();
    let entry_notional: f64 = slices.iter().map(|s| s.entry_price * s.quantity).sum();
    let exit_notional: f64 = slices.iter().map(|s| s.exit_price * s.quantity).sum();
    let pnl: f64 = slices.iter().map(|s| s.pnl).sum();
    let pnl_cad: f64 = slices.iter().map(|s| s.pnl_cad).sum();
    let fees: f64 = slices.iter().map(|s| s.commission).sum();
    let fees_cad: f64 = slices.iter().map(|s| s.fees_cad.unwrap_or(s.commission)).sum();
    let entry_date = slices.iter().map(|s| s.entry_date.clone()).min().unwrap_or_default();
    let exit_date = slices.iter().map(|s| s.exit_date.clone()).max().unwrap_or_default();
    let entry_when = slices
        .iter()
        .map(|s| if s.entry_when.is_empty() { s.entry_date.clone() } else { s.entry_when.clone() })
        .min()
        .unwrap_or_default();
    let exit_when = slices
        .iter()
        .map(|s| if s.exit_when.is_empty() { s.exit_date.clone() } else { s.exit_when.clone() })
        .max()
        .unwrap_or_default();

    let mult = option_multiplier(&t0.symbol);
    let entry = if qty != 0.0 { entry_notional / qty } else { 0.0 };
    let exit_px = if qty != 0.0 { exit_notional / qty } else { t0.exit_price };
    let basis = (entry * qty * mult).abs();
    let sec_id = slices.iter().map(|s| s.security_id.clone()).find(|s| !s.is_empty()).unwrap_or_default();

    let mut ids: Vec<String> = Vec::new();
    for s in &slices {
        for k in [&s.buy_activity_id, &s.sell_activity_id] {
            if !k.is_empty() && !ids.contains(k) {
                ids.push(k.clone());
            }
        }
    }
    let mut fills: Vec<Value> = ids.iter().filter_map(|i| acts_by_id.get(i)).map(fill_row).collect();

    // Label each fill by what it did in this trade, not by the broker's order
    // type: the open/close types are option language, while shares and crypto
    // are simply bought or sold.
    let opened_ids: HashSet<String> = slices.iter().map(|s| s.buy_activity_id.clone()).collect();
    let closed_ids: HashSet<String> = slices.iter().map(|s| s.sell_activity_id.clone()).collect();
    for f in fills.iter_mut() {
        let fid = field_s(f, "id");
        let opened = opened_ids.contains(&fid);
        let closed = closed_ids.contains(&fid);
        let side = if field_s(f, "side") == "BUY" { "BUY" } else { "SELL" };
        let sub = if t0.kind != "Options" {
            format!("{}{}", side, if opened && closed { " (close + open)" } else { "" })
        } else if closed && !opened {
            format!("{} TO CLOSE", side)
        } else if opened && !closed {
            format!("{} TO OPEN", side)
        } else if opened && closed {
            format!("{} (close + open)", side)
        } else {
            field_s(f, "sub")
        };
        if let Value::Object(m) = f {
            m.insert("sub".into(), Value::String(sub));
        }
    }
    fills.sort_by(|a, b| field_s(b, "when").cmp(&field_s(a, "when")));

    let open_side = if t0.open_direction == "LONG" { "BUY" } else { "SELL" };
    let opens = fills.iter().filter(|f| field_s(f, "side") == open_side).count();
    let closes = fills.len() - opens;

    let mut flags: Vec<String> = slices.iter().flat_map(|s| s.flags.clone()).collect();
    flags.sort();
    flags.dedup();

    let entry_j = journal.get(gid).and_then(|v| v.as_object()).cloned().unwrap_or_default();
    let exchange = if t0.kind != "Crypto" { securities.exchange(&sec_id) } else { "Crypto".to_string() };
    let fallback_name = if t0.name.is_empty() { t0.symbol.clone() } else { t0.name.clone() };

    json!({
        "id": gid,
        "status": status,
        "locked": locked,
        "symbol": t0.symbol,
        "underlying": underlying_symbol(&t0.symbol),
        "name": securities.name(&sec_id, &fallback_name),
        "exchange": exchange,
        "kind": t0.kind,
        "currency": t0.currency,
        "account": t0.account,
        "accountId": t0.account_id,
        "securityId": sec_id,
        "side": if t0.open_direction == "LONG" { "SELL" } else { "COVER" },
        "openDirection": t0.open_direction,
        "qty": qty,
        "mult": mult,
        "entry": entry,
        "exit": exit_px,
        "entryDate": entry_date,
        "exitDate": exit_date,
        "entryWhen": entry_when,
        "exitWhen": exit_when,
        "holdDays": days_between(&entry_date, &exit_date),
        "pnl": pnl,
        "pnlCad": pnl_cad,
        "fees": fees,
        "feesCad": fees_cad,
        "pnlPct": if basis > 0.0 { json!(pnl / basis) } else { Value::Null },
        "legs": slices.iter().map(slim_slice).collect::<Vec<_>>(),
        "legCount": slices.len(),
        "fills": fills,
        "opened": {"qty": qty, "avg": entry, "fills": opens},
        "closed": {"qty": qty, "avg": exit_px, "fills": closes},
        "netCash": pnl,
        "flags": flags,
        "grade": entry_j.get("grade").cloned().unwrap_or_else(|| json!("")),
        "thesis": entry_j.get("thesis").cloned().unwrap_or_else(|| json!("")),
        "tags": entry_j.get("tags").cloned().unwrap_or_else(|| json!([])),
    })
}

/// `model.build_trades`: the saved manual groups first, then whatever is left
/// grouped by round trip.
pub fn build_trades(
    closed: &[Slice],
    saved_groups: &[Value],
    acts_by_id: &HashMap<String, Value>,
    securities: &Securities,
    journal: &Journal,
) -> Vec<Value> {
    let mut by_key: HashMap<String, Slice> = HashMap::new();
    for s in closed {
        by_key.insert(slice_member_key(s), s.clone());
    }
    let mut used: HashSet<String> = HashSet::new();
    let mut groups: Vec<(String, Vec<Slice>, bool)> = Vec::new();

    for rec in saved_groups {
        let mut members: Vec<Slice> = Vec::new();
        if let Some(ms) = rec.get("members").and_then(|v| v.as_array()) {
            for k in ms {
                let key = crate::value::s(Some(k));
                if let Some(s) = by_key.get(&key) {
                    let mk = slice_member_key(s);
                    if !used.contains(&mk) {
                        members.push(s.clone());
                        used.insert(mk);
                    }
                }
            }
        }
        if !members.is_empty() {
            let id = field_s(rec, "id");
            let gid = if id.is_empty() {
                group_id_for_keys(&members.iter().map(slice_member_key).collect::<Vec<_>>())
            } else {
                id
            };
            groups.push((gid, members, true));
        }
    }

    let mut by_rt: HashMap<String, Vec<Slice>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for s in closed {
        let mk = slice_member_key(s);
        if used.contains(&mk) {
            continue;
        }
        let rt = s.rt.clone().unwrap_or_else(|| format!("rt:{}", mk));
        if !by_rt.contains_key(&rt) {
            by_rt.insert(rt.clone(), Vec::new());
            order.push(rt.clone());
        }
        by_rt.get_mut(&rt).unwrap().push(s.clone());
    }
    for rt in order {
        let members = by_rt.remove(&rt).unwrap();
        groups.push((rt, members, false));
    }

    let mut trades: Vec<Value> = groups
        .iter()
        .map(|(gid, members, locked)| collapse_trade(gid, members, *locked, "closed", acts_by_id, securities, journal))
        .collect();
    trades.sort_by(|a, b| {
        (field_s(b, "exitDate"), field_s(b, "id")).cmp(&(field_s(a, "exitDate"), field_s(a, "id")))
    });
    trades
}

/// `model.last_fill_prices`: symbol -> the newest fill that carried a price.
pub fn last_fill_prices(activities: &[Value]) -> Map<String, Value> {
    let mut idx: Vec<usize> = (0..activities.len()).collect();
    idx.sort_by(|i, j| {
        let (a, b) = (&activities[*i], &activities[*j]);
        (field_s(a, "transactionDate"), field_s(a, "occurredAt"))
            .cmp(&(field_s(b, "transactionDate"), field_s(b, "occurredAt")))
    });
    let mut out = Map::new();
    for i in idx {
        let a = &activities[i];
        let cat = field_s(a, "category");
        if cat != "trade" && cat != "option_event" {
            continue;
        }
        let px = field_num(a, "unitPrice");
        let sym = field_s(a, "symbol");
        if px > 0.0 && !sym.is_empty() {
            out.insert(sym, json!({"price": px, "date": field_s(a, "transactionDate")}));
        }
    }
    out
}

/// `model.quote_fits`: a quote prices a position only when its source is the
/// kind's. The coin BTC's Coinbase price must never price a share called BTC,
/// and a listing's TMX price never a coin. No source stated is taken as the
/// kind's own.
pub fn quote_fits(quote: Option<&Value>, kind: &str) -> bool {
    let source = quote.map(|q| field_s(q, "source")).unwrap_or_default();
    if source.is_empty() {
        return true;
    }
    match kind {
        "Crypto" => source == "coinbase",
        "Options" => source == "cboe_options",
        _ => source != "coinbase" && source != "cboe_options",
    }
}

/// Only used by the description strings the derived rows carry.
pub fn repr(v: f64) -> String {
    num_repr(v)
}
