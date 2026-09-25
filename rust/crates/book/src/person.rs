//! The facts the person enters (`SPEC.md` §2, "What you enter"): the cost of units
//! that arrived without one, and what a corporate event did to cost. Each entry
//! is a record whose source is the person, derived into an adjustment on the
//! transaction it explains; a source's value for the same fact replaces it
//! (`bagholder_engine::input::Adjustments::choose`).

use bagholder_core::instrument::{Reference, Strength};
use bagholder_core::{Currency, Dec, InstrumentId, Leg, Money, SourceName, TransactionId};
use serde::{Deserialize, Serialize};

use crate::mapping::{AdjustmentDraft, AdjustmentLegDraft, MapContext, Mapped, Mapping};
use crate::records::{Incoming, Stored};
use crate::{new_uuid, Book, BookError, Result};

/// One entry, as the person makes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// What units that arrived without a cost cost the person, and when they
    /// acquired them.
    CostOfArrival { arrival: TransactionId, instrument: InstrumentId, cost: Money, acquired: jiff::civil::Date },
    /// The share of the parent's cost each new holding takes, as the issuer published it.
    SpinOff { event: TransactionId, parent: InstrumentId, children: Vec<(InstrumentId, Dec)> },
    /// The capital a distribution returned per unit, as the issuer published it.
    ReturnOfCapital { distribution: TransactionId, instrument: InstrumentId, per_unit: Money },
}

/// The record an entry is kept as: instruments named by a reference any source
/// can match, never by the book's own ids.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "entry", rename_all = "kebab-case", deny_unknown_fields)]
enum Payload {
    CostOfArrival { applies_to: String, instrument: Vec<(String, String)>, cost: Amount, acquired: String },
    SpinOff { applies_to: String, parent: Vec<(String, String)>, children: Vec<Child> },
    ReturnOfCapital { applies_to: String, instrument: Vec<(String, String)>, per_unit: Amount },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Amount {
    amount: String,
    currency: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Child {
    instrument: Vec<(String, String)>,
    cost_share: String,
}

pub struct PersonMapping;

fn leg() -> Leg {
    Leg::named("entry")
}

fn refs(r: &[(String, String)]) -> std::result::Result<Vec<Reference>, String> {
    r.iter().map(|(s, v)| Ok(Reference::new(bagholder_core::instrument::RefScheme::parse(s).map_err(|e| e.to_string())?, v.clone()))).collect()
}

fn money(a: &Amount) -> std::result::Result<Money, String> {
    Ok(Money::new(Dec::parse(&a.amount).map_err(|e| e.to_string())?, Currency::parse(&a.currency).map_err(|e| e.to_string())?))
}

impl Mapping for PersonMapping {
    fn source(&self) -> SourceName {
        SourceName::person()
    }

    fn version(&self) -> u32 {
        1
    }

    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let p: Payload = match serde_json::from_str(payload) {
            Ok(p) => p,
            Err(e) => return Mapped::unreadable(format!("an entry that does not read: {e}")),
        };
        let read = || -> std::result::Result<AdjustmentDraft, String> {
            Ok(match p {
                Payload::CostOfArrival { applies_to, instrument, cost, acquired } => AdjustmentDraft {
                    leg: leg(),
                    applies_to: TransactionId::parse(&applies_to).map_err(|e| e.to_string())?,
                    legs: vec![AdjustmentLegDraft { to: Some(refs(&instrument)?), cost: Some(money(&cost)?), acquired: Some(acquired.parse().map_err(|e: jiff::Error| e.to_string())?), ..AdjustmentLegDraft::default() }],
                },
                Payload::SpinOff { applies_to, parent, children } => {
                    let from = refs(&parent)?;
                    let mut legs = Vec::new();
                    for c in &children {
                        legs.push(AdjustmentLegDraft { from: Some(from.clone()), to: Some(refs(&c.instrument)?), cost_share: Some(Dec::parse(&c.cost_share).map_err(|e| e.to_string())?), ..AdjustmentLegDraft::default() });
                    }
                    AdjustmentDraft { leg: leg(), applies_to: TransactionId::parse(&applies_to).map_err(|e| e.to_string())?, legs }
                }
                Payload::ReturnOfCapital { applies_to, instrument, per_unit } => {
                    let x = refs(&instrument)?;
                    AdjustmentDraft {
                        leg: leg(),
                        applies_to: TransactionId::parse(&applies_to).map_err(|e| e.to_string())?,
                        legs: vec![AdjustmentLegDraft { from: Some(x.clone()), to: Some(x), cash_per_unit: Some(money(&per_unit)?), ..AdjustmentLegDraft::default() }],
                    }
                }
            })
        };
        match read() {
            Ok(a) => Mapped { legs: vec![], problems: vec![], adjustments: vec![a] },
            Err(why) => Mapped::unreadable(format!("an entry that does not read: {why}")),
        }
    }
}

impl Book {
    /// An instrument's strongest reference, which any source that names it matches.
    fn strong_ref(&self, i: InstrumentId) -> Result<(String, String)> {
        let r = self.instrument_refs(i)?.into_iter().find(|r| r.scheme.strength() == Strength::Strong).ok_or_else(|| BookError::Refused(format!("instrument {i} has no reference a source states")))?;
        Ok((r.scheme.to_text(), r.value))
    }

    /// Keep an entry of the person's, each its own record. An entry the book
    /// cannot stand behind is refused, named: a cost or an amount in another
    /// currency than the instrument's, a share of cost outside (0, 1], a
    /// negative amount, children whose shares add past the whole.
    pub fn enter(&self, entry: &Entry, at: jiff::Timestamp) -> Result<Stored> {
        let refused = |why: String| Err(BookError::Refused(why));
        let currency_of = |i: InstrumentId| -> Result<Currency> { Ok(self.instrument(i)?.currency) };
        let payload = match entry {
            Entry::CostOfArrival { arrival, instrument, cost, acquired } => {
                if cost.currency != currency_of(*instrument)? || cost.amount.is_negative() {
                    return refused(format!("a cost of {} {} for an instrument priced in {}", cost.amount.to_text(), cost.currency, currency_of(*instrument)?));
                }
                Payload::CostOfArrival {
                    applies_to: arrival.to_string(),
                    instrument: vec![self.strong_ref(*instrument)?],
                    cost: Amount { amount: cost.amount.to_text(), currency: cost.currency.to_string() },
                    acquired: acquired.to_string(),
                }
            }
            Entry::SpinOff { event, parent, children } => {
                let mut total = Dec::ZERO;
                let mut out = Vec::new();
                for (child, share) in children {
                    if !share.is_positive() || *share > Dec::ONE {
                        return refused(format!("a share of cost of {} (it is a part of the whole, above 0 and at most 1)", share.to_text()));
                    }
                    total = total.checked_add(*share).map_err(|e| BookError::Refused(e.to_string()))?;
                    out.push(Child { instrument: vec![self.strong_ref(*child)?], cost_share: share.to_text() });
                }
                if total > Dec::ONE {
                    return refused(format!("new holdings taking {} of the parent's cost, more than the whole", total.to_text()));
                }
                Payload::SpinOff { applies_to: event.to_string(), parent: vec![self.strong_ref(*parent)?], children: out }
            }
            Entry::ReturnOfCapital { distribution, instrument, per_unit } => {
                if per_unit.currency != currency_of(*instrument)? || !per_unit.amount.is_positive() {
                    return refused(format!("capital returned of {} {} per unit of an instrument priced in {}", per_unit.amount.to_text(), per_unit.currency, currency_of(*instrument)?));
                }
                Payload::ReturnOfCapital { applies_to: distribution.to_string(), instrument: vec![self.strong_ref(*instrument)?], per_unit: Amount { amount: per_unit.amount.to_text(), currency: per_unit.currency.to_string() } }
            }
        };
        let text = serde_json::to_string(&payload).map_err(|e| BookError::Refused(e.to_string()))?;
        let key = new_uuid(at).to_string();
        self.store(&PersonMapping, &Incoming { connection: None, source_key: &key, payload: &text, refs: vec![] }, at)
    }
}
