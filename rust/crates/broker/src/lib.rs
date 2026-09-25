//! The broker interface (`docs/architecture.md` §10,
//! `docs/plans/stage-3b-wealthsimple.md`, "The interface"): what a broker adapter
//! answers, in Bagholder's terms, and the pull that writes it into the book
//! (`pull`). A second brokerage is a second adapter; nothing here changes for it.

use std::collections::BTreeMap;

use bagholder_book::mapping::Mapping;
use bagholder_core::account::AccountType;
use bagholder_core::instrument::Reference;
use bagholder_core::json::Value;
use bagholder_core::{Broker, Currency, Dec, Money};

pub mod pull;

/// Why a read did not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The broker refused the request (its own error, a status it gave).
    Refused(String),
    /// The broker could not be reached.
    Unreachable(String),
    /// The reply is not of the shape the adapter reads, or does not mean what
    /// it must: named with the field.
    Mismatch(String),
    /// The session is no longer valid: a sign-in is needed, and nothing is
    /// asked again with it.
    Lapsed(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Refused(w) => write!(f, "refused: {w}"),
            Failure::Unreachable(w) => write!(f, "unreachable: {w}"),
            Failure::Mismatch(w) => write!(f, "a reply of another shape: {w}"),
            Failure::Lapsed(w) => write!(f, "the session lapsed: {w}"),
        }
    }
}

pub type Answer<T> = Result<T, Failure>;

/// An account, as the broker states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountStated {
    /// The broker's own id for it.
    pub key: String,
    pub account_type: AccountType,
    pub open: bool,
    pub nickname: Option<String>,
    /// The account the broker states it is linked to.
    pub linked_to: Option<String>,
}

/// One activity row, as the broker sent it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The broker's own id for the row.
    pub key: String,
    /// The broker's account id.
    pub account: String,
    /// The day the broker files it under.
    pub day: jiff::civil::Date,
    /// Whether the row's status is final: a row not yet final is read again.
    pub settled: bool,
    /// Whether what the row moved is read from positions, net of the book's
    /// own moves: it is recorded after the rows that move by themselves.
    pub reads_positions: bool,
    pub value: Value,
}

/// A position as the broker states it on a day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Units {
    /// The broker's own reference for the instrument.
    pub instrument: Reference,
    pub quantity: Dec,
    /// The broker's book value, kept as its statement, never a cost.
    pub book_value: Option<Money>,
}

/// An account's value and net deposits on a day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayValue {
    pub day: jiff::civil::Date,
    pub net_value: Money,
    pub net_deposits: Money,
}

/// What the book's own transactions moved in one account on one day: of an
/// instrument, by the broker's reference, or of cash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub account: String,
    pub day: jiff::civil::Date,
    pub what: MovedWhat,
    pub quantity: Dec,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MovedWhat {
    Instrument(Reference),
    Cash(Currency),
}

/// The book's own moves, for a record read against positions.
pub trait BookMoves {
    fn moves(&mut self, accounts: &[String], days: &[jiff::civil::Date]) -> Vec<Moved>;
}

/// One broker, behind the interface. Every method is a read; each answer says
/// what failed where it did not answer.
pub trait BrokerAdapter {
    fn broker(&self) -> Broker;
    fn mapping(&self) -> &dyn Mapping;
    /// The scheme the broker's own record ids are known by in the book
    /// (`broker-record:wealthsimple`): what an imported record it replaces carries.
    fn record_scheme(&self) -> String {
        format!("broker-record:{}", self.broker())
    }
    fn accounts(&mut self) -> Answer<Vec<AccountStated>>;
    /// An account's activity, from `from` (the whole of it when `None`).
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Row>>;
    /// The rows about to be recorded, so an adapter can read what they share
    /// in as few requests as its broker takes (their securities, in batches).
    fn prepare(&mut self, _rows: &[&Row]) {}
    /// A row's record: the row and every reply read once for it, as the book
    /// stores it.
    fn record(&mut self, row: &Row, book: &mut dyn BookMoves) -> Answer<Value>;
    /// Whether a stored record holds this row as it is now: nothing about it
    /// changed, and it is not put together again.
    fn holds(&self, payload: &Value, row: &Row) -> bool;
    /// Whether a stored record's row is read against positions (its moves are
    /// not the book's own).
    fn reads_positions(&self, payload: &Value) -> bool;
    /// A stored record whose row is not final yet (pending, a placeholder):
    /// its account and day, so the next pull reads it again.
    fn unsettled(&self, payload: &Value) -> Option<(String, jiff::civil::Date)>;
    /// The day the broker files an instant under.
    fn day(&self, at: jiff::Timestamp) -> jiff::civil::Date;
    /// Each account's cash per currency now.
    fn cash(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, BTreeMap<Currency, Dec>>>;
    /// An account's positions as of a day.
    fn units(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Vec<Units>>;
    /// An account's value and net deposits per day, from `from` (the whole of
    /// its history when `None`).
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>>;
}
