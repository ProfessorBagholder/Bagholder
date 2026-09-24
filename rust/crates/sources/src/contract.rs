//! The vocabulary every adapter shares (`docs/plans/stage-3a-sources.md`, "The
//! contract"): the kinds of data, the markets, what a source offers, and the
//! instrument as a source sees it.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use bagholder_core::instrument::{InstrumentKind, RefScheme};
use bagholder_core::{Currency, InstrumentId};

/// A kind of data a source answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataKind {
    Quote,
    DailyClose,
    Benchmark,
    Rate,
    Holidays,
    Distributions,
}

impl DataKind {
    pub const ALL: [DataKind; 6] = [DataKind::Quote, DataKind::DailyClose, DataKind::Benchmark, DataKind::Rate, DataKind::Holidays, DataKind::Distributions];

    pub fn as_str(self) -> &'static str {
        match self {
            DataKind::Quote => "quote",
            DataKind::DailyClose => "daily-close",
            DataKind::Benchmark => "benchmark",
            DataKind::Rate => "rate",
            DataKind::Holidays => "holidays",
            DataKind::Distributions => "distributions",
        }
    }

    pub fn parse(s: &str) -> Option<DataKind> {
        DataKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl fmt::Display for DataKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where an instrument trades, as far as choosing a source goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Market {
    /// The TSX, the TSX Venture, the CSE, and the Canadian ATSs (Alpha).
    Canada,
    /// Cboe Canada: quoted by its own feed, not TMX's.
    CboeCanada,
    UnitedStates,
    Crypto,
    /// US-listed option contracts.
    UsOptions,
}

/// One kind of data a source answers for one market, and how late it is by
/// design (Cboe's chains are fifteen minutes behind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub kind: DataKind,
    pub market: Option<Market>,
    pub late_by: Duration,
}

/// The benchmarks the yearly returns are measured against (`SPEC.md` §2,
/// Index), each read as the total return of an ETF that tracks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Benchmark {
    Sp500,
    Tsx,
    Tx60,
}

impl Benchmark {
    pub const ALL: [Benchmark; 3] = [Benchmark::Sp500, Benchmark::Tsx, Benchmark::Tx60];

    /// The key the engine reads it under.
    pub fn key(self) -> &'static str {
        match self {
            Benchmark::Sp500 => "SP500",
            Benchmark::Tsx => "TSX",
            Benchmark::Tx60 => "TX60",
        }
    }

    pub fn parse(s: &str) -> Option<Benchmark> {
        Benchmark::ALL.into_iter().find(|b| b.key() == s)
    }

    /// The ETF read for it, as Yahoo names it, and the currency it trades in:
    /// SPY for the S&P 500; XIC, which tracks the capped Composite, for the
    /// S&P/TSX Composite; XIU for the S&P/TSX 60.
    pub fn tracker(self) -> (&'static str, Currency) {
        match self {
            Benchmark::Sp500 => ("SPY", Currency::USD),
            Benchmark::Tsx => ("XIC.TO", Currency::CAD),
            Benchmark::Tx60 => ("XIU.TO", Currency::CAD),
        }
    }

    /// The market its tracker trades on, whose sessions its closes settle by.
    pub fn market(self) -> Market {
        match self {
            Benchmark::Sp500 => Market::UnitedStates,
            Benchmark::Tsx | Benchmark::Tx60 => Market::Canada,
        }
    }
}

/// An instrument as the sources see it: what it is, what it is called now and
/// where, and the routing references the book holds for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub id: InstrumentId,
    pub kind: InstrumentKind,
    pub currency: Currency,
    /// The symbol its records use now.
    pub symbol: String,
    /// The venue's market identifier code (ISO 10383), where the record names one.
    pub venue_mic: Option<String>,
    /// The routing references the book holds, by scheme (`TmxForm`, `Yahoo`, …).
    pub routes: BTreeMap<RefScheme, Vec<String>>,
}

impl Listing {
    /// The market the instrument trades in, from its kind and venue; `None` for
    /// an instrument no source of this part covers (an event contract).
    pub fn market(&self) -> Option<Market> {
        match self.kind {
            InstrumentKind::Crypto => Some(Market::Crypto),
            InstrumentKind::OptionContract => Some(Market::UsOptions),
            InstrumentKind::Security => crate::venue::market_of(self.venue_mic.as_deref()),
            _ => None,
        }
    }

    pub fn route(&self, scheme: &RefScheme) -> Option<&str> {
        self.routes.get(scheme).and_then(|v| v.first()).map(String::as_str)
    }
}
