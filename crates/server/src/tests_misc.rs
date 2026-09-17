//! Ported from tests/test_store.py: StoreTest (status, protocol, port, update
//! check, history endpoint), VersionsTest, TilesTest, WatchlistTest,
//! InAppUpdateTest (the parts reachable without a network or a child process).
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::app::{self, app};
use crate::update;

/// The shared app, its store's schema in place.
fn guard() -> std::sync::MutexGuard<'static, ()> {
    let g = crate::tests_common::guard();
    bagholder_store::relabel::ensure(&app().open().unwrap()).unwrap();
    g
}

/// A fresh database of its own, as the Python setUp's temporary home.
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
    let conn = Connection::open(dir.join("bagholder.db")).unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    Db { dir, conn }
}

fn insert_local(conn: &Connection, row: Value) {
    let n = std::cell::Cell::new(0u64);
    let id = || {
        n.set(n.get() + 1);
        format!("00000000-0000-4000-8000-{:012}", n.get())
    };
    bagholder_store::activities::insert_local(conn, &row, &id).unwrap();
}

// ---------------------------------------------------------------------------
// StoreTest
// ---------------------------------------------------------------------------

#[test]
fn test_status_carries_the_data_version_so_the_page_can_reload() {
    let _g = guard();
    let conn = app().open().unwrap();
    let v0 = crate::status_payload()["dataVersion"].as_str().unwrap().to_string();
    assert!(!v0.is_empty());
    let price = 4.75 + (app::now_unix() % 1000.0) / 1e4;
    bagholder_store::market::upsert_quote(&conn, "RDDY", &json!({"price": price, "fetchedAt": "2026-09-07T15:00:00Z"}), "tmx", &app::now_iso()).unwrap();
    let v1 = crate::status_payload()["dataVersion"].as_str().unwrap().to_string();
    assert_ne!(v0, v1);
    bagholder_store::market::upsert_distributions(&conn, "RDDY", &[json!({"exDate": "2026-09-30", "payDate": "2026-10-05", "amount": 0.2, "currency": "CAD"})], "tmx").unwrap();
    let v2 = crate::status_payload()["dataVersion"].as_str().unwrap().to_string();
    // the shared home may already hold this row: then a second, later one moves it
    if v1 == v2 {
        bagholder_store::market::upsert_distributions(&conn, "RDDY", &[json!({"exDate": "2099-09-30", "payDate": "2099-10-05", "amount": 0.2, "currency": "CAD"})], "tmx").unwrap();
    }
    assert_ne!(v1, crate::status_payload()["dataVersion"].as_str().unwrap());
    let _ = conn.execute("DELETE FROM quotes WHERE symbol = 'RDDY'", []);
    let _ = conn.execute("DELETE FROM distributions WHERE symbol = 'RDDY'", []);
    app().invalidate();
}

/// The clock cannot be stood in for here: the version is checked to carry
/// today's local date, which is what makes it change at midnight.
#[test]
fn test_status_version_changes_with_the_date_so_the_page_refetches_at_midnight() {
    let _g = guard();
    let v = crate::status_payload()["dataVersion"].as_str().unwrap().to_string();
    assert!(v.ends_with(&format!("|{}", bagholder_model::clock::today_local())), "{}", v);
}

#[test]
fn test_page_and_server_agree_on_the_protocol_stamp() {
    let _g = guard();
    let page = std::fs::read_to_string(crate::feeds::ledger_path()).unwrap();
    let m = regex::Regex::new(r#"const PROTOCOL = "([^"]+)""#).unwrap().captures(&page).expect("PROTOCOL on the page");
    assert_eq!(&m[1], app::PROTOCOL);
    assert_eq!(crate::status_payload()["protocol"], app::PROTOCOL);
}

#[test]
fn test_port_can_be_chosen_for_a_second_instance() {
    let _g = guard();
    let saved = std::env::var("BAGHOLDER_PORT").ok();
    std::env::set_var("BAGHOLDER_PORT", "8799");
    assert_eq!(crate::port_choices(), vec![8799]);
    std::env::set_var("BAGHOLDER_PORT", "80");
    assert_eq!(crate::port_choices(), crate::PORTS.to_vec(), "a privileged or nonsense port is ignored");
    std::env::remove_var("BAGHOLDER_PORT");
    assert_eq!(crate::port_choices(), crate::PORTS.to_vec());
    if let Some(p) = saved {
        std::env::set_var("BAGHOLDER_PORT", p);
    }
}

/// GitHub answers through `update::FAKE_RELEASE`; notifications are recorded,
/// never shown. Put back what it touched.
struct UpdateFakes;

impl UpdateFakes {
    fn new(answer: Option<Value>) -> Self {
        *update::FAKE_RELEASE.lock().unwrap() = Some((0, answer));
        let mut d = crate::notify::test_hooks::DELIVERED.lock().unwrap();
        if d.is_none() {
            *d = Some(Vec::new());
        }
        UpdateFakes
    }
    fn answer(&self, answer: Option<Value>) {
        *update::FAKE_RELEASE.lock().unwrap() = Some((0, answer));
    }
    fn calls(&self) -> usize {
        update::FAKE_RELEASE.lock().unwrap().as_ref().map(|x| x.0).unwrap_or(0)
    }
}

impl Drop for UpdateFakes {
    fn drop(&mut self) {
        *update::FAKE_RELEASE.lock().unwrap() = None;
        if let Ok(c) = app().open() {
            let _ = c.execute("DELETE FROM meta WHERE key = 'update_check'", []);
            let _ = c.execute("DELETE FROM notifications WHERE key LIKE 'update:%'", []);
        }
        *crate::notify::test_hooks::DELIVERED.lock().unwrap() = None;
    }
}

fn set_checked_at(secs_ago: f64) {
    let c = app().open().unwrap();
    let mut rec = update::update_status();
    rec["checkedAt"] = json!(app::stamp_of((app::now_unix() - secs_ago) as i64));
    bagholder_store::tables::set_meta(&c, "update_check", &rec.to_string()).unwrap();
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
    let fakes = UpdateFakes::new(Some(json!({"tag_name": format!("v{}", app::APP_VERSION), "html_url": "https://github.com/x/y/releases/tag/v1"})));
    let rec = update::check_for_update();
    assert_eq!((rec["ok"].as_bool(), rec["updateAvailable"].as_bool()), (Some(true), Some(false)), "same release: no flag");
    fakes.answer(Some(json!({"tag_name": older, "html_url": "u"})));
    assert_eq!(update::check_for_update()["updateAvailable"], false, "an older release never flags");
    fakes.answer(Some(json!({"tag_name": newer, "html_url": format!("https://github.com/ProfessorBagholder/Bagholder/releases/tag/{}", newer)})));
    set_checked_at(30.0 * 60.0);
    update::check_for_update_if_due();
    assert_eq!(fakes.calls(), 0, "checked half an hour ago: GitHub is not asked again");
    set_checked_at(2.0 * 3600.0);
    let rec = update::check_for_update_if_due();
    assert_eq!((rec["updateAvailable"].as_bool(), rec["latest"].as_str()), (Some(true), Some(newer.as_str())));
    let st = crate::status_payload();
    assert_eq!(
        (st["version"].clone(), st["latestVersion"].clone(), st["updateAvailable"].clone(), st["updateUrl"].clone()),
        (json!(app::APP_VERSION), json!(newer), json!(true), rec["url"].clone())
    );
    fakes.answer(None);
    let rec = update::check_for_update();
    assert_eq!((rec["ok"].as_bool(), rec["updateAvailable"].as_bool()), (Some(false), Some(false)), "offline: silent, no flag");
    fakes.answer(Some(json!({"message": "Not Found"})));
    assert_eq!(update::check_for_update()["updateAvailable"], false, "no release published yet: nothing to flag");
}

#[test]
fn test_history_endpoint_validates_and_serves_bars() {
    let _g = guard();
    assert_eq!(crate::feeds::history_payload("symbol=RDDY")["ok"], false);
    assert_eq!(crate::feeds::history_payload("symbol=RDDY&exchange=TSX&currency=CAD&kind=Shares&from=2026-08-25&to=2026-09-05&tf=2h")["ok"], false);
}

// ---------------------------------------------------------------------------
// VersionsTest
// ---------------------------------------------------------------------------

#[test]
fn test_a_price_moves_the_version_but_not_the_core() {
    let d = db();
    let now = "2026-09-12T10:00:00Z";
    bagholder_store::market::upsert_quote(&d.conn, "AAA", &json!({"price": 10.0, "currency": "CAD"}), "tmx", now).unwrap();
    let (full_before, core_before) = crate::versions::versions(&d.conn).unwrap();
    bagholder_store::market::upsert_quote(&d.conn, "AAA", &json!({"price": 11.0, "currency": "CAD"}), "tmx", now).unwrap();
    let (full_after, core_after) = crate::versions::versions(&d.conn).unwrap();
    assert_ne!(full_before, full_after, "the page is told the price moved");
    assert_eq!(core_before, core_after, "but nothing else did, so the match is kept");
    assert_eq!(crate::versions::data_version(&d.conn).unwrap(), full_after);
}

#[test]
fn test_a_row_moves_both() {
    let d = db();
    let (full_before, core_before) = crate::versions::versions(&d.conn).unwrap();
    insert_local(&d.conn, json!({"id": "r1", "transactionDate": "2026-03-03", "symbol": "BBB", "category": "trade", "activitySubType": "BUY", "quantity": 1, "unitPrice": 3.0, "netCashAmount": -3.0, "currency": "CAD"}));
    let (full_after, core_after) = crate::versions::versions(&d.conn).unwrap();
    assert_ne!(full_before, full_after);
    assert_ne!(core_before, core_after);
}

// ---------------------------------------------------------------------------
// TilesTest
// ---------------------------------------------------------------------------

fn base_of(conn: &Connection, quotes: Option<Value>) -> bagholder_model::base::Base {
    let snapshot = bagholder_store::snapshot::snapshot(conn, true).unwrap();
    let mut market = bagholder_store::market::market_data(conn).unwrap();
    if let Some(q) = quotes {
        market["quotes"] = q;
    }
    let journal = bagholder_store::snapshot::journal(conn).unwrap();
    bagholder_model::base::build_base(&snapshot, &market, &journal, None)
}

/// The model half; the store half is in crates/store/tests/tables.rs.
#[test]
fn test_the_row_is_the_default_until_saved_and_then_what_was_saved() {
    let d = db();
    let rows = |d: &Db| bagholder_model::markets::tile_rows(&base_of(&d.conn, None));
    let got: Vec<(String, String, i64)> = rows(&d).iter().map(|t| (app::f(t, "symbol"), app::f(t, "label"), t["decimals"].as_i64().unwrap())).collect();
    let want: Vec<(String, String, i64)> = [("SPX", "SPX", 2), ("NDX", "NDX", 2), ("DJI", "DJI", 2), ("VIX", "VIX", 2), ("GC", "GOLD", 2), ("BTCUSD", "BITCOIN", 0)]
        .iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), *c))
        .collect();
    assert_eq!(got, want);
    let before = crate::versions::data_version(&d.conn).unwrap();
    bagholder_store::admin::save_tiles(&d.conn, &[json!({"symbol": "tnx", "exchange": "index"}), json!({"symbol": "usdcad", "exchange": "fx"}), json!({"symbol": "", "exchange": "x"}), json!("junk")]).unwrap();
    assert_ne!(crate::versions::data_version(&d.conn).unwrap(), before, "the row is part of the data version");
    let r = rows(&d);
    let got: Vec<(String, String, String, i64)> = r.iter().map(|t| (app::f(t, "symbol"), app::f(t, "label"), app::f(t, "kind"), t["decimals"].as_i64().unwrap())).collect();
    assert_eq!(got, vec![("TNX".into(), "10Y".into(), "Rate".into(), 3), ("USDCAD".into(), "USD/CAD".into(), "Currency".into(), 4)]);
    assert_eq!(r.iter().map(|t| t["last"].clone()).collect::<Vec<_>>(), vec![Value::Null, Value::Null], "no quote yet: a dash, never a zero");
    bagholder_store::admin::save_tiles(&d.conn, &[]).unwrap();
    assert!(rows(&d).is_empty(), "an emptied row stays empty");
}

#[test]
fn test_the_row_reads_its_quotes_where_a_watched_instrument_would() {
    let d = db();
    bagholder_store::admin::save_tiles(&d.conn, &[json!({"symbol": "SPX", "exchange": "Index"})]).unwrap();
    let base = base_of(&d.conn, None);
    let q: Vec<(String, String, String)> = bagholder_model::markets::quote_symbols(&base).iter().map(|r| (app::f(r, "quoteKey"), app::f(r, "yahoo"), app::f(r, "kind"))).collect();
    assert_eq!(q, vec![("SPX@INDEX".to_string(), "^GSPC".to_string(), "Instrument".to_string())], "quoted through the watch path");
    let base = base_of(&d.conn, Some(json!({"SPX@INDEX": {"price": 6742.18, "priceChange": 42.18, "percentChange": 0.63}})));
    let row = &bagholder_model::markets::tile_rows(&base)[0];
    assert_eq!((row["last"].as_f64(), row["change"].as_f64(), row["percentChange"].as_f64()), (Some(6742.18), Some(42.18), Some(0.63)));
}

/// Only the refusal: an accepted save starts a quote fetch against the live
/// sources, which a test must not reach.
#[test]
fn test_the_set_route_keeps_only_directory_instruments_in_order_and_caps_at_twelve() {
    let _g = guard();
    let conn = app().open().unwrap();
    let saved = bagholder_store::tables::get_meta(&conn, bagholder_store::snapshot::TILES_META, "").unwrap();
    bagholder_store::admin::save_tiles(&conn, &[json!({"symbol": "VIX", "exchange": "Index"}), json!({"symbol": "GC", "exchange": "COMEX"})]).unwrap();
    app().invalidate();
    let too_many: Vec<Value> = ["SPX", "NDX", "IXIC", "DJI", "RUT", "VIX", "TSX", "FTSE", "DAX", "N225", "HSI", "STOXX50E", "DXY"].iter().map(|s| json!({"symbol": s, "exchange": "Index"})).collect();
    assert_eq!(crate::feeds::tiles_set(&json!({"tiles": too_many}))["ok"], false);
    let b = app().base().unwrap();
    let syms: Vec<String> = bagholder_model::markets::tile_rows(&b).iter().map(|t| app::f(t, "symbol")).collect();
    assert_eq!(syms, vec!["VIX", "GC"], "a refused save changes nothing");
    if saved.is_empty() {
        conn.execute("DELETE FROM meta WHERE key = ?", [bagholder_store::snapshot::TILES_META]).unwrap();
    } else {
        bagholder_store::tables::set_meta(&conn, bagholder_store::snapshot::TILES_META, &saved).unwrap();
    }
    app().invalidate();
}

// ---------------------------------------------------------------------------
// WatchlistTest
// ---------------------------------------------------------------------------

/// The data-version half of test_add_list_remove (the rest is the store's).
#[test]
fn test_add_list_remove() {
    let d = db();
    let before = crate::versions::data_version(&d.conn).unwrap();
    bagholder_store::feeds::add_watch(&d.conn, "shop", "tsx", "Shopify Inc.", "cad", "", "2026-09-11T14:00:00Z").unwrap();
    assert_ne!(crate::versions::data_version(&d.conn).unwrap(), before, "the model's fingerprint follows the list");
}

#[test]
fn test_quote_refresh_keys_a_watched_listing_by_venue() {
    let d = db();
    let needing = bagholder_market::quotes::quote_symbols_needing_refresh(
        &d.conn,
        &[
            json!({"symbol": "AAPL", "exchange": "NEO", "currency": "CAD", "kind": "Shares"}),
            json!({"symbol": "AAPL", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares", "quoteKey": "AAPL@NASDAQ"}),
        ],
        app::now_unix(),
        15.0,
    )
    .unwrap();
    let got: Vec<(String, String)> = needing.into_iter().map(|(k, src, _)| (k, src)).collect();
    assert_eq!(got, vec![("AAPL".to_string(), "cboe_ca".to_string()), ("AAPL@NASDAQ".to_string(), "yahoo_quote".to_string())], "the held CDR and the watched US listing keep separate quotes, each from a feed live for its market");
}

// ---------------------------------------------------------------------------
// InAppUpdateTest
// ---------------------------------------------------------------------------

/// The Rust release names its archive by target, not `-web.zip`.
#[test]
fn test_release_assets_take_the_web_archive_by_name_and_ignore_the_rest() {
    let rel = |names: &[String]| json!({"tag_name": "v2.0.0", "assets": names.iter().map(|n| json!({"name": n, "browser_download_url": format!("https://x/{}", n)})).collect::<Vec<_>>()});
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let mine = format!("bagholder-v2.0.0-{}.{}", update::target_triple(), ext);
    let got = update::release_assets(&rel(&["bagholder-v2.0.0-android.apk".into(), "bagholder-v2.0.0-web.zip".into(), mine.clone(), format!("{}.sha256", mine), "bagholder-v2.0.0-web.zip.sha256".into()]));
    assert_eq!(got, json!({"archive": format!("https://x/{}", mine), "sha": format!("https://x/{}.sha256", mine)}));
    assert_eq!(update::release_assets(&rel(&["bagholder-v2.0.0-web.zip".into(), "bagholder-v2.0.0-web.zip.sha256".into()])), json!({}), "another platform's archive is not this one's");
    assert_eq!(update::release_assets(&rel(&[mine.clone(), "bagholder-v2.0.0-android.apk".into()])), json!({}), "nothing without its checksum");
}

/// The update-off half: the Host check takes a live request and is not reachable here.
#[test]
fn test_a_container_copy_binds_wide_keeps_the_host_check_and_never_updates() {
    let _g = guard();
    let mine = update::parse_version(app::APP_VERSION).unwrap();
    let newer = format!("v{}.{}.{}", mine.0, mine.1, mine.2 + 1);
    let _fakes = UpdateFakes::new(Some(json!({"tag_name": newer, "html_url": format!("https://github.com/x/y/releases/tag/{}", newer), "assets": [{"name": format!("bagholder-{}-web.zip", newer), "browser_download_url": "u"}]})));
    std::env::set_var("BAGHOLDER_NO_UPDATE", "1");
    let rec = update::check_for_update();
    let st = crate::status_payload();
    let out = update::start_update();
    std::env::remove_var("BAGHOLDER_NO_UPDATE");
    assert_eq!((rec["ok"].as_bool(), rec["updateAvailable"].as_bool(), rec["latest"].as_str()), (Some(true), Some(true), Some(newer.as_str())));
    assert_eq!((st["updateBy"].as_str(), st["updateUrl"].clone()), (Some("image"), json!(update::image_page())), "told of the release, sent to the image");
    assert_eq!((out["ok"].as_bool(), out["error"].as_str()), (Some(false), Some(update::UPDATES_OFF_MESSAGE)));
    assert_eq!(crate::status_payload()["updateBy"], "app");
}

#[test]
fn test_update_button_refuses_during_a_sync() {
    let _g = guard();
    let _fakes = UpdateFakes::new(None);
    let conn = app().open().unwrap();
    bagholder_store::tables::set_meta(&conn, "update_check", &json!({"updateAvailable": true, "latest": "v9.9.9", "assets": {"archive": "z", "sha": "s"}}).to_string()).unwrap();
    {
        let mut st = app().state.lock().unwrap();
        st.syncing = true;
        st.updating.clear();
    }
    let during = update::start_update();
    app().state.lock().unwrap().syncing = false;
    assert_eq!(during["ok"], false);
    bagholder_store::tables::set_meta(&conn, "update_check", &json!({"updateAvailable": false}).to_string()).unwrap();
    assert_eq!(update::start_update()["ok"], false, "nothing to install");
}
