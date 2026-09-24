//! The statistics: ratios, rates and returns, the one place the engine uses
//! floating point (`docs/architecture.md` §6). Each is made from exact figures
//! through `Dec::to_f64`, and nothing comes back from here into money.

pub mod returns;

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

/// One day's return from the value before, the value after and the money moved
/// in (positive) or out between them: `(after − flow) ÷ before − 1`.
pub fn day_return(before: Dec, after: Dec, flow: Dec) -> Option<f64> {
    let b = before.to_f64();
    if b <= 0.0 {
        return None;
    }
    Some((after.to_f64() - flow.to_f64()) / b - 1.0)
}
