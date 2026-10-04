//! What a broker holds against an account for a row not yet final
//! (`docs/plans/broker-check-reserved-cash.md`): cash it keeps aside from the
//! balance it states, which the book rightly does not book. A hold is a fact
//! about the broker's balance, kept beside it, never a transaction.

use crate::dec::Dec;
use crate::ids::{InstrumentId, RecordId};
use crate::money::Currency;

text_enum! {
    /// What a row not yet final holds, by what the broker documents of it.
    HoldKind "kind of hold" {
        /// A buy order working: a limit order holds its stated amount, any other
        /// buy (market, stop, fractional, an IPO bid) an amount it does not state.
        Buy = "buy",
        /// A put sold in an account without margin is secured by cash: strike ×
        /// multiplier × the contracts it opens, less the premium received.
        PutSale = "put-sale",
        /// Money leaving the account (a withdrawal, a transfer out): reserved for
        /// it in an amount the broker does not state.
        Withdrawal = "withdrawal",
    }
}

/// One row's hold, as the broker states it at a statement of the account's cash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
    pub record: RecordId,
    pub kind: HoldKind,
    /// The currency it is held in; none where the row states none, which leaves
    /// every currency of the account unsettled.
    pub currency: Option<Currency>,
    /// The instrument a put sale is on, where the book knows it.
    pub instrument: Option<InstrumentId>,
    /// The cash held as the row states it: a limit buy's amount; a put sale's
    /// strike × multiplier × contracts. None where it is not stated.
    pub amount: Option<Dec>,
    /// A put sale's contracts.
    pub quantity: Option<Dec>,
    /// A put sale's premium, as its stated amount.
    pub premium: Option<Dec>,
}
