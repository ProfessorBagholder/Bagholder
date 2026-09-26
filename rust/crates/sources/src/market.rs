//! Daily closes and the benchmarks' trackers into the market cache
//! (`docs/plans/stage-3a-sources.md`, "Quotes, daily closes and benchmarks",
//! "Periodic reads"; `docs/plans/stage-3a-brief-06.md`).
//!
//! A session's close is read once it has settled: 16:30 Eastern for a listing
//! (the closing auction printed), the end of the UTC day for a coin. A day is due
//! until it is stored or a read made after it settled covered it (the cache's
//! `reads`): a closed day with a close stored is never read again, a day a read
//! answered through with no close (a holiday) is not asked again, and a day after
//! the last one a read held stays due, since a source can lag. A source's word
//! that it does not carry the subject stands for the day it was said: the days
//! are asked again on a later day. A failed read waits out its source's rest.
//!
//! The chains: a listing's closes from Yahoo, under the Yahoo form the book
//! routes it by, else its venue's forms and then its market's others; a coin's
//! from the Exchange, its own pair's market first, else its USD market's
//! (`SPEC.md` §2), and where the Exchange carries neither (a coin Coinbase has
//! delisted), Yahoo's pairs in the same order. Each benchmark's tracker from
//! Yahoo, with its dividends and splits (`SPEC.md` §2, Index). Each reaches back
//! when the oldest day needed moves earlier.

use std::collections::BTreeSet;

use bagholder_core::instrument::RefScheme;
use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use std::time::Duration;

use bagholder_core::{Currency, Dec, InstrumentId, SourceName};

use crate::adapters::{coinbase, yahoo};
use crate::contract::{Benchmark, DataKind, Market};
use crate::needs::CloseNeed;
use crate::cache::{ReadRow, TrackerEvent};
use crate::outcome::{Noted, Outcome, OutcomeKind};
use crate::read::{Ctx, Result};
use crate::venue;

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

/// The rest after `failed` failures in a row of one source: its own rest after
/// the first, twice as long after each one after it, and six hours at most. A
/// source that answers wrongly (a date ten years out) does not mend in a minute,
/// and is not asked again every minute for as long as it is wrong.
pub fn grown_rest(rest: Duration, failed: u32) -> Duration {
    const MOST: Duration = Duration::from_secs(6 * 3600);
    if failed <= 1 {
        return rest.min(MOST);
    }
    rest.checked_mul(1u32 << (failed - 1).min(16)).unwrap_or(MOST).min(MOST)
}

/// Whether the newest read failed within its source's rest: a failure is asked
/// again on the source's own rest, never in a loop.
pub fn resting(reads: &[ReadRow], now: Timestamp, rest: Duration) -> bool {
    reads.first().is_some_and(|r| (r.outcome.is_failure() || r.outcome == OutcomeKind::Refused) && now < r.at + SignedDuration::try_from(rest).unwrap_or(SignedDuration::ZERO))
}

/// Whether a read made after `d` settled covered it: its source answered through
/// it, or said today that it does not carry the subject (a source can take a
/// listing on later, so that answer stands for the day it was given).
pub(crate) fn settled_by_read(reads: &[ReadRow], market: Market, d: Date, now: Timestamp, bank: &TimeZone) -> bool {
    let Some(settled) = settled_at(market, d, bank) else { return false };
    let today = now.to_zoned(bank.clone()).date();
    let counts = |r: &ReadRow| match r.outcome {
        OutcomeKind::Answered => true,
        OutcomeKind::NotCarried => r.at.to_zoned(bank.clone()).date() == today,
        _ => false,
    };
    reads.iter().any(|r| counts(r) && r.first <= d && d <= r.last && r.at >= settled)
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
        if can_trade(market, d) && !state.days.contains(&d) && !settled_by_read(&state.reads, market, d, now, bank) {
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
/// alone where it holds one, else its venue's forms and then its market's others
/// (its currency's market where its venue is none known).
pub fn yahoo_forms(l: &crate::contract::Listing) -> Vec<String> {
    match l.route(&RefScheme::Yahoo) {
        Some(r) => vec![r.to_string()],
        None => venue::yahoo_forms_of(&l.symbol, l.venue_mic.as_deref(), l.currency),
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

pub(crate) fn store(ctx: &Ctx, id: InstrumentId, closes: &[(Date, Dec)], currency: Currency, source: &SourceName, host: &str) -> Result<()> {
    let disagreements = ctx.cache.store_closes(id, closes, currency, source, ctx.now)?;
    for d in disagreements {
        // a later value for a closed day: the first stands, and this is a meaning outcome
        let noted: Noted<()> = Noted { outcome: Outcome::Meaning(format!("{} {}: {} stands, {} came later", id, d.day, d.stands, d.later)), shape_change: None };
        ctx.record(source, host, DataKind::DailyClose, Some(id), &noted)?;
    }
    Ok(())
}

/// Where a close is asked: a coin's Exchange market, or Yahoo's chart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Via {
    Exchange,
    Yahoo,
}

impl Via {
    fn source(self) -> SourceName {
        match self {
            Via::Exchange => coinbase::exchange_source(),
            Via::Yahoo => yahoo::source(),
        }
    }

    fn host(self) -> &'static str {
        match self {
            Via::Exchange => coinbase::EXCHANGE_HOST,
            Via::Yahoo => yahoo::HOST,
        }
    }
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
        let first = match market {
            Market::Crypto => Via::Exchange,
            _ => Via::Yahoo,
        };
        let state = CloseState { days: ctx.cache.close_days(id)?, reads: ctx.cache.reads(&subject, DataKind::DailyClose)? };
        let Some((from, to)) = due_close(need, market, &state, ctx.now, ctx.bank, rest_of(ctx, first.host())) else { continue };
        // the chain in order: a coin's Exchange pairs, then the same pairs on
        // Yahoo; a listing's Yahoo forms; the winner first
        let mut chain: Vec<(Via, String)> = match market {
            Market::Crypto => {
                let pairs = coin_pairs(&need.listing);
                pairs.iter().map(|p| (Via::Exchange, p.clone())).chain(pairs.iter().map(|p| (Via::Yahoo, p.clone()))).collect()
            }
            _ => yahoo_forms(&need.listing).into_iter().map(|f| (Via::Yahoo, f)).collect(),
        };
        if chain.is_empty() {
            let noted: Noted<()> = Noted { outcome: Outcome::NotCarried(format!("{} names no venue Yahoo carries", need.listing.symbol)), shape_change: None };
            ctx.record(&first.source(), first.host(), DataKind::DailyClose, Some(id), &noted)?;
            keep_read(ctx, &subject, DataKind::DailyClose, &first.source(), (from, to), OutcomeKind::NotCarried, None)?;
            continue;
        }
        if let Some((by, won)) = ctx.cache.winner(id, DataKind::DailyClose)? {
            chain.sort_by_key(|(via, f)| !(via.source() == by && *f == won));
        }
        let mut result = OutcomeKind::NotCarried;
        let mut held_last = None;
        let mut last_source = first.source();
        for (via, form) in chain {
            let (source, host) = (via.source(), via.host());
            // a pair's closes are in its quote currency, a listing's in its own
            let quoted = match market {
                Market::Crypto => form.rsplit('-').next().and_then(|c| Currency::parse(c).ok()).unwrap_or(need.listing.currency),
                _ => need.listing.currency,
            };
            let (kind, answered) = match via {
                Via::Exchange => {
                    let noted = coinbase::ask_candles(ctx.net, &form, from, to, ctx.now);
                    ctx.record_detail(&source, host, DataKind::DailyClose, Some(id), &noted, &form)?;
                    (noted.outcome.kind(), match noted.outcome {
                        Outcome::Answered(days) => Some((days, quoted)),
                        _ => None,
                    })
                }
                Via::Yahoo => {
                    let mut noted = yahoo::ask_span(ctx.net, &form, from, to, ctx.now);
                    crate::quotes::same_currency_as(&mut noted.outcome, |c| c.currency, &form, quoted);
                    ctx.record_detail(&source, host, DataKind::DailyClose, Some(id), &noted, &form)?;
                    (noted.outcome.kind(), match noted.outcome {
                        Outcome::Answered(c) => Some((c.closes.into_iter().filter(|(d, _)| *d >= from && *d <= to).collect::<Vec<_>>(), c.currency)),
                        _ => None,
                    })
                }
            };
            result = kind;
            last_source = source.clone();
            if let Some((closes, currency)) = answered {
                held_last = closes.last().map(|c| c.0);
                store(ctx, id, &closes, currency, &source, host)?;
                ctx.cache.won(id, DataKind::DailyClose, &source, &form, ctx.now)?;
            }
            if kind != OutcomeKind::NotCarried {
                break;
            }
        }
        keep_read(ctx, &subject, DataKind::DailyClose, &last_source, (from, to), result, held_last)?;
    }
    Ok(())
}

/// Read the benchmarks' trackers due, from `from` (the person's oldest day):
/// each tracker's closes, dividends and splits from Yahoo. A tracker's days are
/// due as an instrument's closes are: every settled session day from the oldest
/// needed that is neither stored nor covered by a read made after it settled, so
/// the span reaches back when the oldest day moves earlier. A dividend or split
/// is stored with the span that holds its day.
pub fn read_benchmarks(ctx: &Ctx, from: Date) -> Result<()> {
    for b in Benchmark::ALL {
        read_benchmark(ctx, b, from, ctx.today())?;
    }
    Ok(())
}

/// What the cache holds of a benchmark's tracker.
pub fn benchmark_state(ctx: &Ctx, b: Benchmark) -> Result<CloseState> {
    Ok(CloseState { days: ctx.cache.benchmark_days(b)?, reads: ctx.cache.reads(b.key(), DataKind::Benchmark)? })
}

/// Whether every settled session day of `from`..=`to` is stored or covered by
/// a read made after it settled: the days held are then all the market's
/// sessions of the span (a holiday is a day a read answered with nothing).
pub fn covered(market: Market, from: Date, to: Date, state: &CloseState, now: Timestamp, bank: &TimeZone) -> bool {
    let Some(latest) = latest_settled(market, to, now, bank) else { return false };
    if latest < to && can_trade(market, to) {
        return false;
    }
    let mut d = from;
    while d <= latest {
        if can_trade(market, d) && !state.days.contains(&d) && !settled_by_read(&state.reads, market, d, now, bank) {
            return false;
        }
        let Ok(next) = d.tomorrow() else { return false };
        d = next;
    }
    true
}

/// Read one benchmark's tracker for the days of `from`..=`to` not held yet.
pub fn read_benchmark(ctx: &Ctx, b: Benchmark, from: Date, to: Date) -> Result<()> {
    let source = yahoo::source();
    let (symbol, currency) = b.tracker();
    let state = benchmark_state(ctx, b)?;
    let Some((first, last)) = due_span(b.market(), from, to, &state, ctx.now, ctx.bank, rest_of(ctx, yahoo::HOST)) else { return Ok(()) };
    let mut noted = yahoo::ask_span(ctx.net, symbol, first, last, ctx.now);
    if let Outcome::Answered(c) = &noted.outcome {
        if c.currency != currency {
            noted.outcome = Outcome::Meaning(format!("{symbol} is answered in {}, it trades in {currency}", c.currency));
        }
    }
    ctx.record_detail(&source, yahoo::HOST, DataKind::Benchmark, None, &noted, symbol)?;
    let kind = noted.outcome.kind();
    let mut held_last = None;
    if let Outcome::Answered(c) = noted.outcome {
        let inside = |d: &Date| *d >= first && *d <= last;
        let closes: Vec<(Date, Dec)> = c.closes.into_iter().filter(|(d, _)| inside(d)).collect();
        held_last = closes.last().map(|c| c.0);
        let mut events: Vec<(Date, TrackerEvent)> = c.dividends.into_iter().filter(|(d, _)| inside(d)).map(|(d, a)| (d, TrackerEvent::Dividend(a))).collect();
        events.extend(c.splits.into_iter().filter(|s| inside(&s.day)).map(|s| (s.day, TrackerEvent::Split { numerator: s.numerator, denominator: s.denominator })));
        let mut later: Vec<String> = ctx.cache.store_benchmark_closes(b, &closes, &source, ctx.now)?.into_iter().map(|d| format!("{symbol} {}: close {} stands, {} came later", d.day, d.stands, d.later)).collect();
        later.extend(ctx.cache.store_benchmark_events(b, &events, &source, ctx.now)?.into_iter().map(|d| format!("{symbol} {}: {} {} stands, {} came later", d.day, d.kind, d.stands, d.later)));
        for why in later {
            // a later value for a closed day: the first stands, and this is a meaning outcome
            ctx.record(&source, yahoo::HOST, DataKind::Benchmark, None, &Noted::<()> { outcome: Outcome::Meaning(why), shape_change: None })?;
        }
    }
    keep_read(ctx, b.key(), DataKind::Benchmark, &source, (first, last), kind, held_last)?;
    Ok(())
}
