//! The contract between a source and the book: how a record becomes transactions.
//!
//! A mapping is pure. It reads a record's payload and says what the record means
//! in Bagholder's vocabulary, naming instruments and accounts by what the source
//! calls them (references and attributes), never by Bagholder's ids: the book
//! resolves those, creating an instrument the first time one is seen. A mapping
//! is versioned; when its version rises, the book derives every record of its
//! source again (`Book::rederive`).
//!
//! A mapping never guesses. What a record does not state stays empty with a
//! problem; a record it cannot place is `Kind::Unclassified` with a problem; a
//! payload it cannot read gives no transactions and a problem.

use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{InstrumentKind, OptionRight, Reference};
use bagholder_core::record::Problem;
use bagholder_core::transaction::{Effect, Kind};
use bagholder_core::{ConnectionId, Currency, Dec, Leg, Money, SourceName};

/// What a mapping is told besides the payload.
pub struct MapContext<'a> {
    /// The connection the record came through, if any.
    pub connection: Option<ConnectionId>,
    /// The record being mapped: what an adjustment on its own transactions
    /// applies to (a corporate event's row states what the event did).
    pub record: bagholder_core::RecordId,
    pub zones: &'a crate::zones::Zones,
}

pub trait Mapping {
    /// The source whose records this maps.
    fn source(&self) -> SourceName;
    /// Raised whenever what the mapping produces for some payload changes.
    fn version(&self) -> u32;
    /// The record's transactions and problems. `payload` is the record's canonical JSON.
    fn map(&self, ctx: &MapContext, payload: &str) -> Mapped;
}

/// A mapping's answer for one record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapped {
    pub legs: Vec<Draft>,
    pub problems: Vec<Problem>,
    /// What the record says a corporate event did, or what the person states
    /// about a transaction (`bagholder_core::adjustment`).
    pub adjustments: Vec<AdjustmentDraft>,
}

impl Mapped {
    /// A record the mapping could not read at all.
    pub fn unreadable(why: impl Into<String>) -> Mapped {
        Mapped { legs: vec![], problems: vec![Problem::new("unreadable", why)], adjustments: vec![] }
    }
}

/// An adjustment as a mapping describes it: the transaction it explains, and
/// its legs naming instruments by their references (never by Bagholder's ids);
/// the book finds the instruments, and a reference that names none is a problem.
#[derive(Clone, Debug, PartialEq)]
pub struct AdjustmentDraft {
    pub leg: Leg,
    pub applies_to: bagholder_core::TransactionId,
    pub legs: Vec<AdjustmentLegDraft>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdjustmentLegDraft {
    pub from: Option<Vec<Reference>>,
    pub to: Option<Vec<Reference>>,
    pub units_per_unit: Option<Dec>,
    pub cost_share: Option<Dec>,
    pub cash_per_unit: Option<Money>,
    pub cost: Option<Money>,
    pub acquired: Option<jiff::civil::Date>,
}

/// One transaction, as a mapping describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub leg: Leg,
    pub account: AccountRef,
    pub occurred_at: Option<jiff::Timestamp>,
    pub trade_date: jiff::civil::Date,
    pub settle_date: Option<jiff::civil::Date>,
    pub kind: Kind,
    pub effect: Option<Effect>,
    pub instrument: Option<InstrumentDraft>,
    pub quantity: Option<Dec>,
    pub price: Option<Money>,
    pub cash: Option<Money>,
    pub fee: Option<Money>,
    pub fx_rate: Option<Dec>,
    /// For a payment, the units it was paid on, where the record states them.
    pub paid_on: Option<Dec>,
    /// For units that moved in, what the record states they were worth as they arrived.
    pub value: Option<Money>,
}

/// An instrument as a source describes it: what identifies it, and what to
/// record the first time it is seen.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentDraft {
    /// At least one strong or connection-scoped reference; routing references may follow.
    pub refs: Vec<Reference>,
    pub kind: InstrumentKind,
    pub currency: Currency,
    pub name: Option<NameDraft>,
    pub option: Option<OptionDraft>,
}

/// What a record calls an instrument, on the record's day.
#[derive(Clone, Debug, PartialEq)]
pub struct NameDraft {
    pub symbol: String,
    pub venue_mic: Option<String>,
    pub venue_name: Option<String>,
    pub name: Option<String>,
    pub seen: jiff::civil::Date,
}

/// An option's terms as a source states them.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionDraft {
    /// The underlying instrument, described like any other.
    pub underlying: Box<InstrumentDraft>,
    pub expiry: jiff::civil::Date,
    pub strike: Dec,
    pub right: OptionRight,
    pub multiplier: Option<Dec>,
}
