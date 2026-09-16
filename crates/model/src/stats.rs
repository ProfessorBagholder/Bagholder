//! The figures one filtered list of trades produces: the KPI tiles, the
//! by-symbol and monthly tables, the grade buckets and the review queue.
//!
//! Every one of them reads `pnlCad`, so a book in two currencies adds up.

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::dates::{days_between, MONTHS};
use crate::value::{field_s, get, num};

pub const GRADES: [&str; 4] = ["A", "B", "C", "F"];

/// `model.metrics`.
pub fn metrics(trades: &[Value]) -> Value {
    let vals: Vec<f64> = trades.iter().map(|t| num(get(t, "pnlCad"), 0.0)).collect();
    let wins: Vec<f64> = vals.iter().copied().filter(|v| *v > 0.0).collect();
    let losses: Vec<f64> = vals.iter().copied().filter(|v| *v < 0.0).collect();
    let be = vals.iter().filter(|v| **v == 0.0).count();
    let gw: f64 = wins.iter().sum();
    let gl: f64 = losses.iter().sum::<f64>().abs();
    let n = vals.len();
    let total: f64 = vals.iter().sum();

    // A book with wins and no losses has no finite profit factor; the page is
    // told so rather than being handed a division by zero.
    let profit_factor = if gl > 0.0 {
        json!(gw / gl)
    } else if gw > 0.0 {
        Value::Null
    } else {
        json!(0.0)
    };

    json!({
        "realized": total,
        "count": n,
        "wins": wins.len(),
        "losses": losses.len(),
        "breakeven": be,
        "winRate": if n > 0 { json!(wins.len() as f64 / n as f64) } else { Value::Null },
        "grossWin": gw,
        "grossLoss": gl,
        "profitFactor": profit_factor,
        "profitFactorInfinite": gl == 0.0 && gw > 0.0,
        "expectancy": if n > 0 { json!(total / n as f64) } else { Value::Null },
        "avgWin": if !wins.is_empty() { gw / wins.len() as f64 } else { 0.0 },
        "avgLoss": if !losses.is_empty() { -gl / losses.len() as f64 } else { 0.0 },
        "fees": trades.iter().map(|t| num(get(t, "feesCad"), 0.0)).sum::<f64>(),
        "avgHold": if n > 0 {
            json!(trades.iter().map(|t| num(get(t, "holdDays"), 0.0)).sum::<f64>() / n as f64)
        } else { Value::Null },
        "openCount": trades.iter().filter(|t| field_s(t, "status") == "open").count(),
    })
}

/// `model.by_symbol`: grouped by the underlying, so a chain of contracts sits
/// under the name it is written on.
pub fn by_symbol(trades: &[Value]) -> Vec<Value> {
    struct G { pnl: f64, n: usize, wins: usize, hold: f64, legs: f64, ids: Vec<String> }
    let mut by: HashMap<String, G> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for t in trades {
        let k = field_s(t, "underlying");
        if !by.contains_key(&k) {
            by.insert(k.clone(), G { pnl: 0.0, n: 0, wins: 0, hold: 0.0, legs: 0.0, ids: vec![] });
            order.push(k.clone());
        }
        let g = by.get_mut(&k).unwrap();
        let p = num(get(t, "pnlCad"), 0.0);
        g.pnl += p;
        g.n += 1;
        g.legs += num(get(t, "legCount"), 0.0);
        g.hold += num(get(t, "holdDays"), 0.0);
        g.ids.push(field_s(t, "id"));
        if p > 0.0 {
            g.wins += 1;
        }
    }
    let mut rows: Vec<Value> = order
        .iter()
        .map(|k| {
            let g = &by[k];
            json!({
                "symbol": k,
                "pnl": g.pnl,
                "n": g.n,
                "legs": g.legs,
                "winRate": if g.n > 0 { g.wins as f64 / g.n as f64 } else { 0.0 },
                "avgHold": if g.n > 0 { g.hold / g.n as f64 } else { 0.0 },
                "tradeIds": g.ids,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        num(get(b, "pnl"), 0.0).partial_cmp(&num(get(a, "pnl"), 0.0)).unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

/// `model.month_label`: `2026-02` -> `Feb '26`.
pub fn month_label(key: &str) -> String {
    let m: usize = key[5..7].parse().unwrap_or(1);
    format!("{} '{}", MONTHS[m - 1], &key[2..4])
}

/// `model.monthly`.
pub fn monthly(trades: &[Value]) -> Vec<Value> {
    struct B { label: String, value: f64, count: usize, ids: Vec<String> }
    let mut by: std::collections::BTreeMap<String, B> = std::collections::BTreeMap::new();
    for t in trades {
        let k: String = field_s(t, "exitDate").chars().take(7).collect();
        if k.len() < 7 {
            continue;
        }
        let b = by.entry(k.clone()).or_insert_with(|| B { label: month_label(&k), value: 0.0, count: 0, ids: vec![] });
        b.value += num(get(t, "pnlCad"), 0.0);
        b.count += 1;
        b.ids.push(field_s(t, "id"));
    }
    by.into_iter()
        .map(|(k, b)| json!({"key": k, "label": b.label, "value": b.value, "count": b.count, "tradeIds": b.ids}))
        .collect()
}

/// `model.grade_buckets`.
pub fn grade_buckets(trades: &[Value]) -> Value {
    let mut buckets = Vec::new();
    for g in GRADES {
        let rows: Vec<&Value> = trades.iter().filter(|t| field_s(t, "grade") == g).collect();
        buckets.push(json!({
            "grade": g,
            "n": rows.len(),
            "pnl": rows.iter().map(|t| num(get(t, "pnlCad"), 0.0)).sum::<f64>(),
            "tradeIds": rows.iter().map(|t| field_s(t, "id")).collect::<Vec<_>>(),
        }));
    }
    let ungraded = trades.iter().filter(|t| field_s(t, "grade").is_empty()).count();
    json!({"buckets": buckets, "ungraded": ungraded, "graded": trades.len() - ungraded})
}

/// `model.review_queue`: the closed trades still missing a grade or a thesis.
pub fn review_queue(trades: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for t in trades {
        let no_grade = field_s(t, "grade").is_empty();
        let no_thesis = field_s(t, "thesis").trim().is_empty();
        if !(no_grade || no_thesis) {
            continue;
        }
        let missing = if no_grade && no_thesis {
            "no grade or thesis"
        } else if no_grade {
            "no grade"
        } else {
            "no thesis"
        };
        out.push(json!({
            "id": field_s(t, "id"),
            "symbol": field_s(t, "symbol"),
            "date": field_s(t, "exitDate"),
            "pnl": num(get(t, "pnlCad"), 0.0),
            "currency": "CAD",
            "missing": missing,
        }));
    }
    out.sort_by(|a, b| field_s(b, "date").cmp(&field_s(a, "date")));
    out
}

const SCHEDULES: [i64; 8] = [52, 26, 24, 12, 6, 4, 2, 1];

/// `model.payments_per_year`: the frequency read from the payment dates
/// themselves, never assumed from the instrument.
///
/// Only the last three gaps count, so a fund that changes its schedule is
/// re-read after two payments at the new cadence. Payments on one day count
/// once, and fewer than two distinct dates says nothing.
pub fn payments_per_year(dates: &[String]) -> Option<i64> {
    let mut days: Vec<String> = dates
        .iter()
        .map(|d| d.chars().take(10).collect::<String>())
        .filter(|d| !d.is_empty())
        .collect();
    days.sort();
    days.dedup();
    if days.len() < 2 {
        return None;
    }
    let mut gaps: Vec<i64> = days.windows(2).map(|w| days_between(&w[0], &w[1])).filter(|g| *g > 0).collect();
    if gaps.is_empty() {
        return None;
    }
    if gaps.len() > 3 {
        gaps = gaps.split_off(gaps.len() - 3);
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    let per_year = 365.25 / median as f64;
    // `min` keeps the first of equal distances, as Python's does.
    let mut best = SCHEDULES[0];
    let mut best_d = (SCHEDULES[0] as f64 - per_year).abs();
    for s in SCHEDULES.iter().skip(1) {
        let d = (*s as f64 - per_year).abs();
        if d < best_d {
            best = *s;
            best_d = d;
        }
    }
    Some(best)
}
