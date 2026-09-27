//! Status, protocol, port, update
//! check, history endpoint, versions, tiles, watchlist and in-app update
//! (the parts reachable without a network or a child process).
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::app;
use crate::tests_common::{app, app_ref};
use crate::update;

/// The shared app.
fn guard() -> std::sync::MutexGuard<'static, ()> {
    crate::tests_common::guard()
}

/// A fresh market cache of its own, in a temporary home.
struct Db {
    dir: PathBuf,
    conn: Connection,
}

impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn db() -> Db {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("bh-misc-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(crate::figures::CACHE_FILE);
    bagholder_sources::cache::MarketCache::open(&path, app::APP_VERSION, bagholder_core::jiff::Timestamp::now()).unwrap();
    let conn = Connection::open(&path).unwrap();
    Db { dir, conn }
}

/// A listing's news, as a read stores it.
fn news(conn: &Connection, symbol: &str, headline: &str) {
    let item = NewsItem { id: format!("tmx:{headline}"), headline: headline.into(), source: "Business Wire".into(), url: String::new(), published_at: "2026-09-15T13:00:00Z".into(), summary: String::new(), kind: bagholder_store::feeds::NewsKind::Release, via: Feed::Tmx };
    bagholder_store::feeds::replace_news(conn, symbol, "TSX", &[item], &app::now_iso()).unwrap();
}

// ---------------------------------------------------------------------------
// StoreTest
// ---------------------------------------------------------------------------

#[test]
fn test_status_carries_the_data_version_so_the_page_can_reload() {
    let _g = guard();
    let conn = app_ref().cache().unwrap();
    let v0 = crate::status::answer(&app()).unwrap().data_version;
    assert!(!v0.is_empty());
    news(&conn, "VERSQ", &format!("A release at {}", app::now_unix()));
    let v1 = crate::status::answer(&app()).unwrap().data_version;
    assert_ne!(v0, v1, "a news row stored moves the version");
    conn.execute("DELETE FROM news WHERE symbol = 'VERSQ'", []).unwrap();
    assert_ne!(v1, crate::status::answer(&app()).unwrap().data_version, "and one removed");
}

/// The clock cannot be stood in for here: the version is checked to carry
/// today's local date, which is what makes it change at midnight.
#[test]
fn test_status_version_changes_with_the_date_so_the_page_refetches_at_midnight() {
    let _g = guard();
    let v = crate::status::answer(&app()).unwrap().data_version;
    assert!(v.ends_with(&format!("|{}", bagholder_model::clock::today_local())), "{}", v);
}

#[test]
fn test_page_and_server_agree_on_the_protocol_stamp() {
    let _g = guard();
    let page = std::fs::read_to_string(app().root.join("web/src/lib/protocol.ts")).unwrap();
    let m = regex::Regex::new(r"export const PROTOCOL = '([^']+)'").unwrap().captures(&page).expect("PROTOCOL on the page");
    assert_eq!(&m[1], app::PROTOCOL);
    assert_eq!(crate::status::status(&app()).protocol, app::PROTOCOL);
}

#[test]
fn test_port_can_be_chosen_for_a_second_instance() {
    let _g = guard();
    let saved = std::env::var("BAGHOLDER_PORT").ok();
    std::env::set_var("BAGHOLDER_PORT", "8799");
    assert_eq!(crate::port_choices().unwrap(), vec![8799]);
    std::env::set_var("BAGHOLDER_PORT", "80");
    assert_eq!(crate::port_choices().unwrap(), crate::PORTS.to_vec(), "a privileged or nonsense port is ignored");
    std::env::remove_var("BAGHOLDER_PORT");
    assert_eq!(crate::port_choices().unwrap(), crate::PORTS.to_vec());
    if let Some(p) = saved {
        std::env::set_var("BAGHOLDER_PORT", p);
    }
}

/// GitHub answers through `update::FAKE_RELEASE`; notifications are recorded,
/// never shown. Put back what it touched.
struct UpdateFakes;

impl UpdateFakes {
    fn new(answer: Option<update::GithubRelease>) -> Self {
        *update::FAKE_RELEASE.lock().unwrap() = Some((0, answer));
        let mut d = crate::notify::test_hooks::DELIVERED.lock().unwrap();
        if d.is_none() {
            *d = Some(Vec::new());
        }
        UpdateFakes
    }
    fn answer(&self, answer: Option<update::GithubRelease>) {
        *update::FAKE_RELEASE.lock().unwrap() = Some((0, answer));
    }
    fn calls(&self) -> usize {
        update::FAKE_RELEASE.lock().unwrap().as_ref().map(|x| x.0).unwrap_or(0)
    }
}

impl Drop for UpdateFakes {
    fn drop(&mut self) {
        *update::FAKE_RELEASE.lock().unwrap() = None;
        if let Ok(c) = app_ref().cache() {
            let _ = c.execute("DELETE FROM meta WHERE key = 'update_check'", []);
        }
        if let Ok(b) = crate::notify::book(app_ref()) {
            let _ = b.notices().execute("DELETE FROM notifications WHERE key LIKE 'update:%'", []);
        }
        *crate::notify::test_hooks::DELIVERED.lock().unwrap() = None;
    }
}

fn set_checked_at(secs_ago: f64) {
    let c = app_ref().cache().unwrap();
    let mut rec = update::update_status(&app()).unwrap();
    rec.checked_at = app::stamp_of((app::now_unix() - secs_ago) as i64);
    bagholder_store::tables::set_meta(&c, "update_check", &serde_json::to_string(&rec).unwrap()).unwrap();
}

#[test]
fn test_update_check_flags_only_a_newer_release() {
    let _g = guard();
    assert!(update::parse_version(app::APP_VERSION).is_some(), "APP_VERSION must be MAJOR.MINOR.PATCH");
    assert_eq!(update::parse_version("v1.2.3"), Some((1, 2, 3)));
    assert_eq!(update::parse_version("1.10.0"), Some((1, 10, 0)));
    assert!(update::parse_version("v1.10.0") > update::parse_version("v1.9.9"));
    assert_eq!(update::parse_version("latest"), None);
    let mine = update::parse_version(app::APP_VERSION).unwrap();
    let newer = format!("v{}.{}.{}", mine.0, mine.1, mine.2 + 1);
    let older = "v0.9.0";
    let fakes = UpdateFakes::new(Some(update::GithubRelease { tag_name: format!("v{}", app::APP_VERSION), html_url: "https://github.com/x/y/releases/tag/v1".into(), ..Default::default() }));
    let rec = update::check_for_update(&app());
    assert_eq!((rec.ok, rec.update_available), (true, false), "same release: no flag");
    fakes.answer(Some(update::GithubRelease { tag_name: older.into(), html_url: "u".into(), ..Default::default() }));
    assert_eq!(update::check_for_update(&app()).update_available, false, "an older release never flags");
    fakes.answer(Some(update::GithubRelease {
        tag_name: newer.clone(),
        html_url: format!("https://github.com/ProfessorBagholder/Bagholder/releases/tag/{}", newer),
        ..Default::default()
    }));
    set_checked_at(30.0 * 60.0);
    update::check_for_update_if_due(&app());
    assert_eq!(fakes.calls(), 0, "checked half an hour ago: GitHub is not asked again");
    set_checked_at(2.0 * 3600.0);
    let rec = update::check_for_update_if_due(&app());
    assert_eq!((rec.update_available, rec.latest.as_str()), (true, newer.as_str()));
    let st = crate::status::status(&app());
    assert_eq!(
        (st.version, st.latest_version, st.update_available, st.update_url.clone()),
        (app::APP_VERSION.to_string(), newer.clone(), true, rec.url.clone())
    );
    fakes.answer(None);
    let rec = update::check_for_update(&app());
    assert_eq!((rec.ok, rec.update_available), (false, false), "offline: silent, no flag");
    fakes.answer(Some(update::GithubRelease { ..Default::default() }));
    assert_eq!(update::check_for_update(&app()).update_available, false, "no release published yet: nothing to flag");
}

#[test]
fn test_history_endpoint_validates_and_serves_bars() {
    let _g = guard();
    assert!(matches!(crate::feeds::history_payload(&app(), &crate::feeds::HistoryQuery::parse("symbol=RDDY")), crate::feeds::HistoryAnswer::Refused(_)));
    assert!(matches!(
        crate::feeds::history_payload(&app(), &crate::feeds::HistoryQuery::parse("symbol=RDDY&exchange=TSX&currency=CAD&kind=Shares&from=2026-08-25&to=2026-09-05&tf=2h")),
        crate::feeds::HistoryAnswer::Refused(_)
    ));
}

#[test]
fn test_a_daily_chart_is_answered_as_stored_while_a_due_read_runs_in_the_background() {
    let _g = guard();
    let app = app();
    let conn = app.cache().unwrap();
    let (today, now, _) = bagholder_market::clock_now();
    let from = bagholder_model::dates::shift_date(&today, -40);
    let day = bagholder_model::dates::shift_date(&today, -30);
    let px = bagholder_store::bars::Ohlcv { open: Some(10.0), high: Some(10.5), low: Some(9.5), close: 10.2, volume: Some(1000.0) };
    bagholder_store::market::upsert_price_history(&conn, "DLYQ", &[bagholder_store::bars::DayBar { date: day.clone(), px }], "test").unwrap();
    // read from the span's start, but long ago: the copy is stale and the span reaches today
    bagholder_store::market::mark_history_fetched(&conn, "DLYQ", &from, "2020-01-02T00:00:00Z").unwrap();
    let q = crate::feeds::HistoryQuery::parse(&format!("symbol=DLYQ&exchange=TSX&currency=CAD&kind=Shares&from={from}&to={today}&tf=1d"));
    let inst = bagholder_market::history::chart_instrument(&q.read().0);
    assert!(bagholder_market::history::daily_due(&conn, &inst, &from, &today, &today, now).unwrap(), "the stored copy is due a read");

    let crate::feeds::HistoryAnswer::Ok(h) = crate::feeds::history_payload(&app, &q) else { panic!("a known timeframe is answered") };
    let days: Vec<String> = match &h.bars {
        bagholder_store::bars::ChartBars::Days(b) => b.iter().map(|b| b.date.clone()).collect(),
        _ => panic!("a daily chart answers days"),
    };
    assert_eq!(days, vec![day], "the stored bars are answered, not held back for the read");
    assert!(h.pending, "answered while the read is still under way, and the page is told to wait for it");

    // the read ends, whatever it found, and the page's document says so
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while crate::feeds::history_pending(&app, &q) {
        assert!(std::time::Instant::now() < deadline, "the background read ends");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // a read that found nothing (offline here) is not asked again at the next open
    assert!(!bagholder_market::history::daily_due(&conn, &inst, &from, &today, &today, now).unwrap());
    let crate::feeds::HistoryAnswer::Ok(again) = crate::feeds::history_payload(&app, &q) else { panic!() };
    assert!(!again.pending, "nothing is under way after a miss");
}

// ---------------------------------------------------------------------------
// VersionsTest
// ---------------------------------------------------------------------------

#[test]
fn test_a_row_moves_both_and_the_same_row_read_again_moves_neither() {
    let d = db();
    let (full_before, core_before) = crate::versions::versions(&d.conn).unwrap();
    news(&d.conn, "BBB", "A release");
    let (full_after, core_after) = crate::versions::versions(&d.conn).unwrap();
    assert_ne!(full_before, full_after);
    assert_ne!(core_before, core_after);
    assert_eq!(crate::versions::data_version(&d.conn).unwrap(), full_after);
    // read again, the same: only when it was read changed
    news(&d.conn, "BBB", "A release");
    assert_eq!(crate::versions::versions(&d.conn).unwrap(), (full_after, core_after));
}


// ---------------------------------------------------------------------------
// InAppUpdateTest
// ---------------------------------------------------------------------------

/// The Rust release names its archive `-rust-<target>`; any other asset on the release is not this copy's.
#[test]
fn test_release_assets_take_the_web_archive_by_name_and_ignore_the_rest() {
    let rel = |names: &[String]| update::GithubRelease {
        tag_name: "v2.0.0".into(),
        assets: names.iter().map(|n| update::GithubAsset { name: n.clone(), browser_download_url: format!("https://x/{}", n) }).collect(),
        ..Default::default()
    };
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let mine = format!("bagholder-v2.0.0-rust-{}.{}", update::target_triple(), ext);
    assert_eq!(update::archive_name("v2.0.0"), mine);
    // the pre-split name of a target archive is not this one's
    let bare = format!("bagholder-v2.0.0-{}.{}", update::target_triple(), ext);
    assert_eq!(update::release_assets(&rel(&[bare.clone(), format!("{}.sha256", bare)])), None);
    let got = update::release_assets(&rel(&["bagholder-v2.0.0-android.apk".into(), "bagholder-v2.0.0-web.zip".into(), mine.clone(), format!("{}.sha256", mine), "bagholder-v2.0.0-web.zip.sha256".into()]));
    assert_eq!(got, Some(update::ReleaseAssets { archive: format!("https://x/{}", mine), sha: format!("https://x/{}.sha256", mine) }));
    assert_eq!(update::release_assets(&rel(&["bagholder-v2.0.0-web.zip".into(), "bagholder-v2.0.0-web.zip.sha256".into()])), None, "another platform's archive is not this one's");
    assert_eq!(update::release_assets(&rel(&[mine.clone(), "bagholder-v2.0.0-android.apk".into()])), None, "nothing without its checksum");
}

/// The update-off half: the Host check takes a live request and is not reachable here.
#[test]
fn test_a_container_copy_binds_wide_keeps_the_host_check_and_never_updates() {
    let _g = guard();
    let mine = update::parse_version(app::APP_VERSION).unwrap();
    let newer = format!("v{}.{}.{}", mine.0, mine.1, mine.2 + 1);
    let _fakes = UpdateFakes::new(Some(update::GithubRelease {
        tag_name: newer.clone(),
        html_url: format!("https://github.com/x/y/releases/tag/{}", newer),
        assets: vec![update::GithubAsset { name: format!("bagholder-{}-web.zip", newer), browser_download_url: "u".into() }],
    }));
    std::env::set_var("BAGHOLDER_NO_UPDATE", "1");
    let rec = update::check_for_update(&app());
    let st = crate::status::status(&app());
    let out = update::start_update(&app());
    std::env::remove_var("BAGHOLDER_NO_UPDATE");
    assert_eq!((rec.ok, rec.update_available, rec.latest.as_str()), (true, true, newer.as_str()));
    assert_eq!((st.update_by.as_str(), st.update_url.clone()), ("image", update::image_page()), "told of the release, sent to the image");
    assert_eq!((out.ok, out.error.as_deref()), (false, Some(update::UPDATES_OFF_MESSAGE)));
    assert_eq!(crate::status::status(&app()).update_by, "app");
}

#[test]
fn test_update_button_refuses_during_a_sync() {
    let _g = guard();
    let _fakes = UpdateFakes::new(None);
    let conn = app_ref().cache().unwrap();
    bagholder_store::tables::set_meta(&conn, "update_check", &json!({"updateAvailable": true, "latest": "v9.9.9", "assets": {"archive": "z", "sha": "s"}}).to_string()).unwrap();
    {
        let mut st = app_ref().state.lock().unwrap();
        st.syncing = true;
        st.updating.clear();
    }
    let during = update::start_update(&app());
    app().state.lock().unwrap().syncing = false;
    assert_eq!(during.ok, false);
    bagholder_store::tables::set_meta(&conn, "update_check", &json!({"updateAvailable": false}).to_string()).unwrap();
    assert_eq!(update::start_update(&app()).ok, false, "nothing to install");
}

// ---------------------------------------------------------------------------
// news: a searched ticker
// ---------------------------------------------------------------------------

use bagholder_market::news::{self, Ask, Clock, Net, NetError, Readers, WireAnswer};
use bagholder_store::feeds::{Feed, NewsItem};

#[test]
fn test_a_ticker_the_app_has_never_seen_is_placed_before_a_wire_is_asked() {
    // No directory carries every venue, so the venue comes from the app's own knowledge: the
    // security records the sync brought, then TMX's resolver, which names the venue it verified
    // by the quote. A ticker TMX cannot place is a US one.
    let _g = guard();
    let c = app_ref().cache().unwrap();
    let today = bagholder_market::clock_now().0;
    let seen = std::sync::Mutex::new((Vec::<String>::new(), None::<String>));
    let get = |url: &str, _: &[(&str, &str)]| -> Result<String, NetError> {
        seen.lock().unwrap().0.push(url.to_string());
        Ok(r#"{"data": {"rows": []}}"#.into())
    };
    let post = |_: &str, body: &Value, _: &[(&str, &str)]| -> Result<Value, NetError> {
        seen.lock().unwrap().1 = Some(body["variables"]["symbol"].as_str().unwrap_or("").to_string());
        Ok(json!({"data": {"news": [{"newsid": "3", "headline": "QIMC Engages", "source": "TMX Newsfile", "datetime": "2026-09-14T09:13:00-04:00"}]}}))
    };
    let net = Net { get: &get, post: &post, pace: false };
    let wire = |c: &Connection, s: &str, e: &str, cc: &str, cl: &Clock| news::fetch_symbol(c, &net, s, e, cc, cl);
    let extra = |_: Feed, _: &Ask| -> Result<Option<Vec<NewsItem>>, NetError> { Ok(Some(vec![])) };
    let readers = Readers { wire: &wire, extra: &extra };
    // a CSE listing no directory carries: TMX's resolver places it and the news is read under that form
    bagholder_store::tables::set_meta(&c, "tmx_form:QIMC", "@:CNX").unwrap();
    let out = serde_json::to_value(crate::feeds::news_symbol_payload_with(&app(), "QIMC", "", "", &readers, &|_, _, _| Ok(None))).unwrap();
    assert_eq!((out["source"].as_str().unwrap(), seen.lock().unwrap().1.clone()), ("tmx", Some("QIMC:CNX".to_string())));
    *seen.lock().unwrap() = (vec![], None);
    // TMX cannot place it: Nasdaq, whose items name the symbols they belong to
    bagholder_store::tables::set_meta(&c, "tmx_form:KO", &format!("none@{}", today)).unwrap();
    let out = serde_json::to_value(crate::feeds::news_symbol_payload_with(&app(), "KO", "", "", &readers, &|_, _, _| Ok(None))).unwrap();
    assert_eq!((out["source"].as_str().unwrap(), out["exchange"].as_str().unwrap(), seen.lock().unwrap().1.is_some()), ("nasdaq", "NASDAQ", false));
}

#[test]
fn test_a_searched_ticker_is_read_from_every_source_under_the_name_tmx_gives() {
    let _g = guard();
    let c = app_ref().cache().unwrap();
    let now = bagholder_market::clock_now().1 as i64;
    // every source read a moment ago: only a forced read asks them again
    for k in news::EXTRA_SOURCES {
        bagholder_store::tables::set_meta(&c, &format!("news_source_fetched:{}:SXHI@TSX", k.as_str()), &Clock::at(now).stamp()).unwrap();
    }
    let read = std::sync::Mutex::new((None::<(String, String, String)>, Vec::<String>::new()));
    let wire = |_: &Connection, s: &str, e: &str, cc: &str, _: &Clock| {
        read.lock().unwrap().0 = Some((s.to_string(), e.to_string(), cc.to_string()));
        (Feed::Tmx, Some(WireAnswer::default()))
    };
    let extra = |_: Feed, ask: &Ask| -> Result<Option<Vec<NewsItem>>, NetError> {
        read.lock().unwrap().1.push(ask.name.clone());
        Ok(Some(vec![]))
    };
    let listing = |_: &Connection, _: &str, _: &str| {
        Ok(Some(bagholder_model::wire::SymbolMatch { symbol: "SXHI".into(), name: "Ninepoint SpaceX HighShares ETF".into(), exchange: "TSX".into(), currency: "CAD".into(), ..Default::default() }))
    };
    let out = serde_json::to_value(crate::feeds::news_symbol_payload_with(&app(), "SXHI", "", "", &Readers { wire: &wire, extra: &extra }, &listing)).unwrap();
    assert_eq!(out["exchange"], "TSX");
    let got = read.lock().unwrap().clone();
    assert_eq!(got.0, Some(("SXHI".to_string(), "TSX".to_string(), "CAD".to_string())));
    assert!(!got.1.is_empty() && got.1.iter().all(|n| n == "Ninepoint SpaceX HighShares ETF"), "every source asked, forced, under the name TMX gives");
}

/// A checkout builds its update in the Cargo workspace under rust/, and pulls at the repository root.
#[test]
fn test_a_checkout_builds_in_the_rust_workspace_and_pulls_at_the_repository_root() {
    let _g = guard();
    assert!(app().root.join("rust/Cargo.toml").is_file(), "the root is the repository's");
    assert_eq!(update::cargo_dir(&app()), app().root.join("rust"));
    assert!(update::cargo_dir(&app()).join("Cargo.toml").is_file());
}

/// Every place the code waits on a clock, by file, with why it may. A wait that is
/// not here fails the build: a new one is either replaced by waiting for the thing
/// itself (`events::park_until`, a deadline that is known) or argued for in
/// here, with the reason it stays: this list is the one place timers are argued for.
const TIMED_WAITS: [(&str, usize, &str); 18] = [
    ("market/src/localmodel.rs", 2, "a child process coming up: it has no readiness signal"),
    ("market/src/pdftext.rs", 1, "a child process with a deadline: std has no wait with one"),
    ("net/src/machine.rs", 1, "every host's request rate on the one limiter (Yahoo, SEDAR+, the SEC, fund companies, news feeds, the archive at TMX): a turn taken, waited for with no lock held"),
    ("server/src/app.rs", 1, "`wait` itself"),
    ("server/src/broker_reads.rs", 1, "Wealthsimple's reads: until the next pull window (weekdays 2 PM Mountain), the next balances read while a page is open, or a failed read's rest ending"),
    ("server/src/docs.rs", 1, "a ticket's quote, every five seconds while a page shows that ticket and not a moment longer: Wealthsimple offers no quote push"),
    ("server/src/due.rs", 1, "the figure path's reads: until the next known deadline (the day turning in the person's zone, the Bank's 16:30, a close settling, a payer's window, a source's rest ending, a minute for quotes only while a page shows them)"),
    ("server/src/events.rs", 6, "`park_until_or` itself, the 40 ms gather; three in its tests (the day turning is the scheduler's, `due.rs`)"),
    ("server/src/feeds.rs", 13, "outside sources that offer no push, each only while wanted; known deadlines"),
    ("server/src/http/mod.rs", 1, "the five seconds requests in hand are given to finish when the app stops"),
    ("server/src/http/stream.rs", 1, "the event stream's keep-alive comment, every fifteen seconds while it is idle, so a connection that died is noticed: the transport's, not a poll"),
    ("server/src/login.rs", 8, "the sign-in browser: frames and a DevTools socket, only during a sign-in"),
    ("server/src/notify.rs", 4, "its stream's heartbeat (folded into /api/events in stage 6, with the page's structure); three in its tests"),
    ("server/src/orders/brackets.rs", 1, "the bracket engine, parked until a bracket is live: a stop's five-second cadence (SPEC §6)"),
    ("server/src/orders/readback.rs", 1, "Wealthsimple offers no order push: read only while an order is live or shown"),
    ("server/src/orders/ticket.rs", 2, "a ticket's sale waits on Wealthsimple confirming a bracket's exit cancelled, and on the sale's own answer, a second at a time for at most thirty: Wealthsimple pushes neither"),
    ("server/src/session.rs", 1, "the token's refresh deadline"),
    ("server/src/update.rs", 3, "child processes with a deadline: std has no wait with one"),
];

#[test]
fn test_no_wait_on_a_clock_that_is_not_accounted_for() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    // every form a wait on a clock takes: the app's own `wait` with any duration, a
    // thread's or a task's sleep, a bounded park or condvar wait, a timeout, an
    // interval (a stream's keep-alive included), a receive with a deadline
    let forms = regex::Regex::new(r"\.wait\([^)]|thread::sleep|park_until_or\(|wait_timeout|time::sleep|time::timeout|\binterval\(|recv_timeout|sleep_until").unwrap();
    let timed = |line: &str| {
        let l = line.trim_start();
        !l.starts_with("//") && forms.is_match(l)
    };
    // the scan finds each form
    for violation in ["if app.wait(QUOTE_EVERY) {", "std::thread::sleep(d);", "bus.park_until_or(&app, d, f)", "c.wait_timeout(g, d)", "tokio::time::sleep(d).await", "tokio::time::timeout(d, f).await", "KeepAlive::new().interval(KEEPALIVE)", "rx.recv_timeout(d)", "tokio::time::sleep_until(t)"] {
        assert!(timed(violation), "the scan misses {violation}");
    }
    assert!(!timed("child.wait()"), "a process's end is not a clock");
    let mut found: Vec<(String, usize)> = Vec::new();
    let mut dirs = vec![crates.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                if name != "tests" && name != "target" {
                    dirs.push(p);
                }
            } else if name.ends_with(".rs") && !name.starts_with("tests") {
                let n = std::fs::read_to_string(&p).unwrap().lines().filter(|l| timed(l)).count();
                if n > 0 {
                    found.push((p.strip_prefix(&crates).unwrap().to_string_lossy().replace('\\', "/"), n));
                }
            }
        }
    }
    found.sort();
    let want: Vec<(String, usize)> = TIMED_WAITS.iter().map(|(f, n, _)| (f.to_string(), *n)).collect();
    assert_eq!(found, want, "a wait on a clock was added or removed: argue for it in TIMED_WAITS, the list of timers that remain");
}

/// A stand-in for the server: a shell script in `dir`, run by `sh`, that the
/// update replaces. The version kept under `previous` records that it ran.
#[cfg(unix)]
fn update_stand_in(home: &std::path::Path, dir: &std::path::Path) -> std::path::PathBuf {
    let ran = home.join("previous-ran");
    std::fs::create_dir_all(home.join("previous")).unwrap();
    std::fs::write(home.join("previous").join("bagholder"), format!("echo previous > '{}'\nexit 0\n", ran.display())).unwrap();
    // the new version dies at once, inside the healthy window
    std::fs::write(dir.join("bagholder"), "exit 7\n").unwrap();
    ran
}

#[cfg(unix)]
fn assert_failure_said(home: &std::path::Path, tag: &str) {
    assert!(!home.join("update-pending").exists(), "the marker is spent");
    assert!(!home.join("previous").exists(), "the kept copies are back in place, not left behind");
    let a = app::App::new(home.to_path_buf(), home.to_path_buf(), "127.0.0.1".into());
    update::recall_failure(&a);
    let said = a.state.lock().unwrap().update_error.clone();
    assert!(said.starts_with("Update failed") && said.contains(tag), "the restarted server says the update failed: {said:?}");
    let again = app::App::new(home.to_path_buf(), home.to_path_buf(), "127.0.0.1".into());
    update::recall_failure(&again);
    assert_eq!(again.state.lock().unwrap().update_error, "", "said once, by the server the supervisor started");
}

#[cfg(unix)]
fn supervise_stand_in(home: &std::path::Path, dir: &std::path::Path) -> i32 {
    let exe = dir.join("bagholder");
    update::supervise_child(home, dir, update::UPDATE_HEALTHY_SEC, || {
        let mut c = std::process::Command::new("sh");
        c.arg(&exe);
        c
    })
}

#[test]
#[cfg(unix)]
fn test_a_new_version_that_dies_in_the_window_is_rolled_back_and_the_header_says_so() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ran = update_stand_in(home.path(), dir.path());
    update::write_pending(home.path(), &update::Pending { tag: "v99.0.0".into(), git: None }).unwrap();
    assert_eq!(supervise_stand_in(home.path(), dir.path()), 0, "the previous version is started again and runs");
    assert_eq!(std::fs::read_to_string(&ran).unwrap().trim(), "previous");
    assert!(std::fs::read_to_string(dir.path().join("bagholder")).unwrap().contains("previous-ran"), "the previous executable is in place");
    assert_failure_said(home.path(), "v99.0.0");
}

#[test]
#[cfg(unix)]
fn test_a_git_checkout_goes_back_to_its_commit_when_the_new_version_dies() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("target");
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .current_dir(root.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(o.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::write(root.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.path().join("page.html"), "before\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "before"]);
    let before = git(&["rev-parse", "HEAD"]);
    std::fs::write(root.path().join("page.html"), "after\n").unwrap();
    git(&["commit", "-q", "-am", "after"]);
    let ran = update_stand_in(home.path(), &dir);
    let back = update::GitRestore { root: root.path().to_path_buf(), commit: before.clone() };
    update::write_pending(home.path(), &update::Pending { tag: "v99.0.0".into(), git: Some(back) }).unwrap();
    assert_eq!(supervise_stand_in(home.path(), &dir), 0, "the previous version is started again, not left stopped");
    assert_eq!(std::fs::read_to_string(&ran).unwrap().trim(), "previous");
    assert_eq!(git(&["rev-parse", "HEAD"]), before, "the checkout is back on the commit before the pull");
    assert_eq!(std::fs::read_to_string(root.path().join("page.html")).unwrap(), "before\n");
    assert_failure_said(home.path(), "v99.0.0");
}

// ---------------------------------------------------------------------------
// LocalModelTest
// ---------------------------------------------------------------------------

#[test]
fn test_a_test_app_never_turns_the_local_model_on() {
    let _g = guard();
    // what the filings path asks of the model: whether one is up, and a wait for one coming
    assert!(!bagholder_market::enrich::summary_available());
    assert!(!bagholder_market::enrich::wait_for_summary(0.0));
    // the model has no folder at all, so none outside this app's home; and nothing was started
    let home = crate::tests_common::app().home.clone();
    let folder = bagholder_market::localmodel::folder();
    assert!(folder.as_ref().map_or(true, |f| f.starts_with(&home)), "the model folder {:?} is outside {:?}", folder, home);
    assert_eq!(folder, None, "only the running server turns the model on");
    assert_eq!(bagholder_market::localmodel::status(), "off", "nothing detected, downloaded or started");
}

/// A checkout's update builds the page of the pulled commit before the server
/// that carries it; a page that does not build, or no npm to build it, fails the
/// update before the server is built (and `pull` puts the previous commit back).
#[cfg(unix)]
#[test]
fn test_a_checkout_builds_its_page_then_its_server_and_stops_at_the_first_failure() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let (root, bin, log) = (t.path().join("checkout"), t.path().join("bin"), t.path().join("calls"));
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::create_dir_all(root.join("rust")).unwrap();
    std::fs::write(root.join("web/package.json"), "{}").unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    // each records its name, its folder and its arguments; one named in `fail` fails, saying so
    let stand_in = |name: &str| {
        let p = bin.join(name);
        std::fs::write(&p, format!("#!/bin/sh\necho \"{name} $(basename \"$(pwd -P)\") $*\" >> '{}'\nif grep -qx \"{name} $*\" '{}' 2>/dev/null; then echo \"{name} broke\" >&2; exit 1; fi\n", log.display(), t.path().join("fail").display())).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    stand_in("npm");
    stand_in("cargo");
    let find = |cmd: &str| Some(bin.join(cmd)).filter(|p| p.is_file());
    let calls = || -> Vec<String> {
        let c = std::fs::read_to_string(&log).map(|t| t.lines().map(String::from).collect()).unwrap_or_else(|_| vec![]);
        std::fs::remove_file(&log).ok();
        c
    };
    let fail = |what: &str| std::fs::write(t.path().join("fail"), what).unwrap();

    update::build_checkout(&root, &root.join("rust"), &find).unwrap();
    assert_eq!(calls(), ["npm web ci", "npm web run build", "cargo rust build --release --bins"], "the page, then the server that carries it");

    fail("npm ci");
    assert_eq!(update::build_checkout(&root, &root.join("rust"), &find), Err("the page did not build: npm ci failed: npm broke".to_string()));
    assert_eq!(calls(), ["npm web ci"], "nothing built after a step that failed");

    fail("npm run build");
    assert_eq!(update::build_checkout(&root, &root.join("rust"), &find), Err("the page did not build: npm run build failed: npm broke".to_string()));
    assert_eq!(calls(), ["npm web ci", "npm web run build"]);

    fail("cargo build --release --bins");
    assert_eq!(update::build_checkout(&root, &root.join("rust"), &find), Err("the new version did not build: cargo broke".to_string()));
    assert_eq!(calls().len(), 3);

    // no npm: the update fails saying so, and no server is built with a page of another commit
    std::fs::remove_file(bin.join("npm")).unwrap();
    let why = update::build_checkout(&root, &root.join("rust"), &find).unwrap_err();
    assert!(why.contains("npm is not on the PATH"), "{why}");
    assert!(calls().is_empty());
}
