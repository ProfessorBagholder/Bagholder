//! Open positions: `build_positions`.
//!
//! The open lots rolled up per symbol, account, currency and direction. A
//! position and the trade it becomes when it closes share one journal entry,
//! because both are keyed by the round trip that opened it.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

use crate::dates::days_between;
use crate::fifo::Lot;
use crate::securities::Securities;
use crate::symbols::{option_multiplier, underlying_symbol};
use crate::trades::{quote_fits, Journal};
use crate::value::{field_s, get, norm_account_name, num};

/// A number that is absent rather than zero, read with no default.
fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

fn account_nick(acc: &Value) -> String {
    for k in ["nickname", "unifiedAccountType", "type"] {
        let v = field_s(acc, k);
        if !v.is_empty() {
            return norm_account_name(&v);
        }
    }
    String::new()
}

#[allow(clippy::too_many_arguments)]
pub fn build_positions(
    open_lots: &[Lot],
    last_prices: &Map<String, Value>,
    balances: &[Value],
    accounts: &[Value],
    securities: &Securities,
    journal: &Journal,
    today: &str,
    quotes: &Map<String, Value>,
    acts_by_id: &HashMap<String, Value>,
) -> Vec<Value> {
    let mut nick_ids: HashMap<String, HashSet<String>> = HashMap::new();
    for acc in accounts {
        nick_ids.entry(account_nick(acc)).or_default().insert(field_s(acc, "id"));
    }
    let mut bal: HashMap<(String, String), f64> = HashMap::new();
    for b in balances {
        let k = (field_s(b, "accountId"), field_s(b, "securityId"));
        *bal.entry(k).or_insert(0.0) += num(get(b, "quantity"), 0.0);
    }

    type Key = (String, String, String, String);
    let mut groups: HashMap<Key, Vec<Lot>> = HashMap::new();
    let mut order: Vec<Key> = Vec::new();
    for lot in open_lots {
        let k = (lot.symbol.clone(), lot.account_type.clone(), lot.currency.clone(), lot.direction.clone());
        if !groups.contains_key(&k) {
            groups.insert(k.clone(), Vec::new());
            order.push(k.clone());
        }
        groups.get_mut(&k).unwrap().push(lot.clone());
    }

    let mut rows: Vec<Value> = Vec::new();
    for k in &order {
        let mut lots = groups.remove(k).unwrap();
        lots.sort_by(|a, b| (a.date.clone(), a.when.clone()).cmp(&(b.date.clone(), b.when.clone())));
        let (symbol, account, currency, direction) = k.clone();
        let mult = option_multiplier(&symbol);
        let qty: f64 = lots.iter().map(|l| l.qty).fold(0.0, |a, b| a + b);
        if qty <= 1e-9 {
            continue;
        }
        let cost: f64 = lots.iter().map(|l| l.qty * l.price * mult).fold(0.0, |a, b| a + b);
        let fees: f64 = lots.iter().map(|l| l.commission).fold(0.0, |a, b| a + b);
        let sec_id = lots.iter().map(|l| l.security_id.clone()).find(|s| !s.is_empty()).unwrap_or_default();

        let last = last_prices.get(&symbol);
        let mut last_px = match last {
            Some(l) => num(get(l, "price"), 0.0),
            None => if qty != 0.0 { cost / (qty * mult) } else { 0.0 },
        };
        let mut last_at = last.map(|l| field_s(l, "date")).unwrap_or_default();
        let mut price_source = "fill";

        let mut quote = quotes.get(&symbol);
        if let Some(q) = quote {
            if !quote_fits(Some(q), &lots[0].kind) {
                quote = None;
            }
        }
        if let Some(q) = quote {
            if let Some(p) = opt_num(get(q, "price")) {
                if p != 0.0 {
                    last_px = p;
                    last_at = field_s(q, "fetchedAt");
                    price_source = "quote";
                }
            }
        }

        let mv = qty * last_px * mult;
        let unreal = if direction == "LONG" { mv - cost } else { cost - mv };
        let held: f64 = lots.iter().map(|l| l.qty * days_between(&l.date, today) as f64).fold(0.0, |a, b| a + b);

        let mut ws_qty: Option<f64> = None;
        if !sec_id.is_empty() {
            if let Some(ids) = nick_ids.get(&account) {
                let mut total = 0.0;
                let mut found = false;
                for aid in ids {
                    if let Some(v) = bal.get(&(aid.clone(), sec_id.clone())) {
                        total += *v;
                        found = true;
                    }
                }
                if found {
                    ws_qty = Some(total);
                }
            }
        }

        let legacy_pid = format!("pos:{}|{}|{}", account, symbol, currency);
        let pid = lots[0].rt.clone().unwrap_or_else(|| legacy_pid.clone());
        let entry_j = journal
            .get(&pid)
            .and_then(|v| v.as_object())
            .or_else(|| journal.get(&legacy_pid).and_then(|v| v.as_object()))
            .cloned()
            .unwrap_or_default();

        let price_change = quote.and_then(|q| opt_num(get(q, "priceChange")));
        let percent_change = quote.and_then(|q| opt_num(get(q, "percentChange")));

        let mut fills: Vec<Value> = lots
            .iter()
            .filter_map(|l| acts_by_id.get(&l.activity_id))
            .map(crate::trades::fill_row_public)
            .collect();
        fills.sort_by(|a, b| field_s(b, "when").cmp(&field_s(a, "when")));

        let exchange = if lots[0].kind != "Crypto" { securities.exchange(&sec_id) } else { "Crypto".to_string() };
        let fallback_name = if lots[0].name.is_empty() { symbol.clone() } else { lots[0].name.clone() };

        rows.push(json!({
            "id": pid,
            "symbol": symbol,
            "underlying": underlying_symbol(&symbol),
            "name": securities.name(&sec_id, &fallback_name),
            "exchange": exchange,
            "kind": lots[0].kind,
            "account": account,
            "accountId": lots[0].account_id,
            "currency": currency,
            "securityId": sec_id,
            "short": direction == "SHORT",
            "qty": qty,
            "mult": mult as i64,
            "avg": if qty != 0.0 { cost / (qty * mult) } else { 0.0 },
            "cost": cost,
            "fees": fees,
            "last": last_px,
            "lastAt": last_at,
            "priceSource": price_source,
            "priceChange": price_change,
            "percentChange": percent_change,
            // the day's move on the whole position, in its own currency
            "dayChange": price_change.map(|pc| qty * pc * mult * if direction == "SHORT" { -1.0 } else { 1.0 }),
            "mv": mv,
            "unreal": unreal,
            "unrealPct": if cost != 0.0 { json!(unreal / cost) } else { Value::Null },
            "held": if qty != 0.0 { round_half_even(held / qty) } else { 0 },
            "opened": lots[0].date,
            "wsQty": ws_qty,
            "rt": lots[0].rt,
            "lots": lots.iter().map(|l| json!({
                "opened": l.date,
                "qty": l.qty,
                "price": l.price,
                "basis": l.qty * l.price * mult,
                "held": days_between(&l.date, today),
                "flags": l.flags,
                "activityId": l.activity_id,
            })).collect::<Vec<_>>(),
            "fills": fills,
            "grade": entry_j.get("grade").cloned().unwrap_or_else(|| json!("")),
            "thesis": entry_j.get("thesis").cloned().unwrap_or_else(|| json!("")),
            "tags": entry_j.get("tags").cloned().unwrap_or_else(|| json!([])),
        }));
    }

    let book: f64 = rows.iter().map(|r| num(get(r, "cost"), 0.0).abs()).fold(0.0, |a, b| a + b);
    for r in rows.iter_mut() {
        let alloc = if book != 0.0 { num(get(r, "cost"), 0.0).abs() / book } else { 0.0 };
        if let Value::Object(m) = r {
            m.insert("alloc".into(), json!(alloc));
        }
    }
    // A stable sort, descending on one key: `sort_by` with a reversed
    // comparison.
    rows.sort_by(|a, b| {
        num(get(b, "alloc"), 0.0)
            .partial_cmp(&num(get(a, "alloc"), 0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

/// Round half to even, returning an integer.
fn round_half_even(v: f64) -> i64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        (r - v.signum()) as i64
    } else {
        r as i64
    }
}
