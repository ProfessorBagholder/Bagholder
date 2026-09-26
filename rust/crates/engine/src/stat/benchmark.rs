//! Each benchmark as a total return in CAD (`SPEC.md` §2, Index; owner,
//! 2026-09-24): the return the person's own figure is, their account's value
//! net of the money moved, with dividends staying in it.
//!
//! A benchmark is read from an ETF that tracks its index. Each session's return
//! is its close over the session before's, the dividend going ex on it taken
//! out of the price before and a split taking effect on it counted back:
//! `close × n ÷ d ÷ (previous close − dividend × n ÷ d)`, the adjustment Yahoo's
//! adjusted close makes. A tracker in another currency is converted at each
//! session's Bank of Canada rate, so the return is the one a holder in CAD had.
//! The returns chain into a level from the first session with a rate; a session
//! whose rate is not published yet, or whose price before the dividend is not
//! above zero, has no level, and the next session's return is taken over the
//! last one that has.
//!
//! The level is a ratio, not money, and is a binary float like every return.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::Currency;

use crate::fx::rate;
use crate::input::{BenchmarkSeries, Clock, Rates};

/// A benchmark's level per session.
pub type Levels = BTreeMap<Date, f64>;

/// The tracker's total return in CAD, as a level per session.
pub fn total_return_cad(s: &BenchmarkSeries, rates: &Rates, clock: &Clock) -> Levels {
    let cad = |day: Date| -> Option<f64> {
        if s.currency == Currency::CAD {
            return Some(1.0);
        }
        rate(rates, clock, s.currency, day).ok().map(|r| r.to_f64())
    };
    let mut out = BTreeMap::new();
    // the last session with a level: its day, close, rate and level
    let mut last: Option<(Date, f64, f64, f64)> = None;
    for (day, close) in &s.closes {
        let close = close.to_f64();
        let Some(r) = cad(*day) else { continue };
        let Some((before, prev, prev_rate, level)) = last else {
            if close > 0.0 {
                out.insert(*day, 1.0);
                last = Some((*day, close, r, 1.0));
            }
            continue;
        };
        // every split and dividend after the last session with a level, through this one
        let split: f64 = s.splits.range(before..=*day).filter(|(d, _)| **d > before).map(|(_, (n, d))| n.to_f64() / d.to_f64()).product();
        let dividend: f64 = s.dividends.range(before..=*day).filter(|(d, _)| **d > before).map(|(_, a)| a.to_f64()).sum();
        let base = prev - dividend * split;
        if base <= 0.0 || !split.is_finite() || split <= 0.0 {
            continue;
        }
        let level = level * (close * split / base) * (r / prev_rate);
        if level.is_finite() {
            out.insert(*day, level);
            last = Some((*day, close, r, level));
        }
    }
    out
}

/// Every benchmark's level per session.
pub fn build_benchmarks(benchmarks: &BTreeMap<String, BenchmarkSeries>, rates: &Rates, clock: &Clock) -> BTreeMap<String, Levels> {
    benchmarks.iter().map(|(k, s)| (k.clone(), total_return_cad(s, rates, clock))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_core::Dec;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    fn n(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    fn clock() -> Clock {
        let bank = bagholder_core::jiff::tz::TimeZone::get("America/Toronto").unwrap();
        Clock { today: d("2026-01-10"), now: "2026-01-10T23:00:00Z".parse().unwrap(), home: bank.clone(), bank }
    }

    fn series(closes: &[(&str, &str)]) -> BenchmarkSeries {
        BenchmarkSeries { currency: Currency::CAD, closes: closes.iter().map(|(a, b)| (d(a), n(b))).collect(), dividends: BTreeMap::new(), splits: BTreeMap::new() }
    }

    #[test]
    fn a_dividend_stays_in_the_return() {
        // 100, then 99 on the day 1 goes ex: 99 ÷ (100 − 1) = no change
        let mut s = series(&[("2026-01-05", "100"), ("2026-01-06", "99"), ("2026-01-07", "108.9")]);
        s.dividends.insert(d("2026-01-06"), n("1"));
        let l = total_return_cad(&s, &Rates::default(), &clock());
        assert_eq!(l[&d("2026-01-05")], 1.0);
        assert!((l[&d("2026-01-06")] - 1.0).abs() < 1e-12);
        assert!((l[&d("2026-01-07")] - 1.1).abs() < 1e-12);
    }

    #[test]
    fn a_split_is_counted_back() {
        // 2 for 1: 100, then 51 a unit of twice as many = 102 of the old unit
        let mut s = series(&[("2026-01-05", "100"), ("2026-01-06", "51")]);
        s.splits.insert(d("2026-01-06"), (n("2"), n("1")));
        let l = total_return_cad(&s, &Rates::default(), &clock());
        assert!((l[&d("2026-01-06")] - 1.02).abs() < 1e-12);
    }

    #[test]
    fn a_usd_tracker_moves_with_the_days_rate() {
        let mut s = series(&[("2026-01-05", "100"), ("2026-01-06", "100")]);
        s.currency = Currency::USD;
        let mut rates = Rates::default();
        rates.by_currency.insert(Currency::USD, BTreeMap::from([(d("2026-01-05"), n("1.40")), (d("2026-01-06"), n("1.47"))]));
        let l = total_return_cad(&s, &rates, &clock());
        assert!((l[&d("2026-01-06")] - 1.05).abs() < 1e-12);
    }

    #[test]
    fn a_session_without_its_rate_has_no_level_and_the_next_is_over_the_last_that_has() {
        let mut s = series(&[("2026-01-05", "100"), ("2026-01-06", "110"), ("2026-01-07", "121")]);
        s.currency = Currency::USD;
        let mut rates = Rates::default();
        rates.by_currency.insert(Currency::USD, BTreeMap::from([(d("2026-01-05"), n("1.40")), (d("2026-01-07"), n("1.40"))]));
        let l = total_return_cad(&s, &rates, &clock());
        assert!(!l.contains_key(&d("2026-01-06")));
        assert!((l[&d("2026-01-07")] - 1.21).abs() < 1e-12);
    }
}
