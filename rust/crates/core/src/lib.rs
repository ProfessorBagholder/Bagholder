//! The vocabulary of Bagholder (`docs/architecture.md` §5, §6): what things are
//! called and what they are, in Bagholder's own terms and in no source's. It
//! holds no database, no network and no clock (`tests/boundaries.rs`).

#[macro_use]
pub mod text_enum;
pub mod account;
pub mod adjustment;
pub mod bracket;
pub mod dec;
pub mod distribution;
pub mod ids;
pub mod json;
pub mod instrument;
pub mod journal;
pub mod money;
pub mod order;
pub mod names;
pub mod record;
pub mod transaction;

pub use dec::{Dec, DecError, Rounding};
pub use ids::{AccountId, ConnectionId, GroupId, IdError, InstrumentId, IssuerId, Leg, LinkId, RecordId, TradeId, TransactionId};
pub use money::{Currency, Money, MoneyError};
pub use names::{Broker, MappingVersion, SourceName};

/// Dates and instants, for the crates that depend on this one alone (the engine):
/// one version of the calendar everywhere.
pub use jiff;
