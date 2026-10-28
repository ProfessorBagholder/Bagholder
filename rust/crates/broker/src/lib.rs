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

pub mod activity_types;
pub mod codes;
pub mod csv;
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
    /// The margin account this one backs as collateral, by the broker's id for it.
    pub backs: Option<String>,
    /// What the broker states it is worth now, where it states it.
    pub net_value: Option<Money>,
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
    /// The broker's book value in CAD: what a total in CAD adds up, never a cost
    /// the journal takes.
    pub book_value: Option<Money>,
    /// The broker's book value in the listing's own currency: what the units
    /// cost, as the holding shows it (brief 79).
    pub market_book_value: Option<Money>,
    /// What the broker states the position is worth now.
    pub value: Option<Money>,
}

/// One account's positions as of a day, as its broker states them: each
/// holding's units, and the cash in each currency the same read states.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Positions {
    pub units: Vec<Units>,
    pub cash: BTreeMap<Currency, Dec>,
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
    /// Every account's holdings, read together.
    Holdings,
    /// One account's value by day, `n` of `of`.
    History { account: String, n: usize, of: usize },
}

impl AccountStated {
    /// Its name as every screen shows an account: the person's for it, else what it is.
    pub fn name(&self) -> String {
        bagholder_core::account::account_name(self.nickname.as_deref(), &self.account_type)
    }
}

/// The reads beyond the accounts read and the activity that a broker issues for
/// one kind of account (`BrokerAdapter::reads`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    /// Its cash now: the balances read, or a card's own read.
    pub cash: bool,
    /// Its holdings as of a day (the positions read).
    pub positions: bool,
    /// Its value and net deposits by day.
    pub history: bool,
    /// What it can borrow now, and the rates it pays on what it borrows.
    pub margin: bool,
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
    fn accounts(&mut self) -> Answer<Vec<AccountStated>>;
    /// The reads the broker issues for a kind of account (brief 79, amended
    /// 2026-10-28): each read is asked only of the kinds it is issued for, never
    /// to learn a kind by failing. Every kind gets the accounts read and its
    /// activity; a broker that states nothing else gets nothing else.
    fn reads(&self, account_type: &AccountType) -> Reads {
        let _ = account_type;
        Reads::default()
    }
    /// An account's activity, from `from` (the whole of it when `None`).
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Activity>;
    /// The rows about to be recorded, so an adapter can read what they share
    /// in as few requests as its broker takes (their securities, in batches).
    fn prepare(&mut self, _rows: &[&Row]) {}
    /// The rows of the person's accounts the adapter read in this pull for
    /// another row's record (a move's other side, over its days) beside each
    /// account's own read, each given once: the pull stores them as it stores
    /// every row read, so a row a closed account's feed lists after that
    /// account was read whole is stored without the account being read again.
    fn read_beside(&mut self) -> Vec<Row> {
        Vec::new()
    }
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
    /// Whether `refresh` has something to add to a stored record: asked of
    /// every stored record, so it reads nothing.
    fn refreshes(&self, payload: &Value) -> bool {
        let _ = payload;
        false
    }
    /// A stored record put together before the adapter read a part it reads
    /// now: the record with that part added, its other parts as stored, read
    /// once by the pull after the accounts and their activity. None where the
    /// record lacks nothing (or what it lacks cannot be read: it is asked
    /// again on the next pull).
    fn refresh(&mut self, payload: &Value, book: &mut dyn BookMoves) -> Option<Answer<Value>> {
        let _ = (payload, book);
        None
    }
    /// The other account whose rows a stored record holds over some days, as
    /// they were when it was put together: a row of it recorded later is
    /// given to `amend`.
    fn watches(&self, payload: &Value) -> Option<String> {
        let _ = payload;
        None
    }
    /// A stored record that holds `row`'s account's rows over some days, with
    /// `row` added where it falls in them and is not held as it is now; None
    /// where nothing changes. The record is derived again from it.
    fn amend(&self, payload: &Value, row: &Row) -> Option<Value> {
        let _ = (payload, row);
        None
    }
    /// The day the broker files an instant under.
    fn day(&self, at: jiff::Timestamp) -> jiff::civil::Date;
    /// Each account's cash per currency now: an account left out is one the
    /// broker did not state, and nothing is stored for it.
    fn cash(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, BTreeMap<Currency, Dec>>>;
    /// What each margin account can borrow now, in CAD, or why the broker cannot say.
    fn buying_power(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, Result<Dec, String>>>;
    /// A margin account's yearly interest rate on what it borrows, per currency,
    /// as a fraction (0.0445 for 4.45%): none from a broker that states none.
    fn margin_rates(&mut self, _account: &str) -> Answer<Option<BTreeMap<Currency, Dec>>> {
        Ok(None)
    }
    /// Each account's positions as of a day, read together: every account asked
    /// for in as few requests as the broker takes.
    fn positions(&mut self, accounts: &[String], day: jiff::civil::Date) -> Answer<BTreeMap<String, Positions>>;
    /// The person's accounts held outside the broker that the broker's own net
    /// worth feature states (`docs/plans/debts-held-outside-wealthsimple.md`):
    /// none from a broker that states none.
    fn outside_accounts(&mut self) -> Answer<Option<Vec<bagholder_book::debts::OutsideDebt>>> {
        Ok(None)
    }
    /// One outside account's balance by day over `start..=end`, as the broker's
    /// net worth feature states it of that account alone, stated as its balance
    /// is (`OutsideDebt::balance`): none from a broker that states none.
    fn outside_days(&mut self, entity: &bagholder_book::debts::OutsideDebt, start: jiff::civil::Date, end: jiff::civil::Date) -> Answer<Option<Vec<bagholder_book::debts::OutsideDay>>> {
        let _ = (entity, start, end);
        Ok(None)
    }
    /// The broker's description of each instrument it states a holding in that
    /// no row names (seen on `day`), read together: each one's, or why not.
    fn instruments(&mut self, refs: &[Reference], day: jiff::civil::Date) -> Vec<(Reference, Answer<InstrumentDraft>)>;
    /// An account's value and net deposits per day, from `from` (the whole of
    /// its history when `None`).
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>>;
    /// The mapping of the statement rows an earlier version booked into the
    /// book, which the pull withdraws (`pull::withdraw_statement_records`).
    fn statement_mapping(&self) -> Option<&dyn Mapping> {
        None
    }
}
