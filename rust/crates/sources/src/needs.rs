//! What the figures need read, as the caller works it out from the book and the
//! engine (this crate depends on neither the engine nor the server): the
//! currencies converted and from when, the closes that decide a contract's
//! expiry, the contracts whose prices are shown, the payers held, and the span
//! the benchmarks must cover.

use bagholder_core::instrument::OptionRight;
use bagholder_core::jiff::civil::Date;
use bagholder_core::{Currency, Dec, InstrumentId};

use crate::contract::Listing;
use crate::rates::Need;

/// An instrument whose daily closes a figure uses, and the days it needs them:
/// the one day an expiring contract's underlying is needed on, or, for a chart
/// someone opens, its span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseNeed {
    pub listing: Listing,
    pub from: Date,
    pub to: Date,
}

/// A payer held now: its listing, whose distributions and schedule are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayerNeed {
    pub listing: Listing,
    /// The name its records give it (`Mackenzie Canadian Equity Index ETF`).
    pub name: Option<String>,
}

/// An option contract whose price is shown, as its chain is asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNeed {
    pub id: InstrumentId,
    pub currency: Currency,
    /// The underlying's symbol as its listing uses it now (`BRK.B`): the chain's name.
    pub underlying: String,
    pub expiry: Date,
    pub strike: Dec,
    pub right: OptionRight,
    /// Its OCC symbol, where the book states one.
    pub occ: Option<String>,
    /// The day of a corporate event on the underlying while the contract was
    /// held, where the book records one: the contract may have been adjusted,
    /// and only its OCC symbol then says which contract it is.
    pub event_on: Option<Date>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub rates: Vec<Need>,
    pub closes: Vec<CloseNeed>,
    /// The contracts held today, whose prices are shown.
    pub contracts: Vec<ContractNeed>,
    /// The listings held today, quoted.
    pub held: Vec<Listing>,
    pub payers: Vec<PayerNeed>,
    /// The first day the benchmarks must cover: the person's oldest day.
    pub benchmarks_from: Option<Date>,
}

impl Needs {
    pub fn payer(&self, id: InstrumentId) -> Option<&PayerNeed> {
        self.payers.iter().find(|p| p.listing.id == id)
    }
}
