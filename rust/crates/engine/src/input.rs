//! What the engine is given (`docs/plans/stage-2-engine.md`, "The engine's entry
//! point"): the book's ledger, the facts the figures use, the market's prices and
//! the clock. The engine reads nothing else: no database, no network, no clock.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::account::Account;
use bagholder_core::instrument::{Instrument, Name, OptionTerms};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::journal::{Group, JournalEntry, JournalSubject, Trade};
use bagholder_core::record::Problem;
use bagholder_core::transaction::Transaction;
use bagholder_core::{AccountId, Broker, Currency, Dec, InstrumentId, Money, RecordId, SourceName, TransactionId};

/// An account and the broker it is held at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountInfo {
    pub account: Account,
    pub broker: Broker,
}

/// An instrument with what it is called and, for a contract, its terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentInfo {
    pub instrument: Instrument,
    /// Oldest first; never overlapping.
    pub names: Vec<Name>,
    pub terms: Option<OptionTerms>,
}

impl InstrumentInfo {
    /// The name in force on a day: the last one first seen on or before it, else
    /// the earliest.
    pub fn name_on(&self, day: Date) -> Option<&Name> {
        self.names.iter().rev().find(|n| n.first_seen <= day).or(self.names.first())
    }

    /// What it is called now.
    pub fn current_name(&self) -> Option<&Name> {
        self.names.last()
    }
}

/// What the ledger needs to know of a record besides its transactions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordInfo {
    /// The source's own id for it: what orders transactions no instant separates,
    /// so storing a record again never reorders the book.
    pub source_key: String,
    pub problems: Vec<Problem>,
}

/// The book's record: accounts, instruments, the live transactions, and what the
/// person keeps on them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ledger {
    pub accounts: BTreeMap<AccountId, AccountInfo>,
    pub instruments: BTreeMap<InstrumentId, InstrumentInfo>,
    /// Only live records' transactions.
    pub transactions: Vec<Transaction>,
    /// Every live record a transaction belongs to.
    pub records: BTreeMap<RecordId, RecordInfo>,
    /// A transfer out of one of the person's accounts and the transfer into
    /// another that received it, as the book links them.
    pub transfer_links: Vec<(TransactionId, TransactionId)>,
    /// Every trade the book has assigned, with its anchor.
    pub trades: Vec<Trade>,
    pub groups: Vec<Group>,
    pub journal: BTreeMap<JournalSubject, JournalEntry>,
}

/// A fact with where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: SourceName,
}

/// The Bank of Canada's rates and what is known of its calendar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rates {
    /// CAD per unit of the currency, per business day the Bank published.
    pub by_currency: BTreeMap<Currency, BTreeMap<Date, Dec>>,
    /// The currencies the Bank publishes a rate for.
    pub published: BTreeSet<Currency>,
    /// The days the Bank's own schedule says it does not publish (its holidays).
    pub holidays: BTreeSet<Date>,
    /// Per currency, the spans of days completed reads of the Bank's series
    /// covered, and when each read was received: a weekday inside one without a
    /// rate was not published, if that day's 16:30 Eastern had passed by then.
    pub covered: BTreeMap<Currency, Vec<Read>>,
}

/// One completed read of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub first: Date,
    pub last: Date,
    pub at: Timestamp,
}

/// What kind of distribution a fund declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistributionKind {
    /// Its regular cash distribution: what a rate and a frequency are read from.
    Regular,
    /// A one-off cash distribution.
    Special,
    /// Paid in units, or reinvested: no cash.
    NonCash,
}

/// A distribution the fund declared, as its source stated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    pub ex_date: Date,
    pub record_date: Option<Date>,
    pub pay_date: Option<Date>,
    /// Per unit, in the currency it is paid in.
    pub amount: Money,
    pub kind: DistributionKind,
}

/// One read of a fund's declared record, whole: a distribution the fund
/// withdrew is absent from a later read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredRead {
    pub read_at: Timestamp,
    pub source: SourceName,
    pub items: Vec<Declared>,
}

pub use bagholder_core::adjustment::{Adjustment, AdjustmentLeg};

/// The facts the figures use, each kept in the book once used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub rates: Rates,
    /// The newest read of each fund's declared record.
    pub declared: BTreeMap<InstrumentId, DeclaredRead>,
    /// Payments per year as a source states it.
    pub frequencies: BTreeMap<InstrumentId, Sourced<u32>>,
    /// By the transaction each explains.
    pub adjustments: BTreeMap<TransactionId, Adjustment>,
}

/// Where a quote came from, as far as it decides what the quote may price
/// (`SPEC.md` §2 Position, Price).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteSource {
    /// A listing's exchange feed (TMX, Cboe Canada, Yahoo for a US listing).
    Listing,
    /// A coin's spot price.
    Crypto,
    /// An option contract's chain.
    OptionChain,
}

/// A live price for one instrument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quote {
    pub price: Money,
    /// The day's change per unit, where the source states it.
    pub change: Option<Dec>,
    /// The day's change in percent, where the source states it.
    pub change_pct: Option<Dec>,
    pub at: Option<Timestamp>,
    pub source: QuoteSource,
}

/// What a broker states about one account, the check against Bagholder's own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrokerAccount {
    /// The account's net value per day, in CAD, as the broker states it.
    pub net_value: BTreeMap<Date, Dec>,
    /// The money put in less the money taken out to date, in CAD, per day, as
    /// the broker states it beside its net value.
    pub net_deposits: BTreeMap<Date, Dec>,
    /// Its net value now, in CAD.
    pub net_value_now: Option<Dec>,
    /// When the broker stated its balances and holdings now.
    pub as_of: Option<Timestamp>,
    /// Cash per currency now.
    pub cash: BTreeMap<Currency, Dec>,
    /// Units held per instrument now.
    pub held: BTreeMap<InstrumentId, Dec>,
    /// What it can borrow, in CAD, or why the broker cannot say.
    pub buying_power: Option<Result<Dec, String>>,
}

/// The market around the book.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Market {
    pub quotes: BTreeMap<InstrumentId, Quote>,
    /// Daily closes per instrument, in its own currency.
    pub closes: BTreeMap<InstrumentId, BTreeMap<Date, Dec>>,
    /// Index levels per benchmark key (`SP500`, `TSX`, `TX60`), per day.
    pub benchmarks: BTreeMap<String, BTreeMap<Date, Dec>>,
    pub brokers: BTreeMap<AccountId, BrokerAccount>,
}

/// When the engine is asked, and where the person is.
#[derive(Clone, Debug)]
pub struct Clock {
    /// Today, in the person's home zone.
    pub today: Date,
    pub now: Timestamp,
    /// The person's home zone.
    pub home: TimeZone,
    /// The Bank of Canada's zone (Eastern), whose 16:30 is when a day's rate is
    /// published.
    pub bank: TimeZone,
}

impl PartialEq for Clock {
    fn eq(&self, other: &Clock) -> bool {
        self.today == other.today && self.now == other.now && self.home.iana_name() == other.home.iana_name() && self.bank.iana_name() == other.bank.iana_name()
    }
}

/// Everything the engine computes from.
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub ledger: Ledger,
    pub facts: Facts,
    pub market: Market,
    pub clock: Clock,
}
