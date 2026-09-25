//! When each read of the figure path runs (`docs/plans/stage-3c-switch.md`, §3):
//! one task asks the engine what the figures use, runs every reader that is due
//! for it, applies what each stored, and sleeps until the earliest instant any
//! read can next be due, or until something changes that can make one due (a
//! zone stated, the record changed). Each reader decides for itself, from what
//! the book and the cache hold, whether it is due (`bagholder_sources`); what
//! is decided here is only when to look again, and every such instant is a
//! known deadline:
//!
//! - the day turning in the person's zone, and the Bank of Canada's 16:30
//!   Eastern on a weekday, when a day's rate is published;
//! - each market's close settling, for the closes and benchmarks needed;
//! - each payer's next distribution window (`payers::run::next_due`);
//! - a failed read's source rest ending;
//! - a minute, for the quotes of what is held, only while a page is open.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::InstrumentId;
use bagholder_engine::needs::FactNeeds;
use bagholder_sources::contract::{Benchmark, Market};
use bagholder_sources::needs::Needs;
use bagholder_sources::read::Ctx;
use bagholder_sources::{market, payers, quotes, rates};

use crate::app::{log, App};
use crate::figures::{bank_zone, Figures};

/// How often what is held is quoted while a page shows it.
pub const QUOTES_EVERY: Duration = Duration::from_secs(60);
/// How many times the needs are worked out again after a pass of reads.
const PASSES: usize = 4;

/// Run the scheduler until the app stops.
pub fn run(app: Arc<App>) {
    let Some(f) = app.figures.get() else { return };
    while !app.stopping() {
        let now = Timestamp::now();
        let before = f.version();
        let next = match pass(&app, f, now) {
            Ok(next) => next,
            Err(e) => {
                // shown until a pass succeeds; looked at again on the next change
                app.state.lock().unwrap().error = format!("The figures could not be brought up to date: {e}");
                log(&format!("bagholder: the figures could not be brought up to date: {e}"));
                None
            }
        };
        if f.version() != before {
            // a figure moved: every page's stream looks
            app.events.signal();
        }
        // until the next deadline, or until something that can make a read due
        match next {
            Some(n) => {
                let wait = Duration::from_secs(n.duration_since(Timestamp::now()).as_secs().max(1) as u64);
                app.events.park_until_or(&app, wait, || f.woken());
            }
            None => {
                app.events.park_until(&app, || f.woken());
            }
        }
    }
}

/// One pass: the clock, every due read, what each stored applied. The next
/// instant a read can be due, if any.
pub fn pass(app: &App, f: &Figures, now: Timestamp) -> Result<Option<Timestamp>, String> {
    let book = f.book()?;
    let Some(zone) = book.zone().map_err(|e| e.to_string())?.map(|z| z.zone) else {
        // no page has stated a zone: no figure is built, nothing is read for one
        return Ok(None);
    };
    let bank = bank_zone()?;
    let clock = f.read(|e| e.inputs().clock.clone()).ok_or("the engine is not built")?;
    if moved_on(&clock.now, now, &zone, &bank) {
        f.clock_moved(now)?;
    }
    let cache = f.cache()?;
    let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now, bank: &bank };
    let today = now.to_zoned(zone.clone()).date();
    let mut last: Option<FactNeeds> = None;
    let mut needs = Needs::default();
    for _ in 0..PASSES {
        let n = f.read(|e| e.needs()).ok_or("the engine is not built")?;
        if last.as_ref() == Some(&n) {
            break;
        }
        needs = crate::read_sources::needs_of(&book, &n, today)?;
        read_all(f, &ctx, &needs).map_err(|e| e.to_string())?;
        last = Some(n);
    }
    // what is held is quoted while a page shows it
    let open = app.events.watchers() > 0;
    if open {
        quotes::read_quotes(&ctx, &needs.held).map_err(|e| e.to_string())?;
        for l in &needs.held {
            f.price_changed(l.id)?;
        }
    }
    next_due(&ctx, &needs, &zone, open, now).map(Some)
}

/// Read the payers held under `symbol` now, each change applied: the ones read.
#[cfg_attr(test, allow(dead_code))]
pub fn read_payer_now(app: &App, f: &Figures, symbol: &str, now: Timestamp) -> Result<Vec<InstrumentId>, String> {
    let book = f.book()?;
    let Some(zone) = book.zone().map_err(|e| e.to_string())?.map(|z| z.zone) else { return Ok(vec![]) };
    let bank = bank_zone()?;
    let cache = f.cache()?;
    let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now, bank: &bank };
    let n = f.read(|e| e.needs()).ok_or("the engine is not built")?;
    let needs = crate::read_sources::needs_of(&book, &n, now.to_zoned(zone).date())?;
    let mut read = vec![];
    for p in needs.payers.iter().filter(|p| p.listing.symbol.eq_ignore_ascii_case(symbol.trim())) {
        payers::run::read_at_once(&ctx, p).map_err(|e| e.to_string())?;
        f.payer_changed(p.listing.id)?;
        read.push(p.listing.id);
    }
    Ok(read)
}

/// Every reader for the needs, each change applied.
fn read_all(f: &Figures, ctx: &Ctx, needs: &Needs) -> Result<(), String> {
    let e = |e: bagholder_sources::read::RunError| e.to_string();
    rates::read(ctx, &needs.rates).map_err(e)?;
    f.rates_changed()?;
    market::read_closes(ctx, &needs.closes).map_err(e)?;
    let closed: BTreeSet<InstrumentId> = needs.closes.iter().map(|c| c.listing.id).collect();
    for i in closed {
        f.price_changed(i)?;
    }
    if let Some(from) = needs.benchmarks_from {
        market::read_benchmarks(ctx, from).map_err(e)?;
        for b in Benchmark::ALL {
            f.benchmark_changed(b.key())?;
        }
    }
    payers::run::read(ctx, &needs.payers).map_err(e)?;
    for p in &needs.payers {
        f.payer_changed(p.listing.id)?;
    }
    Ok(())
}

/// Whether the clock the engine holds is behind `now` in a way a figure can
/// see: the day turned in the person's zone, or the Bank's publication instant
/// passed.
fn moved_on(held: &Timestamp, now: Timestamp, zone: &TimeZone, bank: &TimeZone) -> bool {
    held.to_zoned(zone.clone()).date() != now.to_zoned(zone.clone()).date() || next_publication(*held, bank).is_some_and(|p| p <= now)
}

/// The next instant after `after` the Bank publishes a day's rate: 16:30 Eastern
/// on a weekday (a holiday is found out by the read, which then asks nothing).
fn next_publication(after: Timestamp, bank: &TimeZone) -> Option<Timestamp> {
    let mut d = after.to_zoned(bank.clone()).date();
    for _ in 0..8 {
        if !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) {
            if let Some(p) = rates::published_at(d, bank).filter(|p| *p > after) {
                return Some(p);
            }
        }
        d = d.tomorrow().ok()?;
    }
    None
}

/// The start of the next day in `zone` after `now`.
fn next_midnight(now: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
    let d: Date = now.to_zoned(zone.clone()).date().tomorrow().ok()?;
    d.to_zoned(zone.clone()).ok().map(|z| z.timestamp())
}

/// The next instant after `now` a market's close settles.
fn next_settle(m: Market, now: Timestamp, bank: &TimeZone) -> Option<Timestamp> {
    let mut d = now.to_zoned(bank.clone()).date().yesterday().ok()?;
    for _ in 0..10 {
        if let Some(s) = market::settled_at(m, d, bank).filter(|s| *s > now) {
            return Some(s);
        }
        d = d.tomorrow().ok()?;
    }
    None
}

/// The earliest instant after `now` any read can next be due.
fn next_due(ctx: &Ctx, needs: &Needs, zone: &TimeZone, open: bool, now: Timestamp) -> Result<Timestamp, String> {
    let bank = ctx.bank;
    let mut at: Vec<Timestamp> = Vec::new();
    at.extend(next_midnight(now, zone));
    if !needs.rates.is_empty() {
        at.extend(next_publication(now, bank));
    }
    let mut markets: BTreeSet<Market> = needs.closes.iter().filter_map(|c| c.listing.market()).collect();
    if needs.benchmarks_from.is_some() {
        markets.extend(Benchmark::ALL.iter().map(|b| b.market()));
    }
    for m in markets {
        at.extend(next_settle(m, now, bank));
    }
    let declared = ctx.book.declared().map_err(|e| e.to_string())?;
    let frequencies = ctx.book.frequencies().map_err(|e| e.to_string())?;
    for p in &needs.payers {
        if payers::adapter_for(p).is_some() {
            at.push(payers::run::next_due(declared.get(&p.listing.id), frequencies.get(&p.listing.id), now, bank));
        }
    }
    if open && !needs.held.is_empty() {
        at.push(now + SignedDuration::try_from(QUOTES_EVERY).map_err(|e| e.to_string())?);
    }
    // a read that failed is asked again when its source's rest ends
    at.extend(rest_ends(ctx, now)?);
    Ok(at.into_iter().filter(|t| *t > now).min().unwrap_or(now + SignedDuration::from_hours(24)))
}

/// When each source whose last outcome failed may be asked again.
fn rest_ends(ctx: &Ctx, now: Timestamp) -> Result<Vec<Timestamp>, String> {
    let mut last: BTreeMap<String, (Timestamp, String)> = BTreeMap::new();
    for source in ctx.cache.sources().map_err(|e| e.to_string())? {
        for o in ctx.cache.outcomes(&source).map_err(|e| e.to_string())? {
            if o.outcome.is_failure() || o.outcome == bagholder_sources::outcome::OutcomeKind::Refused {
                let e = last.entry(o.host.clone()).or_insert((o.at, o.host.clone()));
                if o.at > e.0 {
                    e.0 = o.at;
                }
            }
        }
    }
    Ok(last
        .into_values()
        .filter_map(|(at, host)| {
            let rest = SignedDuration::try_from(ctx.net.limiter().pace(&host).rest).ok()?;
            Some(at + rest).filter(|end| *end > now)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn the_bank_publishes_at_its_1630_on_the_next_weekday() {
        let bank = bank_zone().unwrap();
        // a Friday afternoon before 16:30 Eastern: that day
        assert_eq!(next_publication(t("2026-09-25T19:00:00Z"), &bank), Some(t("2026-09-25T20:30:00Z")));
        // after it: Monday's
        assert_eq!(next_publication(t("2026-09-25T21:00:00Z"), &bank), Some(t("2026-09-28T20:30:00Z")));
        // standard time: 16:30 Eastern is 21:30 UTC
        assert_eq!(next_publication(t("2026-12-01T12:00:00Z"), &bank), Some(t("2026-12-01T21:30:00Z")));
    }

    #[test]
    fn the_day_turns_at_midnight_in_the_persons_zone_whatever_it_is() {
        let db = bagholder_core::jiff::tz::TimeZoneDatabase::bundled();
        let now = t("2026-03-08T06:30:00Z");
        for name in db.available() {
            let z = db.get(name.as_str()).unwrap();
            let next = next_midnight(now, &z).unwrap();
            assert!(next > now);
            let day = |at: Timestamp| at.to_zoned(z.clone()).date();
            assert_eq!(day(next), day(now).tomorrow().unwrap(), "{}", name.as_str());
            assert_ne!(day(next - SignedDuration::from_secs(1)), day(next), "{}: the instant before is the day before", name.as_str());
        }
    }

    #[test]
    fn the_clock_moves_on_when_the_day_turns_or_the_bank_publishes() {
        let bank = bank_zone().unwrap();
        let z = TimeZone::get("Asia/Tokyo").unwrap();
        // 10:00 to 10:05 UTC, the same day in Tokyo, no publication between
        assert!(!moved_on(&t("2026-09-24T10:00:00Z"), t("2026-09-24T10:05:00Z"), &z, &bank));
        // across 15:00 UTC, midnight in Tokyo
        assert!(moved_on(&t("2026-09-24T14:59:00Z"), t("2026-09-24T15:01:00Z"), &z, &bank));
        // across 16:30 Eastern
        assert!(moved_on(&t("2026-09-24T20:29:00Z"), t("2026-09-24T20:31:00Z"), &z, &bank));
    }

    #[test]
    fn a_markets_close_settles_next_at_its_own_hour() {
        let bank = bank_zone().unwrap();
        assert_eq!(next_settle(Market::Canada, t("2026-09-24T19:00:00Z"), &bank), Some(t("2026-09-24T20:30:00Z")));
        assert_eq!(next_settle(Market::Crypto, t("2026-09-24T19:00:00Z"), &bank), Some(t("2026-09-25T00:00:00Z")));
    }
}
