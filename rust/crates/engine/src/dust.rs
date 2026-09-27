//! Dust (`SPEC.md` §2, Dust): an amount of a coin worth less than the smallest
//! coin order its account's broker takes. It cannot be sold, so it is not a
//! holding: what a fill leaves of a coin worth less is written off with it, and
//! a difference from the broker worth less is not a disagreement. A broker whose
//! smallest order is not known leaves nothing dust.

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, Dec, InstrumentId, Money};

use crate::input::Inputs;

/// Whether `qty` of a coin, at `price` a unit in its own currency, is worth less
/// than the smallest coin order the account's broker takes, the value converted
/// at `day`'s rate where the two currencies differ. Not dust where any of it is
/// unknown: an instrument that is not a coin, a broker with no smallest order,
/// a price of nothing, a rate or product that cannot be worked out.
pub fn is_dust(inputs: &Inputs, account: AccountId, instrument: InstrumentId, qty: Dec, price: Dec, day: Date) -> bool {
    let Some(info) = inputs.ledger.instruments.get(&instrument).filter(|i| i.instrument.kind == InstrumentKind::Crypto) else { return false };
    let Some(smallest) = inputs.ledger.accounts.get(&account).and_then(|a| a.coin_minimum) else { return false };
    if !price.is_positive() {
        return false;
    }
    let Ok(value) = qty.abs().mul_to_fit(price) else { return false };
    let value = Money::new(value, info.instrument.currency);
    crate::fx::convert(&inputs.facts.rates, &inputs.clock, value, smallest.currency, day).is_ok_and(|v| v < smallest.amount)
}
