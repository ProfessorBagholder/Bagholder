//! Daily closes and benchmark levels into the market cache
//! (`docs/plans/stage-3a-sources.md`, "Quotes, daily closes and benchmarks",
//! "Periodic reads").
//!
//! A session's close is read once it has settled: 16:30 Eastern for a listing
//! (the closing auction printed), the end of the UTC day for a coin. A closed day
//! with a close stored is never read again. A day the source answered with no
//! close (a holiday) is not asked again: a read is due only when a session has
//! settled since the source last answered for the instrument.
//!
//! The chains: a listing's closes from Yahoo, under the Yahoo form the book
//! routes it by or its venue's; a coin's from the Exchange, its own pair's market
//! first, else its USD market's (`SPEC.md` §2). The S&P 500 from FRED for FRED's
//! trailing ten years and from Yahoo's `^GSPC` before them (read once, an
//! archive); the S&P/TSX Composite and 60 from TMX, from 2001-12-11.

use std::collections::BTreeSet;

use bagholder_core::instrument::RefScheme;
use bagholder_core::jiff::civil::{date, Date, Weekday};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Currency, Dec, InstrumentId, SourceName};

use crate::adapters::{coinbase, fred, tmx, yahoo};
use crate::contract::{Benchmark, DataKind, Market};
use crate::needs::CloseNeed;
use crate::outcome::{Noted, Outcome};
use crate::read::{Ctx, Result};
use crate::venue;

/// The first day TMX holds the S&P/TSX Composite and 60 (research 5).
pub const TSX_FIRST: Date = date(2001, 12, 11);

fn is_weekend(d: Date) -> bool {
    matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// When a day's close has settled for a market: 16:30 Eastern for a listing, the
/// end of the UTC day for a coin.
pub fn settled_at(market: Market, d: Date, bank: &TimeZone) -> Option<Timestamp> {
    match market {
        Market::Crypto => d.tomorrow().ok()?.to_zoned(TimeZone::UTC).ok().map(|z| z.timestamp()),
        _ => d.at(16, 30, 0, 0).to_zoned(bank.clone()).ok().map(|z| z.timestamp()),
    }
}

/// Whether a market can have a session on `d`: every day for a coin, a weekday
/// for a listing (a holiday is known when the source answers with no close).
fn can_trade(market: Market, d: Date) -> bool {
    market == Market::Crypto || !is_weekend(d)
}

/// The latest day on or before `to` whose close has settled by `now`.
pub fn latest_settled(market: Market, to: Date, now: Timestamp, bank: &TimeZone) -> Option<Date> {
    let mut d = to.min(now.to_zoned(bank.clone()).date());
    for _ in 0..10 {
        if can_trade(market, d) && settled_at(market, d, bank).is_some_and(|s| s <= now) {
            return Some(d);
        }
        d = d.yesterday().ok()?;
    }
    None
}

/// What the cache holds of one instrument's closes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloseState {
    pub days: BTreeSet<Date>,
    /// When a source last answered for its closes.
    pub answered: Option<Timestamp>,
}

/// The span of closes due for one need, if any.
pub fn due_close(need: &CloseNeed, market: Market, state: &CloseState, now: Timestamp, bank: &TimeZone) -> Option<(Date, Date)> {
    let latest = latest_settled(market, need.to, now, bank)?;
    // the days stored reach back to the need's first (within a week, which a
    // holiday and a weekend can leave without a session); otherwise from it
    let reach = need.from.checked_add(SignedDuration::from_hours(24 * 7)).unwrap_or(need.from);
    let start = match (state.days.first(), state.days.last()) {
        (Some(first), Some(last)) if *first <= reach => last.tomorrow().ok()?,
        _ => need.from,
    };
    if latest < start {
        return None;
    }
    let settled = settled_at(market, latest, bank)?;
    if state.answered.is_some_and(|a| a >= settled) && state.days.first().is_some_and(|f| *f <= reach) {
        return None;
    }
    Some((start, latest))
}

/// The Yahoo form of a listing: the book's routing reference, else its venue's.
pub fn yahoo_form(l: &crate::contract::Listing) -> Option<String> {
    l.route(&RefScheme::Yahoo).map(str::to_string).or_else(|| l.venue_mic.as_deref().and_then(|mic| venue::yahoo_form(&l.symbol, mic)))
}

/// A coin's Exchange pairs, its own market first.
pub fn coin_pairs(l: &crate::contract::Listing) -> Vec<String> {
    let base = l.symbol.trim().to_ascii_uppercase();
    let mut pairs = vec![format!("{base}-{}", l.currency.as_str())];
    if l.currency != Currency::USD {
        pairs.push(format!("{base}-USD"));
    }
    pairs
}

fn store(ctx: &Ctx, id: InstrumentId, closes: &[(Date, Dec)], currency: Currency, source: &SourceName, host: &str) -> Result<()> {
    let disagreements = ctx.cache.store_closes(id, closes, currency, source, ctx.now)?;
    for d in disagreements {
        // a later value for a closed day: the first stands, and this is a meaning outcome
        let noted: Noted<()> = Noted { outcome: Outcome::Meaning(format!("{} {}: {} stands, {} came later", id, d.day, d.stands, d.later)), shape_change: None };
        ctx.record(source, host, DataKind::DailyClose, Some(id), &noted)?;
    }
    Ok(())
}

/// Read the closes due for every need.
pub fn read_closes(ctx: &Ctx, needs: &[CloseNeed]) -> Result<()> {
    for need in needs {
        let Some(market) = need.listing.market() else { continue };
        let id = need.listing.id;
        let answered = match market {
            Market::Crypto => ctx.cache.last_answered(&coinbase::exchange_source(), DataKind::DailyClose, Some(id))?,
            _ => ctx.cache.last_answered(&yahoo::source(), DataKind::DailyClose, Some(id))?,
        };
        let state = CloseState { days: ctx.cache.close_days(id)?, answered };
        let Some((from, to)) = due_close(need, market, &state, ctx.now, ctx.bank) else { continue };
        match market {
            Market::UsOptions => {}
            Market::Crypto => {
                // the winner first, then the rest of the chain
                let mut pairs = coin_pairs(&need.listing);
                if let Some((_, won)) = ctx.cache.winner(id, DataKind::DailyClose)? {
                    pairs.sort_by_key(|p| *p != won);
                }
                for pair in pairs {
                    let noted = coinbase::ask_candles(ctx.net, &pair, from, to, ctx.now);
                    ctx.record(&coinbase::exchange_source(), coinbase::EXCHANGE_HOST, DataKind::DailyClose, Some(id), &noted)?;
                    match noted.outcome {
                        Outcome::Answered(days) => {
                            let quote = pair.rsplit('-').next().and_then(|c| Currency::parse(c).ok()).unwrap_or(need.listing.currency);
                            store(ctx, id, &days, quote, &coinbase::exchange_source(), coinbase::EXCHANGE_HOST)?;
                            ctx.cache.won(id, DataKind::DailyClose, &coinbase::exchange_source(), &pair, ctx.now)?;
                            break;
                        }
                        Outcome::NotCarried(_) => continue,
                        _ => break,
                    }
                }
            }
            _ => {
                let Some(form) = yahoo_form(&need.listing) else {
                    let noted: Noted<()> = Noted { outcome: Outcome::NotCarried(format!("{} names no venue Yahoo carries", need.listing.symbol)), shape_change: None };
                    ctx.record(&yahoo::source(), yahoo::HOST, DataKind::DailyClose, Some(id), &noted)?;
                    continue;
                };
                let noted = yahoo::ask_span(ctx.net, &form, from, to, ctx.now);
                let answered = match &noted.outcome {
                    Outcome::Answered(c) => Some((c.closes.clone(), c.currency)),
                    _ => None,
                };
                ctx.record(&yahoo::source(), yahoo::HOST, DataKind::DailyClose, Some(id), &noted)?;
                if let Some((closes, currency)) = answered {
                    store(ctx, id, &closes, currency, &yahoo::source(), yahoo::HOST)?;
                    ctx.cache.won(id, DataKind::DailyClose, &yahoo::source(), &form, ctx.now)?;
                }
            }
        }
    }
    Ok(())
}

/// Read the benchmark levels due, from `from` (the person's oldest day).
pub fn read_benchmarks(ctx: &Ctx, from: Date) -> Result<()> {
    let today = ctx.today();
    let Some(latest) = latest_settled(Market::UnitedStates, today, ctx.now, ctx.bank) else { return Ok(()) };
    let settled = settled_at(Market::UnitedStates, latest, ctx.bank);
    let fresh = |source: &SourceName| -> Result<bool> { Ok(ctx.cache.last_answered(source, DataKind::Benchmark, None)?.zip(settled).is_some_and(|(a, s)| a >= s)) };

    // the S&P 500: FRED's trailing ten years
    let sp = ctx.cache.benchmark_days(Benchmark::Sp500)?;
    if !sp.contains(&latest) && !fresh(&fred::source())? {
        let noted = fred::ask(ctx.net);
        ctx.record(&fred::source(), fred::HOST, DataKind::Benchmark, None, &noted)?;
        if let Outcome::Answered(levels) = noted.outcome {
            ctx.cache.store_benchmark(Benchmark::Sp500, &levels, &fred::source(), ctx.now)?;
        }
    }
    // before FRED's first day, Yahoo's ^GSPC, read once for the span it lacks
    let sp = ctx.cache.benchmark_days(Benchmark::Sp500)?;
    if let Some(fred_first) = sp.first().copied() {
        let reach = from.checked_add(SignedDuration::from_hours(24 * 7)).unwrap_or(from);
        if fred_first > reach && ctx.cache.last_answered(&yahoo::source(), DataKind::Benchmark, None)?.is_none() {
            let noted = yahoo::ask_span(ctx.net, "^GSPC", from, fred_first.yesterday().unwrap_or(fred_first), ctx.now);
            let levels = match &noted.outcome {
                Outcome::Answered(c) => Some(c.closes.clone()),
                _ => None,
            };
            ctx.record(&yahoo::source(), yahoo::HOST, DataKind::Benchmark, None, &noted)?;
            if let Some(levels) = levels {
                ctx.cache.store_benchmark(Benchmark::Sp500, &levels, &yahoo::source(), ctx.now)?;
            }
        }
    }
    // the S&P/TSX Composite and 60 from TMX
    for (b, symbol) in [(Benchmark::Tsx, "^TSX"), (Benchmark::Tx60, "^TX60")] {
        let days = ctx.cache.benchmark_days(b)?;
        let start = days.last().and_then(|d| d.tomorrow().ok()).unwrap_or(from.max(TSX_FIRST)).max(TSX_FIRST);
        if start > latest || fresh_for(ctx, symbol, settled)? {
            continue;
        }
        let noted = tmx::ask_series(ctx.net, symbol, start, latest);
        ctx.record_detail(&tmx::source(), tmx::HOST, DataKind::Benchmark, None, &noted, symbol)?;
        if let Outcome::Answered(levels) = noted.outcome {
            ctx.cache.store_benchmark(b, &levels, &tmx::source(), ctx.now)?;
        }
    }
    Ok(())
}

/// Whether TMX answered for `symbol`'s series since the latest settled close.
fn fresh_for(ctx: &Ctx, symbol: &str, settled: Option<Timestamp>) -> Result<bool> {
    let outcomes = ctx.cache.outcomes(&tmx::source())?;
    Ok(outcomes.iter().find(|o| o.kind == DataKind::Benchmark && o.outcome == crate::outcome::OutcomeKind::Answered && o.detail == symbol).zip(settled).is_some_and(|(o, s)| o.at >= s))
}
