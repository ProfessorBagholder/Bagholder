//! What the figures need read, as the caller works it out from the book and the
//! engine (this crate depends on neither the engine nor the server): the
//! currencies converted and from when, the instruments whose closes a figure
//! uses and from when, the payers held, and the span the benchmarks must cover.

use bagholder_core::jiff::civil::Date;
use bagholder_core::InstrumentId;

use crate::contract::Listing;
use crate::rates::Need;

/// An instrument whose daily closes a figure uses, and the days it needs them:
/// from the first day it was held (or the one day an expiring contract's
/// underlying is needed on) to its last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseNeed {
    pub listing: Listing,
    pub from: Date,
    /// The last day a close is needed: today while it is held.
    pub to: Date,
}

/// A payer held now: its listing, whose distributions and schedule are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayerNeed {
    pub listing: Listing,
    /// The name its records give it (`Mackenzie Canadian Equity Index ETF`).
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub rates: Vec<Need>,
    pub closes: Vec<CloseNeed>,
    pub payers: Vec<PayerNeed>,
    /// The first day the benchmarks must cover: the person's oldest day.
    pub benchmarks_from: Option<Date>,
}

impl Needs {
    pub fn payer(&self, id: InstrumentId) -> Option<&PayerNeed> {
        self.payers.iter().find(|p| p.listing.id == id)
    }
}
