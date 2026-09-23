//! Returns on the equity series (`SPEC.md` §2 Equity and returns): yearly
//! returns chain-linked from the daily ones, net of the money moved in and out;
//! the annualized figure; the drawdown of the flow-adjusted index.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::Dec;

/// One day of the series in scope: its value, and the day's return where one
/// was formed (between two days of the same source, `equity::daily_returns`).
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub day: Date,
    pub value: f64,
    pub ret: Option<f64>,
    /// The money moved in (positive) or out that day, where known.
    pub flow: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct YearReturn {
    pub year: i16,
    pub r: f64,
    pub from: Date,
    pub to: Date,
    pub days: i64,
    /// The money moved in less out over the span, where every day's is known.
    pub flow: Option<f64>,
    pub end_value: Option<f64>,
    pub benchmark: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annualized {
    pub rate: Option<f64>,
    pub years: f64,
    pub count: usize,
    pub first: Option<i16>,
    pub last: Option<i16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drawdown {
    pub pct: Option<f64>,
    /// The fall in CAD at the peak's value.
    pub abs: Option<f64>,
    pub at: Option<Date>,
    pub peak_at: Option<Date>,
}

/// The series in scope from the complete days' values and flows, and each
/// account's daily returns weighted by the value each is over.
pub fn combine(values: &BTreeMap<Date, (Dec, Option<Dec>)>, accounts: &[&[(Date, f64, Dec)]]) -> Vec<Day> {
    let mut rets: BTreeMap<Date, (f64, f64)> = BTreeMap::new();
    for a in accounts {
        for (d, r, w) in a.iter() {
            let w = w.to_f64();
            let x = rets.entry(*d).or_insert((0.0, 0.0));
            x.0 += r * w;
            x.1 += w;
        }
    }
    values
        .iter()
        .map(|(d, (v, flow))| Day { day: *d, value: v.to_f64(), ret: rets.get(d).and_then(|(s, w)| (*w > 0.0).then(|| s / w)), flow: flow.map(|x| x.to_f64()) })
        .collect()
}

fn peak(series: &[Day]) -> f64 {
    series.iter().map(|d| d.value).fold(0.0, f64::max)
}

/// The index over the same span: its last level on or before the end over its
/// last level before the start (or its first in the span).
pub fn benchmark_return(levels: &BTreeMap<Date, Dec>, from: Date, to: Date) -> Option<f64> {
    let start = levels.range(..from).next_back().or_else(|| levels.range(from..=to).next()).map(|(_, v)| v.to_f64())?;
    let end = levels.range(..=to).next_back().map(|(_, v)| v.to_f64())?;
    (start != 0.0).then(|| end / start - 1.0)
}

/// Every calendar year of the series, newest last. A balance under 1 % of the
/// series' peak is pre-history: a year that never clears it is left out, and a
/// year that first clears it part way through is measured from that first day.
pub fn yearly_returns(series: &[Day], today: Date, benchmark: Option<&BTreeMap<Date, Dec>>) -> Vec<YearReturn> {
    let floor = peak(series) * 0.01;
    let mut years: Vec<i16> = series.iter().map(|d| d.day.year()).collect();
    years.dedup();
    let mut out = Vec::new();
    for y in years {
        let in_year: Vec<&Day> = series.iter().filter(|d| d.day.year() == y).collect();
        if in_year.iter().all(|d| d.value < floor) {
            continue;
        }
        let jan1 = Date::new(y, 1, 1).ok();
        let dec31 = Date::new(y, 12, 31).ok();
        let (Some(jan1), Some(dec31)) = (jan1, dec31) else { continue };
        let to = dec31.min(today);
        // the year opens on the last day of the year before, if it cleared the floor
        let before = series.iter().rev().find(|d| d.day < jan1).filter(|d| d.value > floor);
        let (from, start_day) = match before {
            Some(_) => (jan1, None),
            None => match in_year.iter().find(|d| d.value > floor) {
                Some(d) => (d.day, Some(d.day)),
                None => continue,
            },
        };
        let mut factor = 1.0;
        let mut any = false;
        let mut flow = Some(0.0);
        for d in in_year.iter().filter(|d| d.day <= to && start_day.is_none_or(|s| d.day > s)) {
            if let Some(r) = d.ret {
                factor *= 1.0 + r;
                any = true;
            }
            flow = match (flow, d.flow) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
        }
        if !any || !factor.is_finite() {
            continue;
        }
        let end_value = series.iter().rev().find(|d| d.day <= to).map(|d| d.value);
        out.push(YearReturn {
            year: y,
            r: factor - 1.0,
            from,
            to,
            days: (to - from).get_days() as i64,
            flow,
            end_value,
            benchmark: benchmark.and_then(|b| benchmark_return(b, from, to)),
        });
    }
    out
}

/// The years compounded and annualized over the days they cover; a year of
/// fewer than 30 days is left out.
pub fn annualized(years: &[YearReturn]) -> Annualized {
    let used: Vec<&YearReturn> = years.iter().filter(|y| y.r > -1.0 && y.days >= 30).collect();
    let days: i64 = used.iter().map(|y| y.days).sum();
    if days == 0 {
        return Annualized { rate: None, years: 0.0, count: 0, first: None, last: None };
    }
    let prod: f64 = used.iter().map(|y| 1.0 + y.r).product();
    let yrs = days as f64 / 365.25;
    Annualized {
        rate: Some(if yrs >= 1.0 / 12.0 { prod.powf(1.0 / yrs) - 1.0 } else { prod - 1.0 }),
        years: yrs,
        count: used.len(),
        first: used.first().map(|y| y.year),
        last: used.last().map(|y| y.year),
    }
}

/// The deepest fall of the index chained from the daily returns, over the days
/// that clear the pre-history floor.
pub fn drawdown(series: &[Day]) -> Drawdown {
    if series.is_empty() {
        return Drawdown { pct: None, abs: None, at: None, peak_at: None };
    }
    let floor = peak(series) * 0.01;
    let mut idx = 1.0;
    let mut peak_idx = 0.0;
    let mut peak_at = None;
    let mut peak_value = 0.0;
    let mut dd = 0.0;
    let mut out = Drawdown { pct: Some(0.0), abs: Some(0.0), at: None, peak_at: None };
    for d in series {
        if let Some(r) = d.ret {
            idx *= 1.0 + r;
        }
        if d.value < floor {
            continue;
        }
        if idx >= peak_idx {
            peak_idx = idx;
            peak_at = Some(d.day);
            peak_value = d.value;
        }
        if peak_idx <= 0.0 {
            continue;
        }
        let drop = idx / peak_idx - 1.0;
        if drop < dd {
            dd = drop;
            out = Drawdown { pct: Some(drop), abs: Some(drop * peak_value), at: Some(d.day), peak_at };
        }
    }
    out
}
