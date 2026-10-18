//! Market data, news, filings and the sweeps behind them: the watchlist and
//! tile writes, the wires, the Disclosures sweep and its notices, the fear
//! meters, short selling, the heatmap universes, quotes, the periodic market
//! records, the intraday archive, the watched folder and the chart history.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ts_rs::TS;

use bagholder_diff_derive::Diff;

use bagholder_market::{exposure, fear, history, news, shorts};
use bagholder_model::instruments;
use bagholder_model::venues::tmx_symbol;
use bagholder_model::securities::Security;
use bagholder_store::bars::ChartBars;
use bagholder_store::feeds::{self as sf, NewsItem, StoredGauge, StoredShorts};


use crate::app::{log, now_iso, now_unix, parse_instant, spawn, App};
use crate::http::OkOr;
use crate::notify;
// the disclosures and the release notices, kept per listing and per filer
pub use crate::filings::*;

/// The market-data loops' own state: what they are reading now, and for whom.
#[derive(Default)]
pub struct FeedsState {
    /// Bare symbols the running news pass has still to read (`*` the market feed).
    news_left: Mutex<HashSet<String>>,
    /// The listing whose disclosures were asked for last: the one someone is
    /// looking at, and so the one whose documents are read first.
    pub(crate) looking_at: Mutex<String>,
    /// The listing(s) whose disclosures are being read now, by document key.
    pub(crate) reading_now: Mutex<HashMap<String, Vec<String>>>,
    /// What the market universes' reads have come to.
    universes: Mutex<UniverseReads>,
    /// The Fear & Greed indexes being read from their publishers this moment.
    fear_reading: Mutex<HashSet<String>>,
    /// Each feed failing now, by feed, in the header's words, until it next answers
    /// (SPEC §1: a failure is said in the header until its source succeeds).
    failing: Mutex<BTreeMap<String, String>>,
    /// Each outside source's reads that failed in a row, by feed: one failed
    /// request is not said; the failure is said once the next asking fails too.
    missed: Mutex<BTreeMap<String, u32>>,
    /// The sources a test's universe reads asked, and the failure it told them to
    /// answer with.
    #[cfg(test)]
    pub(crate) universe_reads: Mutex<Vec<UniverseSource>>,
    #[cfg(test)]
    pub(crate) universe_fails: Mutex<Option<String>>,
    /// How many times a notice has sent for the issuer's record. A test counts the
    /// reads rather than reaching the source.
    #[cfg(test)]
    pub(crate) record_reads: AtomicI64,
    /// The failure a test tells the issuer's record to answer with.
    #[cfg(test)]
    pub(crate) record_fails: Mutex<Option<String>>,
    /// Listings the running short-interest sweep has still to read.
    shorts_left: AtomicI64,
}

/// `feed` has failed: said in the header, in `why`'s words, only once it has failed
/// `SAID_AFTER` times in a row, and then until it next answers (`SPEC.md` §4, the
/// header): a failure the next asking answers is no failure the person needs to
/// know of, and nothing is shown for a moment and gone. A request refused by
/// `BAGHOLDER_OFFLINE` asked nobody, so it is no failure of the feed's and is not
/// said (as the market sources' cache does not record one): known by its kind.
pub(crate) fn feed_failed(app: &Arc<App>, feed: &str, why: impl Into<bagholder_net::Failure>) {
    let why = match why.into() {
        bagholder_net::Failure::Offline => return,
        bagholder_net::Failure::Said(why) => why,
    };
    let missed = {
        let mut m = app.feeds.missed.lock().unwrap_or_else(|e| e.into_inner());
        let n = m.entry(feed.to_string()).or_insert(0);
        *n += 1;
        *n
    };
    // logged as a run of failures starts; said once it is confirmed
    if missed == 1 {
        log(&format!("bagholder feeds: {feed}: {why}"));
    }
    if missed >= bagholder_sources::health::SAID_AFTER {
        feed_down(app, feed, why);
    }
}

/// `feed` is down and no asking again can answer it until something changes (a
/// thread that ended): said in the header at once, until it next answers.
pub(crate) fn feed_down(app: &Arc<App>, feed: &str, why: String) {
    let was = app.feeds.failing.lock().unwrap_or_else(|e| e.into_inner()).insert(feed.to_string(), why.clone());
    if was.as_deref() != Some(why.as_str()) {
        app.events.signal();
    }
}

/// `feed` has answered: its failure, if one was standing, is no longer said.
pub(crate) fn feed_answered(app: &Arc<App>, feed: &str) {
    app.feeds.missed.lock().unwrap_or_else(|e| e.into_inner()).remove(feed);
    if app.feeds.failing.lock().unwrap_or_else(|e| e.into_inner()).remove(feed).is_some() {
        app.events.signal();
    }
}

/// The feeds failing now, one sentence each, in a steady order: part of the header's error line.
/// A listing's short interest failing is kept by listing, and said once for every
/// listing the same failure stopped: a source that is down is one sentence, not one
/// a listing.
pub fn feed_failures(app: &Arc<App>) -> Vec<String> {
    let failing = app.feeds.failing.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Vec::new();
    let mut shorts: Vec<(String, Vec<String>)> = Vec::new();
    for (feed, why) in failing.iter() {
        // each listing by its own key, named with its venue: two listings of one ticker are two
        match feed.strip_prefix(SHORTS_OF).and_then(|k| k.parse::<bagholder_core::ListingKey>().ok()) {
            Some(listing) => match shorts.iter_mut().find(|(w, _)| w == why) {
                Some((_, named)) => named.push(listing.named()),
                None => shorts.push((why.clone(), vec![listing.named()])),
            },
            None => out.push(why.clone()),
        }
    }
    out.extend(shorts.into_iter().map(|(why, named)| format!("The short interest of {} could not be read: {why}", named.join(", "))));
    out
}

/// The header's entry for a listing's short interest, before its symbol.
const SHORTS_OF: &str = "shorts:";

/// The header's entry for the market cache itself, where the feeds are kept: a
/// pass that could not open it says so there until the next open succeeds.
pub(crate) const CACHE: &str = "market-cache";
/// The header's entry for the book the notices are kept in, as the sweeps open it.
const NOTICES_BOOK: &str = "notices-book";
/// The header's entry for the market's context the earlier readers are given.
const CONTEXT: &str = "context";
/// The header's entry for the notification settings the sweeps read.
const NOTICE_SETTINGS: &str = "notify-settings";
/// The header's entries for the disclosures: the sweep and each listing's
/// refresh; the stored lists pages are sent; the reading of documents.
pub(crate) const DISCLOSURES: &str = "disclosures";
pub(crate) const DISCLOSURES_STORED: &str = "disclosures-stored";
pub(crate) const DISCLOSURES_READING: &str = "disclosures-reading";
/// The header's entry for the press releases a wire answered with.
pub(crate) const RELEASES: &str = "releases";
/// The header's entry for the exposure records' pass.
const EXPOSURE: &str = "exposure";
/// The header's entry for the short selling held, as the table reads it.
const SHORTS: &str = "shorts";

/// `feed`'s outcome said: its failure until it next succeeds.
pub(crate) fn went<T, E: Into<bagholder_net::Failure>>(app: &Arc<App>, feed: &str, r: Result<T, E>) -> Option<T> {
    match r {
        Ok(v) => {
            feed_answered(app, feed);
            Some(v)
        }
        Err(why) => {
            feed_failed(app, feed, why);
            None
        }
    }
}

/// The market cache, on loan from the pool; one that will not open is said in
/// the header (`CACHE`) until one does.
pub(crate) fn conn(app: &Arc<App>) -> Result<bagholder_sqlite::pool::Pooled<'_>, String> {
    let c = app.cache().map_err(|e| format!("The market cache could not be opened: {e}"));
    if let Err(why) = &c {
        feed_failed(app, CACHE, why.clone());
    } else {
        feed_answered(app, CACHE);
    }
    c
}

/// The market cache's pool itself, for a reader that borrows on threads of its
/// own. Said as `conn` is.
pub(crate) fn pool(app: &Arc<App>) -> Result<Arc<bagholder_sqlite::pool::Pool>, String> {
    went(app, CACHE, app.cache_pool().map_err(|e| format!("The market cache could not be opened: {e}")))
        .ok_or_else(|| "The market cache could not be opened".to_string())
}

/// A connection to the market cache of the caller's own, not the pool's: for
/// work that hands it to threads it starts itself and keeps it for the length of
/// a pass. Said as `conn` is.
fn own_conn(app: &Arc<App>) -> Result<Connection, String> {
    // the pool's borrow brings the file to this build's schema first
    let c = conn(app).and_then(|_| bagholder_sqlite::open_db(&app.home.join(crate::figures::CACHE_FILE)).map_err(|e| format!("The market cache could not be opened: {e}")));
    went(app, CACHE, c.as_ref().map(|_| ()).map_err(String::clone));
    c
}

/// The book the notices are kept in; one that will not open is said in the
/// header (`NOTICES_BOOK`) until one does.
pub(crate) fn notices(app: &Arc<App>) -> Result<notify::NoticesBook<'_>, String> {
    let b = notify::book(app).map_err(|e| format!("The book could not be opened for the notifications: {e}"));
    // the book not open: its open's own failure is said, once; nothing under this name
    if app.figures.get().is_none() {
        return b;
    }
    went(app, NOTICES_BOOK, b.as_ref().map(|_| ()).map_err(String::clone));
    b
}

/// The market's context the earlier readers are given (stage 5 moves them): none
/// before the figures are built (no page has stated its zone yet), which is no
/// failure; one that cannot be read is said in the header until it can.
pub(crate) fn base(app: &Arc<App>) -> Result<Option<Arc<bagholder_model::context::MarketBase>>, String> {
    if !app.figures.get().is_some_and(|f| f.read(|_| ()).is_some()) {
        return Ok(None);
    }
    let b = app.market_base().map_err(|e| format!("The book's listings could not be read: {e}"));
    went(app, CONTEXT, b.as_ref().map(|_| ()).map_err(String::clone));
    b.map(Some)
}

/// The notification settings a sweep reads: settings that cannot be read are said
/// in the header, and a sweep reading them does nothing until they can.
pub(crate) fn notice_settings<T>(app: &Arc<App>, r: rusqlite::Result<T>) -> Option<T> {
    went(app, NOTICE_SETTINGS, r.map_err(|e| format!("The notification settings could not be read: {e}")))
}


pub(crate) fn today() -> String {
    bagholder_market::clock_now().0
}

pub(crate) fn sha1_hex12(text: &str) -> String {
    let d = openssl::sha::sha1(text.as_bytes());
    d.iter().map(|b| format!("{:02x}", b)).collect::<String>()[..12].to_string()
}

// ---------------------------------------------------------------------------
// exposure
// ---------------------------------------------------------------------------

pub const EXPOSURE_FIRST_SEC: u64 = 20;
pub const EXPOSURE_WORKERS: usize = 4;

enum ExposureJob {
    Sec(Security),
    Under(String, String),
    Watch(String, String, String),
}

/// The exposure record of every held security
/// that has none or an old one, four at a time, each shown as it lands.
pub fn refresh_exposures(app: &Arc<App>) {
    // the book not opening and its listings not read are said by `conn` and `base`
    let Ok(c) = conn(app) else { return };
    let Ok(Some(b)) = base(app) else { return };
    let secs: HashMap<String, Security> = match crate::market_context::securities(app) {
        Ok(v) => v.into_iter().filter(|s| !s.id.is_empty()).map(|s| (s.id.clone(), s)).collect(),
        Err(e) => {
            feed_failed(app, EXPOSURE, format!("The exposure records could not be brought up to date: the book's securities could not be read: {e}"));
            return;
        }
    };
    let held: HashSet<String> = b.positions.iter().filter(|p| p.kind == bagholder_model::activity::Kind::Shares).map(|p| p.security_id.clone()).collect();
    let mut held: Vec<String> = held.into_iter().filter(|sid| !sid.is_empty() && !sid.starts_with("sec-c-")).collect();
    held.sort();
    let (today_s, _, _) = bagholder_market::clock_now();
    let Ok(p) = pool(app) else { return };
    let ctx = exposure::Ctx { conn: &c, pool: p, today: today_s.clone() };
    let exposure_failed = |e: bagholder_net::Failure| feed_failed(app, "exposure", e.map(|e| format!("The exposures could not be refreshed: {e}")));
    let stale = |ids: &[String]| exposure::stale(&ctx, ids);
    let mut todo: Vec<String> = match stale(&held) {
        Ok(t) => t.into_iter().filter(|sid| secs.contains_key(sid)).collect(),
        Err(e) => return exposure_failed(e),
    };
    todo.sort_by_key(|sid| if exposure::is_fund(&secs[sid].name) { 1 } else { 0 });
    // an option's underlying, on its own listing: the contract's record names the
    // underlying's venue (brief 34, U21); one that names none is classified by no one
    let mut unders: Vec<(String, String)> = b
        .positions
        .iter()
        .filter(|p| p.kind == bagholder_model::activity::Kind::Options && !p.underlying.is_empty())
        .filter(|p| bagholder_model::venues::share_key_of(&p.underlying, &p.exchange).is_some())
        .map(|p| (p.underlying.to_uppercase(), p.exchange.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unders.sort();
    let mut stale_unders = Vec::new();
    for (u, ex) in unders {
        let Some(key) = bagholder_model::venues::share_key_of(&u, &ex) else { continue };
        match stale(&[key]) {
            Ok(s) if s.is_empty() => {}
            Ok(_) => stale_unders.push((u, ex)),
            Err(e) => return exposure_failed(e),
        }
    }
    let unders = stale_unders;
    let mut watched: Vec<(String, String, String)> = Vec::new();
    for w in b.watchlist.iter().filter(|w| instruments::find(&w.symbol, &w.exchange).is_none() && w.exchange.to_uppercase() != "CRYPTO") {
        let Some(key) = bagholder_model::venues::share_key_of(&w.symbol, &w.exchange) else { continue };
        match stale(&[key]) {
            Ok(s) if s.is_empty() => {}
            Ok(_) => watched.push((w.symbol.clone(), w.exchange.clone(), w.currency.clone())),
            Err(e) => return exposure_failed(e),
        }
    }

    let mut jobs: Vec<ExposureJob> = todo.iter().map(|sid| ExposureJob::Sec(secs[sid].clone())).collect();
    jobs.extend(unders.into_iter().map(|(u, ex)| ExposureJob::Under(u, ex)));
    jobs.extend(watched.into_iter().map(|(sy, e, cc)| ExposureJob::Watch(sy, e, cc)));
    drop(ctx);
    if jobs.is_empty() {
        feed_answered(app, EXPOSURE);
        return;
    }
    let queue = Arc::new(Mutex::new(jobs.into_iter().collect::<std::collections::VecDeque<_>>()));
    // what each record's read failed on, said together once the pass is over
    let failures: Arc<Mutex<Vec<bagholder_net::Failure>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..EXPOSURE_WORKERS {
        let queue = queue.clone();
        let failures = failures.clone();
        let today_s = today_s.clone();
        let app = app.clone();
        let h = std::thread::Builder::new().name("bagholder-exposure".into()).spawn(move || {
            let (Ok(c), Ok(p)) = (conn(&app), pool(&app)) else { return }; // said in the header by `conn`
            let ctx = exposure::Ctx { conn: &c, pool: p, today: today_s };
            loop {
                let job = match queue.lock().unwrap().pop_front() { Some(j) => j, None => return };
                if app.stopping() {
                    continue;
                }
                let line = match job {
                    ExposureJob::Sec(sec) => {
                        let rec = match exposure::refresh_security(&ctx, &sec) {
                            Ok(rec) => rec,
                            Err(e) => {
                                failures.lock().unwrap_or_else(|p| p.into_inner()).push(e);
                                continue;
                            }
                        };
                        format!(
                            "bagholder exposure: {} {}: {} ({}% covered){}",
                            sec.symbol,
                            if exposure::is_fund(&sec.name) { "fund" } else { "share" },
                            if rec.source.is_empty() { "no source".to_string() } else { rec.source.clone() },
                            (rec.coverage * 100.0).round_ties_even() as i64,
                            if rec.error.is_empty() { String::new() } else { format!(": {}", rec.error) }
                        )
                    }
                    ExposureJob::Watch(sym, ex, ccy) => match exposure::share_exposure(&ctx, &sym, &ex, &ccy) {
                        Ok(_) => format!("bagholder exposure: {} (watched) classified", sym),
                        Err(e) => {
                            failures.lock().unwrap_or_else(|p| p.into_inner()).push(e.map(|e| format!("{sym}: {e}")));
                            continue;
                        }
                    },
                    ExposureJob::Under(under, ex) => match exposure::share_exposure(&ctx, &under, &ex, "") {
                        Ok(_) => format!("bagholder exposure: {} (an option's underlying) classified", under),
                        Err(e) => {
                            failures.lock().unwrap_or_else(|p| p.into_inner()).push(e.map(|e| format!("{under}: {e}")));
                            continue;
                        }
                    },
                };
                log(&line);
                // each record shows as soon as it lands
            }
        });
        // a thread the system will not start is what `std::thread::spawn` panics on too
        handles.push(h.expect("the system could not start a thread"));
    }
    let stopped = handles.into_iter().map(|h| h.join()).filter(Result::is_err).count();
    // a source not asked because the app is offline answered nothing, and failed nothing
    let mut failures: Vec<String> = std::mem::take(&mut *failures.lock().unwrap_or_else(|p| p.into_inner())).into_iter().filter_map(|f| match f {
        bagholder_net::Failure::Said(s) => Some(s),
        bagholder_net::Failure::Offline => None,
    }).collect();
    if stopped > 0 {
        failures.push(format!("{stopped} of the readers stopped short"));
    }
    // what failed are outside sources' answers: said once the next pass fails too (`feed_failed`)
    if failures.is_empty() {
        feed_answered(app, EXPOSURE);
    } else {
        feed_failed(app, EXPOSURE, format!("The exposure records could not be brought up to date: {}", failures.join("; ")));
    }
}

/// Soon after start, when the listings the records are kept for change, and a
/// few times a day.
pub fn exposure_loop(app: Arc<App>) {
    // What a fund holds changes over months. The records are looked over shortly after
    // start, then when the set of listings they are kept for changes -- a holding came
    // or went (the engine's report of what a change moved), a listing watched -- and
    // otherwise a few times a day, not every half hour for ever.
    if app.wait(Duration::from_secs(EXPOSURE_FIRST_SEC)) {
        return;
    }
    loop {
        let before = listings_at(&app);
        refresh_exposures(&app);
        app.events.park_until_or(&app, Duration::from_secs(6 * 3600), || listings_moved(&app, before));
        if app.stopping() {
            return;
        }
    }
}

/// Where the listings the exposure records are kept for stand: the figures'
/// version and the version of what the person follows.
pub(crate) fn listings_at(app: &App) -> (u64, u64) {
    (app.figures.get().map_or(0, |f| f.version()), app.following_version())
}

/// Whether the listings the exposure records are kept for changed since `at`: a
/// listing followed or unfollowed, or a change the engine reports as a holding
/// that came or went (a quote, which moves a holding's figures, is none).
pub(crate) fn listings_moved(app: &App, at: (u64, u64)) -> bool {
    if app.following_version() != at.1 {
        return true;
    }
    match app.figures.get().map(|f| f.moved_since(at.0)) {
        None | Some(crate::figures::Since::Nothing) => false,
        Some(crate::figures::Since::Everything) => true,
        Some(crate::figures::Since::Moved(m)) => crate::market_context::moves_it(&m),
    }
}

// ---------------------------------------------------------------------------
// watchlist and tiles
// ---------------------------------------------------------------------------

/// A watched listing's sector, read once when it is added, from the same public
/// sources a holding's record is (an instrument of the directory and a coin have
/// none to read).
pub fn read_sector(app: &Arc<App>, n: &crate::following::Named) {
    if instruments::find(&n.symbol, &n.exchange).is_some() || n.exchange.eq_ignore_ascii_case("CRYPTO") {
        return;
    }
    let (symbol, exchange, currency) = (tmx_symbol(&n.symbol), n.exchange.clone(), n.currency.clone());
    let said = bagholder_core::ListingKey::on(&n.symbol, &n.exchange).map_or_else(|_| format!("{} on {}", n.symbol, n.exchange), |k| k.named());
    let a = app.clone();
    spawn("watch-sector", move || {
        let (Ok(c), Ok(p)) = (conn(&a), pool(&a)) else { return }; // said in the header by `conn`
        let ctx = exposure::Ctx { conn: &c, pool: p, today: today() };
        if let Err(e) = exposure::share_exposure(&ctx, &symbol, &exchange, &currency) {
            feed_failed(&a, EXPOSURE, format!("The sector of {said} could not be read: {e}"));
        }
    });
}

/// Forget the news read for a listing no longer watched.
pub fn forget_news(app: &Arc<App>, symbol: &str, exchange: &str) -> Result<(), String> {
    // a listing that names no venue had no news kept under a key
    let Some(key) = news::news_listing(symbol, exchange) else { return Ok(()) };
    let c = conn(app)?;
    sf::forget_news(&c, &key).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// news
// ---------------------------------------------------------------------------

/// The market feed, the shares held, the watched listings. One listing, one
/// read, under its bare ticker: the book's QNC.TO and the watchlist's QNC are
/// the same wire. The name the book records for it is what Google is searched
/// for.
pub fn news_listings(app: &Arc<App>) -> Vec<news::Listing> {
    let mut out = vec![news::Listing { symbol: news::MARKET.0.to_string(), exchange: news::MARKET.1.to_string(), currency: news::MARKET.2.to_string(), name: String::new() }];
    // the book's listings not read are said by `base`: the market feed is read still
    let Ok(Some(b)) = base(app) else { return out };
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for p in b.positions.iter().filter(|p| p.kind == bagholder_model::activity::Kind::Shares) {
        let key = (tmx_symbol(&p.symbol), p.exchange.to_uppercase());
        if !key.0.is_empty() && seen.insert(key.clone()) {
            out.push(news::Listing { symbol: key.0, exchange: p.exchange.clone(), currency: p.currency.clone(), name: p.name.clone() });
        }
    }
    for w in b.watchlist.iter() {
        let key = (tmx_symbol(&w.symbol), w.exchange.to_uppercase());
        if !key.0.is_empty() && key.1 != "CRYPTO" && instruments::find(&w.symbol, &w.exchange).is_none() && seen.insert(key.clone()) {
            out.push(news::Listing { symbol: key.0, exchange: w.exchange.clone(), currency: w.currency.clone(), name: w.name.clone() });
        }
    }
    out
}

/// Bare symbols the running news pass has still to read (`*` the market feed).
pub fn news_reading(app: &Arc<App>) -> Vec<String> {
    let mut left: Vec<String> = app.feeds.news_left.lock().unwrap().iter().cloned().collect();
    left.sort();
    left
}

/// `news`: what a page showing the News card is sent beside the model's items -- the
/// listings the running pass has still to read, by key (`*@MARKET` the market feed), so the card
/// reads `Reading…` rather than `No news.` for a listing not read yet. A page showing
/// it is also what the news is read for.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct NewsDoc {
    pub reading: Vec<String>,
}

pub fn news_stored(app: &Arc<App>) -> NewsDoc {
    NewsDoc { reading: news_reading(app) }
}

/// Every source for every listing with one due. Never fails. Each listing's
/// items reach the model as it lands, and the listings still to read are in
/// the status, so the News card says a read is under way instead of `No
/// news.` while a pass runs.
pub fn refresh_news(app: &Arc<App>) -> usize {
    app.single_flight("news", 0, || {
        let listings = news_listings(app);
        let (today_s, now, _) = bagholder_market::clock_now();
        let clock = news::Clock { today: today_s, now: now as i64 };
        // each listing by its key, the market's own feed `*@MARKET`: a listing still to read is never another of its ticker
        let key = |l: &news::Listing| news::news_listing(&l.symbol, &l.exchange).map_or_else(|| format!("{}@", l.symbol.to_uppercase()), |k| k.to_string());
        let start = |due: &[news::Listing]| {
            *app.feeds.news_left.lock().unwrap() = due.iter().map(key).collect();
            app.events.signal(); // the status names the listings still to read
        };
        let done = |l: &news::Listing, _answered: bool| {
            app.feeds.news_left.lock().unwrap().remove(&key(l));
            app.events.signal();
        };
        let on_new = |_: &Connection, sym: &str, ex: &str, rows: &[NewsItem], ids: &[String]| note_wire_releases_said(app, sym, ex, rows, ids);
        // a connection that will not open is said in the header by `own_conn`
        let got = news::refresh(&|| own_conn(app).ok(), &news::LIVE_READERS, &listings, &clock, Some(&on_new), Some(&start), Some(&done), news::LISTINGS_AT_ONCE);
        app.feeds.news_left.lock().unwrap().clear();
        app.events.signal();
        match got {
            Ok(n) => {
                feed_answered(app, "news");
                n
            }
            Err(e) => {
                log(&format!("bagholder news: refresh failed: {}", e));
                feed_failed(app, "news", format!("The news could not be refreshed: {e}"));
                0
            }
        }
    })
}

/// Whether anyone is owed the news: a page shows the News card (the `news`
/// document), or a Releases notification set is on and must hear of a release
/// whoever is looking. A page open on another tab is not owed it.
pub(crate) fn news_wanted(app: &Arc<App>) -> bool {
    app.events.watched("news") || notices(app).is_ok_and(|b| notice_settings(app, crate::notify::any_release_scope(&b)).unwrap_or(false))
}

/// While someone is owed the news: at once when they come to be (a page starting to
/// show the card wakes the park), then every five minutes, each listing's sources
/// read when due (once per fifteen). The sources publish no feed to be told by, so
/// they are read on that clock; with nobody owed them, nothing is read.
pub fn news_loop(app: Arc<App>) {
    while app.events.park_until(&app, || news_wanted(&app)) {
        refresh_news(&app);
        if app.wait(Duration::from_secs(300)) {
            return;
        }
    }
}

/// One listing's news read now from every source, for the News card's search:
/// a ticker neither held nor watched has no rows until asked for. The rows are
/// stored under the listing (tagged as neither held nor watched, so they show
/// only under its chip) and the model reloads.
/// `GET /api/news/symbol`: refused, or how many rows the search found and
/// where.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(untagged)]
pub enum NewsSymbolAnswer {
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
    Ok {
        #[ts(type = "true")]
        ok: bool,
        count: usize,
        source: String,
        exchange: String,
    },
}

impl NewsSymbolAnswer {
    fn err(e: impl Into<String>) -> NewsSymbolAnswer {
        NewsSymbolAnswer::Err { ok: false, error: e.into() }
    }
}

pub fn news_symbol_payload(app: &Arc<App>, key: &bagholder_core::ListingKey, exchange: &str, currency: &str) -> NewsSymbolAnswer {
    news_symbol_payload_with(app, key, exchange, currency, &news::LIVE_READERS, &|sym| bagholder_market::tmx::tmx_listings(sym).map_err(|e| e.to_string()))
}

/// `news_symbol_payload` with the readers and TMX's listings of a symbol given.
pub fn news_symbol_payload_with(
    app: &Arc<App>,
    key: &bagholder_core::ListingKey,
    exchange: &str,
    currency: &str,
    readers: &news::Readers,
    listings_of: &dyn Fn(&str) -> Result<Vec<bagholder_model::wire::SymbolMatch>, String>,
) -> NewsSymbolAnswer {
    let sym = key.symbol().to_string();
    let c = match conn(app) {
        Ok(c) => c,
        Err(e) => return NewsSymbolAnswer::err(e),
    };
    let (today_s, now, _) = bagholder_market::clock_now();
    let clock = news::Clock { today: today_s.clone(), now: now as i64 };
    // the name is what Google is searched for: the book's security on this listing's,
    // else the one TMX's quote gives this listing on this venue, never another listing's
    let (known, known_ex, known_ccy) = instrument_meta(app, key);
    let ex = if exchange.trim().is_empty() { known_ex } else { exchange.trim().to_string() };
    let mut ccy = if currency.trim().is_empty() { known_ccy } else { currency.trim().to_string() };
    let mut name = known;
    if name.is_empty() && key.canadian() {
        match listings_of(&sym) {
            Ok(found) => {
                if let Some(l) = found.into_iter().find(|l| bagholder_core::ListingKey::on(&l.symbol, &l.exchange).is_ok_and(|k| k == *key)) {
                    name = l.name;
                    if ccy.is_empty() {
                        ccy = l.currency;
                    }
                }
            }
            Err(e) => return NewsSymbolAnswer::err(format!("{} could not be read: {e}", key.named())),
        }
    }
    let (src, rows) = match news::read_listing(&c, readers, &sym, &ex, &ccy, &name, true, &clock, None) {
        Ok(got) => got,
        Err(_) => return NewsSymbolAnswer::err("the wire did not answer"),
    };
    let rows = match rows { Some(r) => r, None => return NewsSymbolAnswer::err("the wire did not answer") };
    if let Err(e) = sf::trim_news(&c, news::KEEP) {
        return NewsSymbolAnswer::err(format!("The news could not be kept to its limit: {e}"));
    }
    NewsSymbolAnswer::Ok { ok: true, count: rows.len(), source: src.as_str().to_string(), exchange: ex }
}

// ---------------------------------------------------------------------------
// fear
// ---------------------------------------------------------------------------

pub const FEAR_STALE_MIN: f64 = 15.0;
pub const FEAR_VERSION: i64 = 1;

/// One index read from its publisher and kept; a read that fails is said in the
/// header until the index next answers, whatever reading is held meanwhile.
pub fn read_fear(app: &Arc<App>, index: &str) -> Result<StoredGauge, String> {
    read_fear_with(app, index, || fear::read(index))
}

/// `read_fear` with the publisher given. While the read is in the air a page showing
/// the meter is told so (`FearDoc::reading`), so a meter with nothing held yet reads
/// as being read rather than as a publisher that did not answer.
fn read_fear_with(app: &Arc<App>, index: &str, read: impl FnOnce() -> Result<sf::Gauge, bagholder_net::Failure>) -> Result<StoredGauge, String> {
    let which = index.trim().to_lowercase();
    app.feeds.fear_reading.lock().unwrap_or_else(|e| e.into_inner()).insert(which.clone());
    app.events.signal();
    let got = read();
    let rec = got.map(|gauge| StoredGauge { gauge, fetched_at: now_iso(), read_version: FEAR_VERSION });
    let feed = format!("fear:{which}");
    let rec = rec.and_then(|rec| {
        let c = conn(app)?;
        sf::save_gauge(&c, &which, &rec.gauge, &rec.fetched_at, FEAR_VERSION).map_err(|e| format!("The Fear & Greed reading could not be kept: {e}"))?;
        Ok(rec)
    });
    match &rec {
        Ok(_) => feed_answered(app, &feed),
        Err(why) => feed_failed(app, &feed, why.clone()),
    }
    app.feeds.fear_reading.lock().unwrap_or_else(|e| e.into_inner()).remove(&which);
    app.events.signal();
    rec.map_err(|e| e.to_string())
}

fn fear_stale(rec: &StoredGauge) -> bool {
    if rec.read_version < FEAR_VERSION {
        return true;
    }
    match parse_instant(&rec.fetched_at) {
        Some(then) => now_unix() - then > FEAR_STALE_MIN * 60.0,
        None => true,
    }
}

/// One index's meter, as a page is sent it.
#[derive(Clone, Debug, serde::Serialize, ts_rs::TS, Diff)]
pub struct FearDoc {
    #[ts(type = "true")]
    pub ok: bool,
    pub gauge: Option<StoredGauge>,
    /// A read of the publisher is in the air.
    pub reading: bool,
}

/// `GET /api/fear`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum FearAnswer {
    Ok(FearDoc),
    Refused(OkOr),
}

impl From<Result<FearDoc, String>> for FearAnswer {
    fn from(r: Result<FearDoc, String>) -> FearAnswer {
        match r {
            Ok(d) => FearAnswer::Ok(d),
            Err(e) => FearAnswer::Refused(OkOr::err(e)),
        }
    }
}

/// One index's meter, from the store at once.
pub fn fear_payload(app: &Arc<App>, index: &str) -> Result<FearDoc, String> {
    let which = index.trim().to_lowercase();
    if !fear::INDEXES.contains(&which.as_str()) {
        return Err("no such index".into());
    }
    let held = sf::gauge(&*conn(app)?, &which).map_err(|e| format!("The Fear & Greed reading held could not be read: {e}"))?;
    if let Some(held) = held {
        if fear_stale(&held) {
            let w = which.clone();
            let a = app.clone();
            app.kick(&format!("fear:{}", which), move || {
                match read_fear(&a, &w) {
                    Ok(_) => {}
                    Err(_said) => {} // said in the header by `read_fear` until the index next answers
                }
            });
        }
        return Ok(FearDoc { ok: true, gauge: Some(held), reading: fear_in_flight(app, &which) });
    }
    let rec = read_fear(app, &which)?;
    Ok(FearDoc { ok: true, gauge: Some(rec), reading: false })
}

/// The meter as it is held, never waiting on its publisher: what a page showing it is
/// sent (`docs`).
///
/// A reading held that cannot be read is said in the header (under the index's
/// own entry, until it next answers) and the meter is sent with none.
pub fn fear_stored(app: &Arc<App>, index: &str) -> FearDoc {
    let which = index.trim().to_lowercase();
    FearDoc { ok: true, gauge: fear_held(app, &which), reading: fear_in_flight(app, &which) }
}

/// The reading held for `which`; one that cannot be read is said in the header.
fn fear_held(app: &Arc<App>, which: &str) -> Option<StoredGauge> {
    match conn(app).and_then(|c| sf::gauge(&c, which).map_err(|e| format!("The Fear & Greed reading held could not be read: {e}"))) {
        Ok(held) => held,
        Err(why) => {
            feed_failed(app, &format!("fear:{which}"), why);
            None
        }
    }
}

fn fear_in_flight(app: &Arc<App>, which: &str) -> bool {
    app.feeds.fear_reading.lock().unwrap_or_else(|e| e.into_inner()).contains(which)
}

/// A page has started showing the meter `index`: read it when it is missing or
/// stale, and again as it goes stale, for as long as some page still shows it. The
/// publishers push nothing; with no page showing a meter, nothing reads one.
pub fn fear_shown(app: Arc<App>, doc: String, index: String) {
    let which = index.trim().to_lowercase();
    if !fear::INDEXES.contains(&which.as_str()) {
        return;
    }
    spawn("bagholder-fear-shown", move || {
        app.single_flight(&doc.clone(), (), || {
            while app.events.watched(&doc) && !app.stopping() {
                let held = fear_held(&app, &which);
                if held.as_ref().map_or(true, fear_stale) {
                    match read_fear(&app, &which) {
                        Ok(_) => {}
                        Err(_said) => {} // said in the header by `read_fear` until the index next answers
                    }
                }
                if app.wait(Duration::from_secs(FEAR_STALE_MIN as u64 * 60)) {
                    return;
                }
            }
        });
    });
}

// ---------------------------------------------------------------------------
// shorts
// ---------------------------------------------------------------------------

pub const SHORTS_STALE_HOURS: f64 = 6.0;
pub const SHORTS_VERSION: i64 = 5;
pub const SHORTS_SWEEP_EVERY_SEC: u64 = 1800;

fn shorts_stale(rec: &StoredShorts) -> bool {
    if rec.read_version < SHORTS_VERSION {
        return true;
    }
    match parse_instant(&rec.fetched_at) {
        Some(then) => now_unix() - then > SHORTS_STALE_HOURS * 3600.0,
        None => true,
    }
}

/// One listing's short selling from its regulator,
/// kept. `None` for a market where no one publishes it; the store failing is
/// the error. A source failing is said in the header until the listing's next
/// read goes through, and the record kept stands.
pub fn read_shorts(app: &Arc<App>, key: &bagholder_core::ListingKey, exchange: &str, trend: bool, name: &str) -> Result<Option<StoredShorts>, String> {
    let c = conn(app)?;
    let symbol = key.symbol();
    let feed = format!("{SHORTS_OF}{key}");
    let mut rec = match shorts::for_listing(&c, symbol, exchange, &today(), trend, name) {
        Ok(Some(rec)) => {
            feed_answered(app, &feed);
            rec
        }
        Ok(None) => {
            feed_answered(app, &feed);
            return Ok(None);
        }
        Err(e) => {
            // said with every other listing the same failure stopped (`feed_failures`)
            feed_failed(app, &feed, e);
            return Ok(None);
        }
    };
    if rec.exchange.is_empty() {
        rec.exchange = exchange.to_string();
    }
    let stored = StoredShorts { shorts: rec, fetched_at: now_iso(), read_version: SHORTS_VERSION };
    sf::save_shorts(&c, key, &stored.shorts, &stored.fetched_at, SHORTS_VERSION).map_err(|e| format!("{}'s short selling could not be kept: {e}", key.named()))?;
    Ok(Some(stored))
}

/// `read_shorts` from work that answers nobody: a failure is said in the header,
/// under the listing's own entry, until its next read succeeds.
fn read_shorts_said(app: &Arc<App>, key: &bagholder_core::ListingKey, exchange: &str, trend: bool, name: &str) -> Option<StoredShorts> {
    went(app, &format!("shorts-store:{key}"), read_shorts(app, key, exchange, trend, name)).flatten()
}

/// One report of the position sold short.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[diff(key = date)]
pub struct ShortReport {
    pub date: String,
    pub shares: crate::wire::Dec,
}

/// One listing's short selling as the page is sent it: every share count the
/// regulator states as exact decimal text, and the ratios the cards draw worked
/// out here (`docs/plans/stage-5-interface-and-running.md`, A2).
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = symbol)]
pub struct ShortsView {
    pub symbol: String,
    pub exchange: String,
    pub market: bagholder_store::feeds::ShortMarket,
    pub name: String,
    pub as_of: String,
    /// The position sold short.
    pub shares: Option<crate::wire::Dec>,
    pub previous: Option<crate::wire::Dec>,
    pub previous_of: String,
    pub change: Option<crate::wire::Dec>,
    pub float: Option<crate::wire::Dec>,
    /// The float not sold short: the float less the position.
    pub unshorted: Option<crate::wire::Dec>,
    /// The position over the float, as a fraction.
    pub of_float: Option<f64>,
    pub average_volume: Option<crate::wire::Dec>,
    pub days_to_cover: Option<f64>,
    pub volume_of: String,
    pub volume_span: Option<bagholder_store::feeds::VolumeSpan>,
    pub short_volume: Option<crate::wire::Dec>,
    pub total_volume: Option<crate::wire::Dec>,
    /// The shares traded that were not sold short.
    pub long_volume: Option<crate::wire::Dec>,
    /// The short volume over the total, as a fraction.
    pub of_volume: Option<f64>,
    /// The reports behind the position, oldest first; `None` where they were not read.
    pub series: Option<Vec<ShortReport>>,
    pub fetched_at: String,
}

/// A share count the earlier store keeps as a float, as the exact decimal it was
/// read as: a count is a whole number far inside a float's exact range, and the
/// shortest text that reads back as the float is the source's own.
fn count_of(what: &str, symbol: &str, v: Option<f64>) -> Result<Option<crate::wire::Dec>, String> {
    match v {
        None => Ok(None),
        Some(x) if x.is_finite() => bagholder_core::Dec::parse(&format!("{x}")).map(|d| Some(crate::wire::Dec(d))).map_err(|e| format!("{symbol}'s {what} {x}: {e}")),
        Some(x) => Err(format!("{symbol}'s {what} is {x}, not a count")),
    }
}

impl ShortsView {
    pub fn of(r: &StoredShorts) -> Result<ShortsView, String> {
        let s = &r.shorts;
        let sym = s.symbol.as_str();
        let shares = count_of("short position", sym, s.shares)?;
        let float = count_of("float", sym, s.float)?;
        let short_volume = count_of("short volume", sym, s.short_volume)?;
        let total_volume = count_of("volume", sym, s.total_volume)?;
        let less = |a: Option<crate::wire::Dec>, b: Option<crate::wire::Dec>| -> Result<Option<crate::wire::Dec>, String> {
            match (a, b) {
                (Some(a), Some(b)) => a.0.checked_sub(b.0).map(|d| Some(crate::wire::Dec(d))).map_err(|e| format!("{sym}: {e}")),
                _ => Ok(None),
            }
        };
        let over = |a: Option<crate::wire::Dec>, b: Option<crate::wire::Dec>| match (a, b) {
            (Some(a), Some(b)) if !b.0.is_zero() => Some(a.0.to_f64() / b.0.to_f64()),
            _ => None,
        };
        Ok(ShortsView {
            symbol: s.symbol.clone(),
            exchange: s.exchange.clone(),
            market: s.market,
            name: s.name.clone(),
            as_of: s.as_of.clone(),
            shares,
            previous: count_of("previous short position", sym, s.previous)?,
            previous_of: s.previous_of.clone(),
            change: count_of("change in the short position", sym, s.change)?,
            float,
            unshorted: less(float, shares)?,
            of_float: over(shares, float),
            average_volume: count_of("average volume", sym, s.average_volume)?,
            days_to_cover: s.days_to_cover,
            volume_of: s.volume_of.clone(),
            volume_span: s.volume_span,
            short_volume,
            total_volume,
            long_volume: less(total_volume, short_volume)?,
            of_volume: over(short_volume, total_volume),
            series: match &s.series {
                None => None,
                Some(points) => Some(points.iter().map(|p| Ok(ShortReport { date: p.date.clone(), shares: count_of("reported position", sym, Some(p.shares))?.expect("a count given") })).collect::<Result<Vec<_>, String>>()?),
            },
            fetched_at: r.fetched_at.clone(),
        })
    }
}

/// What `shorts_payload` sends a page asking for one listing's short selling.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShortsPayload {
    #[ts(type = "true")]
    pub ok: bool,
    pub covered: bool,
    pub shorts: Option<ShortsView>,
}

/// `GET /api/shorts`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum ShortsAnswer {
    Ok(ShortsPayload),
    Refused(OkOr),
}

impl From<Result<ShortsPayload, String>> for ShortsAnswer {
    fn from(r: Result<ShortsPayload, String>) -> ShortsAnswer {
        match r {
            Ok(d) => ShortsAnswer::Ok(d),
            Err(e) => ShortsAnswer::Refused(OkOr::err(e)),
        }
    }
}

/// One listing's short selling, from the store at once where it was read before.
pub fn shorts_payload(app: &Arc<App>, key: &bagholder_core::ListingKey, exchange: &str, trend: bool) -> Result<ShortsPayload, String> {
    let sym = key.symbol().to_string();
    let c = conn(app)?;
    let (listed_as, known_ex, _) = instrument_meta(app, key);
    // the venue in words the regulators' readers take: the request's, else the book's
    let ex = if exchange.trim().is_empty() { known_ex } else { exchange.trim().to_string() };
    if shorts::market_of(&sym, &ex).is_none() {
        return Ok(ShortsPayload { ok: true, covered: false, shorts: None });
    }
    if let Some(held) = sf::shorts_for(&c, key).map_err(|e| format!("{}'s short selling held could not be read: {e}", key.named()))? {
        if !trend || held.shorts.series.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
            if shorts_stale(&held) {
                let (k2, e2, n2) = (key.clone(), ex.clone(), listed_as.clone());
                let a = app.clone();
                app.kick(&format!("shorts:{key}"), move || {
                    read_shorts_said(&a, &k2, &e2, true, &n2);
                });
            }
            return Ok(ShortsPayload { ok: true, covered: true, shorts: Some(ShortsView::of(&held)?) });
        }
    }
    match read_shorts(app, key, &ex, trend, &listed_as)? {
        Some(rec) => Ok(ShortsPayload { ok: true, covered: true, shorts: Some(ShortsView::of(&rec)?) }),
        None => Ok(ShortsPayload { ok: true, covered: false, shorts: None }),
    }
}

/// One listing's short selling as a feed across listings carries it: the
/// listing it belongs to, and the holding it opens.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = symbol)]
pub struct ShortsFeedRow {
    #[serde(flatten)]
    #[ts(flatten)]
    pub shorts: ShortsView,
    pub position_id: Option<String>,
    pub held: bool,
    pub watched: bool,
}

/// Every held or watched listing's stored short selling, each marked held or
/// watched.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct ShortsFeed {
    #[ts(type = "true")]
    pub ok: bool,
    pub rows: Vec<ShortsFeedRow>,
    pub reading: bool,
}

/// Every held or watched listing's stored short
/// selling, each marked held or watched.
pub fn shorts_feed(app: &Arc<App>) -> ShortsFeed {
    let reading = app.feeds.shorts_left.load(Ordering::SeqCst) > 0;
    // the book not opening and its listings not read are said in the header by `conn` and `base`
    let (c, b) = match (conn(app), base(app)) { (Ok(c), Ok(Some(b))) => (c, b), _ => return ShortsFeed { ok: true, rows: vec![], reading } };
    // what the feed says of a listing: its name, its venue, and the holding it opens
    struct Known {
        name: String,
        exchange: String,
        position_id: Option<String>,
    }
    let mut held: HashMap<(String, String), Known> = HashMap::new();
    let mut watched: HashMap<(String, String), Known> = HashMap::new();
    for p in b.positions.iter().filter(|p| p.kind == bagholder_model::activity::Kind::Shares) {
        held.insert((tmx_symbol(&p.symbol).to_uppercase(), p.exchange.to_uppercase()), Known { name: p.name.clone(), exchange: p.exchange.clone(), position_id: Some(p.id.clone()) });
    }
    for w in b.watchlist.iter() {
        watched.insert((tmx_symbol(&w.symbol).to_uppercase(), w.exchange.to_uppercase()), Known { name: w.name.clone(), exchange: w.exchange.clone(), position_id: None });
    }
    let mut rows = Vec::new();
    let mut failed = vec![];
    let held_rows = match sf::all_shorts(&c) {
        Ok(r) => r,
        Err(e) => {
            feed_failed(app, SHORTS, format!("The short selling held could not be read: {e}"));
            return ShortsFeed { ok: true, rows: vec![], reading };
        }
    };
    for mut r in held_rows {
        let key = (r.shorts.symbol.clone(), r.shorts.exchange.clone());
        let source = match held.get(&key).or_else(|| watched.get(&key)) { Some(x) => x, None => continue };
        if r.shorts.shares.is_none() {
            continue;
        }
        r.shorts.name = if source.name.is_empty() { r.shorts.name.clone() } else { source.name.clone() };
        r.shorts.exchange = if source.exchange.is_empty() { key.1.clone() } else { source.exchange.clone() };
        match ShortsView::of(&r) {
            Ok(v) => rows.push(ShortsFeedRow { position_id: source.position_id.clone(), held: held.contains_key(&key), watched: watched.contains_key(&key), shorts: v }),
            Err(e) => failed.push(e),
        }
    }
    // said until a read of them all succeeds
    went(app, SHORTS, if failed.is_empty() { Ok(()) } else { Err(failed.join(" ")) });
    ShortsFeed { ok: true, rows, reading: app.feeds.shorts_left.load(Ordering::SeqCst) > 0 }
}

/// `GET /api/listing`: refused, the holding's own id when the symbol is one
/// held, or the listing's page in full.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(untagged)]
pub enum ListingAnswer {
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
    Held {
        #[ts(type = "true")]
        ok: bool,
        symbol: String,
        #[serde(rename = "positionId")]
        position_id: String,
    },
    Full {
        #[ts(type = "true")]
        ok: bool,
        symbol: String,
        exchange: String,
        currency: String,
        kind: String,
        name: String,
        #[serde(rename = "securityId")]
        security_id: String,
        fills: Vec<crate::wire::figures::Fill>,
        /// Its price for a glance, where its source gave one.
        price: Option<crate::wire::Dec>,
        /// The day's change, as a fraction.
        #[serde(rename = "percentChange")]
        percent_change: Option<f64>,
        /// Why there is no price, when its source did not answer.
        #[serde(rename = "priceFailed")]
        price_failed: Option<String>,
    },
}

impl ListingAnswer {
    fn err(e: impl Into<String>) -> ListingAnswer {
        ListingAnswer::Err { ok: false, error: e.into() }
    }
}

/// What the page for one listing needs, held or
/// not.
pub fn listing_payload(app: &Arc<App>, key: &bagholder_core::ListingKey, exchange: &str, currency: &str, name: &str) -> ListingAnswer {
    let sym = key.symbol().to_string();
    let b = match base(app) {
        Ok(Some(b)) => b,
        Ok(None) => return ListingAnswer::err("no page has stated its zone yet"),
        Err(e) => return ListingAnswer::err(e),
    };
    let Some(f) = app.figures.get() else { return ListingAnswer::err("the figures are not open") };
    let names = match f.names() {
        Ok(n) => n,
        Err(e) => return ListingAnswer::err(e),
    };
    // the book's rows of that ticker, by the figures' own ids; which of them are this
    // listing's is decided by key below
    let named = |symbol: &str| tmx_symbol(symbol).trim().to_uppercase() == sym;
    let Some((positions, trades)) = f.read(|e| crate::wire::build::listed(e, &names, &named)) else { return ListingAnswer::err("no page has stated its zone yet") };
    let watchlist: Vec<ListedRow> = b.watchlist.iter().filter(|w| named(&w.symbol)).map(|w| ListedRow { symbol: w.symbol.clone(), exchange: w.exchange.clone(), currency: w.currency.clone(), name: w.name.clone(), ..ListedRow::default() }).collect();
    let securities = crate::market_context::securities(app).unwrap_or_else(|e| {
        log(&format!("bagholder: the book's securities could not be read for {sym}: {e}"));
        vec![]
    });
    listing_payload_in(&securities, &positions, &trades, &watchlist, key, exchange, currency, name, &|s, ex, ccy| crate::following::glance(app, s, ex, ccy))
}

/// A row of the book as the listing page reads it: a holding, a trade or a watched listing.
#[derive(Clone, Debug, Default)]
pub struct ListedRow {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub kind: String,
    pub name: String,
    pub security_id: String,
    pub fills: Vec<crate::wire::figures::Fill>,
}

/// `listing_payload` over the book given, with the quote lookup given.
#[allow(clippy::too_many_arguments)]
pub fn listing_payload_in(
    securities: &[Security],
    positions: &[ListedRow],
    trades: &[ListedRow],
    watchlist: &[ListedRow],
    key: &bagholder_core::ListingKey,
    exchange: &str,
    currency: &str,
    name: &str,
    glance: &dyn Fn(&str, &str, &str) -> Result<crate::following::Glanced, String>,
) -> ListingAnswer {
    let sym = key.symbol().to_string();
    let (mut ex, mut ccy) = (exchange.trim().to_string(), currency.trim().to_string());
    // a row is this listing's when its own symbol and venue make this key: another
    // venue's holding, or a row that names no venue, is never taken for it (brief 34, U17)
    let same = |r: &ListedRow| r.kind != "Options" && bagholder_core::ListingKey::on(&r.symbol, &r.exchange).is_ok_and(|k| k == *key);
    if let Some(h) = positions.iter().find(|p| same(p)) {
        return ListingAnswer::Held { ok: true, symbol: sym, position_id: h.id.clone() };
    }
    let trades: Vec<&ListedRow> = trades.iter().filter(|t| same(t)).collect();
    let watched = watchlist.iter().find(|w| same(w));
    let empty = ListedRow::default();
    let known: &ListedRow = trades.first().copied().or(watched).unwrap_or(&empty);
    let meta = meta_of(securities, key);
    if ex.is_empty() {
        ex = known.exchange.clone();
        if ex.is_empty() {
            ex = meta.1.clone();
        }
    }
    if ccy.is_empty() {
        ccy = known.currency.clone();
        if ccy.is_empty() {
            ccy = meta.2.clone();
        }
    }
    let kind = if known.kind.is_empty() { "Shares".to_string() } else { known.kind.clone() };
    // by day, and within a day by time; a fill whose time was not recorded stands first in its day
    let mut fills: Vec<crate::wire::figures::Fill> = trades.iter().flat_map(|t| t.fills.iter().cloned()).collect();
    fills.sort_by(|a, b| (&a.date, &a.when).cmp(&(&b.date, &b.when)));
    let nm = {
        let n = name.trim().to_string();
        if !n.is_empty() { n } else {
            if !known.name.is_empty() { known.name.clone() } else if meta.0 != sym { meta.0.clone() } else { String::new() }
        }
    };
    let (mut price, mut percent_change, mut price_failed) = (None, None, None);
    if kind == "Shares" {
        match glance(&sym, &ex, &ccy) {
            Ok(g) => {
                price = Some(g.price);
                percent_change = g.percent_change;
            }
            Err(e) => price_failed = Some(e),
        }
    }
    ListingAnswer::Full { ok: true, symbol: sym, exchange: ex, currency: ccy, kind, name: nm, security_id: known.security_id.clone(), fills, price, percent_change, price_failed }
}

/// The shares held and the listings watched whose short selling is published,
/// once each by their key: (key, venue in words, currency, name).
pub fn shorts_listings(app: &Arc<App>, scope: &str) -> Vec<(bagholder_core::ListingKey, String, String, String)> {
    let Ok(Some(b)) = base(app) else { return vec![] }; // a failure is said in the header by `base`
    let mut rows: Vec<(&str, &str, &str, &str)> = Vec::new();
    if scope == "holdings" || scope == "all" {
        rows.extend(b.positions.iter().map(|p| (p.symbol.as_str(), p.exchange.as_str(), p.currency.as_str(), p.name.as_str())));
    }
    if scope == "watchlist" || scope == "all" {
        rows.extend(b.watchlist.iter().map(|w| (w.symbol.as_str(), w.exchange.as_str(), w.currency.as_str(), w.name.as_str())));
    }
    let mut seen: HashSet<bagholder_core::ListingKey> = HashSet::new();
    let mut out = Vec::new();
    for (symbol, ex, ccy, name) in rows {
        // a listing that names no venue is read under none
        let Ok(key) = bagholder_core::ListingKey::on(symbol, ex) else { continue };
        if shorts::market_of(key.symbol(), ex).is_none() || !seen.insert(key.clone()) {
            continue;
        }
        out.push((key, ex.to_string(), ccy.to_string(), name.to_string()));
    }
    out
}

/// Keep every held and watched listing's short
/// selling stored and current.
pub fn sweep_shorts(app: &Arc<App>) -> usize {
    let Ok(c) = conn(app) else { return 0 }; // said in the header by `conn`
    let mut due = Vec::new();
    for (key, ex, ccy, name) in shorts_listings(app, "all") {
        // a row held that cannot be read is read again from its source, and the store's failure said until it answers
        let held = went(app, &format!("shorts-store:{key}"), sf::shorts_for(&c, &key).map_err(|e| format!("{}'s short selling held could not be read: {e}", key.named()))).flatten();
        let fresh = held.as_ref().map(|h| h.shorts.series.as_ref().map(|s| !s.is_empty()).unwrap_or(false) && !shorts_stale(h)).unwrap_or(false);
        if !fresh {
            due.push((key, ex, ccy, name));
        }
    }
    let mut done = 0;
    app.feeds.shorts_left.store(due.len() as i64, Ordering::SeqCst);
    app.events.signal(); // the short-interest table says a read is under way
    struct Reset<'a>(&'a App);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.0.feeds.shorts_left.store(0, Ordering::SeqCst);
            self.0.events.signal();
        }
    }
    let _reset = Reset(app);
    for (key, ex, _, name) in due {
        if read_shorts_said(app, &key, &ex, true, &name).is_some() {
            done += 1;
        }
        app.feeds.shorts_left.fetch_sub(1, Ordering::SeqCst);
        app.events.signal();
    }
    done
}

pub fn shorts_sweep_loop(app: Arc<App>) {
    // the short-interest table is the only reader of a sweep: it runs while some page
    // shows that table, starting the moment one does
    while app.events.park_until(&app, || app.events.watched("shorts")) {
        sweep_shorts(&app);
        if app.wait(Duration::from_secs(SHORTS_SWEEP_EVERY_SEC)) {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// universes
// ---------------------------------------------------------------------------

// The market universes feed the heatmap and nothing else, and their sources push
// nothing: the screener is one request, the TSX 60 sixty-odd. So a universe is read
// only while some page shows it (the `universe:<key>` document): at once when it has
// no rows or its rows are older than `UNIVERSE_STALE_SEC`, and again each time they
// come to that age for as long as a page still shows it. Nobody looking, nothing read
// -- not at start, not through a night. A source's attempt, answered or failed, is
// not made again before the same half hour, so a source that is down costs one
// request per half hour, not one per page.

/// How old a universe's rows may be before a page showing it has them read again.
pub const UNIVERSE_STALE_SEC: f64 = 1800.0;

type UniverseSource = bagholder_market::universes::Source;

/// What the market universes' reads have come to: when each source was last asked
/// (answered or not), and each source's failure until it next answers.
#[derive(Default)]
pub struct UniverseReads {
    asked: HashMap<UniverseSource, f64>,
    failed: HashMap<UniverseSource, String>,
}

/// `universe:<key>`: what the last read of the universe's source came to. Its rows
/// are the model's (`markets.universes`); this says only why they may be missing.
#[derive(Clone, Debug, Serialize, TS, Diff)]
pub struct UniverseDoc {
    /// The source's failure, from its last read until it next answers.
    pub failed: Option<String>,
}

/// The universe `key`'s document as it stands; `None` for a key that is not a market
/// universe.
pub fn universe_stored(app: &Arc<App>, key: &str) -> Option<UniverseDoc> {
    let source = UniverseSource::of(key)?;
    let reads = app.feeds.universes.lock().unwrap_or_else(|e| e.into_inner());
    Some(UniverseDoc { failed: reads.failed.get(&source).cloned() })
}

/// Seconds from `now` until a source is due a read: none while any universe it
/// carries has no rows (`read_at` holds `None` for it), else when the oldest has
/// come to `UNIVERSE_STALE_SEC`; and never sooner than that long after the source
/// was last asked.
pub fn universe_due_in(now: f64, read_at: &[Option<f64>], asked: Option<f64>) -> f64 {
    let rows_due = if read_at.iter().any(|t| t.is_none()) { now } else { read_at.iter().flatten().fold(f64::INFINITY, |a, &t| a.min(t)) + UNIVERSE_STALE_SEC };
    let asked_due = asked.map_or(f64::NEG_INFINITY, |t| t + UNIVERSE_STALE_SEC);
    (rows_due.max(asked_due) - now).max(0.0)
}

fn source_due_in(app: &Arc<App>, source: UniverseSource) -> f64 {
    // a universe whose rows' age cannot be read is due: the read that follows says what fails
    let read_at: Vec<Option<f64>> = match conn(app) {
        Ok(c) => source.keys().iter().map(|k| sf::universe_read_at(&c, k).ok().flatten().as_deref().and_then(parse_instant)).collect(),
        Err(_) => source.keys().iter().map(|_| None).collect(),
    };
    let asked = app.feeds.universes.lock().unwrap_or_else(|e| e.into_inner()).asked.get(&source).copied();
    universe_due_in(now_unix(), &read_at, asked)
}

#[cfg(not(test))]
fn fetch_universes(_app: &Arc<App>, source: UniverseSource) -> Result<Vec<(&'static str, Vec<bagholder_model::input::UniverseRow>)>, String> {
    bagholder_market::universes::read(source)
}

/// A test stands in for the sources: it counts the reads, and answers as it was told.
#[cfg(test)]
fn fetch_universes(app: &Arc<App>, source: UniverseSource) -> Result<Vec<(&'static str, Vec<bagholder_model::input::UniverseRow>)>, String> {
    app.feeds.universe_reads.lock().unwrap().push(source);
    if let Some(why) = app.feeds.universe_fails.lock().unwrap().clone() {
        return Err(why);
    }
    Ok(source
        .keys()
        .iter()
        .map(|k| (*k, vec![bagholder_model::input::UniverseRow { symbol: format!("{}1", k.to_uppercase()), name: "A company".into(), value: 1.0, percent_change: Some(0.5), sector: "Technology".into(), country: String::new() }]))
        .collect())
}

/// Read `source` now, unless it was asked within the half hour (by another page's
/// universe from the same source, say): each answer replaces its universe's rows, a
/// failure is kept for the documents until the source next answers.
fn read_universes(app: &Arc<App>, source: UniverseSource) {
    {
        let mut reads = app.feeds.universes.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_unix();
        if reads.asked.get(&source).is_some_and(|t| now - t < UNIVERSE_STALE_SEC) {
            return;
        }
        reads.asked.insert(source, now);
    }
    let outcome = fetch_universes(app, source).and_then(|answered| {
        let c = conn(app)?;
        let now = now_iso();
        for (key, rows) in answered {
            sf::replace_universe(&c, key, &rows, &now).map_err(|e| format!("The {} universe could not be stored: {e}", key))?;
        }
        Ok(())
    });
    {
        let mut reads = app.feeds.universes.lock().unwrap_or_else(|e| e.into_inner());
        match outcome {
            Ok(()) => reads.failed.remove(&source),
            Err(why) => {
                log(&format!("bagholder universes: {why}"));
                reads.failed.insert(source, why)
            }
        };
    }
    app.events.signal(); // the documents say what the read came to
}

/// A page has started showing the universe `key` (`universe:<key>`): read it when it
/// has no rows or they are stale, and again as they go stale, for as long as some
/// page still shows it.
pub fn universe_shown(app: Arc<App>, doc: String, key: String) {
    let Some(source) = UniverseSource::of(&key) else { return };
    spawn("bagholder-universe-shown", move || loop {
        let ran = app.single_flight(&doc.clone(), false, || {
            while app.events.watched(&doc) && !app.stopping() {
                let due_in = source_due_in(&app, source);
                if due_in <= 0.0 {
                    read_universes(&app, source);
                    continue;
                }
                // until the rows come due, or the page stops showing them
                app.events.park_until_or(&app, Duration::from_secs_f64(due_in), || !app.events.watched(&doc));
            }
            true
        });
        // a page that opened the universe again as this one was leaving found it
        // still running and left it to this one
        if !ran || !app.events.watched(&doc) || app.stopping() {
            return;
        }
    });
}

// ---------------------------------------------------------------------------
// market data
// ---------------------------------------------------------------------------

/// The Bank of Canada's rates a chart in another currency than its bars converts
/// with: CAD per US dollar by day, as the book holds them -- the rates the figures
/// use, read from the engine's inputs once the figures are built, from the book
/// before. Read only when a chart's bars need converting.
pub fn bank_rates(app: &Arc<App>) -> history::Rates {
    let weak = Arc::downgrade(app);
    Arc::new(move || {
        let app = weak.upgrade().ok_or("the app has stopped")?;
        let f = app.figures.get().ok_or("the book is not open yet")?;
        let usd = bagholder_core::Currency::USD;
        let as_text = |rates: Option<&std::collections::BTreeMap<bagholder_core::jiff::civil::Date, bagholder_core::Dec>>| -> std::collections::BTreeMap<String, f64> {
            rates.map(|r| r.iter().map(|(d, v)| (d.to_string(), v.to_f64())).collect()).unwrap_or_default()
        };
        match f.read(|e| as_text(e.inputs().facts.rates.by_currency.get(&usd))) {
            Some(r) => Ok(r),
            None => Ok(as_text(f.book()?.rates().map_err(|e| e.to_string())?.get(&usd))),
        }
    })
}

/// A few instruments per call.
pub fn archive_intraday_bars(app: &Arc<App>, limit: Option<usize>) -> Vec<String> {
    app.single_flight("archive", vec![], || {
        // the book not opening and its listings not read are said in the header by `conn` and `base`
        let (c, b) = match (conn(app), base(app)) { (Ok(c), Ok(Some(b))) => (c, b), _ => return vec![] };
        let recs = bagholder_model::symbols_of::intraday_archive_symbols(&b);
        let limit = limit.map(|l| l.max(1)).unwrap_or(history::ARCHIVE_BATCH);
        let (today_s, now, stamp) = bagholder_market::clock_now();
        let rates = bank_rates(app);
        let worked = history::archive_daily(&c, &rates, &recs, &today_s, now, &stamp, limit).and_then(|mut out| {
            out.extend(history::archive_intraday(&c, &rates, &recs, &today_s, now, &stamp, limit)?);
            Ok(out)
        });
        match worked {
            Ok(out) => out,
            // the app's own upkeep, which leaves nothing the person sees wrong and is
            // done at the next pass: logged, never said (`SPEC.md` §4, the header)
            Err(e) => {
                log(&format!("bagholder feeds: the price bars could not be archived: {e}"));
                vec![]
            }
        }
    })
}

pub const ARCHIVE_DUTY: f64 = 1.0;
pub const ARCHIVE_PASS_SEC: f64 = 1.0;
pub const ARCHIVE_MIN_SEC: f64 = 0.5;

/// The calling thread's processor time, or the
/// monotonic clock where the platform has no such counter.
fn cpu_clock() -> f64 {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Timespec {
            tv_sec: i64,
            tv_nsec: std::os::raw::c_long,
        }
        extern "C" {
            fn clock_gettime(clk: std::os::raw::c_int, tp: *mut Timespec) -> std::os::raw::c_int;
        }
        #[cfg(target_os = "macos")]
        const CLOCK_THREAD_CPUTIME_ID: std::os::raw::c_int = 16;
        #[cfg(not(target_os = "macos"))]
        const CLOCK_THREAD_CPUTIME_ID: std::os::raw::c_int = 3;
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: clock_gettime writes one timespec into the pointer given
        if unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0 {
            return ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9;
        }
    }
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// The backfill paced by the processor time it costs.
pub fn archive_loop(app: Arc<App>) {
    let mut delay = 20.0f64;
    let mut batch = history::ARCHIVE_BATCH;
    while !app.wait(Duration::from_secs_f64(delay)) {
        let started = cpu_clock();
        let worked = archive_intraday_bars(&app, Some(batch));
        let spent = (cpu_clock() - started).max(0.0);
        if worked.is_empty() {
            // Nothing is due. The next top-up falls due at a known moment (a stored
            // read passing its age), and new work can otherwise only come from the
            // book gaining a listing: wait for whichever is first.
            let listings = || base(&app).ok().flatten().map(|b| bagholder_model::symbols_of::intraday_archive_symbols(&b));
            let was = listings();
            let due = match (conn(&app), was.as_ref()) {
                (Ok(c), Some(recs)) => {
                    let (today_s, now, _) = bagholder_market::clock_now();
                    match history::archive_next_due_secs(&c, recs, &today_s, now) {
                        Ok(due) => due,
                        Err(e) => {
                            // the app's own upkeep: logged, and tried again when the book changes
                            log(&format!("bagholder feeds: the price bars could not be archived: {e}"));
                            None
                        }
                    }
                }
                _ => None,
            };
            // the listings charted changed: one traded or held that was not
            let moved = || listings() != was;
            match due {
                Some(secs) => { app.events.park_until_or(&app, Duration::from_secs_f64(secs.max(ARCHIVE_MIN_SEC)), moved); }
                None => { app.events.park_until(&app, moved); }
            }
            delay = 0.0;
            batch = history::ARCHIVE_BATCH;
            continue;
        }
        if spent > ARCHIVE_PASS_SEC && batch > 1 {
            batch = (batch / 2).max(1);
        } else if spent < ARCHIVE_PASS_SEC / 3.0 && batch < history::ARCHIVE_BATCH {
            batch = (batch * 2).min(history::ARCHIVE_BATCH);
        }
        delay = ARCHIVE_MIN_SEC.max(spent * ARCHIVE_DUTY);
    }
}

/// The update check, every `update::UPDATE_EVERY` (`SPEC.md` §2, Versions).
pub fn market_loop(app: Arc<App>) {
    while !app.wait(crate::update::UPDATE_EVERY) {
        crate::update::check_for_update_if_due(&app);
    }
}

pub const WATCH_SCAN_SEC: u64 = 10 * 60;

pub fn watch_loop(app: Arc<App>) {
    // Only while a folder is watched and the figures are built; until then, this
    // waits for them. The folder itself is looked at on a period: the standard
    // library has no file-system notification (docs/architecture.md, "Timers that remain").
    let ready = || app.figures.get().is_some_and(|f| f.read(|_| ()).is_some() && crate::csv_import::watching(f));
    while app.events.park_until(&app, ready) {
        if let Some(f) = app.figures.get() {
            // a failure is kept with the folder, and the folder's dialog says it
            if let Err(e) = crate::csv_import::scan(f, false, bagholder_core::jiff::Timestamp::now()) {
                crate::app::log(&format!("bagholder: the watched folder: {e}"));
            }
        }
        if app.wait(Duration::from_secs(WATCH_SCAN_SEC)) {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// chart history
// ---------------------------------------------------------------------------

fn decode(t: &str) -> String {
    let b = t.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => { out.push(b' '); i += 1 }
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => { out.push(v); i += 3 }
                    None => { out.push(b[i]); i += 1 }
                }
            }
            x => { out.push(x); i += 1 }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// `parse_qs(query).get(k)[0].strip()`: blank values are dropped, as parse_qs
/// drops them.
pub fn qs_one(query: &str, name: &str) -> String {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if decode(k) == name {
            let v = decode(v);
            if !v.is_empty() {
                return v.trim().to_string();
            }
        }
    }
    String::new()
}

/// Bars for one instrument over a span, fetched
/// and cached on demand.
/// Whether the intraday bars this chart asked for are still being read. What the
/// chart watches (`docs`) in place of asking for its history again every few seconds.
/// What a chart watches while it is shown (`history:<its query>`): whether any of
/// its listing's bars are being read, and the timeframes its listing offers now.
/// A miss recorded or bars stored for an intraday timeframe changes what is
/// offered, and that change reaches the chart showing that listing alone.
pub fn history_doc(app: &Arc<App>, q: &HistoryQuery) -> crate::docs::HistoryDoc {
    let (rec, start, _, _) = q.read();
    let Ok(inst) = history::chart_instrument(&rec).keyed() else {
        return crate::docs::HistoryDoc { pending: false, available: vec![] };
    };
    let (today_s, now, _) = bagholder_market::clock_now();
    let available = match conn(app).map_err(|e| e.to_string()).and_then(|c| history::offered_timeframes(&c, &inst, &start, &today_s, now).map_err(|e| e.to_string())) {
        Ok(a) => a.into_iter().map(String::from).collect(),
        Err(e) => {
            feed_failed(app, "bars", format!("The bars could not be read: {e}"));
            vec![]
        }
    };
    let pending = history_pending(app, q) || history::daily_pending(&inst) || history::intraday_pending(&inst);
    crate::docs::HistoryDoc { pending, available }
}

pub fn history_pending(app: &Arc<App>, q: &HistoryQuery) -> bool {
    let (rec, start, _, tf) = q.read();
    // a listing that names no venue is read under none, so nothing is pending
    let Ok(inst) = history::chart_instrument(&rec).keyed() else { return false };
    if !history::INTRADAY_SECONDS.iter().any(|(k, _)| *k == tf) {
        return history::daily_pending(&inst);
    }
    let (today_s, now, _) = bagholder_market::clock_now();
    conn(app).is_ok_and(|c| match history::intraday_ready(&c, &inst, &tf, &start, &today_s, now) {
        Ok(ready) => !ready,
        // nothing is being read that could be waited for: the failure is said
        Err(e) => {
            feed_failed(app, "bars", format!("The bars could not be read: {e}"));
            false
        }
    })
}

/// A chart's question: the listing, the span and the timeframe. The page asks
/// `GET /api/history` with it, and watches `history:<the same query>`.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(default)]
pub struct HistoryQuery {
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub kind: String,
    pub from: String,
    pub to: String,
    pub tf: String,
}

impl HistoryQuery {
    /// Read from a query string, as the `history:` document key carries it.
    pub fn parse(query: &str) -> HistoryQuery {
        let one = |k: &str| qs_one(query, k);
        HistoryQuery { symbol: one("symbol"), exchange: one("exchange"), currency: one("currency"), kind: one("kind"), from: one("from"), to: one("to"), tf: one("tf") }
    }

    /// The listing (CAD and Shares when unsaid), the span's two days and the
    /// timeframe (daily when unsaid), each trimmed as a query value is.
    pub(crate) fn read(&self) -> (bagholder_model::input::Listing, String, String, String) {
        let or = |v: &str, d: &str| { let v = v.trim(); if v.is_empty() { d.to_string() } else { v.to_string() } };
        let rec = bagholder_model::input::Listing::new(self.symbol.trim().to_string(), self.exchange.trim().to_string(), or(&self.currency, "CAD"), or(&self.kind, "Shares"));
        let day = |d: &str| d.trim().chars().take(10).collect::<String>();
        (rec, day(&self.from), day(&self.to), or(&self.tf, "1d"))
    }
}

/// What a chart is sent for a span: the answer `history_payload` builds on
/// success, generated to the page as `web/src/lib/generated/chart.ts`
/// (`BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_chart_types`).
/// The failure case stays an ad hoc `{"ok": false, "error": ...}`, as it
/// always has.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChartHistory {
    pub ok: bool,
    pub symbol: String,
    pub chart_symbol: String,
    pub source: String,
    pub tf: String,
    pub available: Vec<String>,
    pub bars: ChartBars,
    pub pending: bool,
    pub reason: String,
}

/// `GET /api/history`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum HistoryAnswer {
    Ok(ChartHistory),
    Refused(OkOr),
}

/// Why a timeframe is not offered for a listing, named as its button is (`1H`):
/// its sources' own failure where one is noted, else that none had its bars.
fn not_offered(inst: &bagholder_model::input::Keyed, tf: &str) -> String {
    format!("{} is not offered for this listing now. {}", tf.to_uppercase(), history::chart_reason(inst, tf))
}

/// A background read of a chart's bars ended: a store that failed it is said in
/// the header until a read goes through.
fn bars_read(app: &Arc<App>, read: Result<(), String>) {
    match read {
        Ok(()) => feed_answered(app, "bars"),
        Err(e) => feed_failed(app, "bars", e),
    }
}

/// What a listing whose records name no venue shows where its market data would
/// be: it is read under no guessed venue (plan `market-data-keyed-by-listing`,
/// Approach, 5).
pub const NO_VENUE: &str = "Its market data waits for its venue: no record names the exchange it is listed on.";

/// What a listing whose venue gives no key shows: that no record names its venue,
/// or the venue its records do name, which no source here covers.
pub fn no_venue(exchange: &str) -> String {
    match exchange.trim() {
        "" => NO_VENUE.to_string(),
        named => format!("Its market data waits for its venue: its records name {named}, an exchange no source here covers."),
    }
}

pub fn history_payload(app: &Arc<App>, q: &HistoryQuery) -> HistoryAnswer {
    let (rec, start, end, tf) = q.read();
    if rec.symbol.is_empty() || start.chars().count() != 10 || end.chars().count() != 10 || !history::TIMEFRAMES.contains(&tf.as_str()) {
        return HistoryAnswer::Refused(OkOr::err("symbol, from, to and a known tf are required"));
    }
    // a listing that names no venue gets no market data under a guessed one: its
    // chart says what it waits for
    let inst = match history::chart_instrument(&rec).keyed() {
        Ok(inst) => inst,
        Err(_) => {
            let reason = no_venue(&rec.exchange);
            return HistoryAnswer::Ok(ChartHistory {
                ok: true,
                chart_symbol: rec.symbol.clone(),
                symbol: rec.symbol,
                source: String::new(),
                tf,
                available: vec![],
                bars: ChartBars::default(),
                pending: false,
                reason,
            })
        }
    };
    let src = history::history_source(&inst);
    let (today_s, now, stamp) = bagholder_market::clock_now();
    let (c, p) = match (conn(app), pool(app)) {
        (Ok(c), Ok(p)) => (c, p),
        (Err(e), _) | (_, Err(e)) => return HistoryAnswer::Refused(OkOr::err(e)),
    };
    let c = &c;
    let rates = bank_rates(app);
    let available: Vec<&'static str> = match history::offered_timeframes(c, &inst, &start, &today_s, now) {
        Ok(a) => a,
        Err(e) => return HistoryAnswer::Refused(OkOr::err(format!("The bars could not be read: {e}"))),
    };
    // a timeframe the listing does not offer is refused with why, never answered
    // with another timeframe's bars or an empty chart
    if src.is_some() && !available.contains(&tf.as_str()) {
        return HistoryAnswer::Refused(OkOr::err(not_offered(&inst, &tf)));
    }
    let mut pending = false;
    let bars: ChartBars = {
        {
            if !(src.is_some() && available.contains(&tf.as_str())) {
                ChartBars::default()
            } else if history::INTRADAY_SECONDS.iter().any(|(k, _)| *k == tf)
                && !match history::intraday_ready(c, &inst, &tf, &start, &today_s, now) {
                    Ok(ready) => ready,
                    Err(e) => return HistoryAnswer::Refused(OkOr::err(format!("The bars could not be read: {e}"))),
                }
            {
                let told = app.clone();
                // its end is told: what the listing offers may have changed with it
                history::ensure_intraday_in_background(p.clone(), rates.clone(), inst.clone(), tf.clone(), start.clone(), end.clone(), move |read| {
                    bars_read(&told, read);
                    told.events.signal();
                });
                pending = true;
                ChartBars::default()
            } else if history::INTRADAY_SECONDS.iter().any(|(k, _)| *k == tf) {
                match history::ensure_bars(c, &rates, &inst, &tf, &start, &end, &today_s, now, &stamp) {
                    Ok(b) => b,
                    Err(e) => return HistoryAnswer::Refused(OkOr::err(format!("The bars could not be read: {e}"))),
                }
            } else {
                // a day's bars are answered as stored, never after a read of the sources: a
                // read that is due runs in the background and the page is told when it ends
                let due = match history::daily_due(c, &inst, &start, &end, &today_s, now) {
                    Ok(due) => due,
                    Err(e) => return HistoryAnswer::Refused(OkOr::err(format!("The bars could not be read: {e}"))),
                };
                if due {
                    let signal = app.clone();
                    history::ensure_daily_in_background(p.clone(), rates.clone(), inst.clone(), start.clone(), move |read| {
                        bars_read(&signal, read);
                        signal.events.signal();
                    });
                }
                pending = history::daily_pending(&inst);
                let daily = match history::stored_daily(c, &inst, &start, &end) {
                    Ok(d) => d,
                    Err(e) => return HistoryAnswer::Refused(OkOr::err(format!("The bars held could not be read: {e}"))),
                };
                ChartBars::Days(if tf == "1d" { daily } else { history::aggregate_daily(&daily, &tf) })
            }
        }
    };
    let reason = if !bars.is_empty() || pending { String::new() } else { history::chart_reason(&inst, &tf) };
    let payload = ChartHistory {
        ok: true,
        symbol: rec.symbol,
        chart_symbol: inst.symbol.clone(),
        source: src.map(|x| x.0).unwrap_or_default(),
        tf,
        available: available.into_iter().map(|s| s.to_string()).collect(),
        bars,
        pending,
        reason,
    };
    HistoryAnswer::Ok(payload)
}

// ---------------------------------------------------------------------------
// tests: filings and the listing page
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::f;
    use crate::tests_common::app;
    use serde_json::{json, Value};

    /// An app of its own on a home of its own, the recorded month in its book and
    /// its figures built.
    fn built_app() -> (tempfile::TempDir, Arc<App>) {
        crate::tests_common::home(); // offline, dry orders
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let at: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = crate::figures::Figures::open(home.path(), at).unwrap();
        f.state_zone("America/Toronto", at).unwrap();
        app.set_figures(f);
        (home, app)
    }

    /// What a listing offers is its own chart document's to say: for generated
    /// listings and each intraday timeframe, a miss recorded and then bars stored
    /// each change that listing's document once (what it offers) and no other
    /// listing's, and each write reaches the bus every page's stream waits on
    /// (brief 52, 1). A timeframe it no longer offers is refused with why.
    #[test]
    fn a_miss_recorded_or_bars_stored_changes_that_listings_chart_document_once_and_no_others() {
        let (_home, app) = built_app();
        let (today, _, stamp) = bagholder_market::clock_now();
        let from = bagholder_model::dates::shift_date(&today, -10);
        // generated listings on the venues with intraday sources, symbols drawn at random
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let venues = [("TSX", "CAD"), ("TSX-V", "CAD"), ("NASDAQ", "USD"), ("NYSE", "USD")];
        let queries: Vec<HistoryQuery> = (0..12u8)
            .map(|i| {
                let n = next();
                let len = 1 + (n % 4) as usize;
                let symbol: String = (0..len).map(|k| (b'A' + ((n >> (8 * (k + 1))) % 26) as u8) as char).chain(std::iter::once((b'A' + i) as char)).collect();
                let (exchange, currency) = venues[(n >> 56) as usize % venues.len()];
                HistoryQuery::parse(&format!("symbol={symbol}&exchange={exchange}&currency={currency}&kind=Shares&from={from}&to={today}&tf=1d"))
            })
            .collect();
        let docs = || queries.iter().map(|q| history_doc(&app, q)).collect::<Vec<_>>();
        let c = conn(&app).unwrap();
        for (i, q) in queries.iter().enumerate() {
            let key = history::chart_instrument(&q.read().0).keyed().unwrap().key;
            for tf in ["1h", "4h"] {
                let before = docs();
                assert!(before[i].available.iter().any(|a| a == tf), "{} offers {tf} before its miss: {:?}", q.symbol, before[i].available);
                let rung = app.events.stamp();
                history::record_intraday_miss(&c, &key, tf, &stamp).unwrap();
                assert_ne!(app.events.stamp(), rung, "a miss recorded reaches the bus");
                let missed = docs();
                let without: Vec<String> = before[i].available.iter().filter(|a| *a != tf).cloned().collect();
                assert_eq!(bagholder_model::patch::typed(&before[i], &missed[i]), vec![json!(["set", ["available"], without])], "{} {tf}: one change, what it offers", q.symbol);
                for (j, (b, a)) in before.iter().zip(&missed).enumerate() {
                    if j != i {
                        assert!(bagholder_model::patch::typed(b, a).is_empty(), "{}'s miss changed {}'s document", q.symbol, queries[j].symbol);
                    }
                }
                // asked for now, the timeframe is refused with why, never answered with other bars
                let asked = HistoryQuery { tf: tf.into(), ..HistoryQuery::parse(&format!("symbol={}&exchange={}&currency={}&kind=Shares&from={from}&to={today}", q.symbol, q.exchange, q.currency)) };
                match history_payload(&app, &asked) {
                    HistoryAnswer::Refused(r) => assert!(serde_json::to_string(&r).unwrap().contains(&format!("{} is not offered", tf.to_uppercase())), "{}", serde_json::to_string(&r).unwrap()),
                    HistoryAnswer::Ok(h) => panic!("{} {tf} answered with {} bars after its miss", q.symbol, h.tf),
                }
                let rung = app.events.stamp();
                let bar = bagholder_store::bars::TimeBar { time: 1_700_000_000, px: bagholder_store::bars::Ohlcv { open: Some(1.0), high: Some(1.0), low: Some(1.0), close: 1.0, volume: None } };
                bagholder_store::market::upsert_price_bars(&c, &key, tf, &[bar], "test").unwrap();
                assert_ne!(app.events.stamp(), rung, "bars stored reach the bus");
                let stored = docs();
                assert_eq!(bagholder_model::patch::typed(&missed[i], &stored[i]), vec![json!(["set", ["available"], before[i].available])], "{} {tf}: bars stored offer it again", q.symbol);
                for (j, (b, a)) in missed.iter().zip(&stored).enumerate() {
                    if j != i {
                        assert!(bagholder_model::patch::typed(b, a).is_empty(), "{}'s bars changed {}'s document", q.symbol, queries[j].symbol);
                    }
                }
            }
        }
    }

    /// A chart that cannot be keyed says why: no record names its venue, or the
    /// venue its records name is none a source covers. An OTC listing is keyed, and
    /// is charted as any other.
    #[test]
    fn a_chart_with_no_key_names_the_real_reason_and_an_otc_listing_is_charted() {
        let (_home, app) = built_app();
        let reason = |exchange: &str| match history_payload(&app, &HistoryQuery::parse(&format!("symbol=ABC&exchange={exchange}&currency=USD&kind=Shares&from=2025-10-01&to=2025-11-19&tf=1d"))) {
            HistoryAnswer::Ok(h) => h.reason,
            HistoryAnswer::Refused(_) => panic!("refused"),
        };
        assert_eq!(reason(""), NO_VENUE);
        assert!(reason("LSE").contains("its records name LSE"), "{}", reason("LSE"));
        for otc in ["OTC", "OTCM", "PINX", "OTCQX"] {
            assert!(!reason(otc).contains("waits for its venue"), "{otc}: {}", reason(otc));
        }
    }

    /// The exposure pass waits on what the book reports of its own changes: a
    /// holding that came or a listing watched starts it at once; a quote, which
    /// moves a holding's figures and not the listings, does not.
    #[test]
    fn a_new_holding_or_a_watched_listing_starts_the_exposure_read_at_once() {
        let (_home, app) = built_app();
        let f = app.figures.get().unwrap();
        let t: bagholder_core::jiff::Timestamp = "2025-11-19T21:10:00Z".parse().unwrap();
        let at = listings_at(&app);
        assert!(!listings_moved(&app, at), "nothing changed");
        let held = f.read(|e| e.figures().positions[0].instrument).unwrap();
        let cur = f.read(|e| e.figures().positions[0].currency).unwrap();
        f.cache().unwrap().store_quote(&bagholder_sources::cache::StoredQuote { instrument: held, source: bagholder_core::SourceName::named("tmx"), price: bagholder_core::Money::new(bagholder_core::Dec::parse("123.45").unwrap(), cur), change: None, change_pct: None, quoted_at: t, allowance: Default::default(), received_at: t }).unwrap();
        assert!(!f.price_changed(held).unwrap().is_empty(), "the quote moved the holding's figures");
        assert!(!listings_moved(&app, at), "a quote is no new listing");
        crate::entries::enter(f, &crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: "ZZQQ".into(), exchange: "NASDAQ".into(), currency: "USD".into(), day: "2025-11-19".into(), side: "buy".into(), quantity: "1".into(), price: "2".into(), fee: String::new() }, t).unwrap();
        assert!(listings_moved(&app, at), "a holding that came");
        // a listing watched: a pass parked on the listings wakes on it, by the change
        // itself and not by a clock (parked before or after, it ends)
        let at = listings_at(&app);
        let parked = {
            let app = app.clone();
            std::thread::spawn(move || app.events.park_until(&app, || listings_moved(&app, at)))
        };
        app.followed();
        assert!(parked.join().unwrap(), "woken by the listing watched, the app still running");
    }

    /// A chart in another currency than its bars converts with the Bank's rates as
    /// the book holds them (the rates the figures use), not a table of the earlier store.
    #[test]
    fn a_chart_converts_with_the_books_bank_rates() {
        let (_home, app) = built_app();
        let f = app.figures.get().unwrap();
        let t: bagholder_core::jiff::Timestamp = "2025-11-19T21:10:00Z".parse().unwrap();
        let usd = bagholder_core::Currency::USD;
        let (a, b): (bagholder_core::jiff::civil::Date, bagholder_core::jiff::civil::Date) = ("2025-11-17".parse().unwrap(), "2025-11-18".parse().unwrap());
        f.book().unwrap().store_rates(usd, &[(a, bagholder_core::Dec::parse("1.4012").unwrap()), (b, bagholder_core::Dec::parse("1.4050").unwrap())], (a, b), &bagholder_core::SourceName::named("bank-of-canada"), t).unwrap();
        f.rebuild(t).unwrap();
        let rates = bank_rates(&app)().unwrap();
        assert_eq!((rates.get("2025-11-17").copied(), rates.get("2025-11-18").copied()), (Some(1.4012), Some(1.405)));
        let bar = |date: &str| bagholder_store::bars::DayBar { date: date.into(), px: bagholder_store::bars::Ohlcv { open: None, high: None, low: None, close: 10.0, volume: None } };
        let got = history::in_position_currency(&bank_rates(&app), &[bar("2025-11-18"), bar("2025-11-19")], "USD", "CAD").unwrap();
        assert_eq!(got.iter().map(|b| (b.date.as_str(), (b.px.close * 1000.0).round() / 1000.0)).collect::<Vec<_>>(), vec![("2025-11-18", 14.05), ("2025-11-19", 14.05)], "each day at its rate, or the last before it");
    }

    // --- Fear & Greed: what a page showing a meter is told while it is read

    fn gauge(score: f64) -> sf::Gauge {
        sf::Gauge { index: "fear-greed".into(), source: "cnn".into(), score, rating: "Greed".into(), as_of: "2026-09-15".into(), previous: vec![], parts: vec![], series: vec![] }
    }

    #[test]
    fn test_a_meter_being_read_says_so_until_the_read_answers() {
        let _g = crate::tests_common::guard();
        let a = app();
        for index in fear::INDEXES {
            let mut during = None;
            let got = read_fear_with(&a, index, || {
                during = Some(fear_stored(&a, index).reading);
                Ok(gauge(62.0))
            });
            assert_eq!(during, Some(true), "while the read is in the air");
            assert!(got.is_ok());
            let after = fear_stored(&a, index);
            assert!(!after.reading, "once it has answered");
            assert_eq!(after.gauge.map(|g| g.gauge.score), Some(62.0));
        }
    }

    #[test]
    fn test_a_read_that_fails_ends_the_reading_and_keeps_what_was_held() {
        let _g = crate::tests_common::guard();
        let a = app();
        for index in fear::INDEXES {
            read_fear_with(&a, index, || Ok(gauge(40.0))).unwrap();
            let why = format!("{index} could not be reached.");
            assert_eq!(read_fear_with(&a, index, || Err(why.clone().into())).unwrap_err(), why);
            let after = fear_stored(&a, index);
            assert!(!after.reading, "a failed read is not still being read");
            assert_eq!(after.gauge.map(|g| g.gauge.score), Some(40.0));
        }
    }

    #[test]
    fn test_a_failed_read_is_said_in_the_header_over_the_reading_held_until_the_index_answers() {
        let _g = crate::tests_common::guard();
        let a = app();
        read_fear_with(&a, "stocks", || Ok(gauge(40.0))).unwrap();
        // one failed read is not said; the next failing too is
        let _ = read_fear_with(&a, "stocks", || Err("CNN could not be reached.".into()));
        assert!(feed_failures(&a).is_empty(), "one failed read");
        let _ = read_fear_with(&a, "stocks", || Err("CNN could not be reached.".into()));
        assert_eq!(feed_failures(&a), vec!["CNN could not be reached.".to_string()]);
        assert!(crate::status::status(&a).error.contains("CNN could not be reached."), "the header says it");
        assert_eq!(fear_stored(&a, "stocks").gauge.map(|g| g.gauge.score), Some(40.0), "the older reading stays drawn");
        read_fear_with(&a, "stocks", || Ok(gauge(55.0))).unwrap();
        assert!(feed_failures(&a).is_empty(), "gone once the index answers");
        // a read refused by BAGHOLDER_OFFLINE asked nobody: by its kind, never said however often
        for _ in 0..3 {
            let _ = read_fear_with(&a, "crypto", || Err(bagholder_net::Failure::Offline));
        }
        assert!(feed_failures(&a).is_empty(), "a read refused by BAGHOLDER_OFFLINE asked nobody and is not said");
    }

    // --- News: read for a page showing the card, not for any open page

    /// An app of its own on a home of its own, no page open, no notification set on.
    fn own_app() -> (tempfile::TempDir, Arc<App>) {
        let home = tempfile::tempdir().unwrap();
        let a = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        (home, a)
    }

    fn page_showing(a: &Arc<App>, keys: &[&str]) -> crate::events::Feed {
        let feed = crate::events::Feed::open(a.clone());
        let docs = keys.iter().map(|k| (k.to_string(), crate::events::Want { params: json!({}), have: None })).collect();
        assert!(a.events.watch(a, feed.id(), docs));
        feed
    }

    #[test]
    fn test_news_is_owed_to_a_page_showing_the_news_card_and_to_no_other() {
        let (_home, a) = own_app();
        assert!(!news_wanted(&a), "no page open");
        let elsewhere = page_showing(&a, &[]);
        assert!(!news_wanted(&a), "a page open on another tab is not owed the news");
        let news = page_showing(&a, &["news"]);
        assert!(news_wanted(&a), "a page showing the card is");
        drop(news);
        assert!(!news_wanted(&a), "and once it stops, nobody is");
        drop(elsewhere);
    }

    #[test]
    fn test_the_news_document_names_the_listings_the_pass_has_still_to_read() {
        let (_home, a) = own_app();
        let read = |a: &Arc<App>| serde_json::to_value(crate::docs::read(a, "news").expect("the news card's document")).unwrap();
        assert_eq!(read(&a), json!({"reading": []}));
        *a.feeds.news_left.lock().unwrap() = ["*@MARKET".to_string(), "QNC@XTSX".to_string()].into_iter().collect();
        assert_eq!(read(&a), json!({"reading": ["*@MARKET", "QNC@XTSX"]}));
        a.feeds.news_left.lock().unwrap().clear();
        assert_eq!(read(&a), json!({"reading": []}));
    }

    // --- the listing page

    fn fill(when: &str, side: &str, qty: f64, price: f64) -> Value {
        json!({"when": when, "side": side, "qty": qty, "price": price})
    }

    /// A row as the book gives one, from the JSON the cases are written in.
    fn row(r: &Value) -> ListedRow {
        use crate::wire::{Dec, Fig};
        let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
        let dec = |v: &Value, k: &str| Fig::Stated(Dec(bagholder_core::Dec::parse(&v[k].as_f64().unwrap().to_string()).unwrap()));
        ListedRow {
            id: text(r, "id"),
            symbol: text(r, "symbol"),
            exchange: text(r, "exchange"),
            currency: text(r, "currency"),
            kind: text(r, "kind"),
            name: text(r, "name"),
            security_id: text(r, "securityId"),
            fills: r["fills"]
                .as_array()
                .map(|fs| {
                    fs.iter()
                        .map(|x| crate::wire::figures::Fill {
                            id: text(x, "when"),
                            when: Some(text(x, "when")),
                            date: text(x, "when")[..10].to_string(),
                            side: text(x, "side"),
                            sub: String::new(),
                            qty: dec(x, "qty"),
                            price: dec(x, "price"),
                            amount: Fig::Waits { gaps: vec!["value-unstated".into()] },
                            currency: text(r, "currency"),
                            flags: Vec::new(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    const FILL: &str = "2026-03-02T14:31:00Z";
    const LATER: &str = "2026-04-09T15:02:00Z";
    const EARLIER: &str = "2026-01-05T14:40:00Z";

    fn listing(positions: &[Value], trades: &[Value], watchlist: &[Value], q: (&str, f64), args: (&str, &str, &str, &str)) -> Value {
        let rows = |list: &[Value]| -> Vec<ListedRow> { list.iter().map(row).collect() };
        let (positions, trades, watchlist) = (&rows(positions), &rows(trades), &rows(watchlist));
        let quote = move |_: &str, _: &str, _: &str| -> Result<crate::following::Glanced, String> {
            Ok(crate::following::Glanced { price: bagholder_core::Dec::parse(q.0).unwrap().into(), change: None, percent_change: Some(q.1) })
        };
        let key = bagholder_core::ListingKey::on(args.0, args.1).unwrap();
        serde_json::to_value(listing_payload_in(&[], positions, trades, watchlist, &key, args.1, args.2, args.3, &quote)).unwrap()
    }

    /// Each share count as the decimal the source stated, the float's rest and the
    /// short volume's rest worked out exactly, the two ratios the cards draw as
    /// fractions; a count that is not a number fails the whole reading.
    #[test]
    fn test_short_selling_is_sent_in_exact_counts_with_its_ratios() {
        use bagholder_store::feeds::{ShortMarket, ShortPoint, Shorts};
        let stored = |shares: f64| StoredShorts {
            shorts: Shorts {
                symbol: "QNC".into(), exchange: "TSX-V".into(), market: ShortMarket::Ca, name: "Quantum eMotion".into(), as_of: "2026-09-15".into(),
                shares: Some(shares), previous: Some(1_150_000.0), previous_of: "2026-08-31".into(), change: Some(shares - 1_150_000.0),
                float: Some(160_000_000.0), of_float: Some(0.75), average_volume: Some(812_345.5), days_to_cover: Some(1.48),
                volume_of: "2026-09-12".into(), volume_span: None, short_volume: Some(210_000.0), total_volume: Some(900_000.0), volume_pct: Some(23.3),
                series: Some(vec![ShortPoint { date: "2026-08-31".into(), shares: 1_150_000.0 }]),
            },
            fetched_at: "2026-09-15T14:00:00Z".into(),
            read_version: 1,
        };
        let v = serde_json::to_value(ShortsView::of(&stored(1_200_000.0)).unwrap()).unwrap();
        assert_eq!((&v["shares"], &v["change"], &v["float"], &v["unshorted"], &v["averageVolume"]), (&json!("1200000"), &json!("50000"), &json!("160000000"), &json!("158800000"), &json!("812345.5")));
        assert_eq!((&v["shortVolume"], &v["longVolume"], &v["series"]), (&json!("210000"), &json!("690000"), &json!([{"date": "2026-08-31", "shares": "1150000"}])));
        assert_eq!((v["ofFloat"].as_f64(), v["ofVolume"].as_f64()), (Some(0.0075), Some(210_000.0 / 900_000.0)));
        assert!(ShortsView::of(&stored(f64::NAN)).is_err());
    }

    #[test]
    fn test_a_listing_whose_quote_fails_says_so_in_place_of_the_price() {
        let failed = |_: &str, _: &str, _: &str| -> Result<crate::following::Glanced, String> { Err("Yahoo answered 429".into()) };
        let out = serde_json::to_value(listing_payload_in(&[], &[], &[], &[], &bagholder_core::ListingKey::on("RY", "TSX").unwrap(), "TSX", "CAD", "", &failed)).unwrap();
        assert_eq!((&out["ok"], &out["price"], &out["priceFailed"]), (&json!(true), &Value::Null, &json!("Yahoo answered 429")));
    }

    #[test]
    fn test_a_listing_the_book_holds_answers_with_the_holding_whose_page_it_is() {
        let held = [json!({"id": "rt:1", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Quantum eMotion Corp"})];
        let out = listing(&held, &[], &[], ("1.25", -0.02), ("QNC", "TSX-V", "", ""));
        assert_eq!((&out["ok"], &out["positionId"]), (&json!(true), &json!("rt:1")));
    }

    #[test]
    fn test_a_listing_traded_before_carries_the_executions_of_those_trades_in_time() {
        let trades = [
            json!({"id": "t2", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Quantum eMotion Corp",
                   "fills": [fill(FILL, "BUY", 10.0, 5.0), fill(LATER, "SELL", -10.0, 6.5)]}),
            json!({"id": "t1", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "fills": [fill(EARLIER, "BUY", 4.0, 4.0)]}),
            json!({"id": "t3", "symbol": "QNC 16JAN26 5.00 CALL", "underlying": "QNC", "exchange": "TSX-V", "kind": "Options",
                   "fills": [fill("2026-02-02T14:00:00Z", "BUY", 1.0, 1.1)]}),
            json!({"id": "t4", "symbol": "QNC", "exchange": "NYSE", "currency": "USD", "kind": "Shares", "fills": [fill("2026-02-03T14:00:00Z", "BUY", 9.0, 2.2)]}),
        ];
        let out = listing(&[], &trades, &[], ("1.80", 0.015), ("QNC", "TSX-V", "", ""));
        assert!(out.get("positionId").map(|v| v.is_null()).unwrap_or(true));
        let whens: Vec<String> = out["fills"].as_array().unwrap().iter().map(|x| f(x, "when")).collect();
        assert_eq!(whens, [EARLIER, FILL, LATER], "the listing's own trades, oldest first; an option is not the share, and another venue is another listing");
        assert_eq!((f(&out, "name"), f(&out, "exchange"), f(&out, "currency"), f(&out, "kind")),
                   ("Quantum eMotion Corp".into(), "TSX-V".into(), "CAD".into(), "Shares".into()));
        assert_eq!((out["price"].as_str(), out["percentChange"].as_f64()), (Some("1.8"), Some(0.015)));
    }

    #[test]
    fn test_a_listing_never_traded_is_named_by_the_watchlist_and_has_no_executions() {
        let watch = [json!({"symbol": "YES", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Char Technologies Ltd."})];
        let out = listing(&[], &[], &watch, ("0.265", 0.0), ("YES", "TSX-V", "", ""));
        assert_eq!(out["fills"], json!([]));
        assert_eq!((f(&out, "name"), f(&out, "currency")), ("Char Technologies Ltd.".into(), "CAD".into()));
    }

    #[test]
    fn test_a_listing_the_book_has_never_seen_answers_with_what_was_asked_for() {
        let out = listing(&[], &[], &[], ("284.21", -0.0034), ("RY", "TSX", "CAD", "Royal Bank of Canada"));
        assert_eq!((&out["ok"], f(&out, "symbol"), f(&out, "exchange"), f(&out, "name"), &out["fills"]),
                   (&json!(true), "RY".into(), "TSX".into(), "Royal Bank of Canada".into(), &json!([])));
        assert_eq!(out["price"].as_str(), Some("284.21"));
    }

    /// A listing's page is its own venue's: a holding of the ticker on another venue
    /// is not opened for it, and that venue's fills are not merged into it (brief
    /// 34, U17), whichever of the two the page asks for.
    #[test]
    fn test_a_listings_page_never_takes_another_venues_holding_or_fills() {
        let held = [json!({"id": "rt:tsx", "symbol": "TD", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "name": "Toronto-Dominion Bank"})];
        let trades = [
            json!({"id": "t1", "symbol": "TD", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "name": "Toronto-Dominion Bank", "fills": [fill(FILL, "BUY", 10.0, 80.0)]}),
            json!({"id": "t2", "symbol": "TD", "exchange": "NYSE", "currency": "USD", "kind": "Shares", "name": "Toronto-Dominion Bank", "fills": [fill(FILL, "BUY", 5.0, 60.0)]}),
        ];
        let tsx = listing(&held, &trades, &[], ("80", 0.1), ("TD", "TSX", "", ""));
        assert_eq!(tsx["positionId"], "rt:tsx");
        let nyse = listing(&held, &trades, &[], ("60", 0.1), ("TD", "NYSE", "", ""));
        assert!(nyse["positionId"].is_null(), "the TSX holding is not the NYSE listing's");
        let fills = nyse["fills"].as_array().unwrap();
        assert_eq!(fills.len(), 1, "only the NYSE listing's own fills");
        // a row of the book that names no venue is never taken for a listing
        let unplaced = [json!({"id": "t3", "symbol": "TD", "exchange": "", "currency": "CAD", "kind": "Shares", "name": "", "fills": [fill(FILL, "BUY", 1.0, 1.0)]})];
        assert_eq!(listing(&[], &unplaced, &[], ("80", 0.1), ("TD", "TSX", "", ""))["fills"], json!([]));
    }
}
