//! Returns on the equity series (`SPEC.md` §2 Equity and returns): yearly
//! returns chain-linked from the daily ones, net of the money moved in and out;
//! the annualized figure; the drawdown of the flow-adjusted index.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::Dec;

/// One day of the series in scope: its value, and the day's return where one
/// was formed (between two consecutive stated days, `equity::account_equity`).
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub day: Date,
    pub value: f64,
    /// The same value exactly, as the accounts' stated values add up; none when
    /// the sum is too large to hold. What is shown; `value` is what is computed with.
    pub exact: Option<Dec>,
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

/// A statistic's amount in dollars, to the cent: the one place a statistic's
/// float becomes an amount that is shown (a drawdown's size at its peak).
pub fn cents(x: f64) -> Option<Dec> {
    x.is_finite().then(|| Dec::parse(&format!("{x:.2}")).ok()).flatten()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drawdown {
    pub pct: Option<f64>,
    /// The fall in CAD at the peak's value.
    pub abs: Option<f64>,
    pub at: Option<Date>,
    pub peak_at: Option<Date>,
}

/// The accounts' combined series (`SPEC.md` §2, Equity series). A combined day is
/// one on which every account that has begun and whose statements have not ended
/// states a value: an account counts from its first stated day to its last, so one
/// whose statements stop leaves the series from its last day, never ends it. Each
/// combined day's return chains from the combined day before it: every account
/// stating a value on both days is measured over exactly that interval, its value
/// on the later day less the money moved in or out between them, over its value on
/// the earlier day, and the accounts' returns are weighted by their values on the
/// earlier day (GIPS 2010, I.2.A.6: composite returns asset-weighted by beginning-
/// of-period values). An account whose deposits over the interval are not all known
/// forms no return for it; a day one account did not state drops nobody's return,
/// since the interval runs over it. Each account's points are its stated days, oldest
/// first, with the money moved since its stated day before.
pub fn combine(accounts: &[&[crate::equity::DayValue]]) -> Vec<Day> {
    use std::collections::BTreeSet;
    // each account's span: its first and last stated day
    let spans: Vec<Option<(Date, Date)>> = accounts.iter().map(|p| Some((p.first()?.day, p.last()?.day))).collect();
    let on = |a: usize, d: Date| accounts[a].binary_search_by_key(&d, |p| p.day).ok().map(|i| &accounts[a][i]);
    let days: BTreeSet<Date> = accounts.iter().flat_map(|p| p.iter().map(|x| x.day)).collect();
    let combined: Vec<Date> = days
        .into_iter()
        .filter(|d| (0..accounts.len()).all(|a| match spans[a] {
            Some((first, last)) if first <= *d && *d <= last => on(a, *d).is_some(),
            _ => true,
        }))
        .collect();
    let mut out = Vec::with_capacity(combined.len());
    let mut before: Option<Date> = None;
    for d in combined {
        let here: Vec<usize> = (0..accounts.len()).filter(|a| on(*a, d).is_some()).collect();
        // what moved in or out of each account since the combined day before, where every step of it is known
        let moved = |a: usize, from: Date| -> Option<Dec> {
            accounts[a].iter().filter(|p| from < p.day && p.day <= d).try_fold(Dec::ZERO, |sum, p| sum.checked_add(p.flow?).ok())
        };
        let (mut weighted, mut weights) = (0.0, 0.0);
        let mut flow: Option<f64> = Some(0.0);
        for &a in &here {
            let now = on(a, d).expect("here");
            match before.and_then(|b| on(a, b).map(|p| (b, p))) {
                Some((b, then)) => match moved(a, b) {
                    Some(f) => {
                        if let Some(r) = crate::stat::day_return(then.value, now.value, f) {
                            weighted += r * then.value.to_f64();
                            weights += then.value.to_f64();
                        }
                        flow = flow.map(|x| x + f.to_f64());
                    }
                    None => flow = None,
                },
                // its first day in the series: it forms no return yet, and the money it
                // came in with is not known as a movement of the whole, as on any first day
                None => flow = None,
            }
        }
        out.push(Day {
            day: d,
            value: here.iter().map(|a| on(*a, d).expect("here").value.to_f64()).sum(),
            exact: here.iter().try_fold(Dec::ZERO, |sum, a| sum.checked_add(on(*a, d).expect("here").value)).ok(),
            ret: (before.is_some() && weights > 0.0).then(|| weighted / weights),
            flow: if before.is_some() { flow } else { None },
        });
        before = Some(d);
    }
    out
}

/// The pre-history floor: a balance under 1 % of the series' peak. Always
/// taken from the whole history in scope, never from a date range, so a year's
/// return never changes with the range chosen.
pub fn floor(series: &[Day]) -> f64 {
    series.iter().map(|d| d.value).fold(0.0, f64::max) * 0.01
}

/// The index over the same span: its last level on or before the end over its
/// last level before the start (or its first in the span).
pub fn benchmark_return(levels: &BTreeMap<Date, f64>, from: Date, to: Date) -> Option<f64> {
    let start = levels.range(..from).next_back().or_else(|| levels.range(from..=to).next()).map(|(_, v)| *v)?;
    let end = levels.range(..=to).next_back().map(|(_, v)| *v)?;
    (start != 0.0).then(|| end / start - 1.0)
}

/// Every calendar year of the series that `span` keeps, newest last, each over
/// the calendar days `span` gives it (the whole year, or the part of it inside
/// a date range). A balance under `floor` is pre-history: a year that never
/// clears it is left out, and a year that first clears it part way through is
/// measured from that first day.
pub fn yearly_returns(series: &[Day], today: Date, benchmark: Option<&BTreeMap<Date, f64>>, floor: f64, span: impl Fn(i16) -> Option<(Date, Date)>) -> Vec<YearReturn> {
    let mut years: Vec<i16> = series.iter().map(|d| d.day.year()).collect();
    years.dedup();
    let mut out = Vec::new();
    for y in years {
        let Some((first, last)) = span(y) else { continue };
        let to = last.min(today);
        let in_year: Vec<&Day> = series.iter().filter(|d| first <= d.day && d.day <= to).collect();
        if in_year.iter().all(|d| d.value < floor) {
            continue;
        }
        // the span opens on the last day before it, if that cleared the floor
        let before = series.iter().rev().find(|d| d.day < first).filter(|d| d.value > floor);
        // `base` is the day the year's return is measured over: the last value
        // before it, or its own first point clear of the floor
        let (from, start_day, base) = match before {
            Some(b) => (first, None, b.day),
            None => match in_year.iter().find(|d| d.value > floor) {
                Some(d) => (d.day, Some(d.day), d.day),
                None => continue,
            },
        };
        let mut factor = 1.0;
        let mut any = false;
        let mut flow = Some(0.0);
        for d in in_year.iter().filter(|d| start_day.is_none_or(|s| d.day > s)) {
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
            days: (to - base).get_days() as i64,
            flow,
            end_value,
            benchmark: benchmark.and_then(|b| benchmark_return(b, from, to)),
        });
    }
    out
}

/// A span of this many days or more is a year or more.
const YEAR_DAYS: i64 = 365;

/// The years compounded over the days they cover, and annualized only when
/// those days make a year or more: a shorter span is its return over the span,
/// as GIPS rules (only periods of a year or more are annualized). A year of
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
        rate: Some(if days >= YEAR_DAYS { prod.powf(1.0 / yrs) - 1.0 } else { prod - 1.0 }),
        years: yrs,
        count: used.len(),
        first: used.first().map(|y| y.year),
        last: used.last().map(|y| y.year),
    }
}

/// The deepest fall of the index chained from the daily returns, over the days
/// that clear the pre-history floor, the peak taken from the first day given.
pub fn drawdown(series: &[Day], floor: f64) -> Drawdown {
    if series.is_empty() {
        return Drawdown { pct: None, abs: None, at: None, peak_at: None };
    }
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

#[cfg(test)]
mod tests {
    use bagholder_core::jiff::civil::date;

    use super::*;

    fn whole(y: i16) -> Option<(Date, Date)> {
        Some((date(y, 1, 1), date(y, 12, 31)))
    }

    fn day(d: Date, value: f64, ret: Option<f64>) -> Day {
        Day { day: d, value, exact: None, ret, flow: Some(0.0) }
    }

    #[test]
    fn a_span_under_a_year_is_its_return_not_annualized() {
        // two months at +15 %: 15 %, never compounded up to a year's (+131 %)
        let series = [day(date(2026, 7, 1), 100.0, None), day(date(2026, 8, 31), 115.0, Some(0.15))];
        let years = yearly_returns(&series, date(2026, 8, 31), None, floor(&series), whole);
        let a = annualized(&years);
        assert!((a.rate.unwrap() - 0.15).abs() < 1e-12, "{:?}", a.rate);
    }

    #[test]
    fn a_year_counts_its_days_from_the_value_it_is_measured_over() {
        // five full years at 10 % each, measured from each year's last value before it
        let mut series = vec![day(date(2020, 12, 31), 100.0, None)];
        let mut v = 100.0;
        for y in 2021..=2025 {
            v *= 1.1;
            series.push(day(date(y, 12, 31), v, Some(0.1)));
        }
        let years = yearly_returns(&series, date(2025, 12, 31), None, floor(&series), whole);
        assert_eq!(years.iter().map(|y| y.days).collect::<Vec<_>>(), vec![365, 365, 365, 366, 365]);
        let a = annualized(&years);
        assert!((a.rate.unwrap() - 0.10).abs() < 5e-5, "{:?}", a.rate);
    }

    #[test]
    fn a_span_cut_by_a_range_is_measured_from_the_last_value_before_it_and_the_floor_is_the_whole_historys() {
        // 2025: 100 → 110 on Jul 1 → 121 on Dec 31; a range from Jul 1 counts
        // the move on Jul 1 and after, from Jun 30's value
        let series = [day(date(2025, 6, 30), 100.0, Some(0.0)), day(date(2025, 7, 1), 110.0, Some(0.10)), day(date(2025, 12, 31), 121.0, Some(0.10))];
        let from_july = |y: i16| (y == 2025).then(|| (date(2025, 7, 1), date(2025, 12, 31)));
        let years = yearly_returns(&series, date(2026, 1, 5), None, floor(&series), from_july);
        assert_eq!(years.len(), 1);
        assert!((years[0].r - 0.21).abs() < 1e-12, "{:?}", years[0].r);
        assert_eq!((years[0].from, years[0].days), (date(2025, 7, 1), 184));

        // a small year the whole history's floor leaves out stays out under a
        // range that holds nothing larger
        let series = [day(date(2019, 6, 1), 5.0, None), day(date(2019, 12, 31), 6.0, Some(0.2)), day(date(2026, 1, 2), 60_000.0, Some(0.0))];
        let only_2019 = |y: i16| (y == 2019).then(|| (date(2019, 1, 1), date(2019, 12, 31)));
        let f = floor(&series);
        assert!(yearly_returns(&series, date(2026, 1, 5), None, f, only_2019).is_empty());
        assert_eq!(drawdown(&series[..2], f).at, None, "under the floor, no fall is measured");
    }
}
