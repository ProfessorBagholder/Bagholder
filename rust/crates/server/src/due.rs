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
//! - a minute, for the quotes of what is held and of what the person follows (the
//!   watchlist, the tiles), while the app runs, page or none (the owner,
//!   2026-11-04), so a tab opened shows prices already read; with no page open, a
//!   quiet market's quote waits for its next open, or `QUIET_RECHECK` (`quiet`).

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

/// How often what is held and followed is quoted while the app runs.
pub const QUOTES_EVERY: Duration = Duration::from_secs(60);

/// How much older than its read a quote whose source states a trade time may be
/// before its market is taken as not trading, and how long after that read it is
/// asked again with no page open: a quarter of an hour, so the first read after such
/// a market opens is at most that late while nobody looks, and at once when a page
/// opens. A listing whose venue has a calendar here goes by the calendar instead.
pub const QUIET_RECHECK: Duration = Duration::from_secs(15 * 60);

/// When a listing whose market is not trading is next owed a quote while no page is
/// open (`docs/plans/prices-fresh-at-open.md`); none while it is owed now. "Quiet"
/// is a market not trading, never a source refused (that is its rest, `rest_ends`).
/// - On a venue whose calendar the app holds (`sessions::venue_of`): while its
///   session is known closed and the quote kept was received after that session's
///   close settled (`market::settled_at`), until the next open. A date past the
///   published calendar is not known, so the listing is owed as any other.
/// - Otherwise, where the source states the trade's time: while that time was more
///   than `QUIET_RECHECK` before the read that kept it, until `QUIET_RECHECK` after
///   that read. A source that states only when it served the quote (TMX's) never
///   shows a quiet market this way.
pub fn quiet(l: &bagholder_sources::contract::Listing, kept: Option<&bagholder_sources::cache::StoredQuote>, now: Timestamp, bank: &TimeZone) -> Option<Timestamp> {
    use bagholder_core::sessions;
    // a listing never quoted is owed at once
    let q = kept?;
    match l.venue_mic.as_deref().and_then(|mic| sessions::venue_of(mic, false)) {
        Some(v) => {
            if sessions::in_session(v, now)? {
                return None;
            }
            let settled = market::settled_at(l.market()?, sessions::last_closed_day(v, now)?, bank)?;
            if q.received_at < settled {
                // the close not read since it settled: owed, for the session's final price
                return None;
            }
            sessions::next_open(v, now)
        }
        None => {
            let recheck = SignedDuration::try_from(QUIET_RECHECK).ok()?;
            let again = q.received_at + recheck;
            (q.quoted_at + recheck < q.received_at && again > now).then_some(again)
        }
    }
}
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
pub fn pass(app: &Arc<App>, f: &Figures, now: Timestamp) -> Result<Option<Timestamp>, String> {
    // figures a start could not build are built here, or the reason is the pass's
    f.ensure_built(now)?;
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
    let (quoting, quiet_until) = demand(&ctx, &needs.held, app.events.watchers() > 0)?;
    quote_by_source(app, f, &ctx, &quoting, |l| quotes::read_quotes(&ctx, std::slice::from_ref(l)).map_err(|e| e.to_string()))?;
    // the option contracts held: their chains are asked when Cboe's copy has moved on
    // (`options::chain_due`), a contract shown for the first time at once
    bagholder_sources::options::read(&ctx, &needs.contracts).map_err(|e| e.to_string())?;
    for c in &needs.contracts {
        f.price_changed(c.id)?;
    }
    next_due(&ctx, &needs, &zone, !quoting.is_empty(), quiet_until, now).map(Some)
}

/// The listings owed a quote, each from its one source (`ws_quotes`): while signed
/// in, Wealthsimple the shares and funds it carries, twenty to a request, and the
/// ex-dividend dates TMX's quote states for the Canadian ones once a day; the
/// public feeds (`read_public`) the rest, and every one while signed out. A
/// listing Wealthsimple carries never reaches a public feed while signed in, its
/// read failed or not.
fn quote_by_source(app: &Arc<App>, f: &Figures, ctx: &Ctx, listings: &[bagholder_sources::contract::Listing], read_public: impl FnMut(&bagholder_sources::contract::Listing) -> Result<(), String>) -> Result<(), String> {
    let sess = crate::ws_quotes::session(app);
    let (broker, public) = crate::ws_quotes::split(listings, sess.is_some());
    if let Some(sess) = sess.as_ref().filter(|_| !broker.is_empty()) {
        let kept = crate::ws_quotes::read(app, ctx, sess, &broker)?;
        let mut moved = false;
        for l in &kept {
            moved |= !f.price_changed(l.id)?.is_empty();
        }
        if moved {
            app.events.signal();
        }
        quotes::read_ex_dividends(ctx, &broker).map_err(|e| e.to_string())?;
    }
    quote_each(app, f, &public, read_public)
}

/// Each listing read in turn, and its price taken into the figures and said to the
/// pages as soon as it is kept, before the next is asked: a tile or a holding's row
/// moves as its quote lands, never when the last of the pass does.
fn quote_each(app: &App, f: &Figures, listings: &[bagholder_sources::contract::Listing], mut read: impl FnMut(&bagholder_sources::contract::Listing) -> Result<(), String>) -> Result<(), String> {
    for l in listings {
        read(l)?;
        if !f.price_changed(l.id)?.is_empty() {
            app.events.signal();
        }
    }
    Ok(())
}

/// What is quoted now: what is held and what is followed, while the app runs, page
/// or none, less each quiet one (`quiet`), and the earliest instant one of those is
/// owed again. A listing on a venue whose calendar the app holds is quiet from its
/// settled close to its next open, page or none: nothing it shows changes, and the
/// pass wakes at the open, so someone watching still sees it at once. One with no
/// calendar (a tile's index, future, rate or pair) is quiet by its stated time only
/// while no page is open: with one, it is asked every pass, so the S&P 500 tile
/// moves at 9:30 for someone watching, never fifteen minutes later (#670). A failing
/// quote is owed like any other: a refused source is left alone through its rest by
/// the limiter, and `next_due` wakes at the rest's end.
fn demand(ctx: &Ctx, held: &[bagholder_sources::contract::Listing], page_open: bool) -> Result<(Vec<bagholder_sources::contract::Listing>, Option<Timestamp>), String> {
    let mut all: Vec<bagholder_sources::contract::Listing> = Vec::new();
    for l in held.iter().cloned().chain(crate::following::listings(ctx.book)?) {
        if !all.iter().any(|q| q.id == l.id) {
            all.push(l);
        }
    }
    let (mut owed, mut quiet_until) = (Vec::new(), None::<Timestamp>);
    for l in all {
        let kept = ctx.cache.quotes_of(l.id).map_err(|e| e.to_string())?.into_iter().max_by_key(|q| q.received_at);
        let calendar = l.venue_mic.as_deref().and_then(|mic| bagholder_core::sessions::venue_of(mic, false)).is_some();
        match quiet(&l, kept.as_ref(), ctx.now, ctx.bank).filter(|_| calendar || !page_open) {
            Some(at) => quiet_until = Some(quiet_until.map_or(at, |u| u.min(at))),
            None => owed.push(l),
        }
    }
    Ok((owed, quiet_until))
}

/// Read the payers held under `symbol` now, each change applied: the ones read.
#[cfg_attr(test, allow(dead_code))]
pub fn read_payer_now(app: &App, f: &Figures, key: &bagholder_core::ListingKey, now: Timestamp) -> Result<Vec<InstrumentId>, String> {
    let book = f.book()?;
    let Some(zone) = book.zone().map_err(|e| e.to_string())?.map(|z| z.zone) else { return Ok(vec![]) };
    let bank = bank_zone()?;
    let cache = f.cache()?;
    let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now, bank: &bank };
    let n = f.read(|e| e.needs()).ok_or("the engine is not built")?;
    let needs = crate::read_sources::needs_of(&book, &n, now.to_zoned(zone).date())?;
    let mut read = vec![];
    // the payers on that listing alone, never another listing's with the same ticker (brief 34, U15)
    let on = |p: &&bagholder_sources::needs::PayerNeed| p.listing.venue_mic.as_deref().and_then(|mic| bagholder_core::ListingKey::on(&p.listing.symbol, mic).ok()).is_some_and(|k| k == *key);
    for p in needs.payers.iter().filter(on) {
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
fn next_due(ctx: &Ctx, needs: &Needs, zone: &TimeZone, quoting: bool, quiet_until: Option<Timestamp>, now: Timestamp) -> Result<Timestamp, String> {
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
        if let Some(adapter) = payers::adapter_for(p) {
            let (read, frequency) = (declared.get(&p.listing.id), frequencies.get(&p.listing.id));
            at.push(payers::run::next_due(read, frequency, now, bank));
            // the market's record beside a company's, read for the next distribution
            if let Some(read) = read.filter(|r| r.source == adapter.source() && payers::run::beside_record_for(p, adapter.as_ref()).is_some()) {
                at.extend(payers::run::market_next_due(read, frequency, now, bank));
            }
        }
    }
    if quoting {
        at.push(now + SignedDuration::try_from(QUOTES_EVERY).map_err(|e| e.to_string())?);
    }
    // a quiet market's quote, owed again at its next open or its recheck
    at.extend(quiet_until);
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
            // a reader's judgement from before this run rests no longer
            let failed = (o.outcome.is_failure() || o.outcome == OutcomeKind::Refused) && o.outcome.stands(o.at, ctx.net.started());
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

    /// The holdings, the watched listings and the tiles are quoted while the app runs,
    /// page or none, so a tab opened shows prices already read (the owner,
    /// 2026-11-04); a quiet one waits (`quiet`), page or none (the owner, 2026-11-06:
    /// the providers' limits kept).
    #[test]
    fn what_is_held_and_followed_is_quoted_while_the_app_runs_page_or_none() {
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
        let cache = f.cache().unwrap();
        let bank = bank_zone().unwrap();
        let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now, bank: &bank };
        let want: BTreeSet<InstrumentId> = ids(&held).union(&ids(&followed)).copied().collect();
        let (owed, quiet_until) = demand(&ctx, &held, false).unwrap();
        assert_eq!(ids(&owed), want, "no page: the holdings and what is followed, none read yet");
        assert_eq!(quiet_until, None);
        // a watched listing read after its market closed and settled is quiet with no
        // page open, and owed at its next open; a page opening owes it at once
        let watched = followed.iter().find(|l| !held.iter().any(|h| h.id == l.id) && l.venue_mic.is_some()).unwrap().clone();
        // 22:00 Toronto time, the day's close settled at 16:30
        let evening = t("2025-11-20T03:00:00Z");
        let ctx = Ctx { now: evening, ..ctx };
        let read = |at: Timestamp| cache.store_quote(&bagholder_sources::cache::StoredQuote { instrument: watched.id, source: bagholder_core::SourceName::named("tmx"), price: bagholder_core::Money::new(bagholder_core::Dec::parse("10").unwrap(), bagholder_core::Currency::CAD), change: None, change_pct: None, quoted_at: at, allowance: std::time::Duration::ZERO, received_at: at }).unwrap();
        read(t("2025-11-19T21:00:00Z"));
        let (owed, _) = demand(&ctx, &held, false).unwrap();
        assert!(owed.iter().any(|l| l.id == watched.id), "read at the close, before it settled: owed, for the final price");
        read(evening);
        let (owed, quiet_until) = demand(&ctx, &held, false).unwrap();
        assert!(!owed.iter().any(|l| l.id == watched.id), "no page: the quiet listing waits");
        assert_eq!(quiet_until, Some(t("2025-11-20T14:30:00Z")), "until its next open, 9:30 Toronto time");
        // a page open, the same: nothing it shows changes before the open, and the pass
        // wakes at the open, so the page sees it at once
        let feed = Feed::open(app.clone());
        assert!(app.events.watch(&app, feed.id(), [("markets".to_string(), Want { params: serde_json::json!({}), have: None })].into_iter().collect()));
        let (owed, quiet_until) = demand(&ctx, &held, true).unwrap();
        assert!(!owed.iter().any(|l| l.id == watched.id), "a page open: the quiet listing still waits");
        assert_eq!(quiet_until, Some(t("2025-11-20T14:30:00Z")), "and is owed at its open");
        // a tile with no calendar, its last trade an hour old when read: quiet for the
        // recheck with no page open, and asked every pass with one (#670)
        let tile = followed.iter().find(|l| l.venue_mic.is_none()).unwrap().clone();
        cache.store_quote(&bagholder_sources::cache::StoredQuote { instrument: tile.id, source: bagholder_core::SourceName::named("yahoo"), price: bagholder_core::Money::new(bagholder_core::Dec::parse("10").unwrap(), tile.currency), change: None, change_pct: None, quoted_at: t("2025-11-20T02:00:00Z"), allowance: std::time::Duration::ZERO, received_at: evening }).unwrap();
        let (owed, _) = demand(&ctx, &held, false).unwrap();
        assert!(!owed.iter().any(|l| l.id == tile.id), "no page: the stale tile waits for the recheck");
        let (owed, _) = demand(&ctx, &held, true).unwrap();
        assert!(owed.iter().any(|l| l.id == tile.id), "a page open: the tile is asked every pass");
    }

    fn listing(kind: bagholder_core::instrument::InstrumentKind, currency: bagholder_core::Currency, mic: Option<&str>) -> bagholder_sources::contract::Listing {
        bagholder_sources::contract::Listing { id: InstrumentId::parse("00000000-0000-4000-8000-000000000001").unwrap(), kind, currency, symbol: "ZZQT".into(), venue_mic: mic.map(str::to_string), routes: Default::default() }
    }

    fn kept(quoted_at: Timestamp, received_at: Timestamp) -> bagholder_sources::cache::StoredQuote {
        bagholder_sources::cache::StoredQuote { instrument: InstrumentId::parse("00000000-0000-4000-8000-000000000001").unwrap(), source: bagholder_core::SourceName::named("stand-in"), price: bagholder_core::Money::new(bagholder_core::Dec::parse("10").unwrap(), bagholder_core::Currency::CAD), change: None, change_pct: None, quoted_at, allowance: std::time::Duration::ZERO, received_at }
    }

    /// A listing on a venue whose calendar the app holds is quiet while its session is
    /// known closed and its quote was read after the close settled, until the next
    /// open: on the TSX, the CSE and Cboe Canada at 02:00 Sunday Toronto time, owed at
    /// 09:30 Monday; a quote read before the close settled is owed, for the final price.
    #[test]
    fn a_listing_on_a_venue_with_a_calendar_is_quiet_from_the_settled_close_to_the_next_open() {
        use bagholder_core::instrument::InstrumentKind;
        let bank = bank_zone().unwrap();
        // Friday 2026-11-06, EST: the close settles at 16:30 (21:30Z); Monday's open is 14:30Z
        let after_settle = t("2026-11-06T22:00:00Z");
        for mic in ["XTSE", "XTSX", "XCNQ", "NEOE"] {
            let l = listing(InstrumentKind::Security, bagholder_core::Currency::CAD, Some(mic));
            // TMX's quote time is when it served it: the calendar decides, never that time
            let q = kept(after_settle, after_settle);
            assert_eq!(quiet(&l, Some(&q), t("2026-11-08T07:00:00Z"), &bank), Some(t("2026-11-09T14:30:00Z")), "{mic}: 02:00 Sunday, owed at 09:30 Monday");
            assert_eq!(quiet(&l, Some(&q), t("2026-11-09T14:30:00Z"), &bank), None, "{mic}: 09:30 Monday, owed");
            assert_eq!(quiet(&l, Some(&q), t("2026-11-09T15:00:00Z"), &bank), None, "{mic}: in the session, owed");
            let before = kept(t("2026-11-06T21:05:00Z"), t("2026-11-06T21:05:00Z"));
            assert_eq!(quiet(&l, Some(&before), t("2026-11-06T21:10:00Z"), &bank), None, "{mic}: read before the close settled, owed");
            assert_eq!(quiet(&l, None, t("2026-11-08T07:00:00Z"), &bank), None, "{mic}: never read, owed");
            // a holiday is the venue's own: Boxing Day observed, owed again Tuesday
            let boxing = kept(t("2026-12-24T22:00:00Z"), t("2026-12-24T22:00:00Z"));
            assert_eq!(quiet(&l, Some(&boxing), t("2026-12-28T15:00:00Z"), &bank), Some(t("2026-12-29T14:30:00Z")), "{mic}: Boxing Day observed");
        }
        // a US listing by the US calendar: closed on US Thanksgiving, open on Canada's
        let us = listing(InstrumentKind::Security, bagholder_core::Currency::USD, Some("XNAS"));
        let q = kept(t("2026-11-25T22:00:00Z"), t("2026-11-25T22:00:00Z"));
        assert_eq!(quiet(&us, Some(&q), t("2026-11-26T15:00:00Z"), &bank), Some(t("2026-11-27T14:30:00Z")));
        let q = kept(t("2026-10-09T22:00:00Z"), t("2026-10-09T22:00:00Z"));
        assert_eq!(quiet(&us, Some(&q), t("2026-10-12T15:00:00Z"), &bank), None, "Canadian Thanksgiving: the US trades");
        // past what an exchange has published nothing is known: owed every minute
        let tsx = listing(InstrumentKind::Security, bagholder_core::Currency::CAD, Some("XTSE"));
        let q = kept(t("2028-01-07T22:00:00Z"), t("2028-01-07T22:00:00Z"));
        assert_eq!(quiet(&tsx, Some(&q), t("2028-01-09T07:00:00Z"), &bank), None, "past the published calendar");
    }

    /// With no calendar here, a quote whose source states a trade more than
    /// `QUIET_RECHECK` before the read that kept it is quiet until `QUIET_RECHECK`
    /// after that read; one stated fresh is owed.
    #[test]
    fn a_quote_whose_stated_trade_is_old_when_read_is_quiet_for_the_recheck() {
        use bagholder_core::instrument::InstrumentKind;
        let bank = bank_zone().unwrap();
        let read = t("2026-11-08T07:00:00Z");
        let recheck = SignedDuration::try_from(QUIET_RECHECK).unwrap();
        for kind in [InstrumentKind::Index, InstrumentKind::Future, InstrumentKind::CurrencyPair, InstrumentKind::Rate, InstrumentKind::Crypto] {
            let l = listing(kind, bagholder_core::Currency::USD, None);
            let old = kept(read - SignedDuration::from_hours(40), read);
            assert_eq!(quiet(&l, Some(&old), read + SignedDuration::from_mins(1), &bank), Some(read + recheck), "{kind:?}");
            assert_eq!(quiet(&l, Some(&old), read + recheck, &bank), None, "{kind:?}: its recheck is due");
            let fresh = kept(read - SignedDuration::from_secs(20), read);
            assert_eq!(quiet(&l, Some(&fresh), read + SignedDuration::from_mins(1), &bank), None, "{kind:?}: trading");
        }
    }

    /// Each quote is taken into the figures before the next listing is read, so a
    /// tile moves as its quote lands, never when the pass ends.
    #[test]
    fn each_quote_reaches_the_figures_before_the_next_listing_is_read() {
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
        let tiles = crate::following::listings(&book).unwrap();
        assert!(tiles.len() >= 2);
        let cache = f.cache().unwrap();
        let price = |id: InstrumentId| f.read(|e| e.inputs().market.quotes.get(&id).map(|q| q.price.amount)).flatten();
        let mut read = Vec::new();
        quote_each(&app, f, &tiles[..2], |l| {
            if let Some(first) = read.first() {
                assert_eq!(price(*first), Some(bagholder_core::Dec::parse("101").unwrap()), "the first tile's price is in the figures before the second is asked");
            }
            let q = bagholder_sources::cache::StoredQuote { instrument: l.id, source: bagholder_core::SourceName::named("stand-in"), price: bagholder_core::Money::new(bagholder_core::Dec::parse("101").unwrap(), l.currency), change: None, change_pct: None, quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now };
            cache.store_quote(&q).map_err(|e| e.to_string())?;
            read.push(l.id);
            Ok(())
        })
        .unwrap();
        assert_eq!(price(tiles[1].id), Some(bagholder_core::Dec::parse("101").unwrap()));
    }

    /// With every listing quiet, the next pass is when the first is owed again, not a
    /// minute later.
    #[test]
    fn with_every_listing_quiet_the_next_pass_is_when_one_is_owed_again() {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now = t("2025-11-19T21:00:00Z");
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        let book = f.book().unwrap();
        let cache = f.cache().unwrap();
        let bank = bank_zone().unwrap();
        let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now, bank: &bank };
        let zone = TimeZone::get("America/Toronto").unwrap();
        let soon = now + SignedDuration::from_mins(5);
        assert_eq!(next_due(&ctx, &Needs::default(), &zone, false, Some(soon), now).unwrap(), soon, "the earliest quiet listing's");
        assert_eq!(next_due(&ctx, &Needs::default(), &zone, true, Some(soon), now).unwrap(), now + SignedDuration::from_mins(1), "a pass that asked anything: a minute");
    }

    /// The option contracts held are priced from their chains by the app's own pass
    /// while the app runs, page or none (the owner, 2026-11-04, replacing 2026-09-24's
    /// "only when a screen shows it"), `chain_due` deciding when Cboe's copy is asked.
    #[test]
    fn held_contracts_are_read_while_the_app_runs_page_or_none() {
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
        let req = crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: "ZZQQ 15JAN27 12.00 CALL".into(), exchange: "".into(), currency: "USD".into(), day: "2025-11-19".into(), side: "BUY".into(), quantity: "2".into(), price: "0.40".into(), fee: String::new() };
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
        assert!(asked() > 0, "no page: the chain of each held contract is asked for");
    }

    /// A past day's holdings a screen holds are priced by their closes on or before
    /// that day: the needs name each one's close of that day, the reader asks for
    /// them, and nothing is named once no screen holds it. The day is one the pull
    /// read the holdings of (yesterday's, brief 79 §2.4).
    #[test]
    fn a_past_days_holdings_ask_for_their_closes_while_a_screen_holds_them() {
        crate::tests_common::home(); // offline: the closes are asked for and nothing leaves the machine
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now = t("2025-11-19T21:00:00Z");
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let f = app.figures.get().unwrap();
        let book = f.book().unwrap();
        let today = now.to_zoned(TimeZone::get("America/Toronto").unwrap()).date();
        let day: bagholder_core::jiff::civil::Date = "2025-11-18".parse().unwrap();
        let spans = |f: &crate::figures::Figures| -> Vec<(InstrumentId, bagholder_core::jiff::civil::Date)> {
            let n = f.read(|e| e.needs()).unwrap();
            crate::read_sources::needs_of(&book, &n, today).unwrap().closes.iter().filter(|c| c.to == day).map(|c| (c.listing.id, c.from)).collect()
        };
        assert!(spans(f).is_empty(), "no screen holds a past day");
        let held = f.read(|e| e.past(day)).unwrap();
        assert!(!held.positions.is_empty(), "the test book held something that day");
        assert_eq!(f.read(|e| e.past_built()), Some(true), "a past day worked out is said once");
        assert_eq!(f.read(|e| e.past_built()), Some(false));
        let want: BTreeSet<(InstrumentId, bagholder_core::jiff::civil::Date)> = held.positions.iter().filter_map(|p| crate::read_sources::needs_of(&book, &bagholder_engine::needs::FactNeeds { spans: [(p.instrument, [(day, day)].into())].into(), ..Default::default() }, today).unwrap().closes.first().map(|c| (c.listing.id, c.from))).collect();
        assert!(!want.is_empty());
        assert_eq!(spans(f).into_iter().collect::<BTreeSet<_>>(), want, "each holding's close of that day");
        let cache = f.cache().unwrap();
        let asked = || -> usize { want.iter().map(|(i, _)| cache.reads(&i.to_string(), bagholder_sources::contract::DataKind::DailyClose).unwrap().len()).sum() };
        let before = asked();
        pass(&app, f, now).unwrap();
        assert!(asked() > before, "the reader asked for them");
        drop(held);
        assert!(spans(f).is_empty(), "no screen holds it any more: nothing is named");
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

    /// A book the figures could not be built from, then mended: the change that
    /// mends it (a write to the record, or a page stating its zone) asks for a
    /// pass, and that pass builds the figures and clears the header's failure.
    #[test]
    fn a_change_after_a_failed_build_retries_and_a_build_clears_the_header() {
        crate::tests_common::home(); // offline: the pass's reads leave nothing on the machine
        let now = t("2025-11-19T21:00:00Z");
        for through_zone in [false, true] {
            let home = tempfile::tempdir().unwrap();
            crate::tests_common::pulled_book(home.path());
            {
                let (book, _) = bagholder_book::Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
                book.state_zone("America/Toronto", now).unwrap();
                let account = book.accounts().unwrap()[0].id;
                let conn = book.connections().unwrap()[0].id;
                let read = book.broker_read(conn, "accounts", now).unwrap();
                book.store_net_value(account, now, bagholder_core::Money::new(bagholder_core::Dec::from_int(1000), bagholder_core::Currency::CAD), &read).unwrap();
                // a value no reader can read, written around the book
                book.conn_for_tests().execute("UPDATE account_values SET amount = 'not a number'", []).unwrap();
            }
            let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
            app.set_figures(crate::figures::Figures::open(home.path(), now).unwrap());
            let f = app.figures.get().unwrap();
            assert_eq!(settle(&app, pass(&app, f, now)), None);
            assert!(!app.state.lock().unwrap().figures_error.is_empty(), "the failure is in the header");
            f.woken();
            f.book().unwrap().conn_for_tests().execute("UPDATE account_values SET amount = '1000'", []).unwrap();
            if through_zone {
                f.state_zone("America/Toronto", now).unwrap();
            } else {
                f.record_changed(now).unwrap();
            }
            assert!(f.woken(), "the change asks the scheduler for a pass (through_zone {through_zone})");
            settle(&app, pass(&app, f, now));
            assert!(f.read(|_| ()).is_some(), "the figures are built");
            assert!(app.state.lock().unwrap().figures_error.is_empty(), "the pass that built them cleared the header (through_zone {through_zone})");
        }
    }

    /// A share or fund Wealthsimple carries, on a US venue (no TMX read of its
    /// ex-dividend date), with its id.
    fn carried(n: usize) -> bagholder_sources::contract::Listing {
        bagholder_sources::contract::Listing {
            id: InstrumentId::parse(&format!("00000000-0000-4000-8000-{n:012}")).unwrap(),
            kind: bagholder_core::instrument::InstrumentKind::Security,
            currency: bagholder_core::Currency::USD,
            symbol: format!("ZQ{n}"),
            venue_mic: Some("XNAS".into()),
            routes: [(bagholder_core::instrument::RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), vec![format!("sec-s-{n}")])].into(),
        }
    }

    /// An app on the pulled book, signed in, its Wealthsimple answering with `gql`.
    fn signed_in(home: &std::path::Path, gql: crate::orders::seam::Gql) -> Arc<App> {
        crate::tests_common::pulled_book(home);
        let app = App::new(home.to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now = t("2025-11-19T21:00:00Z");
        let f = crate::figures::Figures::open(home, now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        *app.orders.seam.session.lock().unwrap() = Some(bagholder_ws::session::Session { access_token: "tok".into(), ..Default::default() });
        *app.orders.seam.gql.lock().unwrap() = Some(gql);
        app.state.lock().unwrap().connected = true;
        app
    }

    /// Signed in, the shares and funds Wealthsimple carries are quoted by it, twenty
    /// to a request: 45 listings, 3 requests, each quote kept as Wealthsimple states
    /// it, its day change against the close it states (the owner, 2026-11-06;
    /// `SPEC.md` market-data-37).
    #[test]
    fn signed_in_wealthsimple_quotes_what_it_carries_twenty_to_a_request() {
        let home = tempfile::tempdir().unwrap();
        let asked: Arc<std::sync::Mutex<Vec<usize>>> = Arc::default();
        let seen = asked.clone();
        let app = signed_in(home.path(), Arc::new(move |op, vars| {
            assert_eq!(op, "FetchSecuritiesSummary");
            let ids = vars["ids"].as_array().unwrap().clone();
            seen.lock().unwrap().push(ids.len());
            Ok(serde_json::json!({"securities": ids.iter().map(|id| serde_json::json!({"id": id, "currency": "USD", "quoteV2": {"price": 10.5, "previousBaseline": 10, "currency": "USD", "quotedAsOf": "2025-11-19T20:59:00Z"}})).collect::<Vec<_>>()}))
        }));
        let f = app.figures.get().unwrap();
        let (book, cache, bank) = (f.book().unwrap(), f.cache().unwrap(), bank_zone().unwrap());
        let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now: t("2025-11-19T21:00:00Z"), bank: &bank };
        let listings: Vec<_> = (1..=45).map(carried).collect();
        let sess = crate::ws_quotes::session(&app).expect("signed in");
        let kept = crate::ws_quotes::read(&app, &ctx, &sess, &listings).unwrap();
        assert_eq!(*asked.lock().unwrap(), [20, 20, 5], "three requests for 45");
        assert_eq!(kept.len(), 45);
        let q = cache.quotes_of(listings[44].id).unwrap().into_iter().max_by_key(|q| q.received_at).unwrap();
        assert_eq!(q.source, crate::ws_quotes::source());
        assert_eq!(q.price, bagholder_core::Money::new(bagholder_core::Dec::parse("10.5").unwrap(), bagholder_core::Currency::USD));
        assert_eq!(q.change, Some(bagholder_core::Dec::parse("0.5").unwrap()));
        assert_eq!(q.change_pct, Some(bagholder_core::Dec::parse("5").unwrap()));
        assert_eq!(q.quoted_at, t("2025-11-19T20:59:00Z"));
    }

    /// A holding the book's records name by its Wealthsimple id is a listing
    /// Wealthsimple quotes: the listing the figure path asks for carries that id,
    /// though it identifies the holding rather than routes it.
    #[test]
    fn a_holding_read_from_wealthsimple_is_one_it_quotes() {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let now = t("2025-11-19T21:00:00Z");
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let book = f.book().unwrap();
        let n = f.read(|e| e.needs()).unwrap();
        let held = crate::read_sources::needs_of(&book, &n, now.to_zoned(TimeZone::get("America/Toronto").unwrap()).date()).unwrap().held;
        let shares: Vec<_> = held.iter().filter(|l| l.kind == bagholder_core::instrument::InstrumentKind::Security && l.venue_mic.is_some()).collect();
        assert!(!shares.is_empty());
        for l in shares {
            assert!(crate::ws_quotes::carried(l).is_some_and(|id| !id.is_empty()), "{} carries no Wealthsimple id", l.symbol);
        }
    }

    /// A batch Wealthsimple refuses whole for one security it rejects (one delisted or
    /// merged, still in the book) is asked one security at a time from the next pass:
    /// the one it rejects fails alone and the rest are priced, and those back in
    /// batches once they answer.
    #[test]
    fn one_security_wealthsimple_rejects_never_holds_up_its_batch() {
        let home = tempfile::tempdir().unwrap();
        let asked: Arc<std::sync::Mutex<Vec<usize>>> = Arc::default();
        let seen = asked.clone();
        let app = signed_in(home.path(), Arc::new(move |_, vars| {
            let ids = vars["ids"].as_array().unwrap().clone();
            seen.lock().unwrap().push(ids.len());
            if ids.iter().any(|id| id == "sec-s-2") {
                return Err(bagholder_ws::session::CallError::Graphql("FetchSecuritiesSummary: security sec-s-2 not found".into()));
            }
            Ok(serde_json::json!({"securities": ids.iter().map(|id| serde_json::json!({"id": id, "currency": "USD", "quoteV2": {"price": 10.5, "currency": "USD"}})).collect::<Vec<_>>()}))
        }));
        let f = app.figures.get().unwrap();
        let (book, cache, bank) = (f.book().unwrap(), f.cache().unwrap(), bank_zone().unwrap());
        let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now: t("2025-11-19T21:00:00Z"), bank: &bank };
        let listings = vec![carried(1), carried(2), carried(3)];
        let sess = crate::ws_quotes::session(&app).expect("signed in");
        let pass = || {
            asked.lock().unwrap().clear();
            let kept: Vec<InstrumentId> = crate::ws_quotes::read(&app, &ctx, &sess, &listings).unwrap().iter().map(|l| l.id).collect();
            (asked.lock().unwrap().clone(), kept)
        };
        assert_eq!(pass(), (vec![3], vec![]), "refused whole");
        assert_eq!(pass(), (vec![1, 1, 1], vec![carried(1).id, carried(3).id]), "each alone: the rejected one fails alone");
        assert_eq!(pass(), (vec![2, 1], vec![carried(1).id, carried(3).id]), "the answering ones batched again");
    }

    /// A null entry, or one that states no quote, among Wealthsimple's answers fails
    /// its own listing alone: the list is read entry by entry (`lenient::list`), and
    /// the rest of the batch is priced.
    #[test]
    fn an_entry_wealthsimple_cannot_quote_fails_its_own_listing_alone() {
        let home = tempfile::tempdir().unwrap();
        let app = signed_in(home.path(), Arc::new(|_, _| {
            let good = |id: &str| serde_json::json!({"id": id, "currency": "USD", "quoteV2": {"price": 10.5, "currency": "USD"}});
            Ok(serde_json::json!({"securities": [good("sec-s-1"), null, {"id": "sec-s-3", "currency": "USD"}, good("sec-s-4")]}))
        }));
        let f = app.figures.get().unwrap();
        let (book, cache, bank) = (f.book().unwrap(), f.cache().unwrap(), bank_zone().unwrap());
        let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now: t("2025-11-19T21:00:00Z"), bank: &bank };
        let listings: Vec<_> = (1..=4).map(carried).collect();
        let sess = crate::ws_quotes::session(&app).expect("signed in");
        let kept: Vec<InstrumentId> = crate::ws_quotes::read(&app, &ctx, &sess, &listings).unwrap().iter().map(|l| l.id).collect();
        assert_eq!(kept, [carried(1).id, carried(4).id]);
        assert!(app.ws_alone.lock().unwrap().is_empty(), "an answered batch is not split");
        let outcomes = cache.outcomes(&crate::ws_quotes::source()).unwrap();
        let of = |n: usize| outcomes.iter().find(|o| o.instrument == Some(carried(n).id)).map(|o| o.outcome);
        assert_eq!(of(2), Some(bagholder_sources::outcome::OutcomeKind::NotCarried), "the null entry: no security stated");
        assert_eq!(of(3), Some(bagholder_sources::outcome::OutcomeKind::Meaning), "no quote stated");
    }

    /// Signed in, a listing Wealthsimple carries never reaches Yahoo or TMX, its read
    /// failed or not: it is asked of Wealthsimple again at its turn. What Wealthsimple
    /// does not carry (a tile's index) goes to the public feeds; signed out, every
    /// listing does.
    #[test]
    fn a_listing_wealthsimple_carries_reaches_no_public_feed_while_signed_in() {
        let home = tempfile::tempdir().unwrap();
        let app = signed_in(home.path(), Arc::new(|_, _| Err(bagholder_ws::session::CallError::Failed("unreachable".into()))));
        let f = app.figures.get().unwrap();
        let (book, cache, bank) = (f.book().unwrap(), f.cache().unwrap(), bank_zone().unwrap());
        let ctx = Ctx { book: &book, cache: &cache, net: &app.net, now: t("2025-11-19T21:00:00Z"), bank: &bank };
        let index = bagholder_sources::contract::Listing { kind: bagholder_core::instrument::InstrumentKind::Index, routes: Default::default(), ..carried(99) };
        let listings = vec![carried(1), carried(2), index.clone()];
        let mut public = Vec::new();
        quote_by_source(&app, f, &ctx, &listings, |l| {
            public.push(l.id);
            Ok(())
        })
        .unwrap();
        assert_eq!(public, [index.id], "the index alone, Wealthsimple's read failed");
        assert!(cache.quotes_of(carried(1).id).unwrap().is_empty(), "nothing kept from another source in between");
        // signed out, as once Wealthsimple has ended the sign-in: no session
        app.state.lock().unwrap().connected = false;
        assert!(crate::ws_quotes::session(&app).is_none());
        let mut public = Vec::new();
        quote_by_source(&app, f, &ctx, &listings, |l| {
            public.push(l.id);
            Ok(())
        })
        .unwrap();
        assert_eq!(public, listings.iter().map(|l| l.id).collect::<Vec<_>>(), "signed out: the public feeds quote every one");
    }
}
