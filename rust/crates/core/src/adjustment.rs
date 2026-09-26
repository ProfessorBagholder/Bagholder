//! Adjustments: what a corporate event did, or what the person states about a
//! transaction (`docs/plans/stage-2-engine.md`, "Facts and adjustments in the
//! book"). An adjustment is derived from a record like a transaction (the
//! issuer's notice, the exchange's bulletin, the person's own entry) and explains
//! one transaction; a sourced one supersedes the person's.

use crate::dec::Dec;
use crate::ids::{InstrumentId, TransactionId};
use crate::money::Money;
use crate::names::SourceName;

/// One leg of an adjustment: what happened to one holding, or what the person
/// states about one.
///
/// | Event | Legs |
/// |---|---|
/// | split or consolidation | `from = to = X`, `units_per_unit` the ratio |
/// | a holding continuing under a new instrument | `from X, to Y`, `units_per_unit`, `cost_share` 1 |
/// | stock dividend | `from = to = X`, `units_per_unit` new units per unit held, `cost` their value (income) |
/// | spin-off | `from X, to Y`, `units_per_unit`, `cost_share` the part of X's cost Y takes; one leg per child |
/// | merger for cash, cash in lieu | `from X`, no `to`, `cash_per_unit` |
/// | merger for shares and cash | one leg to the new shares, one for the cash |
/// | return of capital | `from = to = X`, `cash_per_unit`, reducing the cost |
/// | the person's cost for a deposit | no `from`, `to X`, `cost` and `acquired` |
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdjustmentLeg {
    pub from: Option<InstrumentId>,
    pub to: Option<InstrumentId>,
    /// Units of `to` per unit of `from` held.
    pub units_per_unit: Option<Dec>,
    /// The share of `from`'s cost that moves to `to`.
    pub cost_share: Option<Dec>,
    /// Cash per unit of `from` held.
    pub cash_per_unit: Option<Money>,
    /// A stated cost or value, for the whole leg.
    pub cost: Option<Money>,
    /// A stated day of acquisition.
    pub acquired: Option<jiff::civil::Date>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adjustment {
    /// The transaction it explains: a corporate event, or a deposit whose cost
    /// the person states.
    pub applies_to: TransactionId,
    pub legs: Vec<AdjustmentLeg>,
    pub source: SourceName,
}
