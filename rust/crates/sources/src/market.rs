//! Daily closes and benchmark levels into the market cache
//! (`docs/plans/stage-3a-sources.md`, "Quotes, daily closes and benchmarks",
//! "Periodic reads").
//!
//! A session's close is read once it has settled: 16:30 Eastern for a listing
//! (the closing auction printed), the end of the UTC day for a coin. A day is due
//! until it is stored or a read made after it settled covered it (the cache's
//! `reads`): a closed day with a close stored is never read again, a day a read
//! answered through with no close (a holiday) is not asked again, and a day after
//! the last one a read held stays due, since a source can lag. A failed read
//! waits out its source's rest.
//!
//! The chains: a listing's closes from Yahoo, under the Yahoo form the book
//! routes it by or its venue's; a coin's from the Exchange, its own pair's market
//! first, else its USD market's (`SPEC.md` §2). The S&P 500 from FRED for FRED's
//! trailing ten years and from Yahoo's `^GSPC` before them; the S&P/TSX Composite
//! and 60 from TMX, from 2001-12-11. Each reaches back when the oldest day needed
//! moves earlier.

use std::collections::BTreeSet;

use bagholder_core::instrument::RefScheme;
use bagholder_core::jiff::civil::{date, Date, Weekday};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use std::time::Duration;

use bagholder_core::{Currency, Dec, InstrumentId, SourceName};

use crate::adapters::{coinbase, fred, tmx, yahoo};
use crate::contract::{Benchmark, DataKind, Market};
use crate::needs::CloseNeed;
use crate::cache::ReadRow;
use crate::outcome::{Noted, Outcome, OutcomeKind};
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

/// What the cache holds of one subject (an instrument's closes, a benchmark's
/// levels): the days stored, and the reads made, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloseState {
    pub days: BTreeSet<Date>,
    pub reads: Vec<ReadRow>,
}

/// Whether the newest read failed within its source's rest: a failure is asked
/// again on the source's own rest, never in a loop.
pub fn resting(reads: &[ReadRow], now: Timestamp, rest: Duration) -> bool {
    reads.first().is_some_and(|r| (r.outcome.is_failure() || r.outcome == OutcomeKind::Refused) && now < r.at + SignedDuration::try_from(rest).unwrap_or(SignedDuration::ZERO))
}

/// Whether a read made after `d` settled covered it: its source answered through
/// it, or said it does not carry the subject.
pub(crate) fn settled_by_read(reads: &[ReadRow], market: Market, d: Date, bank: &TimeZone) -> bool {
    let Some(settled) = settled_at(market, d, bank) else { return false };
    reads.iter().any(|r| matches!(r.outcome, OutcomeKind::Answered | OutcomeKind::NotCarried) && r.first <= d && d <= r.last && r.at >= settled)
}

/// The span of days due for `from`..=`to`: from the first to the last settled
/// session day that is neither stored nor covered by a read made after it
/// settled. A day the source answered with no value (a holiday) is covered by
/// that read and not asked again; a day after the last one a read held is not
/// known yet and stays due.
pub fn due_span(market: Market, from: Date, to: Date, state: &CloseState, now: Timestamp, bank: &TimeZone, rest: Duration) -> Option<(Date, Date)> {
    if resting(&state.reads, now, rest) {
        return None;
    }
    let latest = latest_settled(market, to, now, bank)?;
    let (mut first, mut last) = (None, None);
    let mut d = from;
    while d <= latest {
        if can_trade(market, d) && !state.days.contains(&d) && !settled_by_read(&state.reads, market, d, bank) {
            first.get_or_insert(d);
            last = Some(d);
        }
        d = d.tomorrow().ok()?;
    }
    Some((first?, last?))
}

/// The span of closes due for one need, if any.
pub fn due_close(need: &CloseNeed, market: Market, state: &CloseState, now: Timestamp, bank: &TimeZone, rest: Duration) -> Option<(Date, Date)> {
    due_span(market, need.from, need.to, state, now, bank, rest)
}

/// Keep one read of a subject: an answer settles from the first day asked to the
/// last day it held (nothing, if it held none); a source not carrying the
/// subject, or failing, the span asked.
pub(crate) fn keep_read(ctx: &Ctx, subject: &str, kind: DataKind, source: &SourceName, asked: (Date, Date), outcome: OutcomeKind, held_last: Option<Date>) -> Result<()> {
    let last = match outcome {
        OutcomeKind::Answered => match held_last {
            Some(l) => l.min(asked.1),
            None => return Ok(()),
        },
        _ => asked.1,
    };
    ctx.cache.store_read(subject, kind, &ReadRow { source: source.clone(), first: asked.0, last, outcome, at: ctx.now })?;
    Ok(())
}

fn rest_of(ctx: &Ctx, host: &str) -> Duration {
    ctx.net.limiter().pace(host).rest
}

/// The Yahoo forms of a listing to ask, in order: the book's routing reference
/// alone where it holds one, else its venue's forms.
pub fn yahoo_forms(l: &crate::contract::Listing) -> Vec<String> {
    match l.route(&RefScheme::Yahoo) {
        Some(r) => vec![r.to_string()],
        None => l.venue_mic.as_deref().map(|mic| venue::yahoo_forms(&l.symbol, mic)).unwrap_or_default(),
    }
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

/// Read the closes due for every need. One read is kept per need and run: the
/// chain's answer, or that no source in it carries the listing, or its failure.
pub fn read_closes(ctx: &Ctx, needs: &[CloseNeed]) -> Result<()> {
    for need in needs {
        let Some(market) = need.listing.market() else { continue };
        // option contracts: from their chains (`options`)
        if market == Market::UsOptions {
            continue;
        }
        let id = need.listing.id;
        let subject = id.to_string();
        let (source, host) = match market {
            Market::Crypto => (coinbase::exchange_source(), coinbase::EXCHANGE_HOST),
            _ => (yahoo::source(), yahoo::HOST),
        };
        let state = CloseState { days: ctx.cache.close_days(id)?, reads: ctx.cache.reads(&subject, DataKind::DailyClose)? };
        let Some((from, to)) = due_close(need, market, &state, ctx.now, ctx.bank, rest_of(ctx, host)) else { continue };
        // the forms in order, the winner first
        let mut forms = match market {
            Market::Crypto => coin_pairs(&need.listing),
            _ => yahoo_forms(&need.listing),
        };
        if forms.is_empty() {
            let noted: Noted<()> = Noted { outcome: Outcome::NotCarried(format!("{} names no venue Yahoo carries", need.listing.symbol)), shape_change: None };
            ctx.record(&source, host, DataKind::DailyClose, Some(id), &noted)?;
            keep_read(ctx, &subject, DataKind::DailyClose, &source, (from, to), OutcomeKind::NotCarried, None)?;
            continue;
        }
        if let Some((_, won)) = ctx.cache.winner(id, DataKind::DailyClose)? {
            forms.sort_by_key(|f| *f != won);
        }
        let mut result = OutcomeKind::NotCarried;
        let mut held_last = None;
        for form in forms {
            let (kind, answered) = match market {
                Market::Crypto => {
                    let noted = coinbase::ask_candles(ctx.net, &form, from, to, ctx.now);
                    ctx.record_detail(&source, host, DataKind::DailyClose, Some(id), &noted, &form)?;
                    let kind = noted.outcome.kind();
                    let quote = form.rsplit('-').next().and_then(|c| Currency::parse(c).ok()).unwrap_or(need.listing.currency);
                    (kind, match noted.outcome {
                        Outcome::Answered(days) => Some((days, quote)),
                        _ => None,
                    })
                }
                _ => {
                    let mut noted = yahoo::ask_span(ctx.net, &form, from, to, ctx.now);
                    crate::quotes::same_currency(&mut noted.outcome, |c| c.currency, &need.listing);
                    ctx.record_detail(&source, host, DataKind::DailyClose, Some(id), &noted, &form)?;
                    let kind = noted.outcome.kind();
                    (kind, match noted.outcome {
                        Outcome::Answered(c) => Some((c.closes.into_iter().filter(|(d, _)| *d >= from && *d <= to).collect::<Vec<_>>(), c.currency)),
                        _ => None,
                    })
                }
            };
            result = kind;
            if let Some((closes, currency)) = answered {
                held_last = closes.last().map(|c| c.0);
                store(ctx, id, &closes, currency, &source, host)?;
                ctx.cache.won(id, DataKind::DailyClose, &source, &form, ctx.now)?;
            }
            if kind != OutcomeKind::NotCarried {
                break;
            }
        }
        keep_read(ctx, &subject, DataKind::DailyClose, &source, (from, to), result, held_last)?;
    }
    Ok(())
}

/// Read one benchmark's span from its source, storing what it holds and the read.
fn benchmark_span(ctx: &Ctx, b: Benchmark, source: &SourceName, host: &str, asked: (Date, Date), noted: Noted<Vec<(Date, Dec)>>, detail: &str) -> Result<()> {
    ctx.record_detail(source, host, DataKind::Benchmark, None, &noted, detail)?;
    let kind = noted.outcome.kind();
    let mut held_last = None;
    if let Outcome::Answered(levels) = noted.outcome {
        let levels: Vec<(Date, Dec)> = levels.into_iter().filter(|(d, _)| *d >= asked.0 && *d <= asked.1).collect();
        held_last = levels.last().map(|l| l.0);
        ctx.cache.store_benchmark(b, &levels, source, ctx.now)?;
    }
    keep_read(ctx, b.key(), DataKind::Benchmark, source, asked, kind, held_last)
}

/// Read the benchmark levels due, from `from` (the person's oldest day). Each
/// index's days are due as a close's are: every settled session day from the
/// oldest needed that is neither stored nor covered by a read made after it
/// settled, so the span reaches back when the oldest day moves earlier.
pub fn read_benchmarks(ctx: &Ctx, from: Date) -> Result<()> {
    let today = ctx.today();
    let key = Benchmark::Sp500.key();
    // the S&P 500: FRED's trailing ten years in one reply, which settles from its
    // first day to its last; before FRED's first day, Yahoo's ^GSPC
    let state = |b: Benchmark| -> Result<CloseState> { Ok(CloseState { days: ctx.cache.benchmark_days(b)?, reads: ctx.cache.reads(b.key(), DataKind::Benchmark)? }) };
    let fred_reads: Vec<ReadRow> = state(Benchmark::Sp500)?.reads.into_iter().filter(|r| r.source == fred::source()).collect();
    let fred_from = fred_reads.iter().filter(|r| r.outcome == OutcomeKind::Answered).map(|r| r.first).min().unwrap_or(from).max(from);
    let fred_state = CloseState { days: ctx.cache.benchmark_days(Benchmark::Sp500)?, reads: fred_reads };
    if let Some((_, to)) = due_span(Market::UnitedStates, fred_from, today, &fred_state, ctx.now, ctx.bank, rest_of(ctx, fred::HOST)) {
        let noted = fred::ask(ctx.net);
        // FRED answers its whole window whatever is asked: the read settles from
        // the first day it held
        let first = match &noted.outcome {
            Outcome::Answered(l) => l.first().map_or(fred_from, |f| f.0),
            _ => fred_from,
        };
        benchmark_span(ctx, Benchmark::Sp500, &fred::source(), fred::HOST, (first, to), noted, key)?;
    }
    let fred_first = ctx.cache.reads(key, DataKind::Benchmark)?.iter().filter(|r| r.source == fred::source() && r.outcome == OutcomeKind::Answered).map(|r| r.first).min();
    if let Some(fred_first) = fred_first.filter(|f| *f > from) {
        let before = fred_first.yesterday().unwrap_or(fred_first);
        let gspc = CloseState { days: ctx.cache.benchmark_days(Benchmark::Sp500)?, reads: ctx.cache.reads(key, DataKind::Benchmark)?.into_iter().filter(|r| r.source == yahoo::source()).collect() };
        if let Some((a, b)) = due_span(Market::UnitedStates, from, before, &gspc, ctx.now, ctx.bank, rest_of(ctx, yahoo::HOST)) {
            let noted = yahoo::ask_span(ctx.net, "^GSPC", a, b, ctx.now);
            let noted = Noted { outcome: noted.outcome.map(|c| c.closes), shape_change: noted.shape_change };
            benchmark_span(ctx, Benchmark::Sp500, &yahoo::source(), yahoo::HOST, (a, b), noted, "^GSPC")?;
        }
    }
    // the S&P/TSX Composite and 60 from TMX, from 2001-12-11
    for (b, symbol) in [(Benchmark::Tsx, "^TSX"), (Benchmark::Tx60, "^TX60")] {
        if let Some((a, z)) = due_span(Market::Canada, from.max(TSX_FIRST), today, &state(b)?, ctx.now, ctx.bank, rest_of(ctx, tmx::HOST)) {
            let noted = tmx::ask_series(ctx.net, symbol, a, z);
            benchmark_span(ctx, b, &tmx::source(), tmx::HOST, (a, z), noted, symbol)?;
        }
    }
    Ok(())
}
