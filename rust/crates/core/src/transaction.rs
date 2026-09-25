//! Bagholder's transactions: one vocabulary for every broker (`docs/architecture.md` §6).
//!
//! A transaction is derived from a source record by that source's mapping. It
//! says only what the record states: a quantity where the record moves a
//! position, a price where the source states one, cash signed as the source
//! signs it. What a record does not state stays empty, with a problem on the
//! record, and is never worked out from the rest.

use crate::dec::Dec;
use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::money::Money;
use crate::names::MappingVersion;

text_enum! {
    /// What a transaction is.
    Kind "kind of transaction" {
        Buy = "buy",
        Sell = "sell",
        Dividend = "dividend",
        Interest = "interest",
        /// Interest the account paid (on margin, a line of credit).
        InterestCharge = "interest-charge",
        Fee = "fee",
        WithholdingTax = "withholding-tax",
        /// Money in from the person.
        Deposit = "deposit",
        /// Money in from an employer (a group plan's contribution).
        EmployerDeposit = "employer-deposit",
        /// Money in from a government (a grant to a registered plan).
        GovernmentDeposit = "government-deposit",
        Withdrawal = "withdrawal",
        TransferIn = "transfer-in",
        TransferOut = "transfer-out",
        /// One side of a conversion between currencies.
        CurrencyConversion = "currency-conversion",
        OptionExpiry = "option-expiry",
        OptionAssignment = "option-assignment",
        OptionExercise = "option-exercise",
        /// An event contract settling.
        Resolution = "resolution",
        /// Coins moved in or out of staking: no change to the position.
        StakingMove = "staking-move",
        StakingReward = "staking-reward",
        CardPurchase = "card-purchase",
        CardRefund = "card-refund",
        Cashback = "cashback",
        /// A change to a holding without a trade: a split, a name or ticker
        /// change, a stock dividend, a spin-off. Which, and its values, are
        /// booked from the official record (`docs/plans/stage-2-engine.md`, "Corporate events").
        CorporateEvent = "corporate-event",
        /// A record the mapping cannot place. It carries a problem and counts in
        /// no figure.
        Unclassified = "unclassified",
    }
}

text_enum! {
    /// For an option's buy or sell: whether it opens or closes a position.
    Effect "effect" {
        Open = "open",
        Close = "close",
    }
}

/// One transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub id: TransactionId,
    pub mapping: MappingVersion,
    pub account: AccountId,
    /// The instant, where the source gives one.
    pub occurred_at: Option<jiff::Timestamp>,
    /// The day the broker files it under.
    pub trade_date: jiff::civil::Date,
    /// Where the source states one.
    pub settle_date: Option<jiff::civil::Date>,
    pub kind: Kind,
    pub effect: Option<Effect>,
    pub instrument: Option<InstrumentId>,
    /// The change to the position, signed: into the account is positive. Empty
    /// where the transaction moves no position, or where the record does not
    /// state it (and the record has a problem saying so).
    pub quantity: Option<Dec>,
    /// Per unit of the quantity, where the source states it.
    pub price: Option<Money>,
    /// Cash into (positive) or out of (negative) the account, as the source signs it.
    pub cash: Option<Money>,
    /// Commission and fees, as a positive amount the account paid.
    pub fee: Option<Money>,
    /// The conversion rate the source applied, where it states one.
    pub fx_rate: Option<Dec>,
    /// For a payment (a distribution, interest on a holding), the units it was
    /// paid on, where the source states them: never a change to the position.
    pub paid_on: Option<Dec>,
}
