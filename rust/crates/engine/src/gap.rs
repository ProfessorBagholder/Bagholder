//! What a figure waits on (`docs/architecture.md` §3, §15): a figure the engine
//! cannot state is not zero and not a guess, it is a `Gaps` naming each fact it
//! is waiting for. Every figure is a `Fig<T>`: the value, or why there is none.

use std::collections::BTreeSet;
use std::fmt;

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Currency, DecError, InstrumentId, MoneyError, TransactionId};

/// One fact a figure is waiting for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Gap {
    /// The Bank of Canada publishes this business day's rate at 16:30 Eastern,
    /// and that moment has not come: the one time a rate does not exist yet.
    RatePending { currency: Currency, day: Date },
    /// The Bank has published (or should have) and the rate is not stored: the
    /// Bank source has failed for it.
    RateMissing { currency: Currency, day: Date },
    /// A currency the Bank publishes no rate for.
    RateUnpublished(Currency),
    /// An option contract whose shares per contract no source has stated yet.
    MultiplierUnstated(InstrumentId),
    /// A transaction that moves a position and does not state by how much.
    QuantityUnstated(TransactionId),
    /// A multi-leg order whose record does not state its legs: any contract on
    /// its underlying in its account may be one of them.
    LegUnstated(TransactionId),
    /// A contract past its expiry with nothing on the record, which its
    /// underlying's close does not show expired worthless.
    NoExpiryRecord(InstrumentId),
    /// A corporate event whose kind and values are not known yet.
    EventUnknown(TransactionId),
    /// An asset deposited into the account: what it cost is not on the record.
    BasisUnknown(TransactionId),
    /// A sale or close of more than the account held.
    BeyondHeld(TransactionId),
    /// A fill whose record says it opens a position while the account holds the
    /// opposite one: what it did is not what the record says.
    EffectConflict(TransactionId),
    /// A fill whose cash is in another currency than the instrument's, with no
    /// rate stated for the conversion.
    CurrencyUnstated(TransactionId),
    /// A holding with no price: no quote from its own kind's source and no fill.
    PriceUnknown(InstrumentId),
    /// A holding with no close on a day the equity series needs one.
    CloseUnknown { instrument: InstrumentId, day: Date },
    /// A dividend payer whose payout frequency no source states and no record
    /// shows yet.
    FrequencyUnknown(InstrumentId),
    /// A dividend payer whose amount per unit no declared record states and no
    /// payment shows (none states the units it paid on).
    DistributionUnknown(InstrumentId),
    /// A record with a problem its source or mapping reported (a row the
    /// mapping could not place, a fact the row does not state): the account's
    /// own figures from its day wait until the record is corrected.
    RecordProblem { transaction: TransactionId, code: String },
    /// Arithmetic that could not be done exactly (a figure too large to hold),
    /// or a record whose amounts contradict themselves.
    Arithmetic(String),
}

impl Gap {
    /// The one word the page shows in place of a figure, and the flag it carries.
    pub fn word(&self) -> &'static str {
        match self {
            Gap::RatePending { .. } => "rate-pending",
            Gap::RateMissing { .. } => "rate-missing",
            Gap::RateUnpublished(_) => "rate-unpublished",
            Gap::MultiplierUnstated(_) => "multiplier-unstated",
            Gap::QuantityUnstated(_) => "quantity-unstated",
            Gap::LegUnstated(_) => "leg-unstated",
            Gap::NoExpiryRecord(_) => "no-expiry-record",
            Gap::EventUnknown(_) => "event-unknown",
            Gap::BasisUnknown(_) => "basis-unknown",
            Gap::BeyondHeld(_) => "beyond-held",
            Gap::EffectConflict(_) => "effect-conflict",
            Gap::CurrencyUnstated(_) => "currency-unstated",
            Gap::PriceUnknown(_) => "price-unknown",
            Gap::CloseUnknown { .. } => "close-unknown",
            Gap::FrequencyUnknown(_) => "frequency-unknown",
            Gap::DistributionUnknown(_) => "distribution-unknown",
            Gap::RecordProblem { .. } => "record-problem",
            Gap::Arithmetic(_) => "arithmetic",
        }
    }
}

impl fmt::Display for Gap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Gap::RatePending { currency, day } => write!(f, "the Bank of Canada's {currency} rate for {day} is published at 16:30 Eastern"),
            Gap::RateMissing { currency, day } => write!(f, "the Bank of Canada's {currency} rate for {day} has not been read"),
            Gap::RateUnpublished(c) => write!(f, "the Bank of Canada publishes no rate for {c}"),
            Gap::MultiplierUnstated(i) => write!(f, "the contract size of {i} is not stated yet"),
            Gap::QuantityUnstated(t) => write!(f, "{t} does not state its quantity"),
            Gap::LegUnstated(t) => write!(f, "{t} is a multi-leg order whose legs are not on the record"),
            Gap::NoExpiryRecord(i) => write!(f, "{i} is past its expiry with nothing on the record"),
            Gap::EventUnknown(t) => write!(f, "the corporate event {t} has no known values yet"),
            Gap::BasisUnknown(t) => write!(f, "what the asset deposited by {t} cost is not on the record"),
            Gap::BeyondHeld(t) => write!(f, "{t} takes out more than the account held"),
            Gap::EffectConflict(t) => write!(f, "{t} says it opens a position while the account holds the opposite one"),
            Gap::CurrencyUnstated(t) => write!(f, "{t} is paid in another currency than its instrument's, at no stated rate"),
            Gap::PriceUnknown(i) => write!(f, "{i} has no price"),
            Gap::CloseUnknown { instrument, day } => write!(f, "{instrument} has no close for {day}"),
            Gap::FrequencyUnknown(i) => write!(f, "the payout frequency of {i} is not known yet"),
            Gap::DistributionUnknown(i) => write!(f, "the amount {i} pays per unit is not known yet"),
            Gap::RecordProblem { transaction, code } => write!(f, "{transaction} has a problem on its record ({code})"),
            Gap::Arithmetic(why) => write!(f, "{why}"),
        }
    }
}

/// The facts one figure waits for: never empty when it stands for a figure.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Gaps(BTreeSet<Gap>);

impl Gaps {
    pub fn none() -> Gaps {
        Gaps(BTreeSet::new())
    }

    pub fn of(gap: Gap) -> Gaps {
        Gaps(BTreeSet::from([gap]))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn add(&mut self, gap: Gap) {
        self.0.insert(gap);
    }

    pub fn merge(&mut self, other: &Gaps) {
        self.0.extend(other.0.iter().cloned());
    }

    pub fn iter(&self) -> impl Iterator<Item = &Gap> {
        self.0.iter()
    }

    pub fn contains(&self, gap: &Gap) -> bool {
        self.0.contains(gap)
    }

    /// Whether any gap is of this word (`basis-unknown`, …).
    pub fn has_word(&self, word: &str) -> bool {
        self.0.iter().any(|g| g.word() == word)
    }

    /// The words, sorted and without repeats: the flags a figure carries.
    pub fn words(&self) -> Vec<&'static str> {
        let mut w: Vec<&'static str> = self.0.iter().map(Gap::word).collect();
        w.sort_unstable();
        w.dedup();
        w
    }

    /// `Ok(value)` when there are no gaps, else these gaps.
    pub fn or<T>(self, value: T) -> Fig<T> {
        if self.is_empty() {
            Ok(value)
        } else {
            Err(self)
        }
    }
}

impl From<Gap> for Gaps {
    fn from(g: Gap) -> Gaps {
        Gaps::of(g)
    }
}

impl From<DecError> for Gaps {
    fn from(e: DecError) -> Gaps {
        Gaps::of(Gap::Arithmetic(e.to_string()))
    }
}

impl From<MoneyError> for Gaps {
    fn from(e: MoneyError) -> Gaps {
        Gaps::of(Gap::Arithmetic(e.to_string()))
    }
}

/// A figure, or the facts it waits for.
pub type Fig<T> = Result<T, Gaps>;

/// Two figures combined: the result when both are stated, else every gap of both.
pub fn both<A, B, T>(a: Fig<A>, b: Fig<B>, f: impl FnOnce(A, B) -> Fig<T>) -> Fig<T> {
    match (a, b) {
        (Ok(a), Ok(b)) => f(a, b),
        (Err(mut x), Err(y)) => {
            x.merge(&y);
            Err(x)
        }
        (Err(x), _) | (_, Err(x)) => Err(x),
    }
}

/// Every figure of a list combined: all of them when every one is stated, else
/// every gap of every one.
pub fn all<T>(items: impl IntoIterator<Item = Fig<T>>) -> Fig<Vec<T>> {
    let mut out = Vec::new();
    let mut gaps = Gaps::none();
    for item in items {
        match item {
            Ok(v) => out.push(v),
            Err(g) => gaps.merge(&g),
        }
    }
    gaps.or(out)
}

/// The gaps of a figure, none when it is stated.
pub fn gaps_of<T>(f: &Fig<T>) -> Gaps {
    match f {
        Ok(_) => Gaps::none(),
        Err(g) => g.clone(),
    }
}
