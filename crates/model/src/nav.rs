//! The equity curve and what it says: yearly time-weighted returns, the
//! annualized rate, and the drawdown.
//!
//! Returns are net of deposits and withdrawals throughout: money moved into or
//! out of the account is never counted as a gain or a loss.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use crate::dates::{days_between, shift_date};
use crate::value::{field_s, get, num, EPS};

#[derive(Clone, Debug)]
pub struct Point {
    pub d: String,
    pub v: f64,
    pub dep: Option<f64>,
}

impl Point {
    pub fn to_json(&self) -> Value {
        json!({"d": self.d, "v": self.v, "dep": self.dep})
    }
}

/// A number that is absent rather than zero, as Python's `_num(x, None)`.
fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

/// `model.equity_series`.
pub fn equity_series(points: &[Value]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for p in points {
        let d: String = field_s(p, "date").chars().take(10).collect();
        let v = match opt_num(get(p, "equity")) { Some(v) => v, None => continue };
        if d.is_empty() {
            continue;
        }
        out.push(Point { d, v, dep: opt_num(get(p, "netDeposits")) });
    }
    out.sort_by(|a, b| a.d.cmp(&b.d));
    out
}

/// `model._nav_on`: the equity as of the end of a day.
fn nav_on(series: &[Point], day: &str) -> Option<f64> {
    let mut v = None;
    for p in series {
        if p.d.as_str() > day {
            break;
        }
        v = Some(p.v);
    }
    v
}

/// `model._deposits_on`: the running net deposits as of the end of a day.
fn deposits_on(series: &[Point], day: &str) -> Option<f64> {
    let mut v = None;
    for p in series {
        if p.d.as_str() > day {
            break;
        }
        if p.dep.is_some() {
            v = p.dep;
        }
    }
    v
}

pub struct YearSpan {
    pub r: f64,
    pub from: String,
    pub to: String,
    pub days: i64,
}

/// `model.year_return`: the daily chain-linked return for one calendar year,
/// net of deposits.
///
/// A balance under 1% of the account's peak is pre-history -- a few dollars
/// parked before the real start -- and a chain that began there would turn the
/// first real deposit into a wild return. The chain starts at the first point
/// clearing that floor, and the year is measured from there.
pub fn year_return(series: &[Point], year: &str, today: &str) -> Option<YearSpan> {
    if series.is_empty() {
        return None;
    }
    let cal = format!("{}-01-01", year);
    let year_end = format!("{}-12-31", year);
    let to = if year_end.as_str() < today { year_end } else { today.to_string() };

    let floor = series.iter().map(|p| p.v).fold(f64::NEG_INFINITY, f64::max) * 0.01;
    let start_day = shift_date(&cal, -1);
    let mut start = nav_on(series, &start_day);
    let mut after = start_day.clone();

    if !start.map_or(false, |s| s > floor) {
        let first = series.iter().find(|p| cal <= p.d && p.d <= to && p.v > floor)?;
        start = Some(first.v);
        after = first.d.clone();
    }
    let start = start?;

    let pts: Vec<&Point> = series.iter().filter(|p| p.d > after && p.d <= to).collect();
    if pts.is_empty() {
        return None;
    }

    let mut prev_eq = start;
    let mut prev_dep = deposits_on(series, &after);
    let mut factor = 1.0_f64;
    for p in pts {
        let eq = p.v;
        if !(prev_eq > 0.0) {
            return None;
        }
        let mut cf = 0.0;
        if let (Some(pd), Some(prev)) = (p.dep, prev_dep) {
            cf = pd - prev;
        }
        factor *= 1.0 + (eq - prev_eq - cf) / prev_eq;
        prev_eq = eq;
        if p.dep.is_some() {
            prev_dep = p.dep;
        }
    }
    let r = factor - 1.0;
    if r.is_nan() || r.is_infinite() {
        return None;
    }
    let span_from = if after == start_day { cal } else { after };
    let days = days_between(&span_from, &to);
    Some(YearSpan { r, from: span_from, to, days })
}

/// `model.benchmark_return`: the index over the same span the account's year
/// covers -- the calendar year, or from `start` when the account was funded
/// part way through it.
pub fn benchmark_return(bench: &BTreeMap<String, f64>, year: &str, today: &str, start: Option<&str>) -> Option<f64> {
    if bench.is_empty() {
        return None;
    }
    let cal = match start {
        Some(s) if !s.is_empty() => s.chars().take(10).collect::<String>(),
        _ => format!("{}-01-01", year),
    };
    let year_end = format!("{}-12-31", year);
    let to = if year_end.as_str() < today { year_end } else { today.to_string() };

    let mut prev: Option<f64> = None;
    let mut end: Option<f64> = None;
    for (d, v) in bench {
        if *d < cal {
            prev = Some(*v);
        } else if *d <= to {
            end = Some(*v);
        }
    }
    if prev.is_none() {
        let first = bench.iter().find(|(d, _)| **d >= cal && **d <= to)?;
        prev = Some(*first.1);
    }
    let prev = prev?;
    let end = end?;
    if prev == 0.0 {
        return None;
    }
    Some(end / prev - 1.0)
}

/// `model.yearly_returns`.
pub fn yearly_returns(series: &[Point], bench: &BTreeMap<String, f64>, today: &str) -> Vec<Value> {
    if series.is_empty() {
        return vec![];
    }
    let mut years: Vec<String> = series.iter().map(|p| p.d.chars().take(4).collect()).collect();
    years.sort();
    years.dedup();

    let peak = series.iter().map(|p| p.v).fold(f64::NEG_INFINITY, f64::max);
    let mut out = Vec::new();
    for y in years {
        // A year in which the account never held more than 1% of its peak is
        // pre-history, not a year of trading.
        let year_peak = series
            .iter()
            .filter(|p| p.d.starts_with(&y))
            .map(|p| p.v)
            .fold(0.0_f64, f64::max);
        if peak > 0.0 && year_peak < peak * 0.01 {
            continue;
        }
        let yr = match year_return(series, &y, today) { Some(v) => v, None => continue };
        let jan1 = format!("{}-01-01", y);
        let start_dep = deposits_on(series, &shift_date(&jan1, -1));
        let end_dep = deposits_on(series, &yr.to);
        let flow = match (start_dep, end_dep) {
            (Some(s), Some(e)) => Some(e - s),
            _ => None,
        };
        let end_v = nav_on(series, &yr.to);
        let sp_start = if yr.from != jan1 { Some(yr.from.as_str()) } else { None };
        out.push(json!({
            "year": y,
            "r": yr.r,
            "days": yr.days,
            "from": yr.from,
            "to": yr.to,
            "flow": flow,
            "endV": end_v,
            "spR": benchmark_return(bench, &y, today, sp_start),
        }));
    }
    out
}

/// `model.annualized`: the chain of usable years turned into a yearly rate.
pub fn annualized(years: &[Value]) -> Value {
    let mut prod = 1.0_f64;
    let mut days = 0_i64;
    let mut used: Vec<String> = Vec::new();
    for y in years {
        let r = match opt_num(get(y, "r")) { Some(r) => r, None => continue };
        let d = num(get(y, "days"), 0.0) as i64;
        if r <= -1.0 || d < 30 {
            continue;
        }
        prod *= 1.0 + r;
        days += d;
        used.push(field_s(y, "year"));
    }
    if days == 0 {
        return json!({"rate": Value::Null, "years": 0.0, "count": 0, "first": "", "last": ""});
    }
    let yrs = days as f64 / 365.25;
    let rate = if yrs >= 1.0 / 12.0 { prod.powf(1.0 / yrs) - 1.0 } else { prod - 1.0 };
    json!({
        "rate": rate,
        "years": yrs,
        "count": used.len(),
        "first": used.first().cloned().unwrap_or_default(),
        "last": used.last().cloned().unwrap_or_default(),
    })
}

/// `model._paired_flows`: the net deposit change per day, moved one day later
/// when the equity series only reflects the money the day after the deposit
/// record does.
fn paired_flows(series: &[Point]) -> Vec<f64> {
    let n = series.len();
    let mut flows = vec![0.0; n];
    for i in 1..n {
        let (p, prev) = (&series[i], &series[i - 1]);
        let (pd, prevd) = match (p.dep, prev.dep) { (Some(a), Some(b)) => (a, b), _ => continue };
        let cf = pd - prevd;
        if cf.abs() < EPS {
            continue;
        }
        let change_today = p.v - prev.v;
        if i + 1 < n {
            let change_next = series[i + 1].v - p.v;
            if (change_today - cf).abs() > (change_next - cf).abs() && change_today.abs() < cf.abs() * 0.5 {
                flows[i + 1] += cf;
                continue;
            }
        }
        flows[i] += cf;
    }
    flows
}

/// `model.drawdown`: the deepest fall of the flow-adjusted equity index.
pub fn drawdown(series: &[Point]) -> Value {
    if series.is_empty() {
        return json!({"pct": Value::Null, "abs": Value::Null, "at": "", "peakAt": ""});
    }
    let peak_v = series.iter().map(|p| p.v).fold(f64::NEG_INFINITY, f64::max);
    let floor = peak_v * 0.01;
    let mut idx = 1.0_f64;
    let mut prev: Option<&Point> = None;
    let mut peak_idx = 0.0_f64;
    let mut peak_at = String::new();
    let mut peak_equity = 0.0_f64;
    let mut dd = 0.0_f64;
    let mut dd_abs = 0.0_f64;
    let mut dd_at = String::new();
    let mut dd_peak_at = String::new();
    let flows = paired_flows(series);

    for (i, p) in series.iter().enumerate() {
        if let Some(pv) = prev {
            if pv.v > floor && pv.v > 0.0 {
                idx *= 1.0 + (p.v - pv.v - flows[i]) / pv.v;
            }
        }
        prev = Some(p);
        if p.v < floor {
            continue;
        }
        if idx >= peak_idx {
            peak_idx = idx;
            peak_at = p.d.clone();
            peak_equity = p.v;
        }
        if peak_idx <= 0.0 {
            continue;
        }
        let drop = idx / peak_idx - 1.0;
        if drop < dd {
            dd = drop;
            dd_abs = drop * peak_equity;
            dd_at = p.d.clone();
            dd_peak_at = peak_at.clone();
        }
    }
    json!({"pct": dd, "abs": dd_abs, "at": dd_at, "peakAt": dd_peak_at})
}

/// The benchmark map as the model reads it: dates to levels, in date order.
pub fn bench_map(v: Option<&Value>) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(m)) = v {
        for (k, val) in m {
            if let Some(f) = opt_num(Some(val)) {
                out.insert(k.clone(), f);
            }
        }
    }
    out
}

/// The equity series as the page receives it.
pub fn series_json(series: &[Point]) -> Vec<Value> {
    series.iter().map(|p| p.to_json()).collect()
}

/// Per-account equity series, keyed by the normalized nickname.
pub fn by_account(v: Option<&Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(Value::Object(m)) = v {
        for (nick, pts) in m {
            let arr: Vec<Value> = pts.as_array().cloned().unwrap_or_default();
            let s = equity_series(&arr);
            out.insert(crate::value::norm_account_name(nick), Value::Array(series_json(&s)));
        }
    }
    out
}
