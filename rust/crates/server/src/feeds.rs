//! Market data, news, filings and the sweeps behind them: the watchlist and
//! tile writes, the wires, the Disclosures sweep and its notices, the fear
//! meters, short selling, the heatmap universes, quotes, the periodic market
//! records, the intraday archive, the watched folder and the chart history.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ts_rs::TS;

use bagholder_market::{disclosures, edgar, enrich, exposure, fear, history, localmodel, news, sedar, shorts};
use bagholder_model::base::Base;
use bagholder_model::instruments;
use bagholder_model::venues::{tmx_form, tmx_symbol};
use bagholder_store::bars::ChartBars;
use bagholder_store::feeds as sf;
use bagholder_store::market as sf_market;
use bagholder_store::tables::{get_meta, json_text, set_meta};

use crate::app::{f, log, now_iso, now_unix, num, parse_instant, spawn, truthy, App, ENRICH_VERSION};
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
    /// Whether the heatmap universes were asked for sooner than their own clock.
    universe_kick: (Mutex<bool>, Condvar),
    /// How many times a notice has sent for the issuer's record. A test counts the
    /// reads rather than reaching the source.
    #[cfg(test)]
    pub(crate) record_reads: AtomicI64,
    /// Listings the running short-interest sweep has still to read.
    shorts_left: AtomicI64,
}

fn conn(app: &Arc<App>) -> Option<bagholder_store::pool::Pooled<'_>> {
    app.open().ok()
}

/// A connection of the caller's own, not the pool's: for work that hands it to
/// threads it starts itself and keeps it for the length of a pass.
fn own_conn(app: &Arc<App>) -> Option<Connection> {
    bagholder_store::connect(&app.home).ok()
}

fn base(app: &Arc<App>) -> Option<Arc<Base>> {
    app.base().ok()
}

fn is_true(v: &Value, k: &str) -> bool {
    truthy(v.get(k))
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
    Sec(Value),
    Under(String, String),
    Watch(String, String, String),
}

/// The exposure record of every held security
/// that has none or an old one, four at a time, each shown as it lands.
pub fn refresh_exposures(app: &Arc<App>) -> Value {
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": true, "held": 0, "refreshed": 0}) };
    let b = match base(app) { Some(b) => b, None => return json!({"ok": true, "held": 0, "refreshed": 0}) };
    let snap = bagholder_store::snapshot::snapshot(&c, false).unwrap_or(json!({}));
    let mut secs: HashMap<String, Value> = HashMap::new();
    for sec in snap.get("securities").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let id = f(&sec, "id");
        if !id.is_empty() {
            secs.insert(id, sec);
        }
    }
    let mut held: HashSet<String> = b.positions.iter().filter(|p| p.kind == bagholder_model::activity::Kind::Shares).map(|p| p.security_id.clone()).collect();
    for bal in snap.get("balances").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let sid = f(&bal, "securityId");
        if num(bal.get("quantity"), Some(0.0)).unwrap_or(0.0) > 0.0 && sid.starts_with("sec-s-") {
            held.insert(sid);
        }
    }
    let mut held: Vec<String> = held.into_iter().filter(|sid| !sid.is_empty() && !sid.starts_with("sec-c-")).collect();
    held.sort();
    let (today_s, _, _) = bagholder_market::clock_now();
    let ctx = exposure::Ctx { conn: &c, pool: app.store(), today: today_s.clone() };
    let mut todo: Vec<String> = exposure::stale(&ctx, &held).into_iter().filter(|sid| secs.contains_key(sid)).collect();
    todo.sort_by_key(|sid| if exposure::is_fund(&f(&secs[sid], "name")) { 1 } else { 0 });
    let mut unders: Vec<(String, String)> = b
        .positions
        .iter()
        .filter(|p| p.kind == bagholder_model::activity::Kind::Options && !p.underlying.is_empty())
        .map(|p| (p.underlying.to_uppercase(), p.currency.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unders.sort();
    let unders: Vec<(String, String)> = unders
        .into_iter()
        .filter(|(u, cc)| !exposure::stale(&ctx, &[format!("{}{}:{}", exposure::SHARE_KEY, u, tmx_form("", cc).unwrap_or(""))]).is_empty())
        .collect();
    let watched: Vec<(String, String, String)> = b
        .watchlist
        .iter()
        .filter(|w| instruments::find(&w.symbol, &w.exchange).is_none() && w.exchange.to_uppercase() != "CRYPTO")
        .map(|w| (w.symbol.clone(), w.exchange.clone(), w.currency.clone()))
        .filter(|(sy, e, cc)| !exposure::stale(&ctx, &[bagholder_model::symbols_of::watch_exposure_key(sy, e, cc)]).is_empty())
        .collect();

    let mut jobs: Vec<ExposureJob> = todo.iter().map(|sid| ExposureJob::Sec(secs[sid].clone())).collect();
    jobs.extend(unders.into_iter().map(|(u, cc)| ExposureJob::Under(u, cc)));
    jobs.extend(watched.into_iter().map(|(sy, e, cc)| ExposureJob::Watch(sy, e, cc)));
    let held_n = held.len();
    drop(ctx);
    if jobs.is_empty() {
        return json!({"ok": true, "held": held_n, "refreshed": 0});
    }
    let queue = Arc::new(Mutex::new(jobs.into_iter().collect::<std::collections::VecDeque<_>>()));
    let done = Arc::new(AtomicI64::new(0));
    let mut handles = Vec::new();
    for _ in 0..EXPOSURE_WORKERS {
        let queue = queue.clone();
        let done = done.clone();
        let today_s = today_s.clone();
        let app = app.clone();
        let h = std::thread::Builder::new().name("bagholder-exposure".into()).spawn(move || {
            let c = match conn(&app) { Some(c) => c, None => return };
            let ctx = exposure::Ctx { conn: &c, pool: app.store(), today: today_s };
            loop {
                let job = match queue.lock().unwrap().pop_front() { Some(j) => j, None => return };
                if app.stopping() {
                    continue;
                }
                let line = match job {
                    ExposureJob::Sec(sec) => {
                        let rec = exposure::refresh_security(&ctx, &sec);
                        let cov = num(rec.get("coverage"), Some(0.0)).unwrap_or(0.0);
                        let source = f(&rec, "source");
                        let err = f(&rec, "error");
                        format!(
                            "bagholder exposure: {} {}: {} ({}% covered){}",
                            f(&sec, "symbol"),
                            if exposure::is_fund(&f(&sec, "name")) { "fund" } else { "share" },
                            if source.is_empty() { "no source".to_string() } else { source },
                            (cov * 100.0).round_ties_even() as i64,
                            if err.is_empty() { String::new() } else { format!(": {}", err) }
                        )
                    }
                    ExposureJob::Watch(sym, ex, ccy) => {
                        exposure::share_exposure(&ctx, &sym, &ex, &ccy);
                        format!("bagholder exposure: {} (watched) classified", sym)
                    }
                    ExposureJob::Under(under, ccy) => {
                        exposure::share_exposure(&ctx, &under, "", &ccy);
                        format!("bagholder exposure: {} (an option's underlying) classified", under)
                    }
                };
                done.fetch_add(1, Ordering::SeqCst);
                log(&line);
                // each record shows as soon as it lands
            }
        });
        if let Ok(h) = h {
            handles.push(h);
        }
    }
    for h in handles {
        let _ = h.join();
    }
    json!({"ok": true, "held": held_n, "refreshed": done.load(Ordering::SeqCst)})
}

/// Soon after start and every half hour.
pub fn exposure_loop(app: Arc<App>) {
    // What a fund holds changes over months. The records are looked over shortly after
    // start, then when the set of listings they are kept for changes (a trade, a
    // watchlist row), and otherwise a few times a day -- not every half hour for ever.
    let listed = || conn(&app).and_then(|c| bagholder_store::gens::all(&c).ok()).map(|g| bagholder_store::gens::key(&g, &["activities", "watchlist", "securities"])).unwrap_or_default();
    if app.wait(Duration::from_secs(EXPOSURE_FIRST_SEC)) {
        return;
    }
    loop {
        let before = listed();
        refresh_exposures(&app);
        app.events.park_until_or(&app, Duration::from_secs(6 * 3600), || listed() != before);
        if app.stopping() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// watchlist and tiles
// ---------------------------------------------------------------------------

pub const TILES_MAX: usize = 12;

fn refresh_quote_symbols(app: &Arc<App>, what: &str, sym: &str) -> bool {
    let (c, b) = match (conn(app), base(app)) { (Some(c), Some(b)) => (c, b), _ => return false };
    let (today_s, now, stamp) = bagholder_market::clock_now();
    match bagholder_market::quotes::refresh_quotes(&c, &bagholder_model::markets::quote_symbols(&b), &today_s, now, &stamp) {
        Ok(_) => {
            true
        }
        Err(e) => {
            if what == "watchlist" {
                log(&format!("bagholder watchlist: quote for {} failed: {}", sym, e));
            } else {
                log(&format!("bagholder tiles: quotes failed: {}", e));
            }
            false
        }
    }
}

pub fn watch_add(app: &Arc<App>, body: &Value) -> Value {
    let sym = tmx_symbol(&f(body, "symbol"));
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    let ex = f(body, "exchange");
    let inst = instruments::find(&sym, &ex);
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "store unavailable"}) };
    let name = inst.map(|i| i.name.to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| f(body, "name"));
    let ccy = inst.map(|i| i.currency.to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| f(body, "currency"));
    let row = sf::add_watch(&c, &sym, &ex, &name, &ccy, &f(body, "securityId"), &now_iso()).ok().flatten().unwrap_or(json!({}));
    let is_inst = inst.is_some();
    let crypto = ex.to_uppercase() == "CRYPTO";
    let sym2 = sym.clone();
    let a = app.clone();
    spawn("watch-fetch", move || {
        // its quote and its sector, from the same public sources a holding uses
        refresh_quote_symbols(&a, "watchlist", &sym2);
        if is_inst || crypto {
            return;
        }
        if let Some(c) = conn(&a) {
            let ctx = exposure::Ctx { conn: &c, pool: a.store(), today: today() };
            exposure::share_exposure(&ctx, &f(&row, "symbol"), &f(&row, "exchange"), &f(&row, "currency"));
        }
    });
    json!({"ok": true, "watchlist": sf::list_watchlist(&c).unwrap_or_default()})
}

pub fn watch_remove(app: &Arc<App>, body: &Value) -> Value {
    let sym = tmx_symbol(&f(body, "symbol"));
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "store unavailable"}) };
    let ex = f(body, "exchange");
    let _ = sf::remove_watch(&c, &sym, &ex);
    // a row kept under Wealthsimple's form
    let _ = sf::remove_watch(&c, &f(body, "symbol").trim().to_uppercase(), &ex);
    let _ = sf::forget_news(&c, &sym, &ex);
    json!({"ok": true, "watchlist": sf::list_watchlist(&c).unwrap_or_default()})
}

/// The Markets tab's tile row, only instruments the
/// directory knows, twelve at most.
pub fn tiles_set(app: &Arc<App>, body: &Value) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut seen: HashSet<&'static str> = HashSet::new();
    for r in body.get("tiles").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        if !r.is_object() {
            continue;
        }
        if let Some(inst) = instruments::find(&f(&r, "symbol"), &f(&r, "exchange")) {
            if seen.insert(inst.symbol) {
                rows.push(json!({"symbol": inst.symbol, "exchange": inst.exchange}));
            }
        }
    }
    if rows.len() > TILES_MAX {
        return json!({"ok": false, "error": format!("at most {} tiles", TILES_MAX)});
    }
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "store unavailable"}) };
    let _ = bagholder_store::admin::save_tiles(&c, &rows);
    let a = app.clone();
    spawn("tiles-fetch", move || {
        refresh_quote_symbols(&a, "tiles", "");
    });
    let tiles = base(app).map(|b| bagholder_model::markets::tile_rows(&b)).unwrap_or_default();
    json!({"ok": true, "tiles": tiles})
}

// ---------------------------------------------------------------------------
// news
// ---------------------------------------------------------------------------

/// The market feed, the shares held, the watched listings. One listing, one
/// read, under its bare ticker: the book's QNC.TO and the watchlist's QNC are
/// the same wire. The name the book records for it is what Google is searched
/// for.
pub fn news_listings(app: &Arc<App>) -> Vec<news::Listing> {
    let mut out = vec![(news::MARKET.0.to_string(), news::MARKET.1.to_string(), news::MARKET.2.to_string(), String::new())];
    let b = match base(app) { Some(b) => b, None => return out };
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for p in b.positions.iter().filter(|p| p.kind == bagholder_model::activity::Kind::Shares) {
        let key = (tmx_symbol(&p.symbol), p.exchange.to_uppercase());
        if !key.0.is_empty() && seen.insert(key.clone()) {
            out.push((key.0, p.exchange.clone(), p.currency.clone(), p.name.clone()));
        }
    }
    for w in b.watchlist.iter() {
        let key = (tmx_symbol(&w.symbol), w.exchange.to_uppercase());
        if !key.0.is_empty() && key.1 != "CRYPTO" && instruments::find(&w.symbol, &w.exchange).is_none() && seen.insert(key.clone()) {
            out.push((key.0, w.exchange.clone(), w.currency.clone(), w.name.clone()));
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

/// Every source for every listing with one due. Never fails. Each listing's
/// items reach the model as it lands, and the listings still to read are in
/// the status, so the News card says a read is under way instead of `No
/// news.` while a pass runs.
pub fn refresh_news(app: &Arc<App>) -> usize {
    app.single_flight("news", 0, || {
        let listings = news_listings(app);
        let (today_s, now, _) = bagholder_market::clock_now();
        let clock = news::Clock { today: today_s, now: now as i64 };
        let key = |l: &news::Listing| { let t = tmx_symbol(&l.0); (if t.is_empty() { l.0.clone() } else { t }).to_uppercase() };
        let start = |due: &[news::Listing]| {
            *app.feeds.news_left.lock().unwrap() = due.iter().map(key).collect();
            app.events.signal(); // the status names the listings still to read
        };
        let done = |l: &news::Listing, _answered: bool| {
            app.feeds.news_left.lock().unwrap().remove(&key(l));
            app.events.signal();
        };
        let on_new = |c: &Connection, sym: &str, ex: &str, rows: &[Value], ids: &[String]| note_wire_releases(app, c, sym, ex, rows, ids);
        let got = news::refresh(&|| own_conn(app), &news::LIVE_READERS, &listings, &clock, Some(&on_new), Some(&start), Some(&done), news::LISTINGS_AT_ONCE);
        app.feeds.news_left.lock().unwrap().clear();
        app.events.signal();
        match got {
            Ok(n) => n,
            Err(e) => {
                log(&format!("bagholder news: refresh failed: {}", e));
                0
            }
        }
    })
}

/// At start, then every five minutes, each listing read once per fifteen.
/// Whether anyone is owed the news: a page is open to show it, or a Releases
/// notification set is on and must hear of a release with no page open.
fn news_wanted(app: &Arc<App>) -> bool {
    app.events.watchers() > 0 || conn(app).map_or(false, |c| crate::notify::any_release_scope(&c))
}

pub fn news_loop(app: Arc<App>) {
    // no wire pushes, so the sources are read; but only while someone is owed them
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
pub fn news_symbol_payload(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str) -> Value {
    news_symbol_payload_with(app, symbol, exchange, currency, &news::LIVE_READERS, &|c, sym, today| bagholder_market::tmx::tmx_listing(c, sym, today))
}

pub fn news_symbol_payload_with(
    app: &Arc<App>,
    symbol: &str,
    exchange: &str,
    currency: &str,
    readers: &news::Readers,
    listing_of: &dyn Fn(&Connection, &str, &str) -> Option<Value>,
) -> Value {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "the wire did not answer"}) };
    let (today_s, now, _) = bagholder_market::clock_now();
    let clock = news::Clock { today: today_s.clone(), now: now as i64 };
    let (mut ex, mut ccy) = (exchange.trim().to_string(), currency.trim().to_string());
    // the name is what Google is searched for: the security record's, else the one TMX's quote gives
    let (known, known_ex, known_ccy) = instrument_meta(&c, &sym);
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
        listing = listing_of(&c, &sym, &today_s);
    }
    if let Some(l) = &listing {
        if name.is_empty() && f(l, "name").to_uppercase() != sym {
            name = f(l, "name");
        }
        if ex.is_empty() {
            ex = f(l, "exchange");
            ccy = f(l, "currency");
        }
    }
    if ex.is_empty() {
        let form = bagholder_market::tmx::tmx_resolve(&c, &tmx_symbol(&sym), &today_s);
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
        Err(_) => return json!({"ok": false, "error": "the wire did not answer"}),
    };
    let rows = match rows { Some(r) => r, None => return json!({"ok": false, "error": "the wire did not answer"}) };
    let _ = sf::trim_news(&c, news::KEEP);
    json!({"ok": true, "count": rows.len(), "source": src, "exchange": ex})
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
pub fn instrument_meta(c: &Connection, symbol: &str) -> (String, String, String) {
    let sym = symbol.trim().to_uppercase();
    for sec in bagholder_store::admin::list_securities(c).unwrap_or_default() {
        if f(&sec, "symbol").trim().to_uppercase() == sym {
            let name = f(&sec, "name").trim().to_string();
            return (if name.is_empty() { sym } else { name }, f(&sec, "primaryExchange").trim().to_string(), f(&sec, "currency").trim().to_string());
        }
    }
    (sym, String::new(), String::new())
}

pub fn filings_stale(c: &Connection, symbol: &str, hours: Option<f64>) -> bool {
    let when = sf::filings_fetched_for(c, symbol).unwrap_or_default();
    if when.is_empty() {
        return true;
    }
    match parse_instant(&when) {
        Some(then) => now_unix() - then > hours.unwrap_or(FILINGS_STALE_HOURS) * 3600.0,
        None => true,
    }
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
fn naming_held(c: &Connection, holding: bool) -> bool {
    if !holding {
        let _ = set_meta(c, FILINGS_HOLD_KEY, "");
        return false;
    }
    let since = get_meta(c, FILINGS_HOLD_KEY, "").unwrap_or_default();
    if since.is_empty() {
        let _ = set_meta(c, FILINGS_HOLD_KEY, &now_iso());
        return true;
    }
    match parse_instant(&since) {
        Some(then) => now_unix() - then <= FILINGS_HOLD_MAX_MIN * 60.0,
        None => true,
    }
}

/// What makes a filing itself.
pub fn filing_mark(r: &Value) -> [String; 5] {
    [f(r, "source"), f(r, "date"), f(r, "type"), f(r, "title"), f(r, "size")]
}

fn providers_cover(sym: &str, ex: &str, ccy: &str) -> bool {
    (sedar::available() && sedar::covers(sym, ex, ccy)) || (edgar::available() && edgar::covers(sym, ex, ccy))
}

/// The tickers to watch for filings.
pub fn known_filing_symbols(app: &Arc<App>, scopes: &[String]) -> Vec<Value> {
    let has = |k: &str| scopes.iter().any(|x| x == k);
    let mut rows: Vec<Value> = Vec::new();
    let mut b = None;
    if has("held") || has("all") {
        match app.base() {
            Ok(x) => b = Some(x),
            Err(e) => log(&format!("bagholder disclosures: the book not read for the sweep: {}", e)),
        }
    }
    if let Some(b) = &b {
        rows.extend(bagholder_model::symbols_of::held_symbols(b).iter().map(|l| l.to_value()));
        if has("all") {
            for t in b.trades.iter() {
                let mut rec = json!({"symbol": t.symbol, "exchange": t.exchange, "currency": t.currency, "kind": t.kind});
                if f(&rec, "kind") == "Options" {
                    let under = bagholder_model::symbols::underlying_symbol(&f(&rec, "symbol"));
                    if under.is_empty() || under == "—" {
                        continue;
                    }
                    rec = json!({"symbol": under, "exchange": rec["exchange"], "currency": rec["currency"], "kind": "Shares"});
                }
                rows.push(rec);
            }
        }
    }
    if has("watched") || has("all") {
        if let Some(c) = conn(app) {
            rows.extend(sf::list_watchlist(&c).unwrap_or_default());
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for r in rows {
        let sym = f(&r, "symbol").trim().to_uppercase();
        let kind = f(&r, "kind");
        if sym.is_empty() || seen.contains(&sym) || sym.contains(' ') || kind == "Options" || kind == "Crypto" {
            continue;
        }
        if !providers_cover(&sym, &f(&r, "exchange"), &f(&r, "currency")) {
            continue;
        }
        seen.insert(sym.clone());
        let name = f(&r, "name");
        out.push(json!({"symbol": sym, "name": if name.is_empty() { Value::Null } else { json!(name) }, "exchange": f(&r, "exchange"), "currency": f(&r, "currency")}));
    }
    out
}

/// While a Disclosures or Releases set is on, each
/// chosen ticker not read within half an hour is read again and what is new is
/// told. Returns how many tickers had something new.
pub fn sweep_filings(app: &Arc<App>) -> usize {
    let c = match conn(app) { Some(c) => c, None => return 0 };
    let scopes = notify::disclosure_scopes(&c);
    let rel_scopes = notify::release_scopes(&c);
    if scopes.is_empty() && rel_scopes.is_empty() {
        return 0;
    }
    let mut told = 0;
    let disc_syms: HashSet<String> = if scopes.is_empty() { HashSet::new() } else { known_filing_symbols(app, &scopes).iter().map(|i| f(i, "symbol")).collect() };
    let hold = !disc_syms.is_empty() && naming_held(&c, !can_name_documents());
    let mut both: Vec<String> = scopes.clone();
    for r in &rel_scopes {
        if !both.contains(r) {
            both.push(r.clone());
        }
    }
    for inst in known_filing_symbols(app, &both) {
        let sym = f(&inst, "symbol");
        if !filings_stale(&c, &sym, Some(FILINGS_SWEEP_AGE_MIN / 60.0)) {
            continue;
        }
        if hold && disc_syms.contains(&sym) {
            continue;
        }
        let before: HashSet<[String; 5]> = sf::filings_for(&c, &sym).unwrap_or_default().iter().map(filing_mark).collect();
        let name = f(&inst, "name");
        let wrote = refresh_filings(app, &sym, if name.is_empty() { None } else { Some(&name) }, Some(&f(&inst, "exchange")), Some(&f(&inst, "currency")));
        if wrote < 0 {
            continue;
        }
        let mut by_source: Vec<(String, Vec<Value>)> = Vec::new();
        for r in sf::filings_for(&c, &sym).unwrap_or_default() {
            let src = f(&r, "source");
            match by_source.iter_mut().find(|(k, _)| *k == src) {
                Some((_, v)) => v.push(r),
                None => by_source.push((src, vec![r])),
            }
        }
        let mut new: Vec<Value> = Vec::new();
        for (src, rows) in &by_source {
            let scope = format!("filings:{}:{}", sym, src);
            let events: Vec<String> = rows.iter().map(|r| filing_mark(r).join("|")).collect();
            let met = sf::events_told(&c, &scope, &events).unwrap_or_default();
            new.extend(notify::fresh_since(&c, &scope, rows, |r| f(r, "date"), |r| filing_mark(r).join("|"), |r| {
                before.contains(&filing_mark(r)) || met.contains(&filing_mark(r).join("|"))
            }));
            let _ = sf::mark_told(&c, &scope, &events, &now_iso());
        }
        if new.is_empty() {
            continue;
        }
        let rel: Vec<Value> = new.iter().filter(|r| is_news_release(r)).cloned().collect();
        let rest: Vec<Value> = new.iter().filter(|r| !is_news_release(r)).cloned().collect();
        let mut said = false;
        if !rel.is_empty() && in_release_scope(app, &c, &sym, Some(&rel_scopes)) && !sf::has_wire_release(&c, &sym).unwrap_or(false) {
            let (t, bd) = release_notice(app, &sym, &rel);
            said = notify::emit(app, &c, "releases", &release_key(&sym, &rel), &t, &bd, Some(notice_extra(&sym, None, &rel))).is_some() || said;
        }
        if !rest.is_empty() && disc_syms.contains(&sym) {
            let (t, bd) = filings_notice(app, &sym, &rest);
            let mut marks: Vec<String> = rest.iter().map(|r| filing_mark(r).join("/")).collect();
            marks.sort();
            let digest = sha1_hex12(&marks.join("|"));
            said = notify::emit(app, &c, "disclosures", &format!("filings:{}:{}", sym, digest), &t, &bd, Some(notice_extra(&sym, None, &rest))).is_some() || said;
        }
        if said {
            told += 1;
        }
    }
    told
}

fn feed_scope(key: &str) -> Vec<String> {
    vec![match key {
        "holdings" => "held",
        "watchlist" => "watched",
        _ => "all",
    }
    .to_string()]
}

/// The stored disclosures of every ticker in a set,
/// newest first.
pub fn filings_feed(app: &Arc<App>, scope: &str, limit: i64) -> Value {
    let key = { let k = scope.trim().to_lowercase(); if k.is_empty() { "all".to_string() } else { k } };
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": true, "scope": key, "filings": []}) };
    let mut rows: Vec<Value> = Vec::new();
    for inst in known_filing_symbols(app, &feed_scope(&key)) {
        let sym = f(&inst, "symbol");
        for mut r in fresh_filings(&c, &sym) {
            r["symbol"] = json!(sym);
            r["exchange"] = json!(f(&inst, "exchange"));
            rows.push(r);
        }
    }
    rows.sort_by(|a, b| f(b, "date").cmp(&f(a, "date")));
    rows.truncate(limit.max(1) as usize);
    json!({"ok": true, "scope": key, "filings": rows})
}

/// A filed document that is the company's own
/// press release.
pub fn is_news_release(filing: &Value) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)news release|press release").unwrap()).is_match(&f(filing, "type"))
}

pub fn in_release_scope(app: &Arc<App>, c: &Connection, sym: &str, scopes: Option<&[String]>) -> bool {
    let owned;
    let scopes = match scopes {
        Some(x) => x,
        None => {
            owned = notify::release_scopes(c);
            &owned
        }
    };
    if scopes.is_empty() {
        return false;
    }
    if scopes.iter().any(|x| x == "all") {
        return true;
    }
    let sym = sym.trim().to_uppercase();
    let same = |x: &str| {
        let t = tmx_symbol(x);
        (if t.is_empty() { x.to_string() } else { t }).trim().to_uppercase() == sym
    };
    if scopes.iter().any(|x| x == "held") {
        if let Some(b) = base(app) {
            if b.positions.iter().any(|p| same(&p.symbol)) {
                return true;
            }
        }
    }
    if scopes.iter().any(|x| x == "watched") {
        return sf::list_watchlist(c).map(|w| w.iter().any(|w| same(&f(w, "symbol")))).unwrap_or(false);
    }
    false
}

pub fn release_notice(app: &Arc<App>, sym: &str, rows: &[Value]) -> (String, String) {
    let when = |r: &Value| { let p = f(r, "publishedAt"); if p.is_empty() { f(r, "date") } else { p } };
    let mut newest: Vec<&Value> = rows.iter().collect();
    newest.sort_by(|a, b| when(b).cmp(&when(a)));
    let first = newest[0];
    let mut head = f(first, "headline");
    if head.is_empty() {
        head = f(first, "subject");
    }
    if head.is_empty() {
        head = f(first, "type");
    }
    if head.is_empty() {
        head = "A new release.".into();
    }
    let title = if rows.len() == 1 { "Press release · ".to_string() } else { format!("{} press releases · ", rows.len()) } + sym;
    // A release announcing distributions carries the figures beneath the headline, since the
    // headline alone ("Announces August 2026 Distributions") says nothing a holder can act on.
    let mut detail = if is_distribution_release(&head) { distribution_detail(app, sym) } else { String::new() };
    if detail.is_empty() {
        // what the source said beneath its own headline, where it said anything
        detail = f(first, "summary").trim().to_string();
    }
    if !detail.is_empty() {
        head = format!("{}\n{}", head, detail);
    }
    (title, head)
}

/// The issuer's declared record, read again because a release just announced a
/// distribution.
fn read_record_for_notice(_app: &Arc<App>, _c: &Connection, _sym: &str, _exchange: &str) {
    #[cfg(test)]
    {
        // a test counts the reads rather than reaching the source
        _app.feeds.record_reads.fetch_add(1, Ordering::SeqCst);
    }
    #[cfg(not(test))]
    {
        bagholder_market::refresh::refresh_distributions(_c, &[bagholder_model::input::Listing::new(_sym, _exchange, "", "")], true);
    }
}

/// A release whose subject is a distribution or a dividend.
pub fn is_distribution_release(headline: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)\b(distribution|distributions|dividend|dividends)\b").unwrap()).is_match(headline)
}

/// A per-share amount as a release states it: `$0.1489`, trailing zeros gone
/// below four places.
pub fn money_per_share(amount: Option<f64>, currency: &str) -> String {
    let amount = match amount { Some(a) => a, None => return String::new() };
    let text = format!("{:.4}", amount);
    let text = text.trim_end_matches('0');
    let (whole, cents) = match text.split_once('.') { Some((a, b)) => (a, b), None => (text, "") };
    // never fewer than cents, never more than the record states
    let keep = cents.len().max(2);
    let padded = format!("{}00", cents);
    let sign = if currency.to_uppercase() == "USD" { "US$" } else { "$" };
    format!("{}{}.{}", sign, whole, &padded[..keep])
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
    let c = match conn(app) { Some(c) => c, None => return String::new() };
    distribution_detail_in(&c, sym)
}

pub fn distribution_detail_in(c: &Connection, sym: &str) -> String {
    let key = sym.trim().to_uppercase();
    let mut rows = sf_market::distributions(c).unwrap_or_default().remove(&key).unwrap_or_default();
    if rows.is_empty() {
        return String::new();
    }
    rows.sort_by(|a, b| b.ex_date.cmp(&a.ex_date));
    let latest = &rows[0];
    let amount = money_per_share(latest.amount, &latest.currency);
    if amount.is_empty() {
        return String::new();
    }
    let when = stamp_day(&latest.ex_date);
    let paid = stamp_day(&latest.pay_date);
    let freq = sf_market::quotes(c)
        .unwrap_or_default()
        .remove(&key)
        .map(|q| q.quote.dividend_frequency)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let mut out = format!("{} a share", amount);
    if !freq.is_empty() {
        out += &format!(", {}", freq);
    }
    if !when.is_empty() {
        out += &format!(" · ex {}", when);
    }
    if !paid.is_empty() {
        out += &format!(", paid {}", paid);
    }
    if let Some(was) = rows[1..].iter().find(|r| r.amount.is_some()) {
        if was.amount != latest.amount {
            out += &format!(" · was {}", money_per_share(was.amount, &was.currency));
        }
    }
    out
}

/// When the newest of these happened, as its source dates it.
///
/// A release found today can have been published weeks ago -- the app reads a
/// listing's back catalogue the first time it sees it -- and a notice that
/// shows only when it was told reads as news that is not new.
pub fn notice_moment(rows: &[Value]) -> Value {
    json!({"at": f(&newest_of(rows), "publishedAt_or_date")})
}

/// Where a notification's rows can be read: the newest one's own page.
///
/// A filed document is opened through the app, which is what the Disclosures
/// table does, so it opens the same way from here; anything else carries the
/// source's own link.
pub fn notice_link(rows: &[Value]) -> Value {
    let newest = newest_of(rows);
    let url = f(&newest, "url");
    let (id, source) = (f(&newest, "id"), f(&newest, "source"));
    if !id.is_empty() && matches!(source.as_str(), "SEDAR+" | "SEC" | "SEC EDGAR") {
        return json!({"url": url, "doc": id, "source": source});
    }
    if url.is_empty() { json!({}) } else { json!({"url": url}) }
}

/// The newest row by its own moment, with `publishedAt_or_date` set to it.
fn newest_of(rows: &[Value]) -> Value {
    let when = |r: &Value| { let p = f(r, "publishedAt"); if p.is_empty() { f(r, "date") } else { p } };
    let mut sorted: Vec<&Value> = rows.iter().collect();
    sorted.sort_by(|a, b| when(b).cmp(&when(a)));
    let mut out = sorted[0].clone();
    out["publishedAt_or_date"] = json!(when(sorted[0]));
    out
}

/// A notification's extra: the symbol, the moment and the link, in one map.
fn notice_extra(sym: &str, exchange: Option<&str>, rows: &[Value]) -> Value {
    let mut out = Map::new();
    out.insert("symbol".into(), json!(sym));
    if let Some(ex) = exchange {
        out.insert("exchange".into(), json!(ex));
    }
    for part in [notice_moment(rows), notice_link(rows)] {
        if let Some(m) = part.as_object() {
            for (k, v) in m {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// What a release *is*, independent of the id, the source and the date each
/// carries: the story its headline tells.
///
/// TMX, Yahoo, Seeking Alpha and Google all carry the same release under their
/// own ids, a week apart in their own timestamps; keyed by id, one release is
/// four events.
pub fn release_event(row: &Value) -> String {
    let mut head = f(row, "headline");
    if head.is_empty() {
        head = f(row, "subject");
    }
    if head.is_empty() {
        head = f(row, "type");
    }
    news::news_text(&head)
}

fn release_key(sym: &str, rows: &[Value]) -> String {
    let mut ids: Vec<String> = rows.iter().map(release_event).collect();
    ids.sort();
    format!("release:{}:{}", sym, sha1_hex12(&ids.join("|")))
}

/// The press releases a wire answered with that
/// are newer than any it showed for the listing.
pub fn note_wire_releases(app: &Arc<App>, c: &Connection, symbol: &str, exchange: &str, rows: &[Value], new_ids: &[String]) {
    let scopes = notify::release_scopes(c);
    let t = tmx_symbol(symbol);
    let sym = (if t.is_empty() { symbol.to_string() } else { t }).trim().to_uppercase();
    if scopes.is_empty() || !in_release_scope(app, c, &sym, Some(&scopes)) {
        return;
    }
    let rel: Vec<Value> = rows.iter().filter(|r| f(r, "kind") == "release").cloned().collect();
    // An event is told once. The stream keeps what it has met, by what the thing is rather than by
    // the id a source gave it, so the same release reaching the app again -- from another source,
    // under another id, dated a week apart, or simply returning to a search's results after
    // dropping out of them -- is recognised and passed over. Everything met is recorded, told or
    // not, so the back catalogue a first read brings can never ring later.
    let scope = format!("news:{}", sf::news_key(symbol, exchange));
    let events: Vec<String> = rel.iter().map(release_event).collect();
    let met = sf::events_told(c, &scope, &events).unwrap_or_default();
    let fresh = notify::fresh_since(c, &scope, &rel, |r| f(r, "publishedAt"), notify::default_ident, |r| {
        !new_ids.contains(&f(r, "id")) || met.contains(&release_event(r))
    });
    let _ = sf::mark_told(c, &scope, &events, &now_iso());
    if fresh.is_empty() {
        return;
    }
    if fresh.iter().any(|r| is_distribution_release(&f(r, "headline"))) {
        // the release is the announcement; the record it comes from is what carries the figures,
        // and it is read now rather than at its own twenty-hour clock so the notice is not a day
        // behind it
        read_record_for_notice(app, c, &sym, exchange);
    }
    let (title, body) = release_notice(app, &sym, &fresh);
    notify::emit(app, c, "releases", &release_key(&sym, &fresh), &title, &body, Some(notice_extra(&sym, Some(exchange), &fresh)));
}

/// `New disclosure · QNC` and what was filed.
pub fn filings_notice(app: &Arc<App>, sym: &str, new: &[Value]) -> (String, String) {
    let (mut named, mut said): (Vec<String>, String) = (Vec::new(), String::new());
    for r in new.iter().take(3) {
        let mut title = f(r, "subject").trim().to_string();
        let mut summary = f(r, "summary").trim().to_string();
        if (title.is_empty() || summary.is_empty()) && !f(r, "id").is_empty() {
            let read = filings_enrich(app, sym, &f(r, "id"));
            if title.is_empty() {
                title = f(&read, "subject").trim().to_string();
            }
            if summary.is_empty() {
                summary = f(&read, "summary").trim().to_string();
            }
        }
        if title.is_empty() {
            title = form_name(&f(r, "type"));
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
        let src = f(r, "source").trim().to_string();
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
    let row = conn(app).and_then(|c| sf::filing(&c, &symbol.trim().to_uppercase(), doc_id).ok().flatten()).unwrap_or_else(|| json!({}));
    let pick = |ks: &[&str], fallback: &str| {
        ks.iter().map(|k| f(&row, k)).find(|v| !v.is_empty()).unwrap_or_else(|| fallback.to_string())
    };
    let name = pick(&["subject", "title", "type"], "This document");
    let source = pick(&["source"], "the regulator");
    let when = { let t = f(&row, "dateText"); if t.is_empty() { f(&row, "date").chars().take(10).collect() } else { t } };
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
    let wanted = || conn(&app).map_or(false, |c| !notify::disclosure_scopes(&c).is_empty() || !notify::release_scopes(&c).is_empty());
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
        while read_one_unread(&app) {
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

/// The newest stored filing that has never been read, read: the listing on
/// screen first, then everything else. False when there is none, or nothing
/// could be read.
fn read_one_unread(app: &Arc<App>) -> bool {
    let open = app.feeds.looking_at.lock().unwrap().clone();
    if !open.is_empty() && read_one_of(app, &open) {
        return true;
    }
    read_one_of(app, "")
}

/// One unread document of `only`, or of every followed listing when it is empty.
fn read_one_of(app: &Arc<App>, only: &str) -> bool {
    let c = match conn(app) { Some(c) => c, None => return false };
    let mut best: Option<(String, String, String)> = None;   // date, symbol, id
    let every = known_filing_symbols(app, &["held".to_string(), "watched".to_string()]);
    let list: Vec<Value> = if only.is_empty() { every } else { vec![json!({"symbol": only})] };
    for inst in list {
        let sym = f(&inst, "symbol");
        for r in sf::filings_for(&c, &sym).unwrap_or_default() {
            if !f(&r, "subject").is_empty() || is_true(&r, "enrichFinal") {
                continue;
            }
            let date = f(&r, "date");
            if best.as_ref().map(|(d, _, _)| date > *d).unwrap_or(true) {
                best = Some((date, sym.clone(), f(&r, "id")));
            }
        }
    }
    let (_, sym, id) = match best { Some(b) => b, None => return false };
    let out = filings_enrich(app, &sym, &id);
    truthy(out.get("ok")) && !f(&out, "subject").is_empty()
}

/// One symbol's disclosures from every covering
/// source, stored per source. The total written, or -1 when no source could be
/// reached.
pub fn refresh_filings(app: &Arc<App>, symbol: &str, name: Option<&str>, exchange: Option<&str>, currency: Option<&str>) -> i64 {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return 0;
    }
    app.single_flight(&format!("filings:{}", sym), 0, || {
        let c = match conn(app) { Some(c) => c, None => return -1 };
        refresh_filings_in(app, &c, &sym, name, exchange, currency, &|s, n, e, cy, p| disclosures::fetch(s, n, e, cy, 200, p))
    })
}

/// The disclosures gathering `fetch_filings` gives: (symbol, name, exchange,
/// currency, known SEDAR+ profile) to `{items, sources}`.
pub type FetchFilings<'a> = &'a dyn Fn(&str, &str, &str, &str, &str) -> Value;

/// `refresh_filings` on one connection with the gathering given.
pub fn refresh_filings_in(app: &Arc<App>, c: &Connection, sym: &str, name: Option<&str>, exchange: Option<&str>, currency: Option<&str>, fetch_filings: FetchFilings) -> i64 {
    {
        let c = c;
        let sym = sym.to_string();
        let (mut iname, ex, cur) = instrument_meta(c, &sym);
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            iname = n.to_string();
        }
        let exchange_ = exchange.map(|x| x.to_string()).unwrap_or(ex);
        let currency_ = currency.map(|x| x.to_string()).unwrap_or(cur);
        let known = sf::sedar_profile(c, &sym).unwrap_or_default();
        let result = fetch_filings(&sym, &iname, &exchange_, &currency_, &known);
        let mut total = 0i64;
        let mut any_reached = false;
        let mut profile_no = String::new();
        let mut by_source: HashMap<String, Vec<Value>> = HashMap::new();
        for it in result.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            if f(&it, "source") == sedar::SOURCE && !f(&it, "profileNo").is_empty() {
                profile_no = f(&it, "profileNo");
            }
            by_source.entry(f(&it, "source")).or_default().push(it);
        }
        let held: HashSet<String> = sf::filings_for(c, &sym).unwrap_or_default().iter().map(|r| f(r, "source")).collect();
        let now = now_iso();
        let sources = result.get("sources").and_then(|v| v.as_object()).cloned().unwrap_or_default();
        for (src, status) in &sources {
            if is_true(status, "available") {
                any_reached = true;
            }
            let rows = by_source.get(src).cloned().unwrap_or_default();
            if rows.is_empty() && held.contains(src) {
                log(&format!("bagholder disclosures: {}: {} answered empty; the stored rows stand", sym, src));
                continue;
            }
            if is_true(status, "matched") || is_true(status, "available") {
                total += sf::replace_filings(c, &sym, src, &rows, &now).unwrap_or(0) as i64;
            }
        }
        let _ = sf::mark_filings_fetched(c, &sym, &profile_no, &now);
        FILINGS_STORED.fetch_add(1, Ordering::SeqCst);
        app.events.signal(); // the reading loop has something to look at
        let _ = set_meta(c, &format!("filings_sources:{}", sym), &json_text(&Value::Object(sources)));
        if any_reached { total } else { -1 }
    }
}

fn source_status(c: &Connection, sym: &str) -> Value {
    let stored: Map<String, Value> = serde_json::from_str::<Value>(&get_meta(c, &format!("filings_sources:{}", sym), "").unwrap_or_default())
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let have: HashSet<String> = sf::filings_for(c, sym).unwrap_or_default().iter().map(|r| f(r, "source")).collect();
    let mut out = Map::new();
    for (source, dep) in [(sedar::SOURCE, sedar::available()), (edgar::SOURCE, edgar::available())] {
        let st = stored.get(source).cloned().unwrap_or(json!({}));
        // "available" is whether the last read actually reached the source, not merely
        // whether its dependency is installed: a stored outcome that recorded the source
        // unavailable (an outage, a maintenance page) means it could not be reached even
        // with the dependency present. The error it left travels with it, so the card can
        // say the source is unavailable rather than assert the listing has no filer.
        let reached = match st.get("available") {
            Some(v) => v.as_bool().unwrap_or(false),
            None => dep,
        };
        let error = st.get("error").and_then(|v| v.as_str()).unwrap_or("");
        out.insert(source.into(), json!({
            "available": dep && reached,
            "matched": have.contains(source),
            "filer": is_true(&st, "filer") || have.contains(source),
            "error": error,
        }));
    }
    Value::Object(out)
}

/// The stored disclosures, refreshed first when
/// forced or stale.
pub fn filings_payload(app: &Arc<App>, symbol: &str, refresh: bool, name: Option<&str>, exchange: Option<&str>, currency: Option<&str>) -> Value {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "store unavailable"}) };
    filings_payload_in(app, &c, &sym, refresh, &|| refresh_filings(app, &sym, name, exchange, currency))
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

/// The disclosures as stored, never waiting on a source: what a page showing them
/// is sent. `reading` names the documents the pass has still to read, the first of
/// them the one being read now.
pub fn filings_stored(app: &Arc<App>, symbol: &str) -> Value {
    let sym = symbol.trim().to_uppercase();
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "store unavailable"}) };
    let reading = app.feeds.reading_now.lock().unwrap_or_else(|e| e.into_inner()).get(&sym).cloned().unwrap_or_default();
    json!({
        "ok": true,
        "symbol": sym,
        "available": disclosures::available(),
        "sources": source_status(&c, &sym),
        "categories": disclosures::CATEGORIES,
        "fetchedAt": sf::filings_fetched_for(&c, &sym).unwrap_or_default(),
        "everRead": !sf::filings_fetched_for(&c, &sym).unwrap_or_default().is_empty(),
        "summaryStatus": enrich::summary_status(),
        "reading": reading,
        "filings": fresh_filings(&c, &sym),
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
            if conn(&app).map_or(false, |c| filings_stale(&c, &sym, None)) {
                refresh_filings(&app, &sym, opt(&name), opt(&exchange), opt(&currency));
            }
            let mut tried: HashSet<String> = HashSet::new();
            while app.events.watched(&doc) && !app.stopping() {
                let c = match conn(&app) { Some(c) => c, None => break };
                let mut left: Vec<Value> = sf::filings_for(&c, &sym).unwrap_or_default().into_iter()
                    .filter(|r| (f(r, "subject").is_empty() || f(r, "summary").is_empty()) && !is_true(r, "enrichFinal") && !tried.contains(&f(r, "id")))
                    .collect();
                left.sort_by(|a, b| f(b, "date").cmp(&f(a, "date")));
                let Some(next) = left.first().map(|r| f(r, "id")) else { break };
                set_reading(&app, &sym, left.iter().map(|r| f(r, "id")).collect());
                filings_enrich(&app, &sym, &next); // waits for the local model itself when one is coming up
                tried.insert(next);
                if app.wait(Duration::from_millis(150)) { // a person's pace at the source
                    break;
                }
            }
            set_reading(&app, &sym, vec![]);
        });
    });
}

/// `filings_payload` on one connection with the refresh given.
pub fn filings_payload_in(app: &Arc<App>, c: &Connection, sym: &str, refresh: bool, refresh_filings: &dyn Fn() -> i64) -> Value {
    let sym = sym.trim().to_uppercase();
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    *app.feeds.looking_at.lock().unwrap() = sym.clone();
    let c = c;
    let mut wrote: Option<i64> = None;
    if refresh || filings_stale(c, &sym, None) {
        wrote = Some(refresh_filings());
    }
    json!({
        "ok": true,
        "symbol": sym,
        "available": disclosures::available(),
        "sources": source_status(c, &sym),
        "categories": disclosures::CATEGORIES,
        "profileNo": sf::sedar_profile(c, &sym).unwrap_or_default(),
        "fetchedAt": sf::filings_fetched_for(c, &sym).unwrap_or_default(),
        "refreshed": wrote.map(|w| w > 0).unwrap_or(false),
        "sourceUnavailable": wrote == Some(-1),
        "filings": fresh_filings(c, &sym),
    })
}

/// Rows read by an older logic blanked, categories
/// re-derived.
fn fresh_filings(c: &Connection, sym: &str) -> Vec<Value> {
    let mut rows = sf::filings_for(c, sym).unwrap_or_default();
    for r in rows.iter_mut() {
        if (num(r.get("enrichVersion"), Some(0.0)).unwrap_or(0.0) as i64) < ENRICH_VERSION {
            r["subject"] = json!("");
            r["summary"] = json!("");
        }
        let cat = disclosures::categorize(r);
        r["category"] = cat;
        // the list arrives named: a form and a named document say what they are
        // without being fetched, so only the rest wait on a reading
        if f(r, "subject").is_empty() {
            if let Some(t) = disclosures::quick_title(r) {
                let _ = sf::set_filing_enrichment(c, sym, &f(r, "id"), Some(&t), None, None, None, &now_iso());
                r["subject"] = json!(t);
            }
        }
    }
    rows
}

/// (bytes, content type), or the error.
pub fn filings_document(app: &Arc<App>, symbol: &str, doc_id: &str) -> Result<(Vec<u8>, String), String> {
    let sym = symbol.trim().to_uppercase();
    let c = conn(app).ok_or_else(|| "store unavailable".to_string())?;
    let mut row = sf::filing(&c, &sym, doc_id).ok().flatten();
    if row.is_none() {
        refresh_filings(app, &sym, None, None, None);
        row = sf::filing(&c, &sym, doc_id).ok().flatten();
    }
    let row = row.ok_or_else(|| format!("no such document for {}", sym))?;
    let (data, ct) = disclosures::document(&row).map_err(|e| e.to_string())?;
    if data.is_empty() {
        return Err("empty document".into());
    }
    Ok((data, if ct.is_empty() { "application/octet-stream".into() } else { ct }))
}

/// One document read for its subject and, with a
/// local model, a one-sentence summary; both cached on the row.
pub fn filings_enrich(app: &Arc<App>, symbol: &str, doc_id: &str) -> Value {
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "no such document"}) };
    filings_enrich_in(&c, symbol, doc_id, &LiveReaders)
}

/// What `filings_enrich` reads a document with: the local model and the
/// disclosure sources.
pub trait Readers {
    fn summary_available(&self) -> bool { enrich::summary_available() }
    fn summary_status(&self) -> &'static str { enrich::summary_status() }
    fn wait_for_summary(&self, seconds: f64) -> bool { enrich::wait_for_summary(seconds) }
    fn disclosures_available(&self) -> bool { disclosures::available() }
    fn enrichment(&self, row: &Value) -> Option<Value> { disclosures::enrichment(row) }
    fn content(&self, row: &Value) -> disclosures::Fetched<(Vec<u8>, String)> { disclosures::content(row) }
    fn enrich_document_of(&self, code: &str, source: &str, data: &[u8], ct: &str) -> Value { enrich::enrich_document_of(code, source, data, ct) }
}

struct LiveReaders;
impl Readers for LiveReaders {}

/// `filings_enrich` on one connection with the readers given.
pub fn filings_enrich_in(c: &Connection, symbol: &str, doc_id: &str, r: &dyn Readers) -> Value {
    let sym = symbol.trim().to_uppercase();
    let row = match sf::filing(c, &sym, doc_id).ok().flatten() { Some(r) => r, None => return json!({"ok": false, "error": "no such document"}) };
    let mut subject = f(&row, "subject");
    let mut summary = f(&row, "summary");
    let mut model = r.summary_available();
    let fresh = num(row.get("enrichVersion"), Some(0.0)).unwrap_or(0.0) as i64 >= ENRICH_VERSION;
    let attempted = truthy(row.get("enrichedAt")) && fresh;
    let answer = |subject: &str, summary: &str, avail: bool| {
        json!({"ok": true, "id": doc_id, "subject": subject, "summary": summary, "summaryAvailable": avail, "summaryStatus": r.summary_status()})
    };
    if attempted && (is_true(&row, "enrichFinal") || (!subject.is_empty() && !summary.is_empty()) || !model) {
        return answer(&subject, &summary, model);
    }
    if !r.disclosures_available() {
        return answer(&subject, &summary, model);
    }
    let now = now_iso();
    if let Some(exact) = r.enrichment(&row) {
        // What the source can say exactly: the form's own name, and for the
        // forms it can read in full, the sentence too. A name on its own is
        // kept and the document still read, so the sentence follows it.
        let (sj, sm) = (f(&exact, "subject"), f(&exact, "summary"));
        if !sj.is_empty() && subject.is_empty() {
            subject = sj.clone();
            let _ = sf::set_filing_enrichment(c, &sym, doc_id, Some(&subject), None, None, None, &now);
        }
        if !sm.is_empty() {
            let _ = sf::set_filing_enrichment(c, &sym, doc_id, Some(&sj), Some(&sm), Some(ENRICH_VERSION), None, &now);
            return answer(&sj, &sm, model);
        }
        if is_true(&exact, "final") {
            // a named document: nothing a reading would add
            let _ = sf::set_filing_enrichment(c, &sym, doc_id, Some(&subject), Some(""), Some(ENRICH_VERSION), Some(true), &now);
            return answer(&subject, "", model);
        }
        if !model {
            return answer(&subject, &summary, model);
        }
    }
    let (data, ct) = match r.content(&row) {
        Ok(x) => x,
        Err(e) => return json!({"ok": false, "error": e.to_string()}),
    };
    if data.is_empty() {
        return json!({"ok": false, "error": "the document could not be read"});
    }
    if !model {
        model = r.wait_for_summary(enrich::SUMMARY_WAIT_SEC);
    }
    let info = r.enrich_document_of(&f(&row, "type"), &f(&row, "source"), &data, &ct);
    let new_subject = f(&info, "subject");
    let got_summary = f(&info, "summary");
    if is_true(&info, "final") {
        let _ = sf::set_filing_enrichment(c, &sym, doc_id, Some(&new_subject), Some(&got_summary), Some(ENRICH_VERSION), Some(true), &now);
        return answer(&new_subject, &got_summary, model);
    }
    if model && new_subject.is_empty() && got_summary.is_empty() && subject.is_empty() && summary.is_empty() {
        // a document with no text in it -- a release filed as a picture -- has
        // nothing for a reading to find, now or later: it is named by what it
        // is and never read again
        let named = disclosures::quick_title(&row).unwrap_or_else(|| f(&row, "type"));
        let named: String = named.chars().take(90).collect();
        let _ = sf::set_filing_enrichment(c, &sym, doc_id, Some(&named), Some(""), Some(ENRICH_VERSION), Some(true), &now);
        return answer(&named, "", model);
    }
    if model {
        if fresh {
            if !new_subject.is_empty() {
                subject = new_subject;
            }
            if !got_summary.is_empty() {
                summary = got_summary;
            }
        } else {
            subject = new_subject;
            summary = got_summary;
        }
        let _ = sf::set_filing_enrichment(c, &sym, doc_id, Some(&subject), Some(&summary), Some(ENRICH_VERSION), None, &now);
    } else {
        if !new_subject.is_empty() {
            subject = new_subject.clone();
        }
        let _ = sf::set_filing_enrichment(c, &sym, doc_id, if new_subject.is_empty() { None } else { Some(&new_subject) }, None, None, None, &now);
    }
    answer(&subject, &summary, r.summary_available())
}

// ---------------------------------------------------------------------------
// fear
// ---------------------------------------------------------------------------

pub const FEAR_STALE_MIN: f64 = 15.0;
pub const FEAR_VERSION: i64 = 1;

/// One index read from its publisher and kept.
pub fn read_fear(app: &Arc<App>, index: &str) -> Value {
    let rec = fear::read(index);
    if truthy(Some(&rec)) {
        if let Some(c) = conn(app) {
            let _ = sf::save_gauge(&c, index, &rec, &now_iso(), FEAR_VERSION);
        }
    }
    rec
}

fn fear_stale(rec: &Value) -> bool {
    if (num(rec.get("readVersion"), Some(0.0)).unwrap_or(0.0) as i64) < FEAR_VERSION {
        return true;
    }
    match parse_instant(&f(rec, "fetchedAt")) {
        Some(then) => now_unix() - then > FEAR_STALE_MIN * 60.0,
        None => true,
    }
}

fn has_score(rec: &Option<Value>) -> bool {
    rec.as_ref().map(|r| truthy(Some(r)) && !r.get("score").map(|v| v.is_null()).unwrap_or(true)).unwrap_or(false)
}

/// One index's meter, from the store at once.
pub fn fear_payload(app: &Arc<App>, index: &str) -> Value {
    let which = index.trim().to_lowercase();
    if !fear::INDEXES.contains(&which.as_str()) {
        return json!({"ok": false, "error": "no such index"});
    }
    let held = conn(app).and_then(|c| sf::gauge(&c, &which).ok().flatten());
    if has_score(&held) {
        let held = held.unwrap();
        if fear_stale(&held) {
            let w = which.clone();
            let a = app.clone();
            app.kick(&format!("fear:{}", which), move || {
                read_fear(&a, &w);
            });
        }
        return json!({"ok": true, "gauge": held});
    }
    let rec = read_fear(app, &which);
    if truthy(Some(&rec)) { json!({"ok": true, "gauge": rec}) } else { json!({"ok": false, "error": "the index did not answer"}) }
}

/// The meter as it is held, never waiting on its publisher: what a page showing it is
/// sent (`docs`).
pub fn fear_stored(app: &Arc<App>, index: &str) -> Value {
    let which = index.trim().to_lowercase();
    match conn(app).and_then(|c| sf::gauge(&c, &which).ok().flatten()) {
        Some(g) if has_score(&Some(g.clone())) => json!({"ok": true, "gauge": g}),
        _ => json!({"ok": true, "gauge": Value::Null}),
    }
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
                let held = conn(&app).and_then(|c| sf::gauge(&c, &which).ok().flatten());
                if !(has_score(&held) && !fear_stale(held.as_ref().unwrap())) {
                    read_fear(&app, &which);
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

fn shorts_stale(rec: &Value) -> bool {
    if (num(rec.get("readVersion"), Some(0.0)).unwrap_or(0.0) as i64) < SHORTS_VERSION {
        return true;
    }
    match parse_instant(&f(rec, "fetchedAt")) {
        Some(then) => now_unix() - then > SHORTS_STALE_HOURS * 3600.0,
        None => true,
    }
}

/// One listing's short selling from its regulator,
/// kept. {} for a market where no one publishes it.
pub fn read_shorts(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str, trend: bool, name: &str) -> Value {
    let c = match conn(app) { Some(c) => c, None => return json!({}) };
    let rec = shorts::for_listing(&c, symbol, exchange, currency, &today(), trend, name);
    if truthy(Some(&rec)) {
        let ex = { let e = f(&rec, "exchange"); if e.is_empty() { exchange.to_string() } else { e } };
        let _ = sf::save_shorts(&c, symbol, &ex, &rec, &now_iso(), SHORTS_VERSION);
    }
    rec
}

/// One listing's short selling, from the store at
/// once where it was read before.
pub fn shorts_payload(app: &Arc<App>, symbol: &str, exchange: Option<&str>, currency: Option<&str>, trend: bool) -> Value {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    let c = match conn(app) { Some(c) => c, None => return json!({"ok": false, "error": "store unavailable"}) };
    let meta = instrument_meta(&c, &sym);
    let listed_as = if meta.0 == sym { String::new() } else { meta.0.clone() };
    let mut ex = exchange.unwrap_or("").trim().to_string();
    let mut ccy = currency.unwrap_or("").trim().to_string();
    if ex.is_empty() {
        ex = meta.1.clone();
        if ccy.is_empty() {
            ccy = meta.2.clone();
        }
        if ex.is_empty() {
            let form = bagholder_market::tmx::tmx_resolve(&c, &tmx_symbol(&sym), &today());
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
    if shorts::market_of(&sym, &ex, &ccy).is_empty() {
        return json!({"ok": true, "covered": false});
    }
    if let Some(held) = sf::shorts_for(&c, &sym, &ex).ok().flatten() {
        if !trend || truthy(held.get("series")) {
            if shorts_stale(&held) {
                let (s2, e2, c2, n2) = (sym.clone(), ex.clone(), ccy.clone(), listed_as.clone());
                let a = app.clone();
                app.kick(&format!("shorts:{}|{}", sym, ex), move || {
                    read_shorts(&a, &s2, &e2, &c2, true, &n2);
                });
            }
            return json!({"ok": true, "covered": true, "shorts": held});
        }
    }
    let rec = read_shorts(app, &sym, &ex, &ccy, trend, &listed_as);
    if !truthy(Some(&rec)) {
        return json!({"ok": true, "covered": false});
    }
    json!({"ok": true, "covered": true, "shorts": rec})
}

/// Every held or watched listing's stored short
/// selling, each marked held or watched.
pub fn shorts_feed(app: &Arc<App>) -> Value {
    let reading = app.feeds.shorts_left.load(Ordering::SeqCst) > 0;
    let (c, b) = match (conn(app), base(app)) { (Some(c), Some(b)) => (c, b), _ => return json!({"ok": true, "rows": [], "reading": reading}) };
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
    for mut r in sf::all_shorts(&c).unwrap_or_default() {
        let key = (f(&r, "symbol"), f(&r, "exchange"));
        let source = match held.get(&key).or_else(|| watched.get(&key)) { Some(x) => x, None => continue };
        if r.get("shares").map(|v| v.is_null()).unwrap_or(true) {
            continue;
        }
        r["name"] = json!(if source.name.is_empty() { f(&r, "name") } else { source.name.clone() });
        r["exchange"] = json!(if source.exchange.is_empty() { key.1.clone() } else { source.exchange.clone() });
        r["positionId"] = json!(source.position_id);
        r["held"] = json!(held.contains_key(&key));
        r["watched"] = json!(watched.contains_key(&key));
        rows.push(r);
    }
    json!({"ok": true, "rows": rows, "reading": app.feeds.shorts_left.load(Ordering::SeqCst) > 0})
}

/// What the page for one listing needs, held or
/// not.
pub fn listing_payload(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str, name: &str) -> Value {
    let sym = tmx_symbol(symbol).trim().to_uppercase();
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
    }
    let (c, b) = match (conn(app), base(app)) { (Some(c), Some(b)) => (c, b), _ => return json!({"ok": false, "error": "store unavailable"}) };
    let day = today();
    let named = |symbol: &str| tmx_symbol(symbol).trim().to_uppercase() == sym;
    let positions: Vec<ListedRow> = b.positions.iter().filter(|p| named(&p.symbol)).map(|p| ListedRow { id: p.id.clone(), symbol: p.symbol.clone(), exchange: p.exchange.clone(), currency: p.currency.clone(), kind: p.kind.to_string(), name: p.name.clone(), security_id: p.security_id.clone(), fills: vec![] }).collect();
    let trades: Vec<ListedRow> = b
        .trades
        .iter()
        .filter(|t| named(&t.symbol))
        .map(|t| ListedRow {
            id: t.id.clone(),
            symbol: t.symbol.clone(),
            exchange: t.exchange.clone(),
            currency: t.currency.clone(),
            kind: t.kind.to_string(),
            name: t.name.clone(),
            security_id: t.security_id.clone(),
            fills: t.fills.iter().flat_map(|fills| fills.iter()).map(|x| serde_json::to_value(x).unwrap_or(Value::Null)).collect(),
        })
        .collect();
    let watchlist: Vec<ListedRow> = b.watchlist.iter().filter(|w| named(&w.symbol)).map(|w| ListedRow { symbol: w.symbol.clone(), exchange: w.exchange.clone(), currency: w.currency.clone(), name: w.name.clone(), ..ListedRow::default() }).collect();
    listing_payload_in(&c, &positions, &trades, &watchlist, &sym, exchange, currency, name, &|rec| {
        bagholder_market::quotes::peek_quote(&c, rec, &day)
    })
}

/// A row of the book as the listing page reads it: a holding, a trade or a watched listing.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListedRow {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub kind: String,
    pub name: String,
    pub security_id: String,
    pub fills: Vec<Value>,
}

/// `listing_payload` over the book given, with the quote lookup given.
#[allow(clippy::too_many_arguments)]
pub fn listing_payload_in(
    c: &Connection,
    positions: &[ListedRow],
    trades: &[ListedRow],
    watchlist: &[ListedRow],
    symbol: &str,
    exchange: &str,
    currency: &str,
    name: &str,
    peek_quote: &dyn Fn(&bagholder_model::input::Listing) -> Option<bagholder_market::quotes::Glance>,
) -> Value {
    let sym = tmx_symbol(symbol).trim().to_uppercase();
    if sym.is_empty() {
        return json!({"ok": false, "error": "symbol required"});
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
        return json!({"ok": true, "symbol": sym, "positionId": h.id});
    }
    let trades: Vec<&ListedRow> = trades.iter().filter(|t| same(t)).collect();
    let watched = watchlist.iter().find(|w| same(w));
    let empty = ListedRow::default();
    let known: &ListedRow = trades.first().copied().or(watched).unwrap_or(&empty);
    let meta = instrument_meta(c, &sym);
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
    let mut fills: Vec<Value> = trades.iter().flat_map(|t| t.fills.iter().cloned()).collect();
    fills.sort_by_key(|x| f(x, "when"));
    let nm = {
        let n = name.trim().to_string();
        if !n.is_empty() { n } else {
            if !known.name.is_empty() { known.name.clone() } else if meta.0 != sym { meta.0.clone() } else { String::new() }
        }
    };
    let mut out = json!({"ok": true, "symbol": sym, "exchange": ex, "currency": ccy, "kind": kind, "name": nm,
                         "securityId": known.security_id, "fills": fills, "price": null, "percentChange": null});
    if kind == "Shares" {
        let q = peek_quote(&bagholder_model::input::Listing::new(sym.clone(), ex.clone(), ccy.clone(), kind.clone())).unwrap_or_default();
        out["price"] = json!(q.price);
        out["percentChange"] = json!(q.percent_change);
    }
    out
}

/// The shares held and the listings watched whose
/// short selling is published.
pub fn shorts_listings(app: &Arc<App>, scope: &str) -> Vec<(String, String, String, String)> {
    let b = match base(app) { Some(b) => b, None => return vec![] };
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
        if sym.is_empty() || seen.contains(&key) || shorts::market_of(&sym, ex, ccy).is_empty() {
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
    let c = match conn(app) { Some(c) => c, None => return 0 };
    let mut due = Vec::new();
    for (sym, ex, ccy, name) in shorts_listings(app, "all") {
        let held = sf::shorts_for(&c, &sym, &ex).ok().flatten();
        let fresh = held.as_ref().map(|h| truthy(Some(h)) && truthy(h.get("series")) && !shorts_stale(h)).unwrap_or(false);
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
        if truthy(Some(&read_shorts(app, &sym, &ex, &ccy, true, &name))) {
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

/// The heatmaps' tiles. Never fails.
pub fn refresh_universes(app: &Arc<App>) -> Vec<String> {
    app.single_flight("universes", vec![], || {
        let c = match conn(app) { Some(c) => c, None => return vec![] };
        let done = bagholder_market::universes::refresh(&c, &now_iso());
        if !done.is_empty() {
        }
        done
    })
}

/// At start, then every thirty minutes, or sooner
/// when the page asks.
pub fn universe_loop(app: Arc<App>) {
    // The index constituents feed the heatmap and nothing else: sixty-odd requests a
    // pass. They are read when a page asks for the heatmap (the kick), and again each
    // half hour only while a page is still connected -- not at start, and not through
    // a night with nobody there.
    loop {
        let kicked = || *app.feeds.universe_kick.0.lock().unwrap_or_else(|e| e.into_inner());
        if !app.events.park_until(&app, kicked) {
            return;
        }
        loop {
            *app.feeds.universe_kick.0.lock().unwrap_or_else(|e| e.into_inner()) = false;
            refresh_universes(&app);
            if !app.events.park_until_or(&app, Duration::from_secs(1800), kicked) && (app.stopping() || app.events.watchers() == 0) {
                break;
            }
        }
        if app.stopping() {
            return;
        }
    }
}

pub fn kick_universes(app: &Arc<App>) -> Value {
    let (lock, cv) = &app.feeds.universe_kick;
    *lock.lock().unwrap() = true;
    cv.notify_all();
    json!({"ok": true})
}

// ---------------------------------------------------------------------------
// market data
// ---------------------------------------------------------------------------

/// The page, beside the app.
pub fn ledger_path(app: &Arc<App>) -> std::path::PathBuf {
    app.root.join("ledger.html")
}

fn payer_symbols(app: &Arc<App>) -> Vec<bagholder_model::input::Listing> {
    base(app).map(|b| bagholder_model::symbols_of::payer_symbols(&b)).unwrap_or_default()
}

/// USD/CAD, S&P 500, declared distributions
/// and quotes. Never fails.
pub fn refresh_market_data(app: &Arc<App>) -> Value {
    app.single_flight("market", json!({}), || {
        let skipped = json!({"fx": 0, "benchmark": 0, "distributions": 0, "quotes": 0, "skipped": true});
        let c = match conn(app) { Some(c) => c, None => return skipped };
        let mut out = bagholder_market::refresh::refresh_all(&c, &payer_symbols(app));
        let q = refresh_quotes(app);
        out["quotes"] = json!(q);
        if truthy(out.get("distributions")) || q > 0 {
        }
        out
    })
}

/// Prices for held positions and watched listings.
pub fn refresh_quotes(app: &Arc<App>) -> usize {
    app.single_flight("quotes", 0, || {
        let (c, b) = match (conn(app), base(app)) { (Some(c), Some(b)) => (c, b), _ => return 0 };
        let mut syms = bagholder_model::symbols_of::held_symbols(&b);
        syms.extend(bagholder_model::markets::quote_symbols(&b));
        let (today_s, now, stamp) = bagholder_market::clock_now();
        let n = bagholder_market::quotes::refresh_quotes(&c, &syms, &today_s, now, &stamp).unwrap_or(0);
        if n > 0 {
        }
        n
    })
}

/// The rates, benchmarks and distributions
/// on their own clocks. Never fails; 0 when one is already running.
pub fn refresh_periodic_market(app: &Arc<App>) -> Value {
    app.single_flight("periodic", json!(0), || {
        let c = match conn(app) { Some(c) => c, None => return json!({"fx": 0, "benchmark": 0, "distributions": 0, "skipped": true}) };
        let out = bagholder_market::refresh::refresh_periodic(&c, &payer_symbols(app));
        if truthy(out.get("fx")) || truthy(out.get("benchmark")) || truthy(out.get("distributions")) {
        }
        out
    })
}

/// A few instruments per call.
pub fn archive_intraday_bars(app: &Arc<App>, limit: Option<usize>) -> Vec<String> {
    app.single_flight("archive", vec![], || {
        let (c, b) = match (conn(app), base(app)) { (Some(c), Some(b)) => (c, b), _ => return vec![] };
        let recs = bagholder_model::symbols_of::intraday_archive_symbols(&b);
        let limit = limit.map(|l| l.max(1)).unwrap_or(history::ARCHIVE_BATCH);
        let (today_s, now, stamp) = bagholder_market::clock_now();
        let mut out = history::archive_daily(&c, &recs, &today_s, now, &stamp, limit);
        out.extend(history::archive_intraday(&c, &recs, &today_s, now, &stamp, limit));
        out
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
            let was = base(&app);
            let due = match (conn(&app), was.as_ref()) {
                (Some(c), Some(b)) => {
                    let recs = bagholder_model::symbols_of::intraday_archive_symbols(b);
                    let (today_s, now, _) = bagholder_market::clock_now();
                    history::archive_next_due_secs(&c, &recs, &today_s, now)
                }
                _ => None,
            };
            let moved = || match (base(&app), was.as_ref()) {
                (Some(now), Some(was)) => !Arc::ptr_eq(&now.book, &was.book),
                (now, was) => now.is_some() != was.is_some(),
            };
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

/// Prices, every QUOTE_REFRESH_MINUTES.
pub fn quote_loop(app: Arc<App>) {
    // The exchanges offer no push, so prices are asked for; but only while a page is
    // open to show them. With nobody looking, nothing is fetched; a page that opens
    // asks for fresh quotes itself (`/api/events` -> the model's own kick).
    // Parked, at no cost, until a page connects; then read at once (a page that opens
    // after hours away gets fresh prices) and each minute while one stays. Which
    // listings are asked is narrowed again by whether their market can have moved
    // (`market::quotes::can_have_moved`).
    while app.events.park_until(&app, || app.events.watchers() > 0) {
        refresh_quotes(&app);
        if app.wait(Duration::from_secs_f64(60.0 * bagholder_market::quotes::QUOTE_REFRESH_MINUTES)) {
            return;
        }
    }
}

/// The periodic records and the update check, hourly.
pub fn market_loop(app: Arc<App>) {
    while !app.wait(Duration::from_secs(60 * bagholder_market::refresh::MARKET_CHECK_MINUTES)) {
        refresh_periodic_market(&app);
        crate::update::check_for_update_if_due(&app);
    }
}

pub const WATCH_SCAN_SEC: u64 = 10 * 60;

/// New or changed CSVs imported. Never fails.
pub fn scan_watched_folder(app: &Arc<App>) -> Option<Value> {
    let c = conn(app)?;
    if bagholder_store::csvimport::watch_folder(&c).ok()?.is_empty() {
        return None;
    }
    let result = bagholder_store::csvimport::scan_folder(&c, None, false).ok()?;
    if is_true(&result, "ok") && is_true(&result, "added") {
    }
    Some(result)
}

pub fn watch_loop(app: Arc<App>) {
    // Only while a folder is set to be watched; until one is, this waits for the
    // setting. The folder itself is looked at on a period: the standard library has
    // no file-system notification (docs/architecture.md, "Timers that remain").
    let set = || conn(&app).and_then(|c| bagholder_store::csvimport::watch_folder(&c).ok()).map_or(false, |f| !f.is_empty());
    while app.events.park_until(&app, set) {
        scan_watched_folder(&app);
        if app.wait(Duration::from_secs(WATCH_SCAN_SEC)) {
            return;
        }
    }
}

pub fn sync_then_market(app: &Arc<App>) -> bool {
    let ok = crate::session::run_sync(app, true, true);
    refresh_market_data(app);
    ok
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
pub fn history_pending(app: &Arc<App>, query: &str) -> bool {
    let or = |v: String, d: &str| if v.is_empty() { d.to_string() } else { v };
    let rec = bagholder_model::input::Listing::new(qs_one(query, "symbol"), qs_one(query, "exchange"), or(qs_one(query, "currency"), "CAD"), or(qs_one(query, "kind"), "Shares"));
    let start: String = qs_one(query, "from").chars().take(10).collect();
    let tf = or(qs_one(query, "tf"), "1d");
    if !history::INTRADAY_SECONDS.iter().any(|(k, _)| *k == tf) {
        return false;
    }
    let inst = history::chart_instrument(&rec);
    let (today_s, now, _) = bagholder_market::clock_now();
    conn(app).map_or(false, |c| !history::intraday_ready(&c, &inst, &tf, &start, &today_s, now))
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

pub fn history_payload(app: &Arc<App>, query: &str) -> Value {
    let or = |v: String, d: &str| if v.is_empty() { d.to_string() } else { v };
    let rec = bagholder_model::input::Listing::new(qs_one(query, "symbol"), qs_one(query, "exchange"), or(qs_one(query, "currency"), "CAD"), or(qs_one(query, "kind"), "Shares"));
    let start: String = qs_one(query, "from").chars().take(10).collect();
    let end: String = qs_one(query, "to").chars().take(10).collect();
    let tf = or(qs_one(query, "tf"), "1d");
    if rec.symbol.is_empty() || start.chars().count() != 10 || end.chars().count() != 10 || !history::TIMEFRAMES.contains(&tf.as_str()) {
        return json!({"ok": false, "error": "symbol, from, to and a known tf are required"});
    }
    let inst = history::chart_instrument(&rec);
    let src = history::history_source(&inst);
    let (today_s, now, stamp) = bagholder_market::clock_now();
    let c = conn(app);
    let available: Vec<&'static str> = c.as_ref().map(|c| history::offered_timeframes(c, &inst, &start, &today_s, now)).unwrap_or_default();
    let mut pending = false;
    let bars: ChartBars = match &c {
        None => ChartBars::default(),
        Some(c) => {
            if !(src.is_some() && available.contains(&tf.as_str())) {
                ChartBars::default()
            } else if history::INTRADAY_SECONDS.iter().any(|(k, _)| *k == tf) && !history::intraday_ready(c, &inst, &tf, &start, &today_s, now) {
                history::ensure_intraday_in_background(app.store(), inst.clone(), tf.clone(), start.clone(), end.clone());
                pending = true;
                ChartBars::default()
            } else {
                history::ensure_bars(c, &inst, &tf, &start, &end, &today_s, now, &stamp).unwrap_or_default()
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
    serde_json::to_value(&payload).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// tests: filings and the listing page
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_common::app;
    use std::cell::Cell;

    fn store() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        bagholder_store::schema::init_schema(&c).unwrap();
        c
    }

    fn item(source: &str, i: i64, profile: &str) -> Value {
        let tag = source.split('+').next().unwrap().to_lowercase().replace(' ', "");
        let sec = source == "SEC";
        json!({
            "id": format!("{}:{}", tag, i),
            "source": source,
            "category": "Financials",
            "date": format!("2026-08-{:02}", 10 + i),
            "dateText": format!("2026-08-{:02}", 10 + i),
            "type": if sec { "10-Q" } else { "Interim MD&A" },
            "title": if sec { "Quarterly report" } else { "" },
            "size": if sec { "" } else { "292 KB" },
            "url": if sec { format!("https://www.sec.gov/x/{}", i) } else { format!("https://www.sedarplus.ca/x?drmKey={}", i) },
            "profileNo": profile,
        })
    }

    fn replace(c: &Connection, sym: &str, src: &str, items: &[Value]) {
        sf::replace_filings(c, sym, src, items, &now_iso()).unwrap();
    }

    fn enrichment(c: &Connection, sym: &str, id: &str, subject: &str, summary: &str, version: i64) {
        sf::set_filing_enrichment(c, sym, id, Some(subject), Some(summary), Some(version), None, &now_iso()).unwrap();
    }

    // --- DisclosuresStoreTest

    #[test]
    fn test_rows_from_two_sources_merge_newest_first() {
        let c = store();
        replace(&c, "SHOP", "SEDAR+", &[item("SEDAR+", 1, ""), item("SEDAR+", 3, "")]);
        replace(&c, "SHOP", "SEC", &[item("SEC", 2, ""), item("SEC", 4, "")]);
        let rows = sf::filings_for(&c, "SHOP").unwrap();
        assert_eq!(rows.len(), 4);
        let dates: Vec<String> = rows.iter().map(|r| f(r, "date")).collect();
        let mut sorted = dates.clone();
        sorted.sort();
        sorted.reverse();
        assert_eq!(dates, sorted);
        let srcs: HashSet<String> = rows.iter().map(|r| f(r, "source")).collect();
        assert_eq!(srcs, HashSet::from(["SEDAR+".to_string(), "SEC".to_string()]));
    }

    #[test]
    fn test_replacing_one_source_leaves_the_other() {
        let c = store();
        replace(&c, "SHOP", "SEDAR+", &[item("SEDAR+", 1, ""), item("SEDAR+", 2, "")]);
        replace(&c, "SHOP", "SEC", &[item("SEC", 1, "")]);
        replace(&c, "SHOP", "SEDAR+", &[item("SEDAR+", 9, "")]);
        let rows = sf::filings_for(&c, "SHOP").unwrap();
        let mut srcs: Vec<String> = rows.iter().map(|r| f(r, "source")).collect();
        srcs.sort();
        assert_eq!(srcs, ["SEC", "SEDAR+"]);
        assert_eq!(rows.iter().filter(|r| f(r, "source") == "SEDAR+").count(), 1, "SEDAR+ replaced, not appended");
        assert_eq!(rows.iter().filter(|r| f(r, "source") == "SEC").count(), 1, "SEC untouched");
    }

    #[test]
    fn test_a_single_row_is_fetchable_by_id_for_download() {
        let c = store();
        replace(&c, "SHOP", "SEC", &[item("SEC", 7, "")]);
        let row = sf::filing(&c, "SHOP", "sec:7").unwrap().unwrap();
        assert_eq!(f(&row, "source"), "SEC");
        assert!(f(&row, "url").starts_with("https://www.sec.gov/"));
        assert!(sf::filing(&c, "SHOP", "sec:999").unwrap().is_none());
    }

    #[test]
    fn test_symbols_do_not_bleed_and_the_profile_is_remembered() {
        let c = store();
        replace(&c, "SHOP", "SEDAR+", &[item("SEDAR+", 1, "")]);
        replace(&c, "ATD", "SEDAR+", &[item("SEDAR+", 1, ""), item("SEDAR+", 2, "")]);
        sf::mark_filings_fetched(&c, "ATD", "000012345", &now_iso()).unwrap();
        assert_eq!(sf::filings_for(&c, "SHOP").unwrap().len(), 1);
        assert_eq!(sf::filings_for(&c, "ATD").unwrap().len(), 2);
        assert_eq!(sf::sedar_profile(&c, "ATD").unwrap(), "000012345");
    }

    #[test]
    fn test_forget_clears_rows_and_stamps() {
        let c = store();
        replace(&c, "SHOP", "SEC", &[item("SEC", 1, "")]);
        sf::mark_filings_fetched(&c, "SHOP", "000037100", &now_iso()).unwrap();
        sf::forget_filings(&c, "SHOP").unwrap();
        assert!(sf::filings_for(&c, "SHOP").unwrap().is_empty());
        assert_eq!(sf::filings_fetched_for(&c, "SHOP").unwrap(), "");
        assert_eq!(sf::sedar_profile(&c, "SHOP").unwrap(), "");
    }

    // --- FilingsPayloadTest

    fn stub(items: Value, sources: Value) -> impl Fn(&str, &str, &str, &str, &str) -> Value {
        move |_, _, _, _, _| json!({"items": items.clone(), "sources": sources.clone()})
    }

    fn payload(c: &Connection, sym: &str, fetch: &dyn Fn(&str, &str, &str, &str, &str) -> Value) -> Value {
        let s = sym.trim().to_uppercase();
        filings_payload_in(&app(), c, sym, true, &|| refresh_filings_in(&app(), c, &s, None, None, None, fetch))
    }

    #[test]
    fn test_stale_until_a_fetch_then_fresh_within_a_day() {
        let c = store();
        assert!(filings_stale(&c, "SHOP", None));
        sf::mark_filings_fetched(&c, "SHOP", "", &now_iso()).unwrap();
        assert!(!filings_stale(&c, "SHOP", None));
    }

    #[test]
    fn test_a_day_old_stamp_is_stale() {
        let c = store();
        let old = crate::app::stamp_of(now_unix() as i64 - 25 * 3600);
        sf::mark_filings_fetched(&c, "SHOP", "", &old).unwrap();
        assert!(filings_stale(&c, "SHOP", None));
    }

    #[test]
    fn test_refresh_merges_sources_and_reports_status() {
        let c = store();
        let fetch = stub(
            json!([item("SEDAR+", 1, "000037100"), item("SEC", 2, "")]),
            json!({"SEDAR+": {"available": true, "matched": true, "count": 1, "error": ""},
                   "SEC": {"available": true, "matched": true, "count": 1, "error": ""}}),
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
            json!([item("SEC", 1, "")]),
            json!({"SEDAR+": {"available": true, "matched": false, "count": 0, "error": ""},
                   "SEC": {"available": true, "matched": true, "count": 1, "error": ""}}),
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
            json!([item("SEC", 1, "")]),
            json!({"SEDAR+": {"available": false, "matched": false, "count": 0, "error": "SEDAR+ did not return the searchReportingIssuers form"},
                   "SEC": {"available": true, "matched": true, "count": 1, "error": ""}}),
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
            json!([]),
            json!({"SEDAR+": {"available": false, "matched": false, "count": 0, "error": "the browser helper is missing"},
                   "SEC": {"available": false, "matched": false, "count": 0, "error": "network"}}),
        );
        let out = payload(&c, "SHOP", &fetch);
        assert_eq!(out["ok"], true, "the endpoint still answers cleanly");
        assert_eq!(out["sourceUnavailable"], true);
        assert_eq!(out["filings"], json!([]));
    }

    #[test]
    fn test_empty_symbol_is_rejected() {
        let _g = crate::tests_common::guard();
        assert_eq!(filings_payload(&app(), "", false, None, None, None)["ok"], false);
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
        fn enrichment(&self, _: &Value) -> Option<Value> { None }
        fn content(&self, _: &Value) -> disclosures::Fetched<(Vec<u8>, String)> {
            self.reads.set(self.reads.get() + 1);
            Ok((b"%PDF-1.4 body".to_vec(), "application/pdf".into()))
        }
        fn enrich_document_of(&self, _: &str, _: &str, _: &[u8], _: &str) -> Value {
            let mut v = json!({"subject": self.read.0, "summary": self.read.1});
            if self.final_ {
                v["final"] = json!(true);
            }
            v
        }
    }

    fn fake(model: bool, read: (&'static str, &'static str), final_: bool) -> Fake {
        Fake { model, status: if model { "ready" } else { "off" }, wait: false, read, final_, reads: Cell::new(0), waits: Cell::new(0) }
    }

    const DOC: &str = "sedar:1";

    fn enrich_store() -> Connection {
        let c = store();
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, "")]);
        c
    }

    fn stored(c: &Connection) -> (String, String, i64) {
        let row = sf::filing(c, "QNC", DOC).unwrap().unwrap();
        (f(&row, "subject"), f(&row, "summary"), num(row.get("enrichVersion"), Some(0.0)).unwrap_or(0.0) as i64)
    }

    fn run(c: &Connection, model: bool, read: (&'static str, &'static str), final_: bool) -> (Value, usize) {
        let r = fake(model, read, final_);
        let out = filings_enrich_in(c, "QNC", DOC, &r);
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
        assert_eq!(stored(&c).2, 0);
    }

    // --- WaitingForTheModelTest

    #[test]
    fn test_a_model_that_is_starting_is_waited_for_rather_than_the_read_wasted() {
        let c = enrich_store();
        let r = Fake { model: false, status: "ready", wait: true, read: ("A title", "A sentence."), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = filings_enrich_in(&c, "QNC", DOC, &r);
        assert_eq!(r.waits.get(), 1);
        assert_eq!(out["summary"], "A sentence.");
        assert_eq!(stored(&c), ("A title".to_string(), "A sentence.".to_string(), ENRICH_VERSION));
    }

    #[test]
    fn test_a_model_that_never_comes_up_leaves_the_row_to_be_read_again() {
        let c = enrich_store();
        let r = Fake { model: false, status: "off", wait: false, read: ("A title", ""), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = filings_enrich_in(&c, "QNC", DOC, &r);
        assert_eq!(out["subject"], "A title");
        assert_eq!(stored(&c).2, 0);
    }

    // --- RefreshKeepsWhatWasReadTest

    #[test]
    fn test_a_row_the_source_still_lists_keeps_its_subject_and_summary() {
        let c = store();
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, ""), item("SEDAR+", 2, "")]);
        enrichment(&c, "QNC", "sedar:1", "A title", "A sentence.", 9);
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, ""), item("SEDAR+", 2, ""), item("SEDAR+", 3, "")]);
        let row = sf::filing(&c, "QNC", "sedar:1").unwrap().unwrap();
        assert_eq!((f(&row, "subject"), f(&row, "summary"), row["enrichVersion"].as_i64()), ("A title".into(), "A sentence.".into(), Some(9)));
    }

    #[test]
    fn test_what_the_source_says_about_a_row_is_still_refreshed() {
        let c = store();
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, "")]);
        enrichment(&c, "QNC", "sedar:1", "A title", "A sentence.", 9);
        let mut moved = item("SEDAR+", 1, "");
        moved["url"] = json!("https://www.sedarplus.ca/x?drmKey=fresh");
        replace(&c, "QNC", "SEDAR+", &[moved]);
        let row = sf::filing(&c, "QNC", "sedar:1").unwrap().unwrap();
        assert_eq!(f(&row, "url"), "https://www.sedarplus.ca/x?drmKey=fresh");
        assert_eq!(f(&row, "summary"), "A sentence.");
    }

    #[test]
    fn test_a_row_the_source_no_longer_lists_goes() {
        let c = store();
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, ""), item("SEDAR+", 2, "")]);
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 2, "")]);
        assert!(sf::filing(&c, "QNC", "sedar:1").unwrap().is_none());
        assert!(sf::filing(&c, "QNC", "sedar:2").unwrap().is_some());
    }

    #[test]
    fn test_another_sources_rows_are_untouched() {
        let c = store();
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, "")]);
        replace(&c, "QNC", "SEC", &[item("SEC", 1, "")]);
        enrichment(&c, "QNC", "sec:1", "From EDGAR", "A sentence.", 9);
        replace(&c, "QNC", "SEDAR+", &[item("SEDAR+", 1, "")]);
        assert_eq!(f(&sf::filing(&c, "QNC", "sec:1").unwrap().unwrap(), "subject"), "From EDGAR");
    }

    // --- the listing page

    fn fill(when: &str, side: &str, qty: f64, price: f64) -> Value {
        json!({"when": when, "side": side, "qty": qty, "price": price})
    }

    const FILL: &str = "2026-03-02T14:31:00Z";
    const LATER: &str = "2026-04-09T15:02:00Z";
    const EARLIER: &str = "2026-01-05T14:40:00Z";

    fn listing(positions: &[Value], trades: &[Value], watchlist: &[Value], q: (f64, f64), args: (&str, &str, &str, &str)) -> Value {
        let rows = |list: &[Value]| -> Vec<ListedRow> { list.iter().map(|r| serde_json::from_value(r.clone()).unwrap()).collect() };
        let (positions, trades, watchlist) = (&rows(positions), &rows(trades), &rows(watchlist));
        let c = store();
        let quote = move |_: &bagholder_model::input::Listing| {
            Some(bagholder_market::quotes::Glance { price: Some(q.0), percent_change: Some(q.1), ..Default::default() })
        };
        listing_payload_in(&c, positions, trades, watchlist, args.0, args.1, args.2, args.3, &quote)
    }

    #[test]
    fn test_a_listing_the_book_holds_answers_with_the_holding_whose_page_it_is() {
        let held = [json!({"id": "rt:1", "symbol": "QNC", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Quantum eMotion Corp"})];
        let out = listing(&held, &[], &[], (1.25, -2.0), ("QNC", "TSX-V", "", ""));
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
        let out = listing(&[], &trades, &[], (1.8, 1.5), ("QNC", "TSX-V", "", ""));
        assert!(out.get("positionId").map(|v| v.is_null()).unwrap_or(true));
        let whens: Vec<String> = out["fills"].as_array().unwrap().iter().map(|x| f(x, "when")).collect();
        assert_eq!(whens, [EARLIER, FILL, LATER], "the listing's own trades, oldest first; an option is not the share, and another venue is another listing");
        assert_eq!((f(&out, "name"), f(&out, "exchange"), f(&out, "currency"), f(&out, "kind")),
                   ("Quantum eMotion Corp".into(), "TSX-V".into(), "CAD".into(), "Shares".into()));
        assert_eq!((out["price"].as_f64(), out["percentChange"].as_f64()), (Some(1.8), Some(1.5)));
    }

    #[test]
    fn test_a_listing_never_traded_is_named_by_the_watchlist_and_has_no_executions() {
        let watch = [json!({"symbol": "YES", "exchange": "TSX-V", "currency": "CAD", "kind": "Shares", "name": "Char Technologies Ltd."})];
        let out = listing(&[], &[], &watch, (0.265, 0.0), ("YES", "TSX-V", "", ""));
        assert_eq!(out["fills"], json!([]));
        assert_eq!((f(&out, "name"), f(&out, "currency")), ("Char Technologies Ltd.".into(), "CAD".into()));
    }

    #[test]
    fn test_a_listing_the_book_has_never_seen_answers_with_what_was_asked_for() {
        let out = listing(&[], &[], &[], (284.21, -0.34), ("RY", "TSX", "CAD", "Royal Bank of Canada"));
        assert_eq!((&out["ok"], f(&out, "symbol"), f(&out, "exchange"), f(&out, "name"), &out["fills"]),
                   (&json!(true), "RY".into(), "TSX".into(), "Royal Bank of Canada".into(), &json!([])));
        assert_eq!(out["price"].as_f64(), Some(284.21));
    }

    #[test]
    fn test_a_ticker_with_no_venue_matches_the_book_whatever_venue_it_holds_it_on() {
        let trades = [json!({"id": "t1", "symbol": "SHOP.TO", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "name": "Shopify Inc.",
                             "fills": [fill(FILL, "BUY", 10.0, 5.0)]})];
        let out = listing(&[], &trades, &[], (1.25, -2.0), ("SHOP", "", "", ""));
        assert_eq!((f(&out, "symbol"), f(&out, "exchange"), f(&out, "name")), ("SHOP".into(), "TSX".into(), "Shopify Inc.".into()));
        assert_eq!(out["fills"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_a_ticker_that_is_not_one_is_refused() {
        assert_eq!(listing(&[], &[], &[], (1.25, -2.0), ("  ", "", "", ""))["ok"], false);
    }
}
