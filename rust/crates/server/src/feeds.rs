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

use bagholder_market::disclosures::{self, Gathered, SourceOutcome};
use bagholder_market::{edgar, enrich, exposure, fear, history, localmodel, news, sedar, shorts};
use bagholder_model::instruments;
use bagholder_model::venues::{tmx_form, tmx_symbol};
use bagholder_model::securities::Security;
use bagholder_store::bars::ChartBars;
use bagholder_store::feeds::{self as sf, FiledDocument, Filing, NewsItem, Regulator, StoredGauge, StoredShorts};

use bagholder_store::tables::{get_meta, set_meta};

use crate::app::{log, now_iso, now_unix, parse_instant, spawn, App, ENRICH_VERSION};
use crate::http::OkOr;
use crate::notify;

/// The market-data loops' own state: what they are reading now, and for whom.
#[derive(Default)]
pub struct FeedsState {
    /// Bare symbols the running news pass has still to read (`*` the market feed).
    news_left: Mutex<HashSet<String>>,
    /// The listing whose disclosures were asked for last: the one someone is
    /// looking at, and so the one whose documents are read first.
    looking_at: Mutex<String>,
    /// The listing(s) whose disclosures are being read now, by document key.
    reading_now: Mutex<HashMap<String, Vec<String>>>,
    /// What the market universes' reads have come to.
    universes: Mutex<UniverseReads>,
    /// The Fear & Greed indexes being read from their publishers this moment.
    fear_reading: Mutex<HashSet<String>>,
    /// Each feed failing now, by feed, in the header's words, until it next answers
    /// (SPEC §1: a failure is said in the header until its source succeeds).
    failing: Mutex<BTreeMap<String, String>>,
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

/// `feed` has failed: said in the header, in `why`'s words, until it next answers.
/// A request refused by `BAGHOLDER_OFFLINE` asked nobody, so it is no failure of the
/// feed's and is not said (as the market sources' cache does not record one).
pub(crate) fn feed_failed(app: &Arc<App>, feed: &str, why: String) {
    if bagholder_net::client::is_offline_refusal(&why) {
        return;
    }
    let was = app.feeds.failing.lock().unwrap_or_else(|e| e.into_inner()).insert(feed.to_string(), why.clone());
    if was.as_deref() != Some(why.as_str()) {
        app.events.signal();
    }
}

/// `feed` has answered: its failure, if one was standing, is no longer said.
pub(crate) fn feed_answered(app: &Arc<App>, feed: &str) {
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
    let mut shorts: Vec<(String, Vec<&str>)> = Vec::new();
    for (feed, why) in failing.iter() {
        match feed.strip_prefix(SHORTS_OF) {
            Some(symbol) => match shorts.iter_mut().find(|(w, _)| w == why) {
                Some((_, symbols)) => symbols.push(symbol),
                None => shorts.push((why.clone(), vec![symbol])),
            },
            None => out.push(why.clone()),
        }
    }
    out.extend(shorts.into_iter().map(|(why, symbols)| format!("The short interest of {} could not be read: {why}", symbols.join(", "))));
    out
}

/// The header's entry for a listing's short interest, before its symbol.
const SHORTS_OF: &str = "shorts:";

/// The header's entry for the market cache itself, where the feeds are kept: a
/// pass that could not open it says so there until the next open succeeds.
const CACHE: &str = "market-cache";
/// The header's entry for the book the notices are kept in, as the sweeps open it.
const NOTICES_BOOK: &str = "notices-book";
/// The header's entry for the market's context the earlier readers are given.
const CONTEXT: &str = "context";
/// The header's entry for the notification settings the sweeps read.
const NOTICE_SETTINGS: &str = "notify-settings";
/// The header's entries for the disclosures: the sweep and each listing's
/// refresh; the stored lists pages are sent; the reading of documents.
const DISCLOSURES: &str = "disclosures";
const DISCLOSURES_STORED: &str = "disclosures-stored";
const DISCLOSURES_READING: &str = "disclosures-reading";
/// The header's entry for the press releases a wire answered with.
const RELEASES: &str = "releases";
/// The header's entry for the exposure records' pass.
const EXPOSURE: &str = "exposure";
/// The header's entry for the short selling held, as the table reads it.
const SHORTS: &str = "shorts";

/// `feed`'s outcome said: its failure until it next succeeds.
pub(crate) fn went<T>(app: &Arc<App>, feed: &str, r: Result<T, String>) -> Option<T> {
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
fn conn(app: &Arc<App>) -> Result<bagholder_sqlite::pool::Pooled<'_>, String> {
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
fn pool(app: &Arc<App>) -> Result<Arc<bagholder_sqlite::pool::Pool>, String> {
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
fn notices(app: &Arc<App>) -> Result<notify::NoticesBook<'_>, String> {
    let b = notify::book(app).map_err(|e| format!("The book could not be opened for the notifications: {e}"));
    went(app, NOTICES_BOOK, b.as_ref().map(|_| ()).map_err(String::clone));
    b
}

/// The market's context the earlier readers are given (stage 5 moves them): none
/// before the figures are built (no page has stated its zone yet), which is no
/// failure; one that cannot be read is said in the header until it can.
fn base(app: &Arc<App>) -> Result<Option<Arc<bagholder_model::context::MarketBase>>, String> {
    if !app.figures.get().is_some_and(|f| f.read(|_| ()).is_some()) {
        return Ok(None);
    }
    let b = app.market_base().map_err(|e| format!("The book's listings could not be read: {e}"));
    went(app, CONTEXT, b.as_ref().map(|_| ()).map_err(String::clone));
    b.map(Some)
}

/// The notification settings a sweep reads: settings that cannot be read are said
/// in the header, and a sweep reading them does nothing until they can.
fn notice_settings<T>(app: &Arc<App>, r: rusqlite::Result<T>) -> Option<T> {
    went(app, NOTICE_SETTINGS, r.map_err(|e| format!("The notification settings could not be read: {e}")))
}


fn today() -> String {
    bagholder_market::clock_now().0
}

fn sha1_hex12(text: &str) -> String {
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
    let exposure_failed = |e: String| feed_failed(app, "exposure", format!("The exposures could not be refreshed: {e}"));
    let stale = |ids: &[String]| exposure::stale(&ctx, ids);
    let mut todo: Vec<String> = match stale(&held) {
        Ok(t) => t.into_iter().filter(|sid| secs.contains_key(sid)).collect(),
        Err(e) => return exposure_failed(e),
    };
    todo.sort_by_key(|sid| if exposure::is_fund(&secs[sid].name) { 1 } else { 0 });
    let mut unders: Vec<(String, String)> = b
        .positions
        .iter()
        .filter(|p| p.kind == bagholder_model::activity::Kind::Options && !p.underlying.is_empty())
        .map(|p| (p.underlying.to_uppercase(), p.currency.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unders.sort();
    let mut stale_unders = Vec::new();
    for (u, cc) in unders {
        match stale(&[format!("{}{}:{}", exposure::SHARE_KEY, u, tmx_form("", &cc).unwrap_or(""))]) {
            Ok(s) if s.is_empty() => {}
            Ok(_) => stale_unders.push((u, cc)),
            Err(e) => return exposure_failed(e),
        }
    }
    let unders = stale_unders;
    let mut watched: Vec<(String, String, String)> = Vec::new();
    for w in b.watchlist.iter().filter(|w| instruments::find(&w.symbol, &w.exchange).is_none() && w.exchange.to_uppercase() != "CRYPTO") {
        match stale(&[bagholder_model::symbols_of::watch_exposure_key(&w.symbol, &w.exchange, &w.currency)]) {
            Ok(s) if s.is_empty() => {}
            Ok(_) => watched.push((w.symbol.clone(), w.exchange.clone(), w.currency.clone())),
            Err(e) => return exposure_failed(e),
        }
    }

    let mut jobs: Vec<ExposureJob> = todo.iter().map(|sid| ExposureJob::Sec(secs[sid].clone())).collect();
    jobs.extend(unders.into_iter().map(|(u, cc)| ExposureJob::Under(u, cc)));
    jobs.extend(watched.into_iter().map(|(sy, e, cc)| ExposureJob::Watch(sy, e, cc)));
    drop(ctx);
    if jobs.is_empty() {
        feed_answered(app, EXPOSURE);
        return;
    }
    let queue = Arc::new(Mutex::new(jobs.into_iter().collect::<std::collections::VecDeque<_>>()));
    // what each record's read failed on, said together once the pass is over
    let failures: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
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
                            failures.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{sym}: {e}"));
                            continue;
                        }
                    },
                    ExposureJob::Under(under, ccy) => match exposure::share_exposure(&ctx, &under, "", &ccy) {
                        Ok(_) => format!("bagholder exposure: {} (an option's underlying) classified", under),
                        Err(e) => {
                            failures.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{under}: {e}"));
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
    let mut failures = std::mem::take(&mut *failures.lock().unwrap_or_else(|p| p.into_inner()));
    // a source not asked because the app is offline answered nothing, and failed nothing
    failures.retain(|f| !bagholder_net::client::is_offline_refusal(f));
    if stopped > 0 {
        failures.push(format!("{stopped} of the readers stopped short"));
    }
    went(app, EXPOSURE, if failures.is_empty() { Ok(()) } else { Err(format!("The exposure records could not be brought up to date: {}", failures.join("; "))) });
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
    let a = app.clone();
    spawn("watch-sector", move || {
        let (Ok(c), Ok(p)) = (conn(&a), pool(&a)) else { return }; // said in the header by `conn`
        let ctx = exposure::Ctx { conn: &c, pool: p, today: today() };
        if let Err(e) = exposure::share_exposure(&ctx, &symbol, &exchange, &currency) {
            feed_failed(&a, EXPOSURE, format!("The sector of {symbol} could not be read: {e}"));
        }
    });
}

/// Forget the news read for a listing no longer watched.
pub fn forget_news(app: &Arc<App>, symbol: &str, exchange: &str) -> Result<(), String> {
    let c = conn(app)?;
    sf::forget_news(&c, &tmx_symbol(symbol), exchange).map_err(|e| e.to_string())?;
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
/// listings the running pass has still to read (`*` the market feed), so the card
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
        let key = |l: &news::Listing| { let t = tmx_symbol(&l.symbol); (if t.is_empty() { l.symbol.clone() } else { t }).to_uppercase() };
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

pub fn news_symbol_payload(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str) -> NewsSymbolAnswer {
    news_symbol_payload_with(app, symbol, exchange, currency, &news::LIVE_READERS, &|c, sym, today| bagholder_market::tmx::tmx_listing(c, sym, today).map_err(|e| e.to_string()))
}

pub fn news_symbol_payload_with(
    app: &Arc<App>,
    symbol: &str,
    exchange: &str,
    currency: &str,
    readers: &news::Readers,
    listing_of: &dyn Fn(&Connection, &str, &str) -> Result<Option<bagholder_model::wire::SymbolMatch>, String>,
) -> NewsSymbolAnswer {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return NewsSymbolAnswer::err("symbol required");
    }
    let c = match conn(app) {
        Ok(c) => c,
        Err(e) => return NewsSymbolAnswer::err(e),
    };
    let (today_s, now, _) = bagholder_market::clock_now();
    let clock = news::Clock { today: today_s.clone(), now: now as i64 };
    let (mut ex, mut ccy) = (exchange.trim().to_string(), currency.trim().to_string());
    // the name is what Google is searched for: the security record's, else the one TMX's quote gives
    let (known, known_ex, known_ccy) = instrument_meta(app, &sym);
    let mut name = if known != sym { known } else { String::new() };
    if ex.is_empty() {
        // the venue from what the app already knows: the security records the
        // sync brought, then TMX's own resolver, which names the venue it
        // verified by the quote and so covers the venues no public directory
        // carries (the CSE, Cboe Canada). Nothing is guessed: a ticker TMX
        // cannot place is a US one, and Nasdaq keeps only the items that name it.
        ex = known_ex;
        if !known_ccy.is_empty() {
            ccy = known_ccy;
        }
    }
    let mut listing = None;
    if ex.is_empty() || (name.is_empty() && !matches!(tmx_form(&ex, &ccy), None | Some(":US"))) {
        listing = match listing_of(&c, &sym, &today_s) {
            Ok(l) => l,
            Err(e) => return NewsSymbolAnswer::err(format!("{sym} could not be placed: {e}")),
        };
    }
    if let Some(l) = &listing {
        if name.is_empty() && l.name.to_uppercase() != sym {
            name = l.name.clone();
        }
        if ex.is_empty() {
            ex = l.exchange.clone();
            ccy = l.currency.clone();
        }
    }
    if ex.is_empty() {
        let form = match bagholder_market::tmx::tmx_resolve(&c, &tmx_symbol(&sym), &today_s) {
            Ok(f) => f,
            Err(e) => return NewsSymbolAnswer::err(format!("{sym} could not be placed: {e}")),
        };
        if !form.is_empty() && !form.ends_with(":US") {
            if ccy.is_empty() {
                ccy = "CAD".into();
            }
        } else {
            ex = "NASDAQ".into();
            ccy = "USD".into();
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
// disclosures
// ---------------------------------------------------------------------------

pub const FILINGS_STALE_HOURS: f64 = 24.0;
pub const FILINGS_SWEEP_EVERY_SEC: u64 = 300;
pub const FILINGS_HOLD_MAX_MIN: f64 = 120.0;
pub const FILINGS_HOLD_KEY: &str = "filings:held-since";
pub const FILINGS_SWEEP_AGE_MIN: f64 = 30.0;

/// (issuer name, exchange, currency) the book
/// holds for a symbol, else the bare symbol.
pub fn instrument_meta(app: &App, symbol: &str) -> (String, String, String) {
    let secs = match crate::market_context::securities(app) {
        Ok(v) => v,
        Err(e) => {
            log(&format!("bagholder: the book's securities could not be read for {symbol}: {e}"));
            vec![]
        }
    };
    meta_of(&secs, symbol)
}

/// (issuer name, exchange, currency) of the first of `secs` under `symbol`, else the bare symbol.
pub fn meta_of(secs: &[Security], symbol: &str) -> (String, String, String) {
    let sym = symbol.trim().to_uppercase();
    for sec in secs {
        if sec.symbol.trim().to_uppercase() == sym {
            let name = sec.name.trim().to_string();
            return (if name.is_empty() { sym } else { name }, sec.primary_exchange.trim().to_string(), sec.currency.trim().to_string());
        }
    }
    (sym, String::new(), String::new())
}

pub fn filings_stale(c: &Connection, symbol: &str, hours: Option<f64>) -> Result<bool, String> {
    let when = sf::filings_fetched_for(c, symbol).map_err(|e| e.to_string())?;
    if when.is_empty() {
        return Ok(true);
    }
    Ok(match parse_instant(&when) {
        Some(then) => now_unix() - then > hours.unwrap_or(FILINGS_STALE_HOURS) * 3600.0,
        None => true,
    })
}

fn can_name_documents() -> bool {
    if enrich::summary_available() {
        return true;
    }
    let status = enrich::summary_status();
    if localmodel::COMING_UP.contains(&status) {
        return enrich::wait_for_summary(enrich::SUMMARY_WAIT_SEC);
    }
    status != "downloading"
}

/// A bounded hold on what cannot be named yet.
fn naming_held(c: &Connection, holding: bool) -> Result<bool, String> {
    if !holding {
        set_meta(c, FILINGS_HOLD_KEY, "").map_err(|e| e.to_string())?;
        return Ok(false);
    }
    let since = get_meta(c, FILINGS_HOLD_KEY, "").map_err(|e| e.to_string())?;
    if since.is_empty() {
        set_meta(c, FILINGS_HOLD_KEY, &now_iso()).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    Ok(match parse_instant(&since) {
        Some(then) => now_unix() - then <= FILINGS_HOLD_MAX_MIN * 60.0,
        None => true,
    })
}

/// What makes a filing itself.
pub fn filing_mark(r: &Filing) -> [String; 5] {
    [r.doc.source.as_str().to_string(), r.doc.date.clone(), r.doc.form.clone(), r.doc.title.clone(), r.doc.size.clone()]
}

fn providers_cover(sym: &str, ex: &str, ccy: &str) -> bool {
    (sedar::available() && sedar::covers(sym, ex, ccy)) || (edgar::available() && edgar::covers(sym, ex, ccy))
}

/// A ticker as a filings sweep or read considers it: its symbol, venue and
/// what kind of instrument it is, before it is known to be one the sweep
/// tracks.
#[derive(Clone, Debug, Default)]
struct FilingCandidate {
    symbol: String,
    exchange: String,
    currency: String,
    kind: String,
    name: String,
}

/// A ticker known for filings: what a sweep or a single read fetches and
/// files it under.
#[derive(Clone, Debug, Default)]
pub struct FilingSymbol {
    pub symbol: String,
    pub name: String,
    pub exchange: String,
    pub currency: String,
}

/// The tickers to watch for filings; the book's listings not read is an error.
pub fn known_filing_symbols(app: &Arc<App>, scopes: &[String]) -> Result<Vec<FilingSymbol>, String> {
    let has = |k: &str| scopes.iter().any(|x| x == k);
    let mut rows: Vec<FilingCandidate> = Vec::new();
    let mut b = None;
    if has("held") || has("all") {
        b = base(app)?;
    }
    if let Some(b) = &b {
        rows.extend(bagholder_model::symbols_of::held_symbols(b).iter().map(|l| FilingCandidate {
            symbol: l.symbol.clone(),
            exchange: l.exchange.clone(),
            currency: l.currency.clone(),
            kind: l.kind.clone(),
            name: String::new(),
        }));
        if has("all") {
            for t in b.traded.iter() {
                let mut rec = FilingCandidate { symbol: t.symbol.clone(), exchange: t.exchange.clone(), currency: t.currency.clone(), kind: t.kind.to_string(), name: String::new() };
                if rec.kind == "Options" {
                    let under = bagholder_model::symbols::underlying_symbol(&rec.symbol);
                    if under.is_empty() || under == "—" {
                        continue;
                    }
                    rec = FilingCandidate { symbol: under, exchange: rec.exchange, currency: rec.currency, kind: "Shares".to_string(), name: String::new() };
                }
                rows.push(rec);
            }
        }
    }
    if has("watched") || has("all") {
        {
            let watchlist = base(app)?.map(|b| b.watchlist.clone()).unwrap_or_default();
            rows.extend(watchlist.iter().map(|w| FilingCandidate {
                symbol: w.symbol.clone(),
                exchange: w.exchange.clone(),
                currency: w.currency.clone(),
                kind: String::new(),
                name: w.name.clone(),
            }));
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for r in rows {
        let sym = r.symbol.trim().to_uppercase();
        if sym.is_empty() || seen.contains(&sym) || sym.contains(' ') || r.kind == "Options" || r.kind == "Crypto" {
            continue;
        }
        if !providers_cover(&sym, &r.exchange, &r.currency) {
            continue;
        }
        seen.insert(sym.clone());
        out.push(FilingSymbol { symbol: sym, name: r.name, exchange: r.exchange, currency: r.currency });
    }
    Ok(out)
}

/// While a Disclosures or Releases set is on, each
/// chosen ticker not read within half an hour is read again and what is new is
/// told. Returns how many tickers had something new; a read or a write of the
/// store that fails fails the sweep, which is said in the header until the
/// next one succeeds.
pub fn sweep_filings(app: &Arc<App>) -> usize {
    // said in the header by `conn` and `notices`
    let (Ok(c), Ok(book)) = (conn(app), notices(app)) else { return 0 };
    went(app, DISCLOSURES, sweep_filings_in(app, &c, &book).map_err(|e| format!("The disclosures could not be swept: {e}"))).unwrap_or(0)
}

/// The sweep, the disclosures read and kept in the market cache (`c`), what is
/// told of them and the marks of what was met kept in the book.
fn sweep_filings_in(app: &Arc<App>, c: &Connection, book: &bagholder_book::Book) -> Result<usize, String> {
    let scopes = notify::disclosure_scopes(book).map_err(|e| e.to_string())?;
    let rel_scopes = notify::release_scopes(book).map_err(|e| e.to_string())?;
    if scopes.is_empty() && rel_scopes.is_empty() {
        return Ok(0);
    }
    let mut told = 0;
    let disc_syms: HashSet<String> = if scopes.is_empty() { HashSet::new() } else { known_filing_symbols(app, &scopes)?.iter().map(|i| i.symbol.clone()).collect() };
    let hold = !disc_syms.is_empty() && naming_held(&c, !can_name_documents())?;
    let mut both: Vec<String> = scopes.clone();
    for r in &rel_scopes {
        if !both.contains(r) {
            both.push(r.clone());
        }
    }
    for inst in known_filing_symbols(app, &both)? {
        let sym = inst.symbol.clone();
        if !filings_stale(&c, &sym, Some(FILINGS_SWEEP_AGE_MIN / 60.0))? {
            continue;
        }
        if hold && disc_syms.contains(&sym) {
            continue;
        }
        let before: HashSet<[String; 5]> = sf::filings_for(&c, &sym).map_err(|e| e.to_string())?.iter().map(filing_mark).collect();
        let name = inst.name.clone();
        let wrote = refresh_filings(app, &sym, if name.is_empty() { None } else { Some(&name) }, Some(&inst.exchange), Some(&inst.currency))?;
        if wrote < 0 {
            continue;
        }
        let mut by_source: Vec<(Regulator, Vec<Filing>)> = Vec::new();
        for r in sf::filings_for(&c, &sym).map_err(|e| e.to_string())? {
            let src = r.doc.source;
            match by_source.iter_mut().find(|(k, _)| *k == src) {
                Some((_, v)) => v.push(r),
                None => by_source.push((src, vec![r])),
            }
        }
        // what each source shows now, read first; its marks are kept below with what is told of it
        let mut new: Vec<Filing> = Vec::new();
        let mut marks: Vec<(String, Vec<String>, Option<(String, String)>)> = Vec::new();
        for (src, rows) in &by_source {
            let scope = format!("filings:{}:{}", sym, src.as_str());
            let events: Vec<String> = rows.iter().map(|r| filing_mark(r).join("|")).collect();
            let met = sf::events_told(book.notices(), &scope, &events).map_err(|e| e.to_string())?;
            let fresh = notify::fresh_in(book, &scope, rows, |r| r.doc.date.clone(), |r| filing_mark(r).join("|"), |r| {
                before.contains(&filing_mark(r)) || met.contains(&filing_mark(r).join("|"))
            }).map_err(|e| e.to_string())?;
            new.extend(fresh.items);
            marks.push((scope, events, fresh.mark));
        }
        let rel: Vec<Filing> = new.iter().filter(|r| is_news_release(r)).cloned().collect();
        let rest: Vec<Filing> = new.iter().filter(|r| !is_news_release(r)).cloned().collect();
        let mut notices: Vec<(&str, String, String, String, sf::NotificationExtra)> = Vec::new();
        if !rel.is_empty() && in_release_scope(app, &sym, &rel_scopes)? && !sf::has_wire_release(c, &sym).map_err(|e| e.to_string())? {
            let (t, bd) = release_notice(app, &sym, &rel);
            notices.push(("releases", release_key(&sym, &rel), t, bd, notice_extra(&sym, None, &rel)));
        }
        if !rest.is_empty() && disc_syms.contains(&sym) {
            let (t, bd) = filings_notice(app, &sym, &rest);
            let mut marked: Vec<String> = rest.iter().map(|r| filing_mark(r).join("/")).collect();
            marked.sort();
            let digest = sha1_hex12(&marked.join("|"));
            notices.push(("disclosures", format!("filings:{}:{}", sym, digest), t, bd, notice_extra(&sym, None, &rest)));
        }
        // the marks and the notices kept together: a mark not kept would tell these
        // again, a notice not kept would never tell them; either fails the sweep and
        // leaves both as they were
        let now = now_iso();
        let rows = bagholder_sqlite::atomically(book.notices(), || {
            for (scope, events, mark) in &marks {
                notify::keep_mark(book, mark)?;
                sf::mark_told(book.notices(), scope, events, &now)?;
            }
            let mut rows = vec![];
            for (kind, key, t, bd, extra) in &notices {
                rows.extend(notify::record(book, kind, key, t, bd, Some(extra.clone()))?);
            }
            Ok(rows)
        })
        .map_err(|e| format!("what is new in {sym}'s disclosures could not be told: {e}"))?;
        for row in &rows {
            notify::announce(app, row);
        }
        if !rows.is_empty() {
            told += 1;
        }
    }
    Ok(told)
}

fn feed_scope(key: &str) -> Vec<String> {
    vec![match key {
        "holdings" => "held",
        "watchlist" => "watched",
        _ => "all",
    }
    .to_string()]
}

/// One filing as a feed across listings carries it: the filing itself, with
/// which listing it belongs to.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct FeedFiling {
    #[serde(flatten)]
    #[ts(flatten)]
    pub filing: Filing,
    pub symbol: String,
    pub exchange: String,
}

/// The newest disclosures across a set of listings, as a page shows them.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct FilingsFeed {
    #[ts(type = "true")]
    pub ok: bool,
    pub scope: String,
    pub filings: Vec<FeedFiling>,
    pub reading: bool,
}

/// The stored disclosures of every ticker in a set,
/// newest first. The store failing is said in the header until it answers.
pub fn filings_feed(app: &Arc<App>, scope: &str, limit: i64) -> FilingsFeed {
    let key = { let k = scope.trim().to_lowercase(); if k.is_empty() { "all".to_string() } else { k } };
    let read = || -> Result<Vec<FeedFiling>, String> {
        let c = conn(app)?;
        let mut rows: Vec<FeedFiling> = Vec::new();
        for inst in known_filing_symbols(app, &feed_scope(&key))? {
            let sym = inst.symbol.clone();
            for r in fresh_filings(&c, &sym)? {
                rows.push(FeedFiling { filing: r, symbol: sym.clone(), exchange: inst.exchange.clone() });
            }
        }
        Ok(rows)
    };
    let Some(mut rows) = went(app, DISCLOSURES_STORED, read().map_err(|e| format!("The disclosures held could not be read: {e}"))) else {
        return FilingsFeed { ok: true, scope: key, filings: vec![], reading: false };
    };
    rows.sort_by(|a, b| b.filing.doc.date.cmp(&a.filing.doc.date));
    rows.truncate(limit.max(1) as usize);
    FilingsFeed { ok: true, scope: key, filings: rows, reading: enrich::summary_available() }
}

/// The issuers' own news releases as they filed them, for the News card's
/// Releases tab: every listing's in `scope` (`all`, `holdings`, `watchlist`), or
/// the chip's listing's alone. Read as stored, a named document's title as its
/// name gives it, and nothing written: a title is read by the disclosures'
/// own reading.
pub fn filed_releases(app: &Arc<App>, scope: &str, chip: Option<(&str, &str)>) -> Result<Vec<crate::wire::news::FiledRelease>, String> {
    let c = conn(app)?;
    let listings: Vec<(String, String)> = match chip {
        Some((symbol, exchange)) => vec![(symbol.trim().to_uppercase(), exchange.to_string())],
        None => known_filing_symbols(app, &feed_scope(scope))?.into_iter().map(|f| (f.symbol, f.exchange)).collect(),
    };
    let reading = enrich::summary_available();
    let mut out = Vec::new();
    for (symbol, exchange) in listings {
        for mut r in sf::filings_for(&c, &symbol).map_err(|e| e.to_string())? {
            if !is_news_release(&r) {
                continue;
            }
            if r.enrich_version.unwrap_or(0) < ENRICH_VERSION {
                r.subject = String::new();
            }
            if r.subject.is_empty() {
                if let Some(t) = disclosures::quick_title(&r.doc) {
                    r.subject = t;
                }
            }
            out.push(crate::wire::news::FiledRelease {
                pending: r.subject.is_empty() && !r.enrich_final && reading,
                id: r.doc.id.clone(),
                symbol: symbol.clone(),
                exchange: exchange.clone(),
                subject: r.subject.clone(),
                form: r.doc.form.clone(),
                source: r.doc.source.as_str().to_string(),
                url: r.doc.url.clone(),
                date: r.doc.date.clone(),
            });
        }
    }
    Ok(out)
}

/// A filed document that is the company's own
/// press release.
pub fn is_news_release(filing: &Filing) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)news release|press release").unwrap()).is_match(&filing.doc.form)
}

pub fn in_release_scope(app: &Arc<App>, sym: &str, scopes: &[String]) -> Result<bool, String> {
    if scopes.is_empty() {
        return Ok(false);
    }
    if scopes.iter().any(|x| x == "all") {
        return Ok(true);
    }
    let sym = sym.trim().to_uppercase();
    let same = |x: &str| {
        let t = tmx_symbol(x);
        (if t.is_empty() { x.to_string() } else { t }).trim().to_uppercase() == sym
    };
    let Some(b) = base(app)? else { return Ok(false) };
    if scopes.iter().any(|x| x == "held") && b.positions.iter().any(|p| same(&p.symbol)) {
        return Ok(true);
    }
    if scopes.iter().any(|x| x == "watched") {
        return Ok(b.watchlist.iter().any(|w| same(&w.symbol)));
    }
    Ok(false)
}

/// What a notice is told of: a wire's item or a filed document.
pub trait Notable: Clone {
    fn id(&self) -> String;
    /// What it says it is: a headline, else a filing's subject, else its type.
    fn heading(&self) -> String;
    /// When its source dates it.
    fn moment(&self) -> String;
    fn summary(&self) -> String;
    fn url(&self) -> String;
    /// The regulator a filed document came from ("SEDAR+", "SEC", "SEC EDGAR"), opened through the app.
    fn filed_source(&self) -> Option<String>;
    fn is_release(&self) -> bool;
}

impl Notable for NewsItem {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn heading(&self) -> String {
        self.headline.clone()
    }
    fn moment(&self) -> String {
        self.published_at.clone()
    }
    fn summary(&self) -> String {
        self.summary.clone()
    }
    fn url(&self) -> String {
        self.url.clone()
    }
    fn filed_source(&self) -> Option<String> {
        None
    }
    fn is_release(&self) -> bool {
        self.kind == sf::NewsKind::Release
    }
}

impl Notable for Filing {
    fn id(&self) -> String {
        self.doc.id.clone()
    }
    fn heading(&self) -> String {
        // a filing has no headline
        if self.subject.is_empty() { self.doc.form.clone() } else { self.subject.clone() }
    }
    fn moment(&self) -> String {
        self.doc.date.clone()
    }
    fn summary(&self) -> String {
        self.summary.clone()
    }
    fn url(&self) -> String {
        self.doc.url.clone()
    }
    fn filed_source(&self) -> Option<String> {
        Some(self.doc.source.as_str().to_string())
    }
    fn is_release(&self) -> bool {
        is_news_release(self)
    }
}

pub fn release_notice<T: Notable>(app: &Arc<App>, sym: &str, rows: &[T]) -> (String, String) {
    let first = newest_of(rows);
    let mut head = first.heading();
    if head.is_empty() {
        head = "A new release.".into();
    }
    let title = if rows.len() == 1 { "Press release · ".to_string() } else { format!("{} press releases · ", rows.len()) } + sym;
    // A release announcing distributions carries the figures beneath the headline, since the
    // headline alone ("Announces August 2026 Distributions") says nothing a holder can act on.
    let mut detail = if is_distribution_release(&head) { distribution_detail(app, sym) } else { String::new() };
    if detail.is_empty() {
        // what the source said beneath its own headline, where it said anything
        detail = first.summary().trim().to_string();
    }
    if !detail.is_empty() {
        head = format!("{}\n{}", head, detail);
    }
    (title, head)
}

/// A release whose subject is a distribution or a dividend.
pub fn is_distribution_release(headline: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)\b(distribution|distributions|dividend|dividends)\b").unwrap()).is_match(headline)
}

/// `2026-08-31` as `Aug 31`, and a year that is not this one carries it.
pub fn stamp_day(iso: &str) -> String {
    let day = bagholder_model::dates::head10(iso);
    let (y, m, d) = match bagholder_model::dates::parse_iso(&day) { Some(x) => x, None => return String::new() };
    if !(1..=12).contains(&m) || d == 0 || d > bagholder_model::dates::days_in_month(y, m) {
        return String::new();
    }
    let text = format!("{} {}", bagholder_model::dates::MONTHS[(m - 1) as usize], d);
    let this_year: i64 = bagholder_model::clock::now_utc_stamp()[..4].parse().unwrap_or(0);
    if y == this_year { text } else { format!("{} {}", text, y) }
}

/// What a distribution release means for this listing, from the issuer's own
/// declared record: the amount just announced, when it goes ex and when it is
/// paid, and the one it replaces.
///
/// A release headline says only that distributions were announced; the figure
/// is what the holder wants, and reading it from the record rather than the
/// release's prose keeps it the same figure the Cashflow tab pays from.
pub fn distribution_detail(app: &Arc<App>, sym: &str) -> String {
    let Some(f) = app.figures.get() else { return String::new() };
    // the release is the announcement; the record it comes from carries the figures, and it
    // is read now rather than when it is next due, so the notice is not a day behind it.
    // A notice that goes out without its figures says why in the header, until the next
    // one has them.
    let detail = read_record_for_notice(app, f, sym)
        .map_err(|e| format!("{sym}'s declared distributions could not be read again: {e}"))
        .and_then(|read| f.book().map_err(|e| format!("The book could not be read for {sym}'s distributions: {e}")).and_then(|book| distribution_detail_in(&book, &read)));
    went(app, "distributions", detail).unwrap_or_default()
}

/// The payers held under `sym`, their records read again.
#[cfg(not(test))]
fn read_record_for_notice(app: &Arc<App>, f: &crate::figures::Figures, sym: &str) -> Result<Vec<bagholder_core::InstrumentId>, String> {
    crate::due::read_payer_now(app, f, sym, bagholder_core::jiff::Timestamp::now())
}

/// A test counts the reads rather than reaching the source, and finds the
/// instruments by the symbol the book calls them.
#[cfg(test)]
fn read_record_for_notice(app: &Arc<App>, f: &crate::figures::Figures, sym: &str) -> Result<Vec<bagholder_core::InstrumentId>, String> {
    app.feeds.record_reads.fetch_add(1, Ordering::SeqCst);
    if let Some(why) = app.feeds.record_fails.lock().unwrap().clone() {
        return Err(why);
    }
    let book = f.book()?;
    let mut out = vec![];
    for i in book.instruments().map_err(|e| e.to_string())? {
        if book.names(i.id).map_err(|e| e.to_string())?.last().is_some_and(|n| n.symbol.eq_ignore_ascii_case(sym)) {
            out.push(i.id);
        }
    }
    Ok(out)
}

/// A distribution a unit, exactly as the record states it, never fewer than cents.
fn per_unit(m: &bagholder_core::Money) -> String {
    let text = m.amount.to_text();
    let (whole, cents) = text.split_once('.').unwrap_or((&text, ""));
    let sign = if m.currency.as_str() == "USD" { "US$" } else { "$" };
    format!("{sign}{whole}.{cents:0<2}")
}

/// How often a payer pays, in words.
fn frequency_word(per_year: u32) -> String {
    match per_year {
        52 => "weekly".into(),
        12 => "monthly".into(),
        4 => "quarterly".into(),
        2 => "semi-annual".into(),
        1 => "annual".into(),
        n => format!("{n} a year"),
    }
}

/// The payer's own declared record, as the book keeps it: the newest declaration
/// of the instruments given (the one listing a symbol names), its frequency where
/// one is stated, and the amount it replaces when that differs.
pub fn distribution_detail_in(book: &bagholder_book::Book, instruments: &[bagholder_core::InstrumentId]) -> Result<String, String> {
    let [id] = instruments else { return Ok(String::new()) };
    let (declared, frequencies) = match (book.declared(), book.frequencies()) {
        (Ok(d), Ok(f)) => (d, f),
        (Err(e), _) | (_, Err(e)) => return Err(format!("The declared distributions could not be read: {e}")),
    };
    let Some(read) = declared.get(id) else { return Ok(String::new()) };
    let mut items: Vec<&bagholder_book::facts::DeclaredRow> = read.items.iter().collect();
    items.sort_by(|a, b| b.ex_date.cmp(&a.ex_date));
    let Some(latest) = items.first() else { return Ok(String::new()) };
    let mut out = format!("{} a share", per_unit(&latest.amount));
    if let Some(f) = frequencies.get(id) {
        out += &format!(", {}", frequency_word(f.per_year));
    }
    out += &format!(" · ex {}", stamp_day(&latest.ex_date.to_string()));
    if let Some(p) = latest.pay_date {
        out += &format!(", paid {}", stamp_day(&p.to_string()));
    }
    if let Some(was) = items.get(1) {
        if was.amount != latest.amount {
            out += &format!(" · was {}", per_unit(&was.amount));
        }
    }
    Ok(out)
}

/// When the newest of these happened, as its source dates it.
///
/// A release found today can have been published weeks ago -- the app reads a
/// listing's back catalogue the first time it sees it -- and a notice that
/// shows only when it was told reads as news that is not new.
pub fn notice_moment<T: Notable>(rows: &[T]) -> String {
    newest_of(rows).moment()
}

/// Where a notification's rows can be read: the newest one's own page, and,
/// for a filed document, the regulator it opens through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoticeLink {
    pub url: String,
    pub doc: String,
    pub source: String,
}

/// Where a notification's rows can be read: the newest one's own page.
///
/// A filed document is opened through the app, which is what the Disclosures
/// table does, so it opens the same way from here; anything else carries the
/// source's own link.
pub fn notice_link<T: Notable>(rows: &[T]) -> NoticeLink {
    let newest = newest_of(rows);
    let url = newest.url();
    let id = newest.id();
    if let Some(source) = newest.filed_source() {
        if !id.is_empty() {
            return NoticeLink { url, doc: id, source };
        }
    }
    NoticeLink { url, ..NoticeLink::default() }
}

/// The newest row by its own moment.
fn newest_of<T: Notable>(rows: &[T]) -> &T {
    let mut sorted: Vec<&T> = rows.iter().collect();
    sorted.sort_by(|a, b| b.moment().cmp(&a.moment()));
    sorted[0]
}

/// A notification's extra: the symbol, the moment and the link, in one struct.
fn notice_extra<T: Notable>(sym: &str, exchange: Option<&str>, rows: &[T]) -> bagholder_store::feeds::NotificationExtra {
    let (at, link) = (notice_moment(rows), notice_link(rows));
    bagholder_store::feeds::NotificationExtra {
        symbol: sym.to_string(),
        exchange: exchange.unwrap_or_default().to_string(),
        at,
        url: link.url,
        doc: link.doc,
        source: link.source,
    }
}

/// What a release *is*, independent of the id, the source and the date each
/// carries: the story its headline tells.
///
/// TMX, Yahoo, Seeking Alpha and Google all carry the same release under their
/// own ids, a week apart in their own timestamps; keyed by id, one release is
/// four events.
pub fn release_event<T: Notable>(row: &T) -> String {
    news::news_text(&row.heading())
}

fn release_key<T: Notable>(sym: &str, rows: &[T]) -> String {
    let mut ids: Vec<String> = rows.iter().map(release_event).collect();
    ids.sort();
    format!("release:{}:{}", sym, sha1_hex12(&ids.join("|")))
}

/// The press releases a wire answered with that
/// are newer than any it showed for the listing.
///
/// A read or a write of the book that fails is the error: a mark not kept
/// would tell the same release twice.
pub fn note_wire_releases(app: &Arc<App>, symbol: &str, exchange: &str, rows: &[NewsItem], new_ids: &[String]) -> Result<(), String> {
    let book = notify::book(app).map_err(|e| e.to_string())?;
    let c = &*book;
    let scopes = notify::release_scopes(c).map_err(|e| e.to_string())?;
    let t = tmx_symbol(symbol);
    let sym = (if t.is_empty() { symbol.to_string() } else { t }).trim().to_uppercase();
    if scopes.is_empty() || !in_release_scope(app, &sym, &scopes)? {
        return Ok(());
    }
    let rel: Vec<NewsItem> = rows.iter().filter(|r| r.is_release()).cloned().collect();
    // An event is told once. The stream keeps what it has met, by what the thing is rather than by
    // the id a source gave it, so the same release reaching the app again -- from another source,
    // under another id, dated a week apart, or simply returning to a search's results after
    // dropping out of them -- is recognised and passed over. Everything met is recorded, told or
    // not, so the back catalogue a first read brings can never ring later.
    let scope = format!("news:{}", sf::news_key(symbol, exchange));
    let events: Vec<String> = rel.iter().map(release_event).collect();
    let met = sf::events_told(c.notices(), &scope, &events).map_err(|e| e.to_string())?;
    let fresh = notify::fresh_in(c, &scope, &rel, |r: &NewsItem| r.published_at.clone(), |r: &NewsItem| r.id.clone(), |r: &NewsItem| {
        !new_ids.contains(&r.id) || met.contains(&release_event(r))
    }).map_err(|e| e.to_string())?;
    // the notice is worded first (it may read the issuer's record); then the marks and
    // the notice are kept together, so a failure of either leaves the release to tell again
    let notice = if fresh.items.is_empty() { None } else { Some((release_key(&sym, &fresh.items), release_notice(app, &sym, &fresh.items), notice_extra(&sym, Some(exchange), &fresh.items))) };
    let now = now_iso();
    let row = bagholder_sqlite::atomically(c.notices(), || {
        notify::keep_mark(c, &fresh.mark)?;
        sf::mark_told(c.notices(), &scope, &events, &now)?;
        match &notice {
            Some((key, (title, body), extra)) => notify::record(c, "releases", key, title, body, Some(extra.clone())),
            None => Ok(None),
        }
    })
    .map_err(|e| format!("what is new of {sym} could not be told: {e}"))?;
    if let Some(row) = &row {
        notify::announce(app, row);
    }
    Ok(())
}

/// `note_wire_releases` from the news pass: a failure is said in the header until
/// a wire's releases are next noted.
pub(crate) fn note_wire_releases_said(app: &Arc<App>, symbol: &str, exchange: &str, rows: &[NewsItem], new_ids: &[String]) {
    went(app, RELEASES, note_wire_releases(app, symbol, exchange, rows, new_ids).map_err(|e| format!("The press releases could not be told: {e}")));
}

/// `New disclosure · QNC` and what was filed.
pub fn filings_notice(app: &Arc<App>, sym: &str, new: &[Filing]) -> (String, String) {
    let (mut named, mut said): (Vec<String>, String) = (Vec::new(), String::new());
    for r in new.iter().take(3) {
        let mut title = r.subject.trim().to_string();
        let mut summary = r.summary.trim().to_string();
        if (title.is_empty() || summary.is_empty()) && !r.doc.id.is_empty() {
            if let Ok(read) = filings_enrich_result(app, sym, &r.doc.id) {
                if title.is_empty() {
                    title = read.subject.trim().to_string();
                }
                if summary.is_empty() {
                    summary = read.summary.trim().to_string();
                }
            }
        }
        if title.is_empty() {
            title = form_name(&r.doc.form);
        }
        if !title.is_empty() && !named.contains(&title) {
            named.push(title);
        }
        if said.is_empty() {
            said = summary;
        }
    }
    let mut sources: Vec<String> = Vec::new();
    for r in new {
        let src = r.doc.source.as_str().trim().to_string();
        if !src.is_empty() && !sources.contains(&src) {
            sources.push(src);
        }
    }
    let label = |x: &str| match x.to_lowercase().as_str() {
        "sedar" | "sedar+" => "SEDAR+".to_string(),
        "sec" | "sec edgar" => "SEC EDGAR".to_string(),
        _ => x.to_string(),
    };
    let tail = sources.iter().map(|x| label(x)).collect::<Vec<_>>().join(", ");
    let head = named.join(", ") + if new.len() > 3 { " and more" } else { "" };
    let mut body = head.clone() + if !head.is_empty() && !tail.is_empty() { " · " } else { "" } + &tail;
    if body.is_empty() {
        body = "A new filing.".into();
    }
    // the sentence the document itself yielded, under the line that names it: a form code and a
    // regulator say what arrived, never what it says
    if !said.is_empty() && news::news_text(&said) != news::news_text(&head) {
        body = format!("{}\n{}", body, said);
    }
    let title = if new.len() == 1 { "New disclosure · ".to_string() } else { format!("{} new disclosures · ", new.len()) } + sym;
    (title, body)
}

/// What a form is, for the forms whose code is all a row carries until a
/// document has been read.
///
/// A code names the form and not what happened, and "4" alone tells a holder
/// nothing at all.
pub const FORM_NAMES: &[(&str, &str)] = &[
    ("3", "Insider's first report (Form 3)"),
    ("4", "Insider transaction (Form 4)"),
    ("5", "Insider's annual report (Form 5)"),
    ("8-K", "Material event (8-K)"),
    ("6-K", "Foreign issuer report (6-K)"),
    ("10-K", "Annual report (10-K)"),
    ("10-Q", "Quarterly report (10-Q)"),
    ("144", "Notice of proposed sale (144)"),
    ("S-1", "Registration (S-1)"),
    ("SC 13D", "Beneficial ownership (13D)"),
    ("SC 13G", "Beneficial ownership (13G)"),
    ("DEF 14A", "Proxy statement (DEF 14A)"),
    ("424B5", "Prospectus supplement (424B5)"),
    ("FWP", "Free writing prospectus (FWP)"),
];

/// A form's code as words where the app knows the form, the code itself
/// otherwise.
pub fn form_name(code: &str) -> String {
    let key = code.trim().to_uppercase();
    FORM_NAMES.iter().find(|(c, _)| *c == key).map(|(_, n)| n.to_string()).unwrap_or_else(|| code.trim().to_string())
}

/// What a tab shows when a regulator would not serve a document.
///
/// SEDAR+ mints a document's address inside a live session and puts a bot gate
/// in front of it, so a refusal is ordinary and a retry often works; the page
/// says that in words, names the document, and retries on a click.
pub fn document_error_page(app: &Arc<App>, symbol: &str, doc_id: &str, why: &str) -> String {
    // the document's record names it; one that cannot be read is said beside the source's refusal
    let (row, why) = match conn(app).and_then(|c| sf::filing(&c, &symbol.trim().to_uppercase(), doc_id).map_err(|e| e.to_string())) {
        Ok(row) => (row, why.to_string()),
        Err(e) => (None, format!("{} The document's record could not be read: {e}", if why.is_empty() { "The source did not answer." } else { why })),
    };
    let why = why.as_str();
    let name = row.as_ref().map(|r| {
        let s = r.subject.trim();
        if !s.is_empty() { s.to_string() } else if !r.doc.title.trim().is_empty() { r.doc.title.trim().to_string() } else { r.doc.form.trim().to_string() }
    }).filter(|s| !s.is_empty()).unwrap_or_else(|| "This document".to_string());
    let source = row.as_ref().map(|r| r.doc.source.as_str().to_string()).unwrap_or_else(|| "the regulator".to_string());
    let when = row.as_ref().map(|r| {
        let t = r.doc.date_text.trim();
        if !t.is_empty() { t.to_string() } else { r.doc.date.chars().take(10).collect() }
    }).unwrap_or_default();
    let again = format!("/api/filings/doc?symbol={}&id={}", url_quote(symbol), url_quote(doc_id));
    format!(
        "<!doctype html><meta charset=utf-8><title>{name}</title>\
<style>:root{{color-scheme:dark light}}body{{margin:0;min-height:100vh;display:grid;place-items:center;\
background:#0e1118;color:#e8ecf3;font:400 14px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}}\
main{{max-width:34rem;padding:2rem}}h1{{font:600 16px/1.4 inherit;margin:0 0 .75rem}}p{{margin:0 0 .75rem;color:#aab3c2}}\
b{{color:#e8ecf3;font-weight:500}}a{{color:#7aa2f7}}</style>\
<main><h1>{source} would not serve this document just now</h1>\
<p><b>{name}</b>{when}</p>\
<p>{why}</p>\
<p>SEDAR+ builds a document&rsquo;s address inside a live session and puts a bot gate in front of it, so a \
refusal is ordinary rather than a sign the document is gone. <a href=\"{again}\">Try again</a>.</p></main>",
        name = html_escape(&name),
        source = html_escape(&source),
        when = html_escape(&if when.is_empty() { String::new() } else { format!(" · {}", when) }),
        why = html_escape(if why.is_empty() { "The source did not answer." } else { why }),
        again = html_escape(&again),
    )
}

/// `html.escape`: the five characters that must not read as markup.
fn html_escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// `urllib.parse.quote`: everything but the unreserved characters and `/`.
fn url_quote(t: &str) -> String {
    let mut out = String::new();
    for b in t.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{:02X}", b);
        }
    }
    out
}

pub fn filings_sweep_loop(app: Arc<App>) {
    // The regulators publish no feed to subscribe to, so telling someone of a new
    // filing means asking; but only while they have asked to be told. With no
    // Disclosures or Releases set on, this waits for one to be switched on.
    let wanted = || {
        notices(&app).is_ok_and(|b| {
            notice_settings(&app, notify::disclosure_scopes(&b).and_then(|d| Ok(!d.is_empty() || !notify::release_scopes(&b)?.is_empty()))).unwrap_or(false)
        })
    };
    while app.events.park_until(&app, wanted) {
        sweep_filings(&app);
        if app.wait(Duration::from_secs(FILINGS_SWEEP_EVERY_SEC)) {
            return;
        }
    }
}

/// How long the reader waits between documents, and after a pass that found
/// nothing left to read.
pub const READ_GAP_SEC: u64 = 2;

/// Documents the app reads on its own, newest first, for the listings it
/// follows: a title and a sentence cost a download and a reading each, and a
/// list is no use standing still while someone waits for them. One at a time,
/// paced, and only while a model is up to do the reading.
pub fn disclosure_read_loop(app: Arc<App>) {
    // Reads while there is something unread and a model to read it; otherwise waits
    // for one of the two to change -- a filing stored (`FILINGS_STORED`) or the
    // model coming up (`localmodel::on_change`) -- and consults nothing in between.
    use bagholder_market::enrich::{summary_ensure, summary_ready};
    loop {
        let stored = FILINGS_STORED.load(Ordering::SeqCst);
        summary_ensure();
        if !app.events.park_until(&app, || summary_ready() || FILINGS_STORED.load(Ordering::SeqCst) != stored) {
            return;
        }
        if !summary_ready() {
            continue; // more was stored and there is still no model: try bringing one up again
        }
        // a row a read moved nothing on (the fetch failed, or the model went away
        // under it) is passed over until the loop is woken again, so it neither
        // holds the others back nor is fetched over and over
        let mut passed: HashSet<(String, String)> = HashSet::new();
        loop {
            match read_one_unread(&app, &mut passed) {
                Ok(None) => break,
                // a document that could not be read is said in the header until one is, and passed over
                Ok(Some(read)) => {
                    went(&app, DISCLOSURES_READING, read);
                }
                // the store failing is said the same way, and the loop waits for a change rather than asking again
                Err(e) => {
                    feed_failed(&app, DISCLOSURES_READING, format!("The documents could not be read: {e}"));
                    break;
                }
            }
            if app.wait(Duration::from_secs(READ_GAP_SEC)) {
                return;
            }
        }
        if !app.events.park_until(&app, || FILINGS_STORED.load(Ordering::SeqCst) != stored || !summary_ready()) {
            return;
        }
    }
}

/// Counts the times filings were stored: what the reading loop waits on.
static FILINGS_STORED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The newest stored filing a reading could still add to, read: the listing on
/// screen first, then everything else. False when there is none left to try.
fn read_one_unread(app: &Arc<App>, passed: &mut HashSet<(String, String)>) -> Result<Option<Result<(), String>>, String> {
    let open = app.feeds.looking_at.lock().unwrap().clone();
    if !open.is_empty() {
        if let Some(read) = read_one_of(app, &open, passed)? {
            return Ok(Some(read));
        }
    }
    read_one_of(app, "", passed)
}

/// One document of `only`, or of every followed listing when it is empty, that a
/// reading could still add to (`wants_reading`): one never read, and one holding a
/// title without its sentence or the other way round. True when there was one to
/// try, with how its read went; one the read moved nothing on, or whose read
/// failed, is added to `passed`. The store failing is the error.
fn read_one_of(app: &Arc<App>, only: &str, passed: &mut HashSet<(String, String)>) -> Result<Option<Result<(), String>>, String> {
    let c = conn(app)?;
    let list: Vec<String> = if only.is_empty() {
        known_filing_symbols(app, &["held".to_string(), "watched".to_string()])?.into_iter().map(|i| i.symbol).collect()
    } else {
        vec![only.to_string()]
    };
    read_one_in(&c, &list, passed, &LiveReaders)
}

/// `read_one_of` on one connection, for the listings given, with the readers given.
fn read_one_in(c: &Connection, symbols: &[String], passed: &mut HashSet<(String, String)>, readers: &dyn Readers) -> Result<Option<Result<(), String>>, String> {
    let mut best: Option<(String, String, Filing)> = None;   // date, symbol, row
    for sym in symbols {
        let sym = sym.trim().to_uppercase();
        for r in sf::filings_for(c, &sym).map_err(|e| e.to_string())? {
            if !wants_reading(&r) || passed.contains(&(sym.clone(), r.doc.id.clone())) {
                continue;
            }
            let date = r.doc.date.clone();
            if best.as_ref().map(|(d, _, _)| date > *d).unwrap_or(true) {
                best = Some((date, sym.clone(), r));
            }
        }
    }
    let (_, sym, before) = match best { Some(b) => b, None => return Ok(None) };
    if let Err(e) = filings_enrich_in(c, &sym, &before.doc.id, readers) {
        passed.insert((sym.clone(), before.doc.id.clone()));
        return Ok(Some(Err(format!("{sym}'s document {} could not be read: {e}", before.doc.id))));
    }
    let after = sf::filing(c, &sym, &before.doc.id).map_err(|e| e.to_string())?;
    let moved = after.as_ref().map_or(false, |a| {
        (&a.subject, &a.summary, a.enrich_version, a.enrich_final, a.enrich_reads)
            != (&before.subject, &before.summary, before.enrich_version, before.enrich_final, before.enrich_reads)
    });
    if !moved {
        passed.insert((sym, before.doc.id));
    }
    Ok(Some(Ok(())))
}

/// Two reads that could have answered bound a document's reading: what they
/// cannot produce, the document does not have, and the row settles with
/// whichever half it has (SPEC §4 Disclosures).
pub const ENRICH_READS: i64 = 2;

/// Whether a reading could still add to a stored row: it was never read under the
/// current logic, or it holds a title without its sentence (or the other way
/// round) and has not settled. What every reader of documents picks rows by.
pub fn wants_reading(r: &Filing) -> bool {
    r.enrich_version.unwrap_or(0) < ENRICH_VERSION || (!r.enrich_final && (r.subject.is_empty() || r.summary.is_empty()))
}

/// One symbol's disclosures from every covering
/// source, stored per source. The total written, or -1 when no source could be
/// reached; the store failing is the error.
pub fn refresh_filings(app: &Arc<App>, symbol: &str, name: Option<&str>, exchange: Option<&str>, currency: Option<&str>) -> Result<i64, String> {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return Ok(0);
    }
    app.single_flight(&format!("filings:{}", sym), Ok(0), || {
        let c = conn(app)?;
        refresh_filings_in(app, &c, &sym, name, exchange, currency, &|s, n, e, cy, p| disclosures::fetch(s, n, e, cy, 200, p))
    })
}

/// The disclosures gathering `fetch_filings` gives: (symbol, name, exchange,
/// currency, known SEDAR+ profile) to every covering source's documents.
pub type FetchFilings<'a> = &'a dyn Fn(&str, &str, &str, &str, &str) -> Gathered;

/// `refresh_filings` on one connection with the gathering given.
pub fn refresh_filings_in(app: &Arc<App>, c: &Connection, sym: &str, name: Option<&str>, exchange: Option<&str>, currency: Option<&str>, fetch_filings: FetchFilings) -> Result<i64, String> {
    {
        let c = c;
        let sym = sym.to_string();
        let (mut iname, ex, cur) = instrument_meta(app, &sym);
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            iname = n.to_string();
        }
        let exchange_ = exchange.map(|x| x.to_string()).unwrap_or(ex);
        let currency_ = currency.map(|x| x.to_string()).unwrap_or(cur);
        let known = sf::sedar_profile(c, &sym).map_err(|e| e.to_string())?;
        let result = fetch_filings(&sym, &iname, &exchange_, &currency_, &known);
        let mut total = 0i64;
        let mut any_reached = false;
        let mut profile_no = String::new();
        let mut by_source: HashMap<Regulator, Vec<FiledDocument>> = HashMap::new();
        for it in result.items {
            if it.source == Regulator::Sedar && !it.profile_no.is_empty() {
                profile_no = it.profile_no.clone();
            }
            by_source.entry(it.source).or_default().push(it);
        }
        let held: HashSet<Regulator> = sf::filings_for(c, &sym).map_err(|e| e.to_string())?.iter().map(|r| r.doc.source).collect();
        let now = now_iso();
        let sources = result.sources;
        for (src, status) in &sources {
            if status.available {
                any_reached = true;
            }
            let rows = by_source.get(src).cloned().unwrap_or_default();
            if rows.is_empty() && held.contains(src) {
                log(&format!("bagholder disclosures: {}: {} answered empty; the stored rows stand", sym, src.as_str()));
                continue;
            }
            if status.matched || status.available {
                total += sf::replace_filings(c, &sym, *src, &rows, &now).map_err(|e| format!("{sym}'s {} filings could not be stored: {e}", src.as_str()))? as i64;
            }
        }
        sf::mark_filings_fetched(c, &sym, &profile_no, &now).map_err(|e| e.to_string())?;
        FILINGS_STORED.fetch_add(1, Ordering::SeqCst);
        app.events.signal(); // the reading loop has something to look at
        let outcomes = serde_json::to_string(&sources).expect("each regulator's outcome, keyed by its name, always serializes");
        set_meta(c, &format!("filings_sources:{}", sym), &outcomes).map_err(|e| e.to_string())?;
        Ok(if any_reached { total } else { -1 })
    }
}

/// A source's outcome the last time a listing's disclosures were refreshed.
#[derive(Clone, Debug, Default, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub available: bool,
    pub matched: bool,
    pub filer: bool,
    pub error: String,
}

/// `#[derive(Diff)]` compares a map field as the model's own `BTreeMap<String,
/// V>`, so a source's status lives under its regulator's own JSON key
/// (`"SEDAR+"`, `"SEC"`) rather than under `Regulator` itself.
fn source_status(c: &Connection, sym: &str) -> Result<BTreeMap<String, SourceStatus>, String> {
    // none stored before a first refresh; stored text that does not parse is an error
    let raw = get_meta(c, &format!("filings_sources:{}", sym), "").map_err(|e| e.to_string())?;
    let stored: BTreeMap<Regulator, SourceOutcome> = if raw.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_str(&raw).map_err(|e| format!("what {sym}'s sources last answered could not be read: {e}"))?
    };
    let have: HashSet<Regulator> = sf::filings_for(c, sym).map_err(|e| e.to_string())?.iter().map(|r| r.doc.source).collect();
    let mut out = BTreeMap::new();
    for (source, dep) in [(Regulator::Sedar, sedar::available()), (Regulator::Sec, edgar::available())] {
        let st = stored.get(&source);
        // "available" is whether the last read actually reached the source, not merely
        // whether its dependency is installed: a stored outcome that recorded the source
        // unavailable (an outage, a maintenance page) means it could not be reached even
        // with the dependency present. The error it left travels with it, so the card can
        // say the source is unavailable rather than assert the listing has no filer.
        let reached = st.map(|s| s.available).unwrap_or(dep);
        let error = st.map(|s| s.error.clone()).unwrap_or_default();
        out.insert(source.as_str().to_string(), SourceStatus {
            available: dep && reached,
            matched: have.contains(&source),
            filer: st.map(|s| s.filer).unwrap_or(false) || have.contains(&source),
            error,
        });
    }
    Ok(out)
}

/// The stored disclosures, refreshed first when
/// forced or stale.
/// `GET /api/filings`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum FilingsAnswer {
    Ok(FilingsPayload),
    Refused(OkOr),
}

pub fn filings_payload(app: &Arc<App>, symbol: &str, refresh: bool, name: Option<&str>, exchange: Option<&str>, currency: Option<&str>) -> FilingsAnswer {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return FilingsAnswer::Refused(OkOr::err("symbol required"));
    }
    let c = match conn(app) {
        Ok(c) => c,
        Err(e) => return FilingsAnswer::Refused(OkOr::err(e)),
    };
    match filings_payload_in(app, &c, &sym, refresh, &|| refresh_filings(app, &sym, name, exchange, currency)) {
        Ok(p) => FilingsAnswer::Ok(p),
        Err(e) => FilingsAnswer::Refused(OkOr::err(e)),
    }
}

// --- a listing's disclosures while a page shows them ------------------------------
//
// The page used to drive the reading itself: ask for one document's title, wait, ask
// for the next, and re-kick the whole pass on timers of a second and a half to
// fifteen while the local model was coming up. The reading is the server's work. A
// page says it is showing a listing's disclosures (`docs`); the server brings the
// list up to date, then reads each document that has no title yet, newest first, for
// as long as some page is still showing them. Each title and sentence is committed
// as it is read and reaches its row as a change to that row.

fn set_reading(app: &Arc<App>, sym: &str, ids: Vec<String>) {
    {
        let mut r = app.feeds.reading_now.lock().unwrap_or_else(|e| e.into_inner());
        if ids.is_empty() { r.remove(sym); } else { r.insert(sym.to_string(), ids); }
    }
    app.events.signal(); // the rows still to be read wear the shimmer
}

/// What `filings_stored` sends a page showing a listing's disclosures.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct FilingsDoc {
    #[ts(type = "true")]
    pub ok: bool,
    pub symbol: String,
    pub available: bool,
    pub sources: BTreeMap<String, SourceStatus>,
    pub categories: Vec<String>,
    pub fetched_at: String,
    pub ever_read: bool,
    pub summary_status: String,
    pub reading: Vec<String>,
    pub filings: Vec<Filing>,
}

/// The disclosures as stored, never waiting on a source: what a page showing them
/// is sent. `reading` names the documents the pass has still to read, the first of
/// them the one being read now.
pub fn filings_stored(app: &Arc<App>, symbol: &str) -> Result<FilingsDoc, String> {
    let sym = symbol.trim().to_uppercase();
    let c = conn(app)?;
    let reading = app.feeds.reading_now.lock().unwrap_or_else(|e| e.into_inner()).get(&sym).cloned().unwrap_or_default();
    let fetched_at = sf::filings_fetched_for(&c, &sym).map_err(|e| e.to_string())?;
    Ok(FilingsDoc {
        ok: true,
        symbol: sym.clone(),
        available: disclosures::available(),
        sources: source_status(&c, &sym)?,
        categories: disclosures::CATEGORIES.iter().map(|s| s.to_string()).collect(),
        ever_read: !fetched_at.is_empty(),
        fetched_at,
        summary_status: enrich::summary_status().to_string(),
        reading,
        filings: fresh_filings(&c, &sym)?,
    })
}

/// A page has started showing `symbol`'s disclosures (`doc` is the document's key):
/// bring the list up to date if it is stale, then read what has no title, until no
/// page shows them any more. One pass per listing at a time.
pub fn filings_shown(app: Arc<App>, doc: String, symbol: String, name: String, exchange: String, currency: String) {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return;
    }
    spawn("bagholder-filings-shown", move || {
        app.single_flight(&format!("filings-shown:{}", sym), (), || {
            *app.feeds.looking_at.lock().unwrap() = sym.clone();
            fn opt(s: &str) -> Option<&str> { if s.is_empty() { None } else { Some(s) } }
            // the list brought up to date: a store that fails is said in the header until it answers
            let refreshed = conn(&app).and_then(|c| filings_stale(&c, &sym, None)).and_then(|stale| {
                if stale { refresh_filings(&app, &sym, opt(&name), opt(&exchange), opt(&currency)).map(|_| ()) } else { Ok(()) }
            });
            went(&app, DISCLOSURES, refreshed.map_err(|e| format!("{sym}'s disclosures could not be brought up to date: {e}")));
            let mut tried: HashSet<String> = HashSet::new();
            while app.events.watched(&doc) && !app.stopping() {
                let left = conn(&app).and_then(|c| sf::filings_for(&c, &sym).map_err(|e| e.to_string()));
                let Some(left) = went(&app, DISCLOSURES_STORED, left.map_err(|e| format!("The disclosures held could not be read: {e}"))) else { break };
                let mut left: Vec<Filing> = left.into_iter()
                    .filter(|r| wants_reading(r) && !tried.contains(&r.doc.id))
                    .collect();
                left.sort_by(|a, b| b.doc.date.cmp(&a.doc.date));
                let Some(next) = left.first().map(|r| r.doc.id.clone()) else { break };
                set_reading(&app, &sym, left.iter().map(|r| r.doc.id.clone()).collect());
                // waits for the local model itself when one is coming up; a document that
                // could not be read is said in the header until one is
                went(&app, DISCLOSURES_READING, filings_enrich_result(&app, &sym, &next).map_err(|e| format!("{sym}'s document {next} could not be read: {e}")));
                tried.insert(next);
                if app.wait(Duration::from_millis(150)) { // a person's pace at the source
                    break;
                }
            }
            set_reading(&app, &sym, vec![]);
        });
    });
}

/// What `filings_payload` returns: the `GET /api/filings?…&refresh=1` answer.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct FilingsPayload {
    #[ts(type = "true")]
    pub ok: bool,
    pub symbol: String,
    pub available: bool,
    pub sources: BTreeMap<String, SourceStatus>,
    pub categories: Vec<String>,
    pub profile_no: String,
    pub fetched_at: String,
    pub refreshed: bool,
    pub source_unavailable: bool,
    pub filings: Vec<Filing>,
}

/// `filings_payload` on one connection with the refresh given.
pub fn filings_payload_in(app: &Arc<App>, c: &Connection, sym: &str, refresh: bool, refresh_filings: &dyn Fn() -> Result<i64, String>) -> Result<FilingsPayload, String> {
    let sym = sym.trim().to_uppercase();
    if sym.is_empty() {
        return Err("symbol required".to_string());
    }
    *app.feeds.looking_at.lock().unwrap() = sym.clone();
    let c = c;
    let mut wrote: Option<i64> = None;
    if refresh || filings_stale(c, &sym, None)? {
        wrote = Some(refresh_filings()?);
    }
    Ok(FilingsPayload {
        ok: true,
        available: disclosures::available(),
        sources: source_status(c, &sym)?,
        categories: disclosures::CATEGORIES.iter().map(|s| s.to_string()).collect(),
        profile_no: sf::sedar_profile(c, &sym).map_err(|e| e.to_string())?,
        fetched_at: sf::filings_fetched_for(c, &sym).map_err(|e| e.to_string())?,
        refreshed: wrote.map(|w| w > 0).unwrap_or(false),
        source_unavailable: wrote == Some(-1),
        filings: fresh_filings(c, &sym)?,
        symbol: sym,
    })
}

/// Rows read by an older logic blanked, categories
/// re-derived.
fn fresh_filings(c: &Connection, sym: &str) -> Result<Vec<Filing>, String> {
    let mut rows = sf::filings_for(c, sym).map_err(|e| e.to_string())?;
    for r in rows.iter_mut() {
        if r.enrich_version.unwrap_or(0) < ENRICH_VERSION {
            r.subject = String::new();
            r.summary = String::new();
        }
        r.doc.category = disclosures::categorize(&r.doc);
        // the list arrives named: a form and a named document say what they are
        // without being fetched, so only the rest wait on a reading
        if r.subject.is_empty() {
            if let Some(t) = disclosures::quick_title(&r.doc) {
                sf::set_filing_enrichment(c, sym, &r.doc.id, Some(&t), None, None, None, &now_iso()).map_err(|e| e.to_string())?;
                r.subject = t;
            }
        }
    }
    Ok(rows)
}

/// (bytes, content type), or the error.
pub fn filings_document(app: &Arc<App>, symbol: &str, doc_id: &str) -> Result<(Vec<u8>, String), String> {
    let sym = symbol.trim().to_uppercase();
    let c = conn(app)?;
    let mut row = sf::filing(&c, &sym, doc_id).map_err(|e| e.to_string())?;
    if row.is_none() {
        refresh_filings(app, &sym, None, None, None)?;
        row = sf::filing(&c, &sym, doc_id).map_err(|e| e.to_string())?;
    }
    let row = row.ok_or_else(|| format!("no such document for {}", sym))?;
    let (data, ct) = disclosures::document(&row.doc).map_err(|e| e.to_string())?;
    if data.is_empty() {
        return Err("empty document".into());
    }
    Ok((data, if ct.is_empty() { "application/octet-stream".into() } else { ct }))
}

/// `filings_enrich` on the app's own connection, typed.
fn filings_enrich_result(app: &Arc<App>, symbol: &str, doc_id: &str) -> Result<Enriched, String> {
    let c = conn(app)?;
    filings_enrich_in(&c, symbol, doc_id, &LiveReaders)
}

/// One document read for its subject and, with a
/// local model, a one-sentence summary; both cached on the row.
/// `GET /api/filings/enrich`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum EnrichAnswer {
    Ok(Enriched),
    Refused(OkOr),
}

pub fn filings_enrich(app: &Arc<App>, symbol: &str, doc_id: &str) -> EnrichAnswer {
    match filings_enrich_result(app, symbol, doc_id) {
        Ok(e) => EnrichAnswer::Ok(e),
        Err(e) => EnrichAnswer::Refused(OkOr::err(e)),
    }
}

/// What `filings_enrich_in` reads a document with: the local model and the
/// disclosure sources.
pub trait Readers {
    fn summary_available(&self) -> bool { enrich::summary_available() }
    fn summary_status(&self) -> &'static str { enrich::summary_status() }
    fn wait_for_summary(&self, seconds: f64) -> bool { enrich::wait_for_summary(seconds) }
    fn disclosures_available(&self) -> bool { disclosures::available() }
    fn enrichment(&self, row: &FiledDocument) -> Option<disclosures::Enrichment> { disclosures::enrichment(row) }
    fn content(&self, row: &FiledDocument) -> disclosures::Fetched<(Vec<u8>, String)> { disclosures::content(row) }
    fn enrich_document_of(&self, code: &str, source: &str, data: &[u8], ct: &str) -> disclosures::Enrichment { enrich::enrich_document_of(code, source, data, ct) }
}

struct LiveReaders;
impl Readers for LiveReaders {}

/// One document's title and summary, as `filings_enrich` answers.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Enriched {
    #[ts(type = "true")]
    pub ok: bool,
    pub id: String,
    pub subject: String,
    pub summary: String,
    pub summary_available: bool,
    pub summary_status: String,
}

/// `filings_enrich` on one connection with the readers given.
///
/// A row is read until it holds both a title and a sentence, or until two reads
/// that could have answered have been made: a read counts when it reached the
/// document with a model up to write the sentence. A read with no model up only
/// failed to ask; it keeps the title it found and the row waits, fetched again
/// only once a model is up (SPEC §4 Disclosures).
pub fn filings_enrich_in(c: &Connection, symbol: &str, doc_id: &str, r: &dyn Readers) -> Result<Enriched, String> {
    let sym = symbol.trim().to_uppercase();
    let row = sf::filing(c, &sym, doc_id).map_err(|e| e.to_string())?.ok_or_else(|| "no such document".to_string())?;
    let mut subject = row.subject.clone();
    let mut summary = row.summary.clone();
    let mut model = r.summary_available();
    let fresh = row.enrich_version.unwrap_or(0) >= ENRICH_VERSION;
    let attempted = !row.enriched_at.is_empty() && fresh;
    // reads under an older logic say nothing about what this one can find
    let reads = if fresh { row.enrich_reads } else { 0 };
    let answer = |subject: &str, summary: &str, avail: bool| Enriched {
        ok: true, id: doc_id.to_string(), subject: subject.to_string(), summary: summary.to_string(),
        summary_available: avail, summary_status: r.summary_status().to_string(),
    };
    if attempted && (row.enrich_final || (!subject.is_empty() && !summary.is_empty()) || !model || reads >= ENRICH_READS) {
        return Ok(answer(&subject, &summary, model));
    }
    if !r.disclosures_available() {
        return Ok(answer(&subject, &summary, model));
    }
    let now = now_iso();
    // No model to write the sentence: what was found is kept under the current
    // logic's stamp, so the row waits for a model rather than being fetched again,
    // and the read is not counted against the document.
    let waits = |subject: &str, summary: &str| -> Result<(), String> {
        sf::set_filing_enrichment(c, &sym, doc_id, Some(subject), Some(summary), Some(ENRICH_VERSION), Some(false), &now).map_err(|e| e.to_string())?;
        sf::set_filing_reads(c, &sym, doc_id, reads).map_err(|e| e.to_string())?;
        Ok(())
    };
    if !fresh {
        // only the first read under the current logic replaces both halves
        subject = String::new();
        summary = String::new();
    }
    if let Some(exact) = r.enrichment(&row.doc) {
        // What the source can say exactly: the form's own name, and for the
        // forms it can read in full, the sentence too. A name on its own is
        // kept and the document still read, so the sentence follows it.
        let (sj, sm) = (exact.subject.clone(), exact.summary.clone());
        if !sj.is_empty() && subject.is_empty() {
            subject = sj.clone();
            sf::set_filing_enrichment(c, &sym, doc_id, Some(&subject), None, None, None, &now).map_err(|e| e.to_string())?;
        }
        if !sm.is_empty() {
            sf::set_filing_enrichment(c, &sym, doc_id, Some(&sj), Some(&sm), Some(ENRICH_VERSION), None, &now).map_err(|e| e.to_string())?;
            return Ok(answer(&sj, &sm, model));
        }
        if exact.final_ {
            // a named document: nothing a reading would add
            sf::set_filing_enrichment(c, &sym, doc_id, Some(&subject), Some(""), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
            return Ok(answer(&subject, "", model));
        }
        if !model {
            waits(&subject, &summary)?;
            return Ok(answer(&subject, &summary, model));
        }
    }
    let (data, ct) = match r.content(&row.doc) {
        Ok(x) => x,
        Err(e) => return Err(e.to_string()),
    };
    if data.is_empty() {
        return Err("the document could not be read".to_string());
    }
    if !model {
        model = r.wait_for_summary(enrich::SUMMARY_WAIT_SEC);
    }
    let info = r.enrich_document_of(&row.doc.form, row.doc.source.as_str(), &data, &ct);
    let new_subject = info.subject.clone();
    let got_summary = info.summary.clone();
    if info.final_ {
        sf::set_filing_enrichment(c, &sym, doc_id, Some(&new_subject), Some(&got_summary), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
        return Ok(answer(&new_subject, &got_summary, model));
    }
    if model && new_subject.is_empty() && got_summary.is_empty() && subject.is_empty() && summary.is_empty() {
        // a document with no text in it -- a release filed as a picture -- has
        // nothing for a reading to find, now or later: it is named by what it
        // is and never read again
        let named = disclosures::quick_title(&row.doc).unwrap_or_else(|| row.doc.form.clone());
        let named: String = named.chars().take(90).collect();
        sf::set_filing_enrichment(c, &sym, doc_id, Some(&named), Some(""), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
        return Ok(answer(&named, "", model));
    }
    // reading again fills what is missing and never empties what is there
    if !new_subject.is_empty() {
        subject = new_subject;
    }
    if model {
        if !got_summary.is_empty() {
            summary = got_summary;
        }
        // a read that could have answered: two of them settle the row
        let reads = reads + 1;
        let settled = (subject.is_empty() || summary.is_empty()) && reads >= ENRICH_READS;
        sf::set_filing_enrichment(c, &sym, doc_id, Some(&subject), Some(&summary), Some(ENRICH_VERSION), Some(settled), &now).map_err(|e| e.to_string())?;
        sf::set_filing_reads(c, &sym, doc_id, reads).map_err(|e| e.to_string())?;
    } else {
        waits(&subject, &summary)?;
    }
    Ok(answer(&subject, &summary, r.summary_available()))
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
fn read_fear_with(app: &Arc<App>, index: &str, read: impl FnOnce() -> Result<sf::Gauge, String>) -> Result<StoredGauge, String> {
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
    rec
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
pub fn read_shorts(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str, trend: bool, name: &str) -> Result<Option<StoredShorts>, String> {
    let c = conn(app)?;
    let feed = format!("{SHORTS_OF}{}", symbol.trim().to_uppercase());
    let mut rec = match shorts::for_listing(&c, symbol, exchange, currency, &today(), trend, name) {
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
            feed_failed(app, &feed, e.to_string());
            return Ok(None);
        }
    };
    if rec.exchange.is_empty() {
        rec.exchange = exchange.to_string();
    }
    let stored = StoredShorts { shorts: rec, fetched_at: now_iso(), read_version: SHORTS_VERSION };
    sf::save_shorts(&c, &stored.shorts, &stored.fetched_at, SHORTS_VERSION).map_err(|e| format!("{symbol}'s short selling could not be kept: {e}"))?;
    Ok(Some(stored))
}

/// `read_shorts` from work that answers nobody: a failure is said in the header,
/// under the listing's own entry, until its next read succeeds.
fn read_shorts_said(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str, trend: bool, name: &str) -> Option<StoredShorts> {
    went(app, &format!("shorts-store:{}|{}", symbol, exchange), read_shorts(app, symbol, exchange, currency, trend, name)).flatten()
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

/// One listing's short selling, from the store at
/// once where it was read before.
pub fn shorts_payload(app: &Arc<App>, symbol: &str, exchange: Option<&str>, currency: Option<&str>, trend: bool) -> Result<ShortsPayload, String> {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return Err("symbol required".to_string());
    }
    let c = conn(app)?;
    let meta = instrument_meta(app, &sym);
    let listed_as = if meta.0 == sym { String::new() } else { meta.0.clone() };
    let mut ex = exchange.unwrap_or("").trim().to_string();
    let mut ccy = currency.unwrap_or("").trim().to_string();
    if ex.is_empty() {
        ex = meta.1.clone();
        if ccy.is_empty() {
            ccy = meta.2.clone();
        }
        if ex.is_empty() {
            let form = bagholder_market::tmx::tmx_resolve(&c, &tmx_symbol(&sym), &today()).map_err(|e| format!("{sym} could not be placed: {e}"))?;
            if !form.is_empty() && !form.ends_with(":US") {
                let tail = if form.contains(':') { form.rsplit(':').next().unwrap_or("") } else { "" };
                ex = match tail { "CNX" => "CSE", "AQL" => "Cboe Canada", _ => "" }.to_string();
                if ccy.is_empty() {
                    ccy = "CAD".into();
                }
            } else {
                ex = "NASDAQ".into();
                ccy = "USD".into();
            }
        }
    }
    if shorts::market_of(&sym, &ex, &ccy).is_none() {
        return Ok(ShortsPayload { ok: true, covered: false, shorts: None });
    }
    if let Some(held) = sf::shorts_for(&c, &sym, &ex).map_err(|e| format!("{sym}'s short selling held could not be read: {e}"))? {
        if !trend || held.shorts.series.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
            if shorts_stale(&held) {
                let (s2, e2, c2, n2) = (sym.clone(), ex.clone(), ccy.clone(), listed_as.clone());
                let a = app.clone();
                app.kick(&format!("shorts:{}|{}", sym, ex), move || {
                    read_shorts_said(&a, &s2, &e2, &c2, true, &n2);
                });
            }
            return Ok(ShortsPayload { ok: true, covered: true, shorts: Some(ShortsView::of(&held)?) });
        }
    }
    match read_shorts(app, &sym, &ex, &ccy, trend, &listed_as)? {
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
pub fn listing_payload(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str, name: &str) -> ListingAnswer {
    let sym = tmx_symbol(symbol).trim().to_uppercase();
    if sym.is_empty() {
        return ListingAnswer::err("symbol required");
    }
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
    let named = |symbol: &str| tmx_symbol(symbol).trim().to_uppercase() == sym;
    // the book's holdings and trades of that ticker, by the figures' own ids
    let Some((positions, trades)) = f.read(|e| crate::wire::build::listed(e, &names, &named)) else { return ListingAnswer::err("no page has stated its zone yet") };
    let watchlist: Vec<ListedRow> = b.watchlist.iter().filter(|w| named(&w.symbol)).map(|w| ListedRow { symbol: w.symbol.clone(), exchange: w.exchange.clone(), currency: w.currency.clone(), name: w.name.clone(), ..ListedRow::default() }).collect();
    let securities = crate::market_context::securities(app).unwrap_or_else(|e| {
        log(&format!("bagholder: the book's securities could not be read for {sym}: {e}"));
        vec![]
    });
    listing_payload_in(&securities, &positions, &trades, &watchlist, &sym, exchange, currency, name, &|s, ex, ccy| crate::following::glance(app, s, ex, ccy))
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
    symbol: &str,
    exchange: &str,
    currency: &str,
    name: &str,
    glance: &dyn Fn(&str, &str, &str) -> Result<crate::following::Glanced, String>,
) -> ListingAnswer {
    let sym = tmx_symbol(symbol).trim().to_uppercase();
    if sym.is_empty() {
        return ListingAnswer::err("symbol required");
    }
    let (mut ex, mut ccy) = (exchange.trim().to_string(), currency.trim().to_string());
    let exu = ex.to_uppercase();
    let same = |r: &ListedRow| {
        if r.kind == "Options" || tmx_symbol(&r.symbol).trim().to_uppercase() != sym {
            return false;
        }
        let there = r.exchange.trim().to_uppercase();
        exu.is_empty() || there.is_empty() || there == exu
    };
    if let Some(h) = positions.iter().find(|p| same(p)) {
        return ListingAnswer::Held { ok: true, symbol: sym, position_id: h.id.clone() };
    }
    let trades: Vec<&ListedRow> = trades.iter().filter(|t| same(t)).collect();
    let watched = watchlist.iter().find(|w| same(w));
    let empty = ListedRow::default();
    let known: &ListedRow = trades.first().copied().or(watched).unwrap_or(&empty);
    let meta = meta_of(securities, &sym);
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

/// The shares held and the listings watched whose
/// short selling is published.
pub fn shorts_listings(app: &Arc<App>, scope: &str) -> Vec<(String, String, String, String)> {
    let Ok(Some(b)) = base(app) else { return vec![] }; // a failure is said in the header by `base`
    let mut rows: Vec<(&str, &str, &str, &str)> = Vec::new();
    if scope == "holdings" || scope == "all" {
        rows.extend(b.positions.iter().map(|p| (p.symbol.as_str(), p.exchange.as_str(), p.currency.as_str(), p.name.as_str())));
    }
    if scope == "watchlist" || scope == "all" {
        rows.extend(b.watchlist.iter().map(|w| (w.symbol.as_str(), w.exchange.as_str(), w.currency.as_str(), w.name.as_str())));
    }
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    for (symbol, ex, ccy, name) in rows {
        let sym = tmx_symbol(symbol);
        let key = (sym.to_uppercase(), ex.to_uppercase());
        if sym.is_empty() || seen.contains(&key) || shorts::market_of(&sym, ex, ccy).is_none() {
            continue;
        }
        seen.insert(key);
        out.push((sym, ex.to_string(), ccy.to_string(), name.to_string()));
    }
    out
}

/// Keep every held and watched listing's short
/// selling stored and current.
pub fn sweep_shorts(app: &Arc<App>) -> usize {
    let Ok(c) = conn(app) else { return 0 }; // said in the header by `conn`
    let mut due = Vec::new();
    for (sym, ex, ccy, name) in shorts_listings(app, "all") {
        // a row held that cannot be read is read again from its source, and the store's failure said until it answers
        let held = went(app, &format!("shorts-store:{}|{}", sym, ex), sf::shorts_for(&c, &sym, &ex).map_err(|e| format!("{sym}'s short selling held could not be read: {e}"))).flatten();
        let fresh = held.as_ref().map(|h| h.shorts.series.as_ref().map(|s| !s.is_empty()).unwrap_or(false) && !shorts_stale(h)).unwrap_or(false);
        if !fresh {
            due.push((sym, ex, ccy, name));
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
    for (sym, ex, ccy, name) in due {
        if read_shorts_said(app, &sym, &ex, &ccy, true, &name).is_some() {
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
            Ok(out) => {
                feed_answered(app, "archive");
                out
            }
            Err(e) => {
                feed_failed(app, "archive", format!("The price bars could not be archived: {e}"));
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
                            // said until a pass reads the store again, which the book changing starts
                            feed_failed(&app, "archive", format!("The price bars could not be archived: {e}"));
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

/// The update check, hourly (`SPEC.md` §2, Versions).
pub fn market_loop(app: Arc<App>) {
    while !app.wait(Duration::from_secs(60 * bagholder_market::refresh::MARKET_CHECK_MINUTES)) {
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
pub fn history_pending(app: &Arc<App>, q: &HistoryQuery) -> bool {
    let (rec, start, _, tf) = q.read();
    let inst = history::chart_instrument(&rec);
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

/// A background read of a chart's bars ended: a store that failed it is said in
/// the header until a read goes through.
fn bars_read(app: &Arc<App>, read: Result<(), String>) {
    match read {
        Ok(()) => feed_answered(app, "bars"),
        Err(e) => feed_failed(app, "bars", e),
    }
}

pub fn history_payload(app: &Arc<App>, q: &HistoryQuery) -> HistoryAnswer {
    let (rec, start, end, tf) = q.read();
    if rec.symbol.is_empty() || start.chars().count() != 10 || end.chars().count() != 10 || !history::TIMEFRAMES.contains(&tf.as_str()) {
        return HistoryAnswer::Refused(OkOr::err("symbol, from, to and a known tf are required"));
    }
    let inst = history::chart_instrument(&rec);
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
                history::ensure_intraday_in_background(p.clone(), rates.clone(), inst.clone(), tf.clone(), start.clone(), end.clone(), move |read| bars_read(&told, read));
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
        chart_symbol: inst.symbol,
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
    use std::cell::Cell;

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
        crate::entries::enter(f, &crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: "ZZQQ".into(), currency: "USD".into(), day: "2025-11-19".into(), side: "buy".into(), quantity: "1".into(), price: "2".into(), fee: String::new() }, t).unwrap();
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

    /// The market cache's schema, where the filings are kept.
    fn store() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        for m in &bagholder_sources::cache::MIGRATIONS {
            c.execute_batch(m.sql).unwrap();
        }
        c
    }

    fn item(source: Regulator, i: i64, profile: &str) -> FiledDocument {
        let sec = source == Regulator::Sec;
        let tag = if sec { "sec" } else { "sedar" };
        FiledDocument {
            id: format!("{}:{}", tag, i),
            source,
            category: "Financials".into(),
            profile_no: profile.into(),
            issuer: String::new(),
            form: if sec { "10-Q" } else { "Interim MD&A" }.into(),
            title: if sec { "Quarterly report" } else { "" }.into(),
            date: format!("2026-08-{:02}", 10 + i),
            date_text: format!("2026-08-{:02}", 10 + i),
            size: if sec { "" } else { "292 KB" }.into(),
            url: if sec { format!("https://www.sec.gov/x/{}", i) } else { format!("https://www.sedarplus.ca/x?drmKey={}", i) },
        }
    }

    fn replace(c: &Connection, sym: &str, src: Regulator, items: &[FiledDocument]) {
        sf::replace_filings(c, sym, src, items, &now_iso()).unwrap();
    }

    fn enrichment(c: &Connection, sym: &str, id: &str, subject: &str, summary: &str, version: i64) {
        sf::set_filing_enrichment(c, sym, id, Some(subject), Some(summary), Some(version), None, &now_iso()).unwrap();
    }

    /// A typed result as `filings_payload_in`/`filings_enrich_in` serve it: the
    /// success shape, or `{"ok": false, "error": …}`.
    fn v<T: serde::Serialize>(r: Result<T, String>) -> Value {
        match r {
            Ok(x) => serde_json::to_value(x).unwrap(),
            Err(e) => json!({"ok": false, "error": e}),
        }
    }

    // --- DisclosuresStoreTest

    #[test]
    fn test_rows_from_two_sources_merge_newest_first() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 3, "")]);
        replace(&c, "SHOP", Regulator::Sec, &[item(Regulator::Sec, 2, ""), item(Regulator::Sec, 4, "")]);
        let rows = sf::filings_for(&c, "SHOP").unwrap();
        assert_eq!(rows.len(), 4);
        let dates: Vec<String> = rows.iter().map(|r| r.doc.date.clone()).collect();
        let mut sorted = dates.clone();
        sorted.sort();
        sorted.reverse();
        assert_eq!(dates, sorted);
        let srcs: HashSet<Regulator> = rows.iter().map(|r| r.doc.source).collect();
        assert_eq!(srcs, HashSet::from([Regulator::Sedar, Regulator::Sec]));
    }

    #[test]
    fn test_replacing_one_source_leaves_the_other() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        replace(&c, "SHOP", Regulator::Sec, &[item(Regulator::Sec, 1, "")]);
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 9, "")]);
        let rows = sf::filings_for(&c, "SHOP").unwrap();
        let mut srcs: Vec<&str> = rows.iter().map(|r| r.doc.source.as_str()).collect();
        srcs.sort();
        assert_eq!(srcs, ["SEC", "SEDAR+"]);
        assert_eq!(rows.iter().filter(|r| r.doc.source == Regulator::Sedar).count(), 1, "SEDAR+ replaced, not appended");
        assert_eq!(rows.iter().filter(|r| r.doc.source == Regulator::Sec).count(), 1, "SEC untouched");
    }

    #[test]
    fn test_a_single_row_is_fetchable_by_id_for_download() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sec, &[item(Regulator::Sec, 7, "")]);
        let row = sf::filing(&c, "SHOP", "sec:7").unwrap().unwrap();
        assert_eq!(row.doc.source, Regulator::Sec);
        assert!(row.doc.url.starts_with("https://www.sec.gov/"));
        assert!(sf::filing(&c, "SHOP", "sec:999").unwrap().is_none());
    }

    #[test]
    fn test_symbols_do_not_bleed_and_the_profile_is_remembered() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        replace(&c, "ATD", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        sf::mark_filings_fetched(&c, "ATD", "000012345", &now_iso()).unwrap();
        assert_eq!(sf::filings_for(&c, "SHOP").unwrap().len(), 1);
        assert_eq!(sf::filings_for(&c, "ATD").unwrap().len(), 2);
        assert_eq!(sf::sedar_profile(&c, "ATD").unwrap(), "000012345");
    }

    #[test]
    fn test_forget_clears_rows_and_stamps() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sec, &[item(Regulator::Sec, 1, "")]);
        sf::mark_filings_fetched(&c, "SHOP", "000037100", &now_iso()).unwrap();
        sf::forget_filings(&c, "SHOP").unwrap();
        assert!(sf::filings_for(&c, "SHOP").unwrap().is_empty());
        assert_eq!(sf::filings_fetched_for(&c, "SHOP").unwrap(), "");
        assert_eq!(sf::sedar_profile(&c, "SHOP").unwrap(), "");
    }

    // --- FilingsPayloadTest

    fn stub(items: Vec<FiledDocument>, sources: &[(Regulator, SourceOutcome)]) -> impl Fn(&str, &str, &str, &str, &str) -> Gathered {
        let sources: BTreeMap<Regulator, SourceOutcome> = sources.iter().cloned().collect();
        move |_, _, _, _, _| Gathered { items: items.clone(), sources: sources.clone() }
    }

    fn outcome(available: bool, matched: bool, count: usize, error: &str) -> SourceOutcome {
        SourceOutcome { available, matched, filer: false, count, error: error.to_string() }
    }

    fn payload(c: &Connection, sym: &str, fetch: &dyn Fn(&str, &str, &str, &str, &str) -> Gathered) -> Value {
        let s = sym.trim().to_uppercase();
        v(filings_payload_in(&app(), c, sym, true, &|| refresh_filings_in(&app(), c, &s, None, None, None, fetch)))
    }

    #[test]
    fn test_stale_until_a_fetch_then_fresh_within_a_day() {
        let c = store();
        assert!(filings_stale(&c, "SHOP", None).unwrap());
        sf::mark_filings_fetched(&c, "SHOP", "", &now_iso()).unwrap();
        assert!(!filings_stale(&c, "SHOP", None).unwrap());
    }

    #[test]
    fn test_a_day_old_stamp_is_stale() {
        let c = store();
        let old = crate::app::stamp_of(now_unix() as i64 - 25 * 3600);
        sf::mark_filings_fetched(&c, "SHOP", "", &old).unwrap();
        assert!(filings_stale(&c, "SHOP", None).unwrap());
    }

    #[test]
    fn test_refresh_merges_sources_and_reports_status() {
        let c = store();
        let fetch = stub(
            vec![item(Regulator::Sedar, 1, "000037100"), item(Regulator::Sec, 2, "")],
            &[(Regulator::Sedar, outcome(true, true, 1, "")), (Regulator::Sec, outcome(true, true, 1, ""))],
        );
        let out = payload(&c, "SHOP", &fetch);
        assert_eq!(out["ok"], true);
        // `available` is whether a source can be reached on this machine
        assert_eq!(out["available"], json!(disclosures::available()));
        assert_eq!(out["refreshed"], true);
        assert_eq!(out["filings"].as_array().unwrap().len(), 2);
        assert_eq!(out["profileNo"], "000037100", "the SEDAR+ profile is remembered from the items");
        let keys: HashSet<&str> = out["sources"].as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, HashSet::from(["SEDAR+", "SEC"]));
        assert!(out["categories"].as_array().unwrap().contains(&json!("Financials")));
    }

    #[test]
    fn test_only_one_source_matches() {
        let c = store();
        let fetch = stub(
            vec![item(Regulator::Sec, 1, "")],
            &[(Regulator::Sedar, outcome(true, false, 0, "")), (Regulator::Sec, outcome(true, true, 1, ""))],
        );
        let out = payload(&c, "NVDA", &fetch);
        let srcs: Vec<String> = out["filings"].as_array().unwrap().iter().map(|r| f(r, "source")).collect();
        assert_eq!(srcs, ["SEC"]);
        assert_eq!(out["sources"]["SEC"]["matched"], true);
        assert_eq!(out["sources"]["SEDAR+"]["matched"], false);
    }

    #[test]
    fn test_an_unreachable_source_reads_unavailable_and_carries_its_error() {
        // SEDAR+ could not be reached (an outage, a maintenance page); SEC answered.
        // The failed source must read as unavailable and carry its reason, so the card
        // can say it is unavailable rather than assert the listing has no filer.
        let c = store();
        let fetch = stub(
            vec![item(Regulator::Sec, 1, "")],
            &[(Regulator::Sedar, outcome(false, false, 0, "SEDAR+ did not return the searchReportingIssuers form")), (Regulator::Sec, outcome(true, true, 1, ""))],
        );
        let out = payload(&c, "SHOP", &fetch);
        assert_eq!(out["sources"]["SEDAR+"]["available"], false, "a source that failed to answer is unavailable, whatever its dependency");
        assert_eq!(out["sources"]["SEDAR+"]["error"], "SEDAR+ did not return the searchReportingIssuers form");
        assert_eq!(out["sources"]["SEC"]["error"], "", "the source that answered carries no error");
    }

    #[test]
    fn test_all_sources_unreachable_is_reported() {
        let c = store();
        let fetch = stub(
            vec![],
            &[(Regulator::Sedar, outcome(false, false, 0, "the browser helper is missing")), (Regulator::Sec, outcome(false, false, 0, "network"))],
        );
        let out = payload(&c, "SHOP", &fetch);
        assert_eq!(out["ok"], true, "the endpoint still answers cleanly");
        assert_eq!(out["sourceUnavailable"], true);
        assert_eq!(out["filings"], json!([]));
    }

    /// What the page is sent of a listing's disclosures, pinned in
    /// `tests/golden/filings_payload.json` with what depends on this machine or
    /// on the clock (whether a source is reachable here, the stamps, the local
    /// model's state) left out. After an intended change:
    /// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_disclosures_the_page_is_sent`.
    #[test]
    fn test_the_disclosures_the_page_is_sent_are_what_they_were() {
        let _g = crate::tests_common::guard();
        fn mask(v: &mut Value) {
            match v {
                Value::Object(m) => {
                    for k in ["available", "fetchedAt", "enrichedAt", "summaryStatus", "summaryAvailable"] {
                        if m.contains_key(k) {
                            m.insert(k.into(), json!("*"));
                        }
                    }
                    m.values_mut().for_each(mask);
                }
                Value::Array(a) => a.iter_mut().for_each(mask),
                _ => {}
            }
        }
        let c = store();
        let mut form4 = item(Regulator::Sec, 5, "");
        form4.form = "4".into();
        form4.title = String::new();
        let fetch = stub(
            vec![item(Regulator::Sedar, 1, "000037100"), item(Regulator::Sedar, 3, "000037100"), item(Regulator::Sec, 2, ""), item(Regulator::Sec, 4, ""), form4],
            &[
                (Regulator::Sedar, SourceOutcome { available: true, matched: true, filer: true, count: 2, error: String::new() }),
                (Regulator::Sec, SourceOutcome { available: true, matched: true, filer: true, count: 3, error: String::new() }),
            ],
        );
        let first = payload(&c, "SHOP", &fetch);
        enrichment(&c, "SHOP", "sec:2", "Quarterly report", "Revenue rose.", ENRICH_VERSION);
        enrichment(&c, "SHOP", "sedar:1", "An old reading", "Read by an older logic.", ENRICH_VERSION - 1);
        sf::set_filing_enrichment(&c, "SHOP", "sedar:3", Some("Interim MD&A"), None, Some(ENRICH_VERSION), Some(true), &now_iso()).unwrap();
        let again = v(filings_payload_in(&app(), &c, "SHOP", false, &|| panic!("fresh: not read again")));
        let enriched = v(filings_enrich_in(&c, "SHOP", "sec:2", &fake(true, ("x", "y"), false)));
        let mut have = json!({"first": first, "again": again, "enriched": enriched});
        mask(&mut have);
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/filings_payload.json");
        if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
            return;
        }
        let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/filings_payload.json")).unwrap();
        assert_eq!(have, want, "the disclosures the page is sent are not what they were");
    }

    #[test]
    fn test_empty_symbol_is_rejected() {
        let _g = crate::tests_common::guard();
        assert!(matches!(filings_payload(&app(), "", false, None, None, None), FilingsAnswer::Refused(_)));
    }

    // --- EnrichTest

    struct Fake {
        model: bool,
        status: &'static str,
        wait: bool,
        read: (&'static str, &'static str),
        final_: bool,
        reads: Cell<usize>,
        waits: Cell<usize>,
    }

    impl Readers for Fake {
        fn summary_available(&self) -> bool { self.model }
        fn summary_status(&self) -> &'static str { self.status }
        fn wait_for_summary(&self, _: f64) -> bool { self.waits.set(self.waits.get() + 1); self.wait }
        fn disclosures_available(&self) -> bool { true }
        fn enrichment(&self, _: &FiledDocument) -> Option<disclosures::Enrichment> { None }
        fn content(&self, _: &FiledDocument) -> disclosures::Fetched<(Vec<u8>, String)> {
            self.reads.set(self.reads.get() + 1);
            Ok((b"%PDF-1.4 body".to_vec(), "application/pdf".into()))
        }
        fn enrich_document_of(&self, _: &str, _: &str, _: &[u8], _: &str) -> disclosures::Enrichment {
            disclosures::Enrichment { subject: self.read.0.to_string(), summary: self.read.1.to_string(), final_: self.final_ }
        }
    }

    fn fake(model: bool, read: (&'static str, &'static str), final_: bool) -> Fake {
        Fake { model, status: if model { "ready" } else { "off" }, wait: false, read, final_, reads: Cell::new(0), waits: Cell::new(0) }
    }

    const DOC: &str = "sedar:1";

    fn enrich_store() -> Connection {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        c
    }

    fn stored(c: &Connection) -> (String, String, i64) {
        let row = sf::filing(c, "QNC", DOC).unwrap().unwrap();
        (row.subject, row.summary, row.enrich_version.unwrap_or(0))
    }

    fn run(c: &Connection, model: bool, read: (&'static str, &'static str), final_: bool) -> (Value, usize) {
        let r = fake(model, read, final_);
        let out = v(filings_enrich_in(c, "QNC", DOC, &r));
        (out, r.reads.get())
    }

    fn halves(c: &Connection) -> (String, String) {
        let s = stored(c);
        (s.0, s.1)
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn test_a_row_with_both_halves_is_not_read_again() {
        let c = enrich_store();
        run(&c, true, ("A title", "A sentence."), false);
        assert_eq!(halves(&c), pair("A title", "A sentence."));
        let (out, reads) = run(&c, true, ("other", "other."), false);
        assert_eq!(reads, 0);
        assert_eq!(out["subject"], "A title");
    }

    #[test]
    fn test_a_document_read_for_good_is_never_fetched_again_even_with_nothing_to_show() {
        let c = enrich_store();
        let (out, _) = run(&c, true, ("", ""), true);
        assert_eq!((f(&out, "subject"), f(&out, "summary")), pair("", ""));
        assert_eq!(stored(&c).2, ENRICH_VERSION, "stamped, so the row is done");
        for _ in 0..3 {
            let (_, reads) = run(&c, true, ("", ""), true);
            assert_eq!(reads, 0, "the document is not fetched again");
        }
    }

    #[test]
    fn test_a_document_read_for_good_while_no_model_was_up_is_still_done() {
        let c = enrich_store();
        run(&c, false, ("Exempt distribution of $1,500,000", "$1,500,000 distributed."), true);
        assert_eq!(halves(&c), pair("Exempt distribution of $1,500,000", "$1,500,000 distributed."));
        let (_, reads) = run(&c, true, ("other", "other."), true);
        assert_eq!(reads, 0, "a form needs no model, so a model arriving later changes nothing");
    }

    #[test]
    fn test_a_row_holding_only_a_title_is_read_again_for_its_summary() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        assert_eq!(halves(&c), pair("A title", ""));
        let (out, reads) = run(&c, true, ("A title", "The sentence."), false);
        assert_eq!(reads, 1);
        assert_eq!(out["summary"], "The sentence.");
    }

    #[test]
    fn test_a_row_holding_only_a_summary_is_read_again_for_its_title() {
        let c = enrich_store();
        run(&c, true, ("", "A sentence."), false);
        assert_eq!(halves(&c), pair("", "A sentence."));
        let (out, reads) = run(&c, true, ("The title", "A sentence."), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "The title");
    }

    #[test]
    fn test_reading_again_fills_what_is_missing_and_empties_nothing() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        let (out, reads) = run(&c, true, ("", ""), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "A title");
        assert_eq!(stored(&c).0, "A title");
    }

    #[test]
    fn test_the_first_read_under_the_current_logic_still_clears_an_older_junk_title() {
        let c = enrich_store();
        enrichment(&c, "QNC", DOC, "00012345.pdf", "", 1);
        let (out, reads) = run(&c, true, ("", "A sentence."), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "");
        assert_eq!(out["summary"], "A sentence.");
    }

    #[test]
    fn test_with_no_model_up_a_row_already_read_is_not_fetched_again() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        let (out, reads) = run(&c, false, ("A title", "never asked"), false);
        assert_eq!(reads, 0);
        assert_eq!(out["summary"], "");
    }

    #[test]
    fn test_a_row_never_read_is_read_even_with_no_model() {
        let c = enrich_store();
        let (out, reads) = run(&c, false, ("A title", ""), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "A title");
        assert_eq!(halves(&c), pair("A title", ""), "the title found is kept and shown");
    }

    #[test]
    fn test_a_row_that_found_no_model_waits_and_is_not_fetched_again_until_one_is_up() {
        let c = enrich_store();
        run(&c, false, ("A title", ""), false);
        for _ in 0..3 {
            let (_, reads) = run(&c, false, ("A title", ""), false);
            assert_eq!(reads, 0, "with no model up, the document is not fetched from the regulator again");
        }
        let row = sf::filing(&c, "QNC", DOC).unwrap().unwrap();
        assert!(wants_reading(&row) && !row.enrich_final, "the row waits rather than finishing");
        let (out, reads) = run(&c, true, ("A title", "The sentence."), false);
        assert_eq!(reads, 1, "a model coming up is what reads it again");
        assert_eq!(out["summary"], "The sentence.");
    }

    #[test]
    fn test_a_half_filled_row_is_read_at_most_twice_with_a_model_up_then_settles() {
        for half in [("A title", ""), ("", "A sentence.")] {
            let c = enrich_store();
            let mut fetched = 0;
            for _ in 0..6 {
                fetched += run(&c, true, half, false).1;
            }
            assert_eq!(fetched, ENRICH_READS as usize, "two reads that could have answered, then no more");
            let row = sf::filing(&c, "QNC", DOC).unwrap().unwrap();
            assert_eq!((row.subject.as_str(), row.summary.as_str()), half, "it settles with whichever half it has");
            assert!(row.enrich_final && !wants_reading(&row));
        }
    }

    #[test]
    fn test_reads_with_no_model_up_do_not_count_against_the_document() {
        let c = enrich_store();
        let mut fetched = 0;
        for _ in 0..4 {
            fetched += run(&c, false, ("A title", ""), false).1;
        }
        assert_eq!(fetched, 1);
        let mut with_model = 0;
        for _ in 0..4 {
            with_model += run(&c, true, ("A title", ""), false).1;
        }
        assert_eq!(with_model, ENRICH_READS as usize, "the bound is reads a model could have answered, however many came before");
    }

    #[test]
    fn test_reads_under_an_older_logic_do_not_count_under_this_one() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        run(&c, true, ("A title", ""), false);
        assert!(sf::filing(&c, "QNC", DOC).unwrap().unwrap().enrich_final);
        enrichment(&c, "QNC", DOC, "A title", "", ENRICH_VERSION - 1);
        let mut fetched = 0;
        for _ in 0..4 {
            fetched += run(&c, true, ("A title", ""), false).1;
        }
        assert_eq!(fetched, ENRICH_READS as usize);
    }

    #[test]
    fn test_a_count_survives_the_list_being_read_again() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        assert_eq!(sf::filing(&c, "QNC", DOC).unwrap().unwrap().enrich_reads, 1);
    }

    // --- the reader that works behind the page

    #[test]
    fn test_the_background_reader_picks_a_titled_row_without_its_sentence() {
        let c = enrich_store();
        run(&c, false, ("A title", ""), false);
        let r = fake(true, ("A title", "The sentence."), false);
        let mut passed = HashSet::new();
        assert_eq!(read_one_in(&c, &["QNC".to_string()], &mut passed, &r), Ok(Some(Ok(()))));
        assert_eq!(r.reads.get(), 1);
        assert_eq!(halves(&c), pair("A title", "The sentence."));
        assert_eq!(read_one_in(&c, &["QNC".to_string()], &mut passed, &r), Ok(None), "nothing is left to read");
    }

    #[test]
    fn test_the_background_reader_ends_on_rows_it_cannot_move() {
        let c = enrich_store();
        run(&c, false, ("A title", ""), false);
        let r = fake(false, ("A title", ""), false);
        let mut passed = HashSet::new();
        let mut tries = 0;
        while read_one_in(&c, &["QNC".to_string()], &mut passed, &r).unwrap().is_some() {
            tries += 1;
            assert!(tries < 5, "a row a read cannot move is passed over, not read in a loop");
        }
        assert_eq!(r.reads.get(), 0);
    }

    #[test]
    fn test_what_wants_reading() {
        let c = enrich_store();
        let row = || sf::filing(&c, "QNC", DOC).unwrap().unwrap();
        assert!(wants_reading(&row()), "never read");
        enrichment(&c, "QNC", DOC, "A title", "", ENRICH_VERSION);
        assert!(wants_reading(&row()), "a title without its sentence");
        enrichment(&c, "QNC", DOC, "", "A sentence.", ENRICH_VERSION);
        assert!(wants_reading(&row()), "a sentence without its title");
        enrichment(&c, "QNC", DOC, "A title", "A sentence.", ENRICH_VERSION);
        assert!(!wants_reading(&row()), "both halves");
        enrichment(&c, "QNC", DOC, "A title", "A sentence.", ENRICH_VERSION - 1);
        assert!(wants_reading(&row()), "read under an older logic");
        sf::set_filing_enrichment(&c, "QNC", DOC, Some("A title"), Some(""), Some(ENRICH_VERSION), Some(true), &now_iso()).unwrap();
        assert!(!wants_reading(&row()), "settled");
    }

    // --- WaitingForTheModelTest

    #[test]
    fn test_a_model_that_is_starting_is_waited_for_rather_than_the_read_wasted() {
        let c = enrich_store();
        let r = Fake { model: false, status: "ready", wait: true, read: ("A title", "A sentence."), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = v(filings_enrich_in(&c, "QNC", DOC, &r));
        assert_eq!(r.waits.get(), 1);
        assert_eq!(out["summary"], "A sentence.");
        assert_eq!(stored(&c), ("A title".to_string(), "A sentence.".to_string(), ENRICH_VERSION));
    }

    #[test]
    fn test_a_model_that_never_comes_up_leaves_the_row_to_be_read_again() {
        let c = enrich_store();
        let r = Fake { model: false, status: "off", wait: false, read: ("A title", ""), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = v(filings_enrich_in(&c, "QNC", DOC, &r));
        assert_eq!(out["subject"], "A title");
        let row = sf::filing(&c, "QNC", DOC).unwrap().unwrap();
        assert!(wants_reading(&row) && !row.enrich_final && row.enrich_reads == 0, "the read did not count, and the row waits");
    }

    // --- RefreshKeepsWhatWasReadTest

    #[test]
    fn test_a_row_the_source_still_lists_keeps_its_subject_and_summary() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        enrichment(&c, "QNC", "sedar:1", "A title", "A sentence.", 9);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, ""), item(Regulator::Sedar, 3, "")]);
        let row = sf::filing(&c, "QNC", "sedar:1").unwrap().unwrap();
        assert_eq!((row.subject, row.summary, row.enrich_version), ("A title".into(), "A sentence.".into(), Some(9)));
    }

    #[test]
    fn test_what_the_source_says_about_a_row_is_still_refreshed() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        enrichment(&c, "QNC", "sedar:1", "A title", "A sentence.", 9);
        let mut moved = item(Regulator::Sedar, 1, "");
        moved.url = "https://www.sedarplus.ca/x?drmKey=fresh".into();
        replace(&c, "QNC", Regulator::Sedar, &[moved]);
        let row = sf::filing(&c, "QNC", "sedar:1").unwrap().unwrap();
        assert_eq!(row.doc.url, "https://www.sedarplus.ca/x?drmKey=fresh");
        assert_eq!(row.summary, "A sentence.");
    }

    #[test]
    fn test_a_row_the_source_no_longer_lists_goes() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 2, "")]);
        assert!(sf::filing(&c, "QNC", "sedar:1").unwrap().is_none());
        assert!(sf::filing(&c, "QNC", "sedar:2").unwrap().is_some());
    }

    #[test]
    fn test_another_sources_rows_are_untouched() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        replace(&c, "QNC", Regulator::Sec, &[item(Regulator::Sec, 1, "")]);
        enrichment(&c, "QNC", "sec:1", "From EDGAR", "A sentence.", 9);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        assert_eq!(sf::filing(&c, "QNC", "sec:1").unwrap().unwrap().subject, "From EDGAR");
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
            assert_eq!(read_fear_with(&a, index, || Err(why.clone())).unwrap_err(), why);
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
        let _ = read_fear_with(&a, "stocks", || Err("CNN could not be reached.".into()));
        assert_eq!(feed_failures(&a), vec!["CNN could not be reached.".to_string()]);
        assert!(crate::status::status(&a).error.contains("CNN could not be reached."), "the header says it");
        assert_eq!(fear_stored(&a, "stocks").gauge.map(|g| g.gauge.score), Some(40.0), "the older reading stays drawn");
        read_fear_with(&a, "stocks", || Ok(gauge(55.0))).unwrap();
        assert!(feed_failures(&a).is_empty(), "gone once the index answers");
        let _ = read_fear_with(&a, "crypto", || Err(bagholder_net::client::OFFLINE.into()));
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
        *a.feeds.news_left.lock().unwrap() = ["*".to_string(), "QNC".to_string()].into_iter().collect();
        assert_eq!(read(&a), json!({"reading": ["*", "QNC"]}));
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
        serde_json::to_value(listing_payload_in(&[], positions, trades, watchlist, args.0, args.1, args.2, args.3, &quote)).unwrap()
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
        let out = serde_json::to_value(listing_payload_in(&[], &[], &[], &[], "RY", "TSX", "CAD", "", &failed)).unwrap();
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

    #[test]
    fn test_a_ticker_with_no_venue_matches_the_book_whatever_venue_it_holds_it_on() {
        let trades = [json!({"id": "t1", "symbol": "SHOP.TO", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "name": "Shopify Inc.",
                             "fills": [fill(FILL, "BUY", 10.0, 5.0)]})];
        let out = listing(&[], &trades, &[], ("1.25", -0.02), ("SHOP", "", "", ""));
        assert_eq!((f(&out, "symbol"), f(&out, "exchange"), f(&out, "name")), ("SHOP".into(), "TSX".into(), "Shopify Inc.".into()));
        assert_eq!(out["fills"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_a_ticker_that_is_not_one_is_refused() {
        assert_eq!(listing(&[], &[], &[], ("1.25", -0.02), ("  ", "", "", ""))["ok"], false);
    }
}
