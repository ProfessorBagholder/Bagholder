//! What a reader works with: the book the facts go to, the market cache, the
//! network, and the time, handed in (this crate reads no clock). Every request's
//! outcome is recorded in the cache as it happens.

use std::fmt;

use bagholder_book::{Book, BookError};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{InstrumentId, SourceName};
use bagholder_net::Net;

use crate::cache::{CacheError, MarketCache, OutcomeRow, ReadRow};
use crate::contract::DataKind;
use crate::outcome::{Noted, OutcomeKind};

/// A failure to keep what was read: the book or the cache refused a write.
/// Unlike a source's failure, this stops the run.
#[derive(Debug)]
pub enum RunError {
    Book(BookError),
    Cache(CacheError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Book(e) => write!(f, "the book: {e}"),
            RunError::Cache(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<BookError> for RunError {
    fn from(e: BookError) -> Self {
        RunError::Book(e)
    }
}

impl From<CacheError> for RunError {
    fn from(e: CacheError) -> Self {
        RunError::Cache(e)
    }
}

pub type Result<T> = std::result::Result<T, RunError>;

pub struct Ctx<'a> {
    pub book: &'a Book,
    pub cache: &'a MarketCache,
    pub net: &'a Net,
    /// Now, as the caller's clock says.
    pub now: Timestamp,
    /// The Bank of Canada's zone (Eastern), whose 16:30 is when a day's rate is
    /// published and whose day is the markets' session day.
    pub bank: &'a TimeZone,
}

impl Ctx<'_> {
    /// Whether the newest read of `subject`'s `kind` failed within its host's
    /// rest: a failure is asked again on the source's own rest, never in a loop.
    pub fn resting(&self, subject: &str, kind: DataKind, host: &str) -> Result<bool> {
        let reads = self.cache.reads(subject, kind)?;
        let Some(newest) = reads.first() else { return Ok(false) };
        // the rest grows with each failure in a row of this subject's source
        let instrument = InstrumentId::parse(subject).ok();
        let failed = self.cache.failures_in_a_row(&newest.source, kind, instrument)?;
        Ok(crate::market::resting(&reads, self.now, crate::market::grown_rest(self.net.limiter().pace(host).rest, failed)))
    }

    /// Keep that `subject`'s `kind` was read today, and how it ended: what
    /// [`Ctx::resting`] reads.
    pub fn attempted(&self, subject: &str, kind: DataKind, source: &SourceName, outcome: OutcomeKind) -> Result<()> {
        let today = self.today();
        self.cache.store_read(subject, kind, &ReadRow { source: source.clone(), first: today, last: today, outcome, at: self.now })?;
        Ok(())
    }

    /// Today in the Bank's zone.
    pub fn today(&self) -> Date {
        self.now.to_zoned(self.bank.clone()).date()
    }

    /// Record one request's outcome.
    pub fn record<A>(&self, source: &SourceName, host: &str, kind: DataKind, instrument: Option<InstrumentId>, noted: &Noted<A>) -> Result<()> {
        self.record_row(source, host, kind, instrument, noted, noted.outcome.detail())
    }

    /// Record one request's outcome under what was asked (`^TSX`), for requests
    /// of one source and kind that are for different things with no instrument.
    pub fn record_detail<A>(&self, source: &SourceName, host: &str, kind: DataKind, instrument: Option<InstrumentId>, noted: &Noted<A>, asked: &str) -> Result<()> {
        let detail = match noted.outcome.detail() {
            d if d.is_empty() => asked.to_string(),
            d => format!("{asked}: {d}"),
        };
        self.record_row(source, host, kind, instrument, noted, detail)
    }

    fn record_row<A>(&self, source: &SourceName, host: &str, kind: DataKind, instrument: Option<InstrumentId>, noted: &Noted<A>, detail: String) -> Result<()> {
        self.cache.record(&OutcomeRow {
            source: source.clone(),
            host: host.to_string(),
            kind,
            instrument,
            outcome: noted.outcome.kind(),
            detail,
            shape_change: noted.shape_change.as_ref().map(|c| c.to_string()),
            at: self.now,
        })?;
        Ok(())
    }
}
