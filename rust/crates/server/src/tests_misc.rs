//! Status, protocol, port, update
//! check, history endpoint, versions, tiles, watchlist and in-app update
//! (the parts reachable without a network or a child process).
use rusqlite::Connection;
use bagholder_model::input::Listing;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::app;
use crate::tests_common::{app, app_ref};
use crate::update;

/// The shared app, its store's schema in place.
fn guard() -> std::sync::MutexGuard<'static, ()> {
    let g = crate::tests_common::guard();
    bagholder_store::relabel::ensure(&app_ref().open().unwrap()).unwrap();
    g
}

/// A fresh database of its own, in a temporary home.
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
    let row: bagholder_store::activities::ActivityRow = serde_json::from_value(row).unwrap();
    bagholder_store::activities::insert_local(conn, &row, &id).unwrap();
}

// ---------------------------------------------------------------------------
// StoreTest
// ---------------------------------------------------------------------------

#[test]
fn test_status_carries_the_data_version_so_the_page_can_reload() {
    let _g = guard();
    let conn = app_ref().open().unwrap();
    let v0 = crate::status::answer(&app()).data_version;
    assert!(!v0.is_empty());
    let price = 4.75 + (app::now_unix() % 1000.0) / 1e4;
    bagholder_store::market::upsert_quote(&conn, "RDDY", &bagholder_store::market::QuoteRecord { price: Some(price), ..Default::default() }, "tmx", &app::now_iso()).unwrap();
    let v1 = crate::status::answer(&app()).data_version;
    assert_ne!(v0, v1);
    bagholder_store::market::upsert_distributions(&conn, "RDDY", &[bagholder_store::market::DistributionRecord { ex_date: "2026-09-30".into(), pay_date: "2026-10-05".into(), amount: Some(0.2), currency: "CAD".into() }], "tmx").unwrap();
    let v2 = crate::status::answer(&app()).data_version;
    // the shared home may already hold this row: then a second, later one moves it
    if v1 == v2 {
        bagholder_store::market::upsert_distributions(&conn, "RDDY", &[bagholder_store::market::DistributionRecord { ex_date: "2099-09-30".into(), pay_date: "2099-10-05".into(), amount: Some(0.2), currency: "CAD".into() }], "tmx").unwrap();
    }
    assert_ne!(v1, crate::status::answer(&app()).data_version);
    let _ = conn.execute("DELETE FROM quotes WHERE symbol = 'RDDY'", []);
    let _ = conn.execute("DELETE FROM distributions WHERE symbol = 'RDDY'", []);
    app().invalidate();
}

/// The clock cannot be stood in for here: the version is checked to carry
/// today's local date, which is what makes it change at midnight.
#[test]
fn test_status_version_changes_with_the_date_so_the_page_refetches_at_midnight() {
    let _g = guard();
    let v = crate::status::answer(&app()).data_version;
    assert!(v.ends_with(&format!("|{}", bagholder_model::clock::today_local())), "{}", v);
}

#[test]
fn test_page_and_server_agree_on_the_protocol_stamp() {
    let _g = guard();
    let page = std::fs::read_to_string(crate::feeds::ledger_path(&app())).unwrap();
    let m = regex::Regex::new(r#"const PROTOCOL = "([^"]+)""#).unwrap().captures(&page).expect("PROTOCOL on the page");
    assert_eq!(&m[1], app::PROTOCOL);
    assert_eq!(crate::status::status(&app()).protocol, app::PROTOCOL);
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
        if let Ok(c) = app_ref().open() {
            let _ = c.execute("DELETE FROM meta WHERE key = 'update_check'", []);
            let _ = c.execute("DELETE FROM notifications WHERE key LIKE 'update:%'", []);
        }
        *crate::notify::test_hooks::DELIVERED.lock().unwrap() = None;
    }
}

fn set_checked_at(secs_ago: f64) {
    let c = app_ref().open().unwrap();
    let mut rec = update::update_status(&app());
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
    let rec = update::check_for_update(&app());
    assert_eq!((rec["ok"].as_bool(), rec["updateAvailable"].as_bool()), (Some(true), Some(false)), "same release: no flag");
    fakes.answer(Some(json!({"tag_name": older, "html_url": "u"})));
    assert_eq!(update::check_for_update(&app())["updateAvailable"], false, "an older release never flags");
    fakes.answer(Some(json!({"tag_name": newer, "html_url": format!("https://github.com/ProfessorBagholder/Bagholder/releases/tag/{}", newer)})));
    set_checked_at(30.0 * 60.0);
    update::check_for_update_if_due(&app());
    assert_eq!(fakes.calls(), 0, "checked half an hour ago: GitHub is not asked again");
    set_checked_at(2.0 * 3600.0);
    let rec = update::check_for_update_if_due(&app());
    assert_eq!((rec["updateAvailable"].as_bool(), rec["latest"].as_str()), (Some(true), Some(newer.as_str())));
    let st = crate::status::status(&app());
    assert_eq!(
        (st.version, st.latest_version, st.update_available, json!(st.update_url)),
        (app::APP_VERSION.to_string(), newer.clone(), true, rec["url"].clone())
    );
    fakes.answer(None);
    let rec = update::check_for_update(&app());
    assert_eq!((rec["ok"].as_bool(), rec["updateAvailable"].as_bool()), (Some(false), Some(false)), "offline: silent, no flag");
    fakes.answer(Some(json!({"message": "Not Found"})));
    assert_eq!(update::check_for_update(&app())["updateAvailable"], false, "no release published yet: nothing to flag");
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

// ---------------------------------------------------------------------------
// VersionsTest
// ---------------------------------------------------------------------------

#[test]
fn test_a_price_moves_the_version_but_not_the_core() {
    let d = db();
    let now = "2026-09-12T10:00:00Z";
    bagholder_store::market::upsert_quote(&d.conn, "AAA", &bagholder_store::market::QuoteRecord { price: Some(10.0), ..Default::default() }, "tmx", now).unwrap();
    let (full_before, core_before) = crate::versions::versions(&d.conn).unwrap();
    bagholder_store::market::upsert_quote(&d.conn, "AAA", &bagholder_store::market::QuoteRecord { price: Some(11.0), ..Default::default() }, "tmx", now).unwrap();
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

fn base_of(conn: &Connection, quotes: Option<Value>) -> std::sync::Arc<bagholder_model::base::Base> {
    if let Some(q) = quotes {
        for (key, v) in q.as_object().cloned().unwrap_or_default() {
            let rec = bagholder_store::market::QuoteRecord {
                price: v.get("price").and_then(|x| x.as_f64()),
                price_change: v.get("priceChange").and_then(|x| x.as_f64()),
                percent_change: v.get("percentChange").and_then(|x| x.as_f64()),
                ..Default::default()
            };
            bagholder_store::market::upsert_quote(conn, &key, &rec, "test", "2026-01-01T00:00:00Z").unwrap();
        }
    }
    let cache = crate::model_cache::ModelCache::new();
    let today = bagholder_model::clock::today_local();
    cache.base(conn, &today).unwrap()
}

/// The model half; the store half is in crates/store/tests/tables.rs.
#[test]
fn test_the_row_is_the_default_until_saved_and_then_what_was_saved() {
    let d = db();
    let rows = |d: &Db| bagholder_model::testing::sent(&bagholder_model::markets::tile_rows(&base_of(&d.conn, None)));
    let got: Vec<(String, String, i64)> = rows(&d).iter().map(|t| (app::f(t, "symbol"), app::f(t, "label"), t["decimals"].as_i64().unwrap())).collect();
    let want: Vec<(String, String, i64)> = [("SPX", "SPX", 2), ("NDX", "NDX", 2), ("DJI", "DJI", 2), ("VIX", "VIX", 2), ("GC", "GOLD", 2), ("BTCUSD", "BITCOIN", 0)]
        .iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), *c))
        .collect();
    assert_eq!(got, want);
    let before = crate::versions::data_version(&d.conn).unwrap();
    let tiles_in: Vec<bagholder_model::input::TileRef> = bagholder_model::lenient::rows(&json!([{"symbol": "tnx", "exchange": "index"}, {"symbol": "usdcad", "exchange": "fx"}, {"symbol": "", "exchange": "x"}, "junk"]));
    bagholder_store::admin::save_tiles(&d.conn, &tiles_in).unwrap();
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
    bagholder_store::admin::save_tiles(&d.conn, &[bagholder_model::input::TileRef { symbol: "SPX".into(), exchange: "Index".into() }]).unwrap();
    let base = base_of(&d.conn, None);
    let q: Vec<(String, String, String)> = bagholder_model::markets::quote_symbols(&base).into_iter().map(|r| (r.quote_key.unwrap_or_default(), r.yahoo.unwrap_or_default(), r.kind)).collect();
    assert_eq!(q, vec![("SPX@INDEX".to_string(), "^GSPC".to_string(), "Instrument".to_string())], "quoted through the watch path");
    let base = base_of(&d.conn, Some(json!({"SPX@INDEX": {"price": 6742.18, "priceChange": 42.18, "percentChange": 0.63}})));
    let row = &bagholder_model::testing::sent(&bagholder_model::markets::tile_rows(&base))[0];
    assert_eq!((row["last"].as_f64(), row["change"].as_f64(), row["percentChange"].as_f64()), (Some(6742.18), Some(42.18), Some(0.63)));
}

/// Only the refusal: an accepted save starts a quote fetch against the live
/// sources, which a test must not reach.
#[test]
fn test_the_set_route_keeps_only_directory_instruments_in_order_and_caps_at_twelve() {
    let _g = guard();
    let conn = app_ref().open().unwrap();
    let saved = bagholder_store::tables::get_meta(&conn, bagholder_store::rows::TILES_META, "").unwrap();
    bagholder_store::admin::save_tiles(&conn, &[bagholder_model::input::TileRef { symbol: "VIX".into(), exchange: "Index".into() }, bagholder_model::input::TileRef { symbol: "GC".into(), exchange: "COMEX".into() }]).unwrap();
    app().invalidate();
    let too_many: Vec<Value> = ["SPX", "NDX", "IXIC", "DJI", "RUT", "VIX", "TSX", "FTSE", "DAX", "N225", "HSI", "STOXX50E", "DXY"].iter().map(|s| json!({"symbol": s, "exchange": "Index"})).collect();
    let too_many_tiles: Vec<bagholder_model::input::TileRef> = too_many.iter().map(|v| bagholder_model::input::TileRef { symbol: v["symbol"].as_str().unwrap().to_string(), exchange: v["exchange"].as_str().unwrap().to_string() }).collect();
    assert_eq!(serde_json::to_value(crate::feeds::tiles_set(&app(), &too_many_tiles)).unwrap()["ok"], false);
    let b = app().base().unwrap();
    let syms: Vec<String> = bagholder_model::markets::tile_rows(&b).iter().map(|t| t.symbol.to_string()).collect();
    assert_eq!(syms, vec!["VIX", "GC"], "a refused save changes nothing");
    if saved.is_empty() {
        conn.execute("DELETE FROM meta WHERE key = ?", [bagholder_store::rows::TILES_META]).unwrap();
    } else {
        bagholder_store::tables::set_meta(&conn, bagholder_store::rows::TILES_META, &saved).unwrap();
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
            Listing::new("AAPL", "NEO", "CAD", "Shares"),
            Listing { quote_key: Some("AAPL@NASDAQ".into()), ..Listing::new("AAPL", "NASDAQ", "USD", "Shares") },
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

/// The Rust release names its archive `-rust-<target>`, beside the Python app's `-web.zip`.
#[test]
fn test_release_assets_take_the_web_archive_by_name_and_ignore_the_rest() {
    let rel = |names: &[String]| json!({"tag_name": "v2.0.0", "assets": names.iter().map(|n| json!({"name": n, "browser_download_url": format!("https://x/{}", n)})).collect::<Vec<_>>()});
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let mine = format!("bagholder-v2.0.0-rust-{}.{}", update::target_triple(), ext);
    assert_eq!(update::archive_name("v2.0.0"), mine);
    // the pre-split name of a target archive is not this one's
    let bare = format!("bagholder-v2.0.0-{}.{}", update::target_triple(), ext);
    assert_eq!(update::release_assets(&rel(&[bare.clone(), format!("{}.sha256", bare)])), json!({}));
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
    let rec = update::check_for_update(&app());
    let st = crate::status::status(&app());
    let out = update::start_update(&app());
    std::env::remove_var("BAGHOLDER_NO_UPDATE");
    assert_eq!((rec["ok"].as_bool(), rec["updateAvailable"].as_bool(), rec["latest"].as_str()), (Some(true), Some(true), Some(newer.as_str())));
    assert_eq!((st.update_by.as_str(), json!(st.update_url)), ("image", json!(update::image_page())), "told of the release, sent to the image");
    assert_eq!((out.ok, out.error.as_deref()), (false, Some(update::UPDATES_OFF_MESSAGE)));
    assert_eq!(crate::status::status(&app()).update_by, "app");
}

#[test]
fn test_update_button_refuses_during_a_sync() {
    let _g = guard();
    let _fakes = UpdateFakes::new(None);
    let conn = app_ref().open().unwrap();
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
    let c = app_ref().open().unwrap();
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
    let out = serde_json::to_value(crate::feeds::news_symbol_payload_with(&app(), "QIMC", "", "", &readers, &|_, _, _| None)).unwrap();
    assert_eq!((out["source"].as_str().unwrap(), seen.lock().unwrap().1.clone()), ("tmx", Some("QIMC:CNX".to_string())));
    *seen.lock().unwrap() = (vec![], None);
    // TMX cannot place it: Nasdaq, whose items name the symbols they belong to
    bagholder_store::tables::set_meta(&c, "tmx_form:KO", &format!("none@{}", today)).unwrap();
    let out = serde_json::to_value(crate::feeds::news_symbol_payload_with(&app(), "KO", "", "", &readers, &|_, _, _| None)).unwrap();
    assert_eq!((out["source"].as_str().unwrap(), out["exchange"].as_str().unwrap(), seen.lock().unwrap().1.is_some()), ("nasdaq", "NASDAQ", false));
}

#[test]
fn test_a_searched_ticker_is_read_from_every_source_under_the_name_tmx_gives() {
    let _g = guard();
    let c = app_ref().open().unwrap();
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
    let listing = |_: &Connection, _: &str, _: &str| Some(json!({"symbol": "SXHI", "name": "Ninepoint SpaceX HighShares ETF", "exchange": "TSX", "currency": "CAD"}));
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
    assert!(app().root.join("ledger.html").is_file(), "the root is the repository's");
    assert_eq!(update::cargo_dir(&app()), app().root.join("rust"));
    assert!(update::cargo_dir(&app()).join("Cargo.toml").is_file());
}

/// Every place the code waits on a clock, by file, with why it may. A wait that is
/// not here fails the build: a new one is either replaced by waiting for the thing
/// itself (`events::park_until`, a deadline that is known) or argued for in
/// docs/architecture.md, "Timers that remain", and then counted here.
const TIMED_WAITS: [(&str, usize, &str); 15] = [
    ("market/src/localmodel.rs", 2, "a child process coming up: it has no readiness signal"),
    ("market/src/pace.rs", 1, "a host's request rate (the SEC, fund companies, news feeds, the archive at TMX): a turn taken, waited for with no lock held"),
    ("market/src/pdftext.rs", 1, "a child process with a deadline: std has no wait with one"),
    ("market/src/quotes.rs", 3, "Yahoo's request rate, waited for outside the lock"),
    ("market/src/sedar.rs", 1, "SEDAR+'s request rate on its one session"),
    ("server/src/app.rs", 1, "`wait` itself"),
    ("server/src/events.rs", 7, "`park_until_or` itself, the 40 ms gather, midnight; three in its tests"),
    ("server/src/feeds.rs", 14, "outside sources that offer no push, each only while wanted; known deadlines"),
    ("server/src/http/mod.rs", 1, "the five seconds requests in hand are given to finish when the app stops"),
    ("server/src/login.rs", 8, "the sign-in browser: frames and a DevTools socket, only during a sign-in"),
    ("server/src/notify.rs", 2, "its stream's heartbeat (folded into /api/events in stage 6); a test"),
    ("server/src/orders/brackets.rs", 2, "the bracket engine, parked until a bracket is armed: a stop's cadence; a cancel given its seconds to land"),
    ("server/src/orders/readback.rs", 1, "Wealthsimple offers no order push: read only while an order is live or shown"),
    ("server/src/session.rs", 2, "the portfolio while a page is open; the token and pull deadlines"),
    ("server/src/update.rs", 3, "child processes with a deadline: std has no wait with one"),
];

#[test]
fn test_no_wait_on_a_clock_that_is_not_accounted_for() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let timed = |line: &str| {
        let l = line.trim_start();
        !l.starts_with("//") && [".wait(Duration", "thread::sleep", "park_until_or(", "wait_timeout", "time::sleep", "time::timeout", "time::interval"].iter().any(|p| l.contains(p))
    };
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
    assert_eq!(found, want, "a wait on a clock was added or removed: see docs/architecture.md, \"Timers that remain\"");
}
