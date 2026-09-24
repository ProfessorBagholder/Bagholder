//! The Bank of Canada's rates and holidays into the book
//! (`docs/plans/stage-3a-sources.md`, "The fact readers", "Periodic reads").
//!
//! Which reads are due is a pure function of what the book holds and the time
//! handed in ([`due_daily`], [`due_daily_whole`], [`due_archives`],
//! [`due_holidays`]); [`read`] runs them. For each currency the engine converts,
//! from the oldest day it converts it on:
//!
//! - **the daily series**: read whole the first time (so its first day, and
//!   whether the Bank marks it historical, are its own), then forward from the
//!   last day read, once a business day's 16:30 Eastern has passed without its
//!   rate stored;
//! - **the noon archive**, when the oldest day is before the daily series (or the
//!   currency has none): each of its series once, whole, since it never changes;
//! - **Statistics Canada's archive**, when the oldest day is before the noon
//!   series and the table holds the currency: once, whole, to 2007-04-30.
//!
//! The group of daily series is read when a currency needed has no series stored
//! from any source, at most once a day: that is how a currency is found to be one
//! the Bank publishes, or not. The holiday page is read once each month.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::facts::RateSeries;
use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Currency, SourceName};

use crate::adapters::{boc, holidays, statcan};
use crate::contract::DataKind;
use crate::outcome::{Noted, Outcome, OutcomeKind};
use crate::read::{Ctx, Result};

/// One currency the engine converts, and the oldest day it converts it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub currency: Currency,
    pub oldest: Date,
}

/// What the book and the cache hold that decides which reads are due.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub series: Vec<RateSeries>,
    /// Per currency, the spans completed reads covered, and when each was received.
    pub reads: BTreeMap<Currency, Vec<(Date, Date, Timestamp)>>,
    pub holidays: BTreeSet<Date>,
    /// When the group of daily series was last answered.
    pub list_read: Option<Timestamp>,
    /// When the holiday page was last answered.
    pub holidays_read: Option<Timestamp>,
}

/// One read the Bank's sources are due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    DailyList,
    /// The whole daily series of a currency the Bank lists, read once.
    DailyWhole(Currency),
    /// A daily series from the day after the last read to today.
    DailyForward(Currency, Date, Date),
    Noon(Currency),
    Archive(Currency),
}

fn series_of<'a>(held: &'a Held, c: Currency, source: &str) -> Option<&'a RateSeries> {
    held.series.iter().find(|s| s.currency == c && s.source.as_str() == source)
}

fn is_weekend(d: Date) -> bool {
    matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// 16:30 in the Bank's zone on `d`: when its rate for `d` is published.
pub fn published_at(d: Date, bank: &TimeZone) -> Option<Timestamp> {
    d.at(16, 30, 0, 0).to_zoned(bank.clone()).ok().map(|z| z.timestamp())
}

/// The first business day after `d`, as far as the Bank's own schedule says.
fn next_business_day(d: Date, holidays: &BTreeSet<Date>) -> Option<Date> {
    let mut n = d.tomorrow().ok()?;
    while is_weekend(n) || holidays.contains(&n) {
        n = n.tomorrow().ok()?;
    }
    Some(n)
}

/// The reads due of the first phase: the group of daily series and the daily
/// series themselves.
pub fn due_daily(needs: &[Need], held: &Held, now: Timestamp, bank: &TimeZone) -> Vec<Due> {
    let today = now.to_zoned(bank.clone()).date();
    let mut out = Vec::new();
    let unknown = needs.iter().any(|n| !held.series.iter().any(|s| s.currency == n.currency));
    if unknown && held.list_read.is_none_or(|at| now.duration_since(at) >= SignedDuration::from_hours(24)) {
        out.push(Due::DailyList);
    }
    for n in needs {
        let Some(daily) = series_of(held, n.currency, boc::DAILY) else { continue };
        if daily.ended {
            continue;
        }
        // the last day a read of the daily series settled: a day it covered whose
        // rate was due when the read was received (a read at midnight covers
        // the day, and settles only the day before)
        let settled = |(first, last, at): &(Date, Date, Timestamp)| -> Option<Date> {
            let mut d = *last;
            while published_at(d, bank).is_none_or(|due| due > *at) {
                d = d.yesterday().ok()?;
            }
            (d >= *first && d >= daily.first_day).then_some(d)
        };
        let last = held.reads.get(&n.currency).into_iter().flatten().filter_map(settled).max().unwrap_or(daily.last_day);
        let Some(next) = next_business_day(last, &held.holidays) else { continue };
        if next <= today && published_at(next, bank).is_some_and(|at| now >= at) {
            out.push(Due::DailyForward(n.currency, last.tomorrow().expect("a real day"), today));
        }
    }
    out
}

/// The daily series of the currencies the group lists and the book does not
/// hold yet: each read whole, once.
pub fn due_daily_whole(needs: &[Need], held: &Held, listed: &BTreeSet<Currency>) -> Vec<Due> {
    needs.iter().filter(|n| listed.contains(&n.currency) && series_of(held, n.currency, boc::DAILY).is_none()).map(|n| Due::DailyWhole(n.currency)).collect()
}

/// The reads due of the second phase, once the daily series are known: the
/// archives, for the days before them.
pub fn due_archives(needs: &[Need], held: &Held) -> Vec<Due> {
    let mut out = Vec::new();
    for n in needs {
        let c = n.currency;
        let daily_first = series_of(held, c, boc::DAILY).map(|s| s.first_day);
        let noon = boc::noon_series(c);
        if let Some(noon_first) = noon.first().map(|s| s.first) {
            if daily_first.is_none_or(|f| n.oldest < f) && series_of(held, c, boc::NOON_SOURCE).is_none() {
                out.push(Due::Noon(c));
            }
            if let Some(a) = statcan::series_of(c) {
                if n.oldest < noon_first && n.oldest <= statcan::ERA_END && a.first <= statcan::ERA_END && series_of(held, c, statcan::SOURCE).is_none() {
                    out.push(Due::Archive(c));
                }
            }
        }
    }
    out
}

/// The holiday page: once in each month, in the Bank's zone.
pub fn due_holidays(held: &Held, now: Timestamp, bank: &TimeZone) -> bool {
    let month = |t: Timestamp| {
        let d = t.to_zoned(bank.clone()).date();
        (d.year(), d.month())
    };
    held.holidays_read.is_none_or(|at| month(at) != month(now))
}

/// What the book and the cache hold now.
pub fn held(ctx: &Ctx) -> Result<Held> {
    let last_answered = |source: SourceName, detail: Option<&str>| -> Result<Option<Timestamp>> {
        Ok(ctx.cache.outcomes(&source)?.into_iter().find(|o| o.outcome == OutcomeKind::Answered && detail.is_none_or(|d| o.detail == d)).map(|o| o.at))
    };
    Ok(Held {
        series: ctx.book.rate_series()?,
        reads: ctx.book.rate_reads()?,
        holidays: ctx.book.bank_holidays()?,
        list_read: last_answered(boc::daily_source(), Some(LIST_DETAIL))?,
        holidays_read: last_answered(holidays::source(), None)?,
    })
}

/// How an answered read of the group is told from a read of a series.
const LIST_DETAIL: &str = "the group of daily series";

/// Run every read due for `needs`, storing what answers into the book. A
/// source's failure is recorded and the run goes on; a refusal to store stops it.
pub fn read(ctx: &Ctx, needs: &[Need]) -> Result<()> {
    let needs: Vec<Need> = needs.iter().copied().filter(|n| n.currency != Currency::CAD).collect();
    let h = held(ctx)?;
    let mut listed = BTreeSet::new();
    for due in due_daily(&needs, &h, ctx.now, ctx.bank) {
        match due {
            Due::DailyList => {
                let noted = boc::ask_daily_list(ctx.net);
                record_list(ctx, &Noted { outcome: noted.outcome.clone().map(|_| ()), shape_change: noted.shape_change })?;
                if let Outcome::Answered(list) = noted.outcome {
                    listed = list.iter().map(|s| s.currency).collect();
                }
            }
            Due::DailyForward(c, from, to) => daily(ctx, c, Some((from, to)))?,
            _ => {}
        }
    }
    for due in due_daily_whole(&needs, &h, &listed) {
        if let Due::DailyWhole(c) = due {
            daily(ctx, c, None)?;
        }
    }
    let h = held(ctx)?;
    for due in due_archives(&needs, &h) {
        match due {
            Due::Noon(c) => noon(ctx, c)?,
            Due::Archive(c) => archive(ctx, c)?,
            _ => {}
        }
    }
    if due_holidays(&h, ctx.now, ctx.bank) {
        let noted = holidays::ask(ctx.net);
        ctx.record(&holidays::source(), holidays::HOST, DataKind::Holidays, None, &noted)?;
        if let Outcome::Answered(days) = noted.outcome {
            ctx.book.store_bank_holidays(&days, &holidays::source(), ctx.now)?;
        }
    }
    Ok(())
}

fn record_list(ctx: &Ctx, noted: &Noted<()>) -> Result<()> {
    ctx.cache.record(&crate::cache::OutcomeRow {
        source: boc::daily_source(),
        host: boc::HOST.into(),
        kind: DataKind::Rate,
        instrument: None,
        outcome: noted.outcome.kind(),
        detail: if noted.outcome.kind() == OutcomeKind::Answered { LIST_DETAIL.into() } else { format!("{LIST_DETAIL}: {}", noted.outcome.detail()) },
        shape_change: noted.shape_change.as_ref().map(|c| c.to_string()),
        at: ctx.now,
    })?;
    Ok(())
}

/// A daily series: whole (`span` none), or from the day after the last read.
fn daily(ctx: &Ctx, c: Currency, span: Option<(Date, Date)>) -> Result<()> {
    let code = boc::daily_code(c);
    let mut noted = boc::ask_observations(ctx.net, &code, span);
    // the description says what the series is, and whether it has ended
    let ended = match &noted.outcome {
        Outcome::Answered(o) => match boc::daily_description(&o.description) {
            Ok(ended) => Some(ended),
            Err(why) => {
                noted.outcome = Outcome::Meaning(why);
                None
            }
        },
        _ => None,
    };
    ctx.record(&boc::daily_source(), boc::HOST, DataKind::Rate, None, &noted)?;
    let (Outcome::Answered(o), Some(ended)) = (noted.outcome, ended) else { return Ok(()) };
    let today = ctx.today();
    let (Some(first), Some(last)) = (o.rates.first().map(|r| r.0), o.rates.last().map(|r| r.0)) else {
        // a read forward that found nothing new: the days it covered are still known
        if let Some((from, to)) = span {
            ctx.book.store_rates(c, &[], (from, to), &boc::daily_source(), ctx.now)?;
        }
        return Ok(());
    };
    // what the read covered: the span asked, or from the series' first day to
    // today; a series that has ended covers no day after its last
    let covered = match (span, ended) {
        (Some((from, _)), true) => (from, last),
        (Some((from, to)), false) => (from, to),
        (None, true) => (first, last),
        (None, false) => (first, today.max(last)),
    };
    ctx.book.store_rates(c, &o.rates, covered, &boc::daily_source(), ctx.now)?;
    let first_day = match span {
        // a forward read keeps the series' first day as the whole read found it
        Some(_) => ctx.book.rate_series()?.into_iter().find(|s| s.currency == c && s.source.as_str() == boc::DAILY).map_or(first, |s| s.first_day),
        None => first,
    };
    ctx.book.store_rate_series(&[RateSeries { currency: c, source: boc::daily_source(), first_day, last_day: last, ended }], ctx.now)?;
    Ok(())
}

/// A currency's noon archive, each of its series whole.
fn noon(ctx: &Ctx, c: Currency) -> Result<()> {
    let series = boc::noon_series(c);
    for s in &series {
        let span = (s.first, s.last);
        let mut noted = boc::ask_observations(ctx.net, s.code, Some(span));
        if let Outcome::Answered(o) = &noted.outcome {
            if o.description != s.description {
                noted.outcome = Outcome::Meaning(format!("{} is described as {:?}, not {:?}: another series than the noon spot rate", s.code, o.description, s.description));
            }
        }
        ctx.record(&boc::noon_source(), boc::HOST, DataKind::Rate, None, &noted)?;
        let Outcome::Answered(o) = noted.outcome else { return Ok(()) };
        ctx.book.store_rates(c, &o.rates, span, &boc::noon_source(), ctx.now)?;
    }
    if let (Some(first), Some(last)) = (series.first(), series.last()) {
        ctx.book.store_rate_series(&[RateSeries { currency: c, source: boc::noon_source(), first_day: first.first, last_day: last.last, ended: true }], ctx.now)?;
    }
    Ok(())
}

/// A currency's Statistics Canada archive, whole, to the day before the noon series.
fn archive(ctx: &Ctx, c: Currency) -> Result<()> {
    let Some(a) = statcan::series_of(c) else { return Ok(()) };
    let span = (a.first, statcan::ERA_END);
    let noted = statcan::ask(ctx.net, &a, span);
    ctx.record(&statcan::source(), statcan::HOST, DataKind::Rate, None, &noted)?;
    let Outcome::Answered(rates) = noted.outcome else { return Ok(()) };
    ctx.book.store_rates(c, &rates, span, &statcan::source(), ctx.now)?;
    ctx.book.store_rate_series(&[RateSeries { currency: c, source: statcan::source(), first_day: a.first, last_day: statcan::ERA_END, ended: true }], ctx.now)?;
    Ok(())
}
