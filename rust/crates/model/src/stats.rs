//! The figures one filtered list of trades produces: the KPI tiles, the
//! by-symbol and monthly tables, the grade buckets and the review queue.
//!
//! Every one of them reads the trade's CAD figures, so a book in two currencies
//! adds up.

use std::collections::{BTreeMap, HashMap};

use crate::dates::{days_between, MONTHS};
use crate::value::FSum;
use crate::wire::{BySymbolRow, GradeBucket, Grades, Kpi, MonthlyBar, QueueRow, Trade};

pub const GRADES: [&str; 4] = ["A", "B", "C", "F"];

pub fn metrics(trades: &[&Trade]) -> Kpi {
    let vals: Vec<f64> = trades.iter().map(|t| t.pnl_cad).collect();
    let wins: Vec<f64> = vals.iter().copied().filter(|v| *v > 0.0).collect();
    let losses: Vec<f64> = vals.iter().copied().filter(|v| *v < 0.0).collect();
    let gross_win: f64 = wins.iter().fsum();
    let gross_loss: f64 = losses.iter().fsum().abs();
    let n = vals.len();
    let total: f64 = vals.iter().fsum();
    Kpi {
        realized: total + 0.0,
        count: n,
        wins: wins.len(),
        losses: losses.len(),
        breakeven: vals.iter().filter(|v| **v == 0.0).count(),
        win_rate: (n > 0).then(|| wins.len() as f64 / n as f64),
        gross_win: gross_win + 0.0,
        gross_loss: gross_loss + 0.0,
        // A book with wins and no losses has no finite profit factor; the page is
        // told so rather than being handed a division by zero.
        profit_factor: if gross_loss > 0.0 { Some(gross_win / gross_loss) } else if gross_win > 0.0 { None } else { Some(0.0) },
        profit_factor_infinite: gross_loss == 0.0 && gross_win > 0.0,
        expectancy: (n > 0).then(|| total / n as f64),
        avg_win: if wins.is_empty() { 0.0 } else { gross_win / wins.len() as f64 },
        avg_loss: if losses.is_empty() { 0.0 } else { -gross_loss / losses.len() as f64 },
        fees: trades.iter().map(|t| t.fees_cad).fsum() + 0.0,
        avg_hold: (n > 0).then(|| trades.iter().map(|t| t.hold_days as f64).fsum() / n as f64),
        open_count: trades.iter().filter(|t| t.status == "open").count(),
    }
}

/// Grouped by the underlying, so a chain of contracts sits under the name it is
/// written on. Largest gain first.
pub fn by_symbol(trades: &[&Trade]) -> Vec<BySymbolRow> {
    #[derive(Default)]
    struct Group {
        pnl: f64,
        wins: usize,
        hold: f64,
        legs: usize,
        ids: Vec<String>,
    }
    let mut by: HashMap<&str, Group> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for t in trades {
        if !by.contains_key(t.underlying.as_str()) {
            order.push(&t.underlying);
        }
        let g = by.entry(&t.underlying).or_default();
        g.pnl += t.pnl_cad;
        g.legs += t.leg_count;
        g.hold += t.hold_days as f64;
        g.ids.push(t.id.clone());
        if t.pnl_cad > 0.0 {
            g.wins += 1;
        }
    }
    let mut rows: Vec<BySymbolRow> = order
        .into_iter()
        .map(|symbol| {
            let g = by.remove(symbol).unwrap();
            let n = g.ids.len();
            BySymbolRow { symbol: symbol.to_string(), pnl: g.pnl, n, legs: g.legs, win_rate: g.wins as f64 / n as f64, avg_hold: g.hold / n as f64, trade_ids: g.ids }
        })
        .collect();
    rows.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

/// `2026-02` -> `Feb '26`.
pub fn month_label(key: &str) -> String {
    let m: usize = key[5..7].parse().unwrap_or(1);
    format!("{} '{}", MONTHS[m - 1], &key[2..4])
}

pub fn monthly(trades: &[&Trade]) -> Vec<MonthlyBar> {
    let mut by: BTreeMap<String, MonthlyBar> = BTreeMap::new();
    for t in trades {
        let key: String = t.exit_date.chars().take(7).collect();
        if key.len() < 7 {
            continue;
        }
        let bar = by.entry(key.clone()).or_insert_with(|| MonthlyBar { label: month_label(&key), key, value: 0.0, count: 0, trade_ids: vec![] });
        bar.value += t.pnl_cad;
        bar.count += 1;
        bar.trade_ids.push(t.id.clone());
    }
    by.into_values().collect()
}

pub fn grade_buckets(trades: &[&Trade]) -> Grades {
    let buckets = GRADES
        .iter()
        .map(|grade| {
            let rows: Vec<&&Trade> = trades.iter().filter(|t| t.grade == *grade).collect();
            GradeBucket { grade, n: rows.len(), pnl: rows.iter().map(|t| t.pnl_cad).fsum() + 0.0, trade_ids: rows.iter().map(|t| t.id.clone()).collect() }
        })
        .collect();
    let ungraded = trades.iter().filter(|t| t.grade.is_empty()).count();
    Grades { buckets, ungraded, graded: trades.len() - ungraded }
}

/// The closed trades still missing a grade or a thesis, newest first.
pub fn review_queue(trades: &[&Trade]) -> Vec<QueueRow> {
    let mut out: Vec<QueueRow> = trades
        .iter()
        .filter_map(|t| {
            let missing = match (t.grade.is_empty(), t.thesis.trim().is_empty()) {
                (true, true) => "no grade or thesis",
                (true, false) => "no grade",
                (false, true) => "no thesis",
                (false, false) => return None,
            };
            Some(QueueRow { id: t.id.clone(), symbol: t.symbol.clone(), date: t.exit_date.clone(), pnl: t.pnl_cad, currency: "CAD", missing })
        })
        .collect();
    out.sort_by(|a, b| b.date.cmp(&a.date));
    out
}

const SCHEDULES: [i64; 8] = [52, 26, 24, 12, 6, 4, 2, 1];

/// `payments_per_year`: the frequency read from the payment dates
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
    // `min` keeps the first of equal distances.
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
