//! Converting a native figure to CAD on the day it happened.
//!
//! Per-trade numbers stay in the instrument's own currency; anything that adds
//! trades together uses the CAD value converted on the fill date with the Bank
//! of Canada rate. A rate is published on business days only, so a weekend or
//! a holiday walks back to the last one.

use std::collections::HashMap;

use crate::dates::{head10, shift_date};
use crate::fifo::Slice;
use crate::symbols::option_multiplier;

/// The rate used when the book carries none at all, so a missing rate cannot
/// empty a figure. `FX_FALLBACK`.
pub const FX_FALLBACK: f64 = 1.35;

pub type Fx = HashMap<String, f64>;

/// `rate_on`: the rate on the day, or the most recent one within a
/// fortnight before it.
pub fn rate_on(fx: &Fx, day: &str) -> f64 {
    let mut d = head10(day);
    if d.is_empty() {
        return FX_FALLBACK;
    }
    for _ in 0..12 {
        if let Some(r) = fx.get(&d) {
            if *r > 0.0 {
                return *r;
            }
        }
        d = shift_date(&d, -1);
    }
    FX_FALLBACK
}

/// `to_cad`: only USD is converted; everything else is already CAD.
pub fn to_cad(fx: &Fx, amount: f64, currency: &str, day: &str) -> f64 {
    let ccy = if currency.is_empty() { "CAD".to_string() } else { currency.to_uppercase() };
    if ccy != "USD" {
        return amount;
    }
    amount * rate_on(fx, day)
}

/// `apply_fx`: each leg is converted on its own date, so a trade held
/// across a move in the dollar keeps the gain the dollar made.
pub fn apply_fx(slices: &mut [Slice], fx: &Fx) {
    for t in slices.iter_mut() {
        let ccy = if t.currency.is_empty() { "CAD".to_string() } else { t.currency.to_uppercase() };
        if ccy != "USD" {
            t.pnl_cad = t.pnl;
            t.fees_cad = Some(t.commission);
            continue;
        }
        let qty = t.quantity;
        let mult = option_multiplier(&t.symbol);
        let entry_c = t.entry_commission;
        let exit_c = t.exit_commission;
        let entry_notional = t.entry_price * qty * mult;
        let exit_notional = t.exit_price * qty * mult;
        t.pnl_cad = if t.open_direction == "SHORT" {
            to_cad(fx, entry_notional - entry_c, &ccy, &t.entry_date)
                - to_cad(fx, exit_notional + exit_c, &ccy, &t.exit_date)
        } else {
            to_cad(fx, exit_notional - exit_c, &ccy, &t.exit_date)
                - to_cad(fx, entry_notional + entry_c, &ccy, &t.entry_date)
        };
        t.fees_cad = Some(to_cad(fx, entry_c, &ccy, &t.entry_date) + to_cad(fx, exit_c, &ccy, &t.exit_date));
    }
}
