//! The statistics: ratios, rates and returns, the one place the engine uses
//! floating point (`docs/architecture.md` §6). Each is made from exact figures
//! through `Dec::to_f64`, and nothing comes back from here into money.

pub mod returns;

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Dec, Money};

/// A ratio, a rate or a return: a statistic, never money.
pub type Ratio = f64;

/// `part / whole` as a ratio; none when the whole is zero.
pub fn ratio(part: Dec, whole: Dec) -> Option<f64> {
    if whole.is_zero() {
        return None;
    }
    Some(part.to_f64() / whole.to_f64())
}

/// A money amount over another, when they are in one currency.
pub fn money_ratio(part: Money, whole: Money) -> Option<f64> {
    if part.currency != whole.currency {
        return None;
    }
    ratio(part.amount, whole.amount)
}

/// `n / d` for counts.
pub fn count_ratio(n: usize, d: usize) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

/// The schedules a payout frequency is snapped to (`SPEC.md` §2).
pub const SCHEDULES: [u32; 8] = [52, 26, 24, 12, 6, 4, 2, 1];

/// Payments per year from payment or ex-dates: the median of the last three
/// gaps between distinct days, snapped to the nearest schedule (the first of two
/// equally near). Fewer than two distinct days say nothing.
pub fn payments_per_year(days: &[Date]) -> Option<u32> {
    let mut days: Vec<Date> = days.to_vec();
    days.sort();
    days.dedup();
    let mut gaps: Vec<i64> = days.windows(2).map(|w| (w[1] - w[0]).get_days() as i64).filter(|g| *g > 0).collect();
    if gaps.is_empty() {
        return None;
    }
    if gaps.len() > 3 {
        gaps = gaps.split_off(gaps.len() - 3);
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    let per_year = 365.25 / median as f64;
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

/// One day's return from the value before, the value after and the money moved
/// in (positive) or out between them: `(after − flow) ÷ before − 1`.
pub fn day_return(before: Dec, after: Dec, flow: Dec) -> Option<f64> {
    let b = before.to_f64();
    if b <= 0.0 {
        return None;
    }
    Some((after.to_f64() - flow.to_f64()) / b - 1.0)
}
