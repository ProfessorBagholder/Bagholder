//! A price bar. It always has a close: a row without one is not a bar and is
//! dropped before it reaches one of these types. `Ohlcv` is the price
//! fields every bar shares; `DayBar`, `TimeBar` and `SourceBar` each add the
//! stamp that places it: a calendar date, a bar's start in unix seconds, or
//! -- for a bar as a source's own feed reports it, before it is put on a day
//! or a session grid -- the exchange-local day, minute and offset alongside
//! the time.

use serde::Serialize;
use ts_rs::TS;

/// The price fields every bar shares. A bar without a positive close was
/// never a bar; every reader that builds one has already checked that.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, TS)]
pub struct Ohlcv {
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: f64,
    pub volume: Option<f64>,
}

/// A daily (or weekly, or monthly) bar.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct DayBar {
    pub date: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub px: Ohlcv,
}

/// An intraday bar, on the grid the chart draws -- its `time` is the bar's
/// own start, in unix seconds.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct TimeBar {
    pub time: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub px: Ohlcv,
}

/// A bar as a source's own feed reports it -- Yahoo's chart or TMX's minute
/// feed -- before it is put on a day or a session grid: its epoch, the
/// exchange-local day and minute of day it falls on, and that day's UTC
/// offset in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct SourceBar {
    pub time: i64,
    pub day: String,
    pub minute: i64,
    pub offset: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub px: Ohlcv,
}

impl SourceBar {
    /// The bar on the intraday time grid -- its day, minute and offset
    /// dropped, since a session or clock aggregation only ever wants the
    /// epoch.
    pub fn at_time(&self) -> TimeBar {
        TimeBar { time: self.time, px: self.px }
    }

    /// The bar as a day of daily history -- its own exchange-local day,
    /// exactly as the source reported it.
    pub fn on_day(&self) -> DayBar {
        DayBar { date: self.day.clone(), px: self.px }
    }
}

/// What every kind of bar has in common: the day it falls on (a `DayBar`'s
/// own date; a `TimeBar` or `SourceBar`'s UTC day of its `time`) and its
/// price fields, read or written in place.
pub trait Bar {
    fn day(&self) -> String;
    fn px(&self) -> &Ohlcv;
    fn px_mut(&mut self) -> &mut Ohlcv;
}

impl Bar for DayBar {
    fn day(&self) -> String {
        self.date.clone()
    }
    fn px(&self) -> &Ohlcv {
        &self.px
    }
    fn px_mut(&mut self) -> &mut Ohlcv {
        &mut self.px
    }
}

impl Bar for TimeBar {
    fn day(&self) -> String {
        day_of_epoch(self.time)
    }
    fn px(&self) -> &Ohlcv {
        &self.px
    }
    fn px_mut(&mut self) -> &mut Ohlcv {
        &mut self.px
    }
}

impl Bar for SourceBar {
    fn day(&self) -> String {
        day_of_epoch(self.time)
    }
    fn px(&self) -> &Ohlcv {
        &self.px
    }
    fn px_mut(&mut self) -> &mut Ohlcv {
        &mut self.px
    }
}

/// The UTC calendar day a unix-second epoch falls on.
pub fn day_of_epoch(ts: i64) -> String {
    let (y, m, d) = bagholder_model::dates::from_days(ts.div_euclid(86400));
    bagholder_model::dates::fmt(y, m, d)
}

/// What the chart is sent for a span: daily (or weekly/monthly) bars, or
/// hourly ones -- never both, so the wire says which without a separate tag.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(untagged)]
pub enum ChartBars {
    Days(Vec<DayBar>),
    Hours(Vec<TimeBar>),
}

impl Default for ChartBars {
    fn default() -> ChartBars {
        ChartBars::Days(vec![])
    }
}

impl ChartBars {
    pub fn is_empty(&self) -> bool {
        match self {
            ChartBars::Days(v) => v.is_empty(),
            ChartBars::Hours(v) => v.is_empty(),
        }
    }
}

/// The daily-history fetch stamp for one symbol: how far back it reaches and
/// when it was last read.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryFetch {
    pub start: String,
    pub fetched_at: String,
}

/// The intraday-bar fetch stamp for one symbol and timeframe.
#[derive(Clone, Debug, PartialEq)]
pub struct BarFetch {
    pub start_ts: Option<i64>,
    pub fetched_at: String,
}
