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
//! - a minute, for the quotes of what is held, only while a page shows a
//!   holding's price (`PRICED`), and of what the person follows (the watchlist,
//!   the tiles) only while a page shows the Markets tab (`FOLLOWED`).

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
/// The subscriptions that show a holding's price: while a page shows one, what is
/// held is quoted (`docs/architecture.md` §13, sources are read on demand).
pub const PRICED: [&str; 6] = ["positions", "exposure", "markets", "heatmap", "cashflow", "trade"];
/// The subscriptions that show a followed instrument's price: while a page shows
/// one, the watched listings and the tiles are quoted.
pub const FOLLOWED: [&str; 2] = ["markets", "heatmap"];
/// How many times the needs are worked out again after a pass of reads.
const PASSES: usize = 4;

/// Run the scheduler until the app stops.
pub fn run(app: Arc<App>) {
    let Some(f) = app.figures.get() else { return };
    while !app.stopping() {
        let now = Timestamp::now();
        let before = f.version();
        let next = settle(&app, pass(&app, f, now));
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

/// What a pass came to, in the header: its failure is shown until a pass
/// succeeds, and the pass that does clears it (and only it: Wealthsimple's
/// errors and the sources' failures are each cleared by their own success). A
/// failed pass is looked at again on the next change.
pub fn settle(app: &App, passed: Result<Option<Timestamp>, String>) -> Option<Timestamp> {
    match passed {
        Ok(next) => {
            // written (and so said to the page) only when it changes
            let mut st = app.state.lock().unwrap();
            if !st.figures_error.is_empty() {
                st.figures_error.clear();
            }
            next
        }
        Err(e) => {
            let said = format!("The figures could not be brought up to date: {e}");
            {
                let mut st = app.state.lock().unwrap();
                if st.figures_error != said {
                    st.figures_error = said;
                }
            }
            log(&format!("bagholder: the figures could not be brought up to date: {e}"));
            None
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
    let quoting = demand(app, &book, &needs.held)?;
    quotes::read_quotes(&ctx, &quoting).map_err(|e| e.to_string())?;
    for l in &quoting {
        f.price_changed(l.id)?;
    }
    // the option contracts held, while a page shows a holding's price: their chains
    // are asked when Cboe's copy has moved on (`options::chain_due`), a contract
    // shown for the first time at once
    if app.events.showing(&PRICED) {
        bagholder_sources::options::read(&ctx, &needs.contracts).map_err(|e| e.to_string())?;
        for c in &needs.contracts {
            f.price_changed(c.id)?;
        }
    }
    next_due(&ctx, &needs, &zone, !quoting.is_empty(), now).map(Some)
}

/// What is quoted now: what is held while a page shows a holding's price, what
/// is followed while a page shows the Markets tab.
fn demand(app: &App, book: &bagholder_book::Book, held: &[bagholder_sources::contract::Listing]) -> Result<Vec<bagholder_sources::contract::Listing>, String> {
    let mut quoting: Vec<bagholder_sources::contract::Listing> = Vec::new();
    if app.events.showing(&PRICED) {
        quoting.extend(held.iter().cloned());
    }
    if app.events.showing(&FOLLOWED) {
        for l in crate::following::listings(book)? {
            if !quoting.iter().any(|q| q.id == l.id) {
                quoting.push(l);
            }
        }
    }
    Ok(quoting)
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
fn next_due(ctx: &Ctx, needs: &Needs, zone: &TimeZone, quoting: bool, now: Timestamp) -> Result<Timestamp, String> {
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
    if quoting {
        at.push(now + SignedDuration::try_from(QUOTES_EVERY).map_err(|e| e.to_string())?);
    }
    // a read that failed is asked again when its source's rest ends
    at.extend(rest_ends(ctx, now)?);
    Ok(at.into_iter().filter(|t| *t > now).min().unwrap_or(now + SignedDuration::from_hours(24)))
}

/// When each source whose last outcome failed may be asked again.
fn rest_ends(ctx: &Ctx, now: Timestamp) -> Result<Vec<Timestamp>, String> {
    use bagholder_sources::outcome::OutcomeKind;
    // each subject of each source (its host, what was read, for which instrument):
    // its newest failure and how many failed in a row, which grows its rest
    let mut streaks: BTreeMap<(String, String, String), (Timestamp, u32, bool)> = BTreeMap::new();
    for source in ctx.cache.sources().map_err(|e| e.to_string())? {
        for o in ctx.cache.outcomes(&source).map_err(|e| e.to_string())? {
            if o.outcome == OutcomeKind::NotCarried {
                continue;
            }
            let key = (o.host.clone(), o.kind.as_str().to_string(), o.instrument.map(|i| i.to_string()).unwrap_or_default());
            let failed = o.outcome.is_failure() || o.outcome == OutcomeKind::Refused;
            let e = streaks.entry(key).or_insert((o.at, 0, !failed));
            // newest first: count failures until the first answer
            if !e.2 {
                if failed {
                    e.1 += 1;
                } else {
                    e.2 = true;
                }
            }
        }
    }
    Ok(streaks
        .into_iter()
        .filter(|(_, (_, n, _))| *n > 0)
        .filter_map(|((host, _, _), (at, n, _))| {
            let rest = SignedDuration::try_from(bagholder_sources::market::grown_rest(ctx.net.limiter().pace(&host).rest, n)).ok()?;
            Some(at + rest).filter(|end| *end > now)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The watched listings and the tiles are quoted only while a page shows the
    /// Markets tab; the holdings while a page shows a price; nothing with no page.
    #[test]
    fn what_is_followed_is_quoted_only_while_the_markets_tab_is_shown() {
        use crate::events::{Feed, Want};
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now = t("2025-11-19T21:00:00Z");
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let f = app.figures.get().unwrap();
        crate::following::ensure(&app).unwrap();
        let book = f.book().unwrap();
        let shop = crate::following::Named { symbol: "SHOP".into(), exchange: "TSX".into(), currency: "CAD".into(), ..Default::default() };
        book.watch(&crate::following::draft(&book, &shop).unwrap(), now).unwrap();
        let n = f.read(|e| e.needs()).unwrap();
        let held = crate::read_sources::needs_of(&book, &n, now.to_zoned(TimeZone::get("America/Toronto").unwrap()).date()).unwrap().held;
        assert!(!held.is_empty());
        let followed = crate::following::listings(&book).unwrap();
        assert_eq!(followed.len(), 7, "the six tiles and the watched listing");
        let ids = |ls: &[bagholder_sources::contract::Listing]| -> BTreeSet<InstrumentId> { ls.iter().map(|l| l.id).collect() };
        let show = |keys: &[&str]| {
            let feed = Feed::open(app.clone());
            let docs = keys.iter().map(|k| (k.to_string(), Want { params: serde_json::json!({}), have: None })).collect();
            assert!(app.events.watch(&app, feed.id(), docs));
            feed
        };
        assert!(demand(&app, &book, &held).unwrap().is_empty(), "no page: nothing quoted");
        {
            let _positions = show(&["positions"]);
            assert_eq!(ids(&demand(&app, &book, &held).unwrap()), ids(&held), "the Positions tab: the holdings alone");
        }
        {
            let _markets = show(&["markets"]);
            let want: BTreeSet<InstrumentId> = ids(&held).union(&ids(&followed)).copied().collect();
            assert_eq!(ids(&demand(&app, &book, &held).unwrap()), want, "the Markets tab: the holdings and what is followed");
        }
        assert!(demand(&app, &book, &held).unwrap().is_empty(), "the page closed: nothing quoted");
    }

    /// The option contracts held are priced from their chains by the app's own
    /// pass while a page shows a holding's price, and not asked for with no page.
    #[test]
    fn held_contracts_are_read_while_a_page_shows_a_holding_s_price() {
        use crate::events::{Feed, Want};
        crate::tests_common::home(); // offline: the chain is asked for and nothing leaves the machine
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now = t("2025-11-19T21:00:00Z");
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let f = app.figures.get().unwrap();
        // a contract entered by hand, held today
        let req = crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: "ZZQQ 15JAN27 12.00 CALL".into(), currency: "USD".into(), day: "2025-11-19".into(), side: "BUY".into(), quantity: "2".into(), price: "0.40".into(), fee: String::new() };
        crate::entries::enter(f, &req, now).unwrap();
        let book = f.book().unwrap();
        let n = f.read(|e| e.needs()).unwrap();
        let contracts = crate::read_sources::needs_of(&book, &n, now.to_zoned(TimeZone::get("America/Toronto").unwrap()).date()).unwrap().contracts;
        assert!(!contracts.is_empty(), "the test book holds an option contract");
        let asked = || -> usize {
            let cache = f.cache().unwrap();
            contracts.iter().map(|c| cache.reads(&format!("chain:{}", c.underlying), bagholder_sources::contract::DataKind::Quote).unwrap().len()).sum()
        };
        pass(&app, f, now).unwrap();
        assert_eq!(asked(), 0, "no page: no chain asked for");
        let feed = Feed::open(app.clone());
        assert!(app.events.watch(&app, feed.id(), [("positions".to_string(), Want { params: serde_json::json!({}), have: None })].into_iter().collect()));
        pass(&app, f, now).unwrap();
        assert!(asked() > 0, "a page showing the holdings: the chain of each held contract is asked for");
    }

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
