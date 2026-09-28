//! The broker interface (`docs/architecture.md` §10,
//! `docs/plans/stage-3b-wealthsimple.md`, "The interface"): what a broker adapter
//! answers, in Bagholder's terms, and the pull that writes it into the book
//! (`pull`). A second brokerage is a second adapter; nothing here changes for it.

use std::collections::BTreeMap;

use bagholder_book::mapping::{InstrumentDraft, Mapping};
use bagholder_core::account::AccountType;
use bagholder_core::instrument::Reference;
use bagholder_core::json::Value;
use bagholder_core::{Broker, Currency, Dec, Money};

pub mod codes;
pub mod csv;
pub mod pull;
pub mod statements;

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
    /// The margin account this one backs as collateral, by the broker's id for it.
    pub backs: Option<String>,
}

/// One activity row, as the broker sent it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The broker's own id for the row.
    pub key: String,
    /// The broker's account id.
    pub account: String,
    /// The day the broker files it under; None where the row states none
    /// that reads (it is then `unread`).
    pub day: Option<jiff::civil::Date>,
    /// Whether the row's status is final: a row not yet final is read again.
    pub settled: bool,
    /// Whether what the row moved is read from positions, net of the book's
    /// own moves: it is recorded after the rows that move by themselves.
    pub reads_positions: bool,
    /// Why the adapter could not read the row, where it could not: it is
    /// kept as a record with that problem and read again.
    pub unread: Option<String>,
    pub value: Value,
}

/// An account's activity as read: its rows, and each row that states no id
/// of its own to keep it by (the read is then incomplete).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Activity {
    pub rows: Vec<Row>,
    pub unkeyed: Vec<String>,
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

/// Where a pull is, as it goes, for whoever shows it. An account is named as
/// every screen names it (`AccountStated::name`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The accounts and the links between them.
    Accounts,
    /// One account's activity, `n` of `of`.
    Activity { account: String, n: usize, of: usize },
    /// The rows read, being recorded: `done` of `of`.
    Recording { done: usize, of: usize },
    /// Cash and what margin can borrow.
    Balances,
    /// The monthly statements of an account whose cash disagrees.
    Statements,
    /// One account's holdings, `n` of `of`.
    Holdings { account: String, n: usize, of: usize },
    /// One account's value by day, `n` of `of`.
    History { account: String, n: usize, of: usize },
}

impl AccountStated {
    /// Its name as every screen shows an account: the person's for it, else what it is.
    pub fn name(&self) -> String {
        bagholder_core::account::account_name(self.nickname.as_deref(), &self.account_type)
    }
}

/// One row of a broker's monthly statement, as it states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatementRow {
    /// The day the statement files it under (a trade's settlement).
    pub day: jiff::civil::Date,
    /// The day the row states it was executed, where it states one.
    pub executed: Option<jiff::civil::Date>,
    /// The broker's transaction code (`WD`, `TRFIN`, …).
    pub code: String,
    pub description: String,
    pub currency: Currency,
    /// The signed cash it moved.
    pub cash: Dec,
    /// The account's cash in that currency after it.
    pub balance: Dec,
}

impl StatementRow {
    /// The day the book files what it moved under: its execution where the
    /// row states one (the feed dates a trade at execution), else its own.
    pub fn book_day(&self) -> jiff::civil::Date {
        self.executed.unwrap_or(self.day)
    }
}

/// A month's statement of one account, as read.
#[derive(Clone, Debug, PartialEq)]
pub enum StatementRead {
    /// Issued: the reply as it came, kept whole, and its rows.
    Issued { payload: Value, rows: Vec<StatementRow> },
    /// Not issued yet (the first days after a month ends): neither a failure
    /// nor an empty month, and asked again once the broker could have issued it.
    NotIssued,
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
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Activity>;
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
    /// A stored record whose row is not final yet (pending, a placeholder, one
    /// the adapter could not read): its account and day, so the next pull
    /// reads it again (the whole account where its day is not known).
    fn unsettled(&self, payload: &Value) -> Option<(String, Option<jiff::civil::Date>)>;
    /// A stored record's account and day: whether a read of that account
    /// from a day covers it.
    fn placed(&self, payload: &Value) -> Option<(String, jiff::civil::Date)>;
    /// The day the broker files an instant under.
    fn day(&self, at: jiff::Timestamp) -> jiff::civil::Date;
    /// Each account's cash per currency now: an account left out is one the
    /// broker did not state, and nothing is stored for it.
    fn cash(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, BTreeMap<Currency, Dec>>>;
    /// An account's positions as of a day.
    /// What each margin account can borrow now, in CAD, or why the broker cannot say.
    fn buying_power(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, Result<Dec, String>>>;
    fn units(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Vec<Units>>;
    /// The broker's description of each instrument it states a holding in that
    /// no row names (seen on `day`), read together: each one's, or why not.
    fn instruments(&mut self, refs: &[Reference], day: jiff::civil::Date) -> Vec<(Reference, Answer<InstrumentDraft>)>;
    /// An account's value and net deposits per day, from `from` (the whole of
    /// its history when `None`).
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>>;
    /// An account's statement for the calendar month starting `month`: the
    /// movements the activity feed may leave out (`statements`). A broker
    /// that issues none says so.
    fn statement(&mut self, account: &str, month: jiff::civil::Date) -> Answer<StatementRead> {
        let _ = month;
        Err(Failure::Refused(format!("{} issues no monthly statement for {account}", self.broker())))
    }
    /// A kept statement's rows, read again from the reply as it came, for the
    /// broker's account it is of.
    fn statement_rows(&self, account: &str, payload: &Value) -> Answer<Vec<StatementRow>> {
        let _ = (account, payload);
        Err(Failure::Refused(format!("{} issues no monthly statement", self.broker())))
    }
    /// The mapping of a statement row booked into the book: a movement the
    /// activity feed left out.
    fn statement_mapping(&self) -> Option<&dyn Mapping> {
        None
    }
    /// A statement row booked into the book: the key its record is kept by and
    /// the record, for one of the broker's accounts, the month and the row's
    /// place among the month's rows.
    fn statement_record(&self, account: &str, month: jiff::civil::Date, position: usize, row: &StatementRow) -> Option<(String, Value)> {
        let _ = (account, month, position, row);
        None
    }
    /// A month's statement opening off where the statement before closed, with
    /// no row for the difference, booked: the key and the record.
    fn statement_gap_record(&self, gap: &crate::statements::Gap) -> Option<(String, Value)> {
        let _ = gap;
        None
    }
    /// The side paid of a conversion whose record states only the side
    /// received, as the broker's stated cash shows it: the key and the record.
    fn conversion_paid_record(&self, paid: &crate::statements::Paid) -> Option<(String, Value)> {
        let _ = paid;
        None
    }
    /// A fill as the statement states it, in the place of the feed's row for
    /// it: the key and the record.
    fn fill_record(&self, fill: &crate::statements::Fill) -> Option<(String, Value)> {
        let _ = fill;
        None
    }
}
