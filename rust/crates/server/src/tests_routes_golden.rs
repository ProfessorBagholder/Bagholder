//! Golden (characterisation) test for the HTTP routes ahead of the
//! `Value` -> typed-answer conversion (stage 5d7d): the exact JSON answer
//! for one representative request per route, pinned before any route is
//! converted so a change in shape -- not just a change in the Rust type
//! that builds it -- is caught.
//!
//! On an app and a store of its own, not `tests_common`'s shared one: every
//! other test in this binary shares one app and one store (serialized by
//! `tests_common::guard`), and several insert accounts, securities or
//! activities that would otherwise show up in `/api/book` and `/api/data`
//! here, in whatever order the suite happens to run. `tests_common::guard`
//! is still taken, only to serialize the process-wide environment
//! variables this test sets (`BAGHOLDER_DRY_ORDERS`, `notify::MODE_ENV`)
//! against the shared app's own tests setting them.
//!
//! Only routes whose one representative request neither reaches the
//! network nor opens a real socket are exercised here: the local reads and
//! writes of `http::model` and `http::notifications`, and the two
//! `http::markets` lookups whose empty input answers before any request
//! goes out. `http::orders`, `http::session` and the rest of
//! `http::markets` reach Wealthsimple or a market-data provider even to
//! build a "not connected" answer in some branches, or kick a background
//! refresh; they stay untyped until stage 5d7d has a way to fake those
//! upstreams for a test.
//!
//! After an intended change to one of these routes' answer shape:
//! `BAGHOLDER_BLESS=1 cargo test -p bagholder-server tests_routes_golden`,
//! then read the diff in `tests/golden/routes.json` before committing it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bagholder_store::orders as so;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request};
use serde_json::{json, Map, Value};
use tower::ServiceExt;

use crate::app::App;
use crate::http::{router, AppState};

/// An app on a temp home of its own -- nothing another test wrote is in it,
/// and nothing this test writes is seen by another.
fn isolated() -> Arc<App> {
    let dir = std::env::temp_dir().join(format!("bh-routes-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let app = App::new(dir, root, "127.0.0.1".into());
    bagholder_store::relabel::ensure(&app.open().unwrap()).unwrap();
    // the figure path, on the recorded month, as the shared test app has it
    let book = app.home.join("figures");
    std::fs::create_dir_all(&book).unwrap();
    crate::tests_common::pulled_book(&book);
    let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
    let f = crate::figures::Figures::open(&book, now).unwrap();
    f.state_zone("America/Toronto", now).unwrap();
    let _ = app.figures.set(f);
    app
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

fn from_the_page(app: &Arc<App>, method: Method, uri: &str, body: Option<Value>) -> Request<Body> {
    let host = format!("127.0.0.1:{}", *app.port.lock().unwrap());
    let mut req = Request::builder().method(method).uri(uri).header(header::HOST, host).header("sec-fetch-site", "same-origin");
    let text = body.map(|b| b.to_string());
    if text.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    let mut req = req.body(Body::from(text.unwrap_or_default())).unwrap();
    req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
    req
}

async fn json_of(app: Arc<App>, req: Request<Body>) -> Value {
    let res = router(AppState { app }).oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    json!({"status": status, "body": body})
}

/// One route's name, and the request that answers it without touching the
/// network or opening a socket.
fn cases(app: &Arc<App>) -> Vec<(&'static str, Request<Body>)> {
    let req = |method: Method, uri: &str, body: Option<Value>| from_the_page(app, method, uri, body);
    vec![
        ("status", req(Method::GET, "/api/status", None)),
        ("journal", req(Method::POST, "/api/journal", Some(json!({"id": "golden-journal", "grade": "B", "tags": ["golden"], "thesis": "golden fixture"})))),
        ("watch_status", req(Method::GET, "/api/watch", None)),
        ("watch_set_no_path", req(Method::POST, "/api/watch", Some(json!({"path": "", "account": ""})))),
        ("watch_scan_no_folder", req(Method::POST, "/api/watch/scan", None)),
        ("watch_clear", req(Method::POST, "/api/watch/clear", None)),
        ("import_no_text", req(Method::POST, "/api/import", Some(json!({"name": "a.csv", "text": "", "account": ""})))),
        ("notifications_list", req(Method::GET, "/api/notifications", None)),
        ("notifications_settings", req(Method::POST, "/api/notifications/settings", Some(json!({"fills": true})))),
        ("notifications_test", req(Method::POST, "/api/notifications/test", None)),
        ("notifications_read", req(Method::POST, "/api/notifications/read", Some(json!({"ids": [999_999_999]})))),
        ("notifications_seen", req(Method::POST, "/api/notifications/seen", Some(json!({"ids": [999_999_999]})))),
        ("symbols_search_empty", req(Method::GET, "/api/symbols/search?q=", None)),
        ("symbols_quote_empty", req(Method::GET, "/api/symbols/quote", None)),
        ("listing_missing_symbol", req(Method::GET, "/api/listing", None)),
        ("news_symbol_missing", req(Method::GET, "/api/news/symbol", None)),
        ("watchlist_add_missing_symbol", req(Method::POST, "/api/watchlist/add", Some(json!({})))),
        ("watchlist_remove_missing_symbol", req(Method::POST, "/api/watchlist/remove", Some(json!({})))),
        ("tiles_set_empty", req(Method::POST, "/api/tiles/set", Some(json!({"tiles": []})))),
        ("filings_missing_symbol", req(Method::GET, "/api/filings?symbol=%20", None)),
        ("filings_no_documents", req(Method::GET, "/api/filings?symbol=GOLDEN", None)),
        ("filings_feed_empty", req(Method::GET, "/api/filings/feed", None)),
        ("filings_enrich_no_such_doc", req(Method::GET, "/api/filings/enrich?symbol=GOLDEN&id=none", None)),
        ("fear_empty", req(Method::GET, "/api/fear", None)),
        ("shorts_missing_symbol", req(Method::GET, "/api/shorts", None)),
        ("shorts_feed_empty", req(Method::GET, "/api/shorts/feed", None)),
        ("history_missing_params", req(Method::GET, "/api/history", None)),
        // `login_start` is never called here: it launches a real browser.
        ("login_cancel", req(Method::POST, "/api/login/cancel", None)),
        ("login_input_no_window", req(Method::POST, "/api/login/input", Some(json!({"kind": "click", "x": 1, "y": 2})))),
        ("disconnect", req(Method::POST, "/api/disconnect", None)),
        ("update_offline", req(Method::POST, "/api/update", None)),
        ("refresh_no_session", req(Method::POST, "/api/refresh", None)),
        ("sync_no_session", req(Method::POST, "/api/sync", None)),
        ("capture_offline", req(Method::POST, "/api/capture", Some(json!({"access_token": "tok", "refresh_token": "r", "client_id": bagholder_ws::standin::FAKE_CLIENT_ID})))),
    ]
}

/// A saved login, refreshable through the stand-in Wealthsimple server
/// (`bagholder_ws::standin`) rather than the real one -- the interesting
/// branch of `/api/refresh`, which a missing session or an offline network
/// never reaches.
fn seed_refreshable_session(app: &Arc<App>) {
    let sess = bagholder_ws::session::Session {
        access_token: "old-access".into(),
        refresh_token: "old-refresh".into(),
        client_id: bagholder_ws::standin::FAKE_CLIENT_ID.into(),
        ids: bagholder_ws::session::IdentityKeys { identity_canonical_id: "identity-1".into(), ..Default::default() },
        ..Default::default()
    };
    crate::session::save_session(app, &sess).unwrap();
}

/// Blanks the store's own path (this test's temp folder) and the app's own
/// start time (now, every run).
/// Blanks any nested `startedAt` or `fetchedAt` key, wherever it sits: both
/// are `now` on every run, never a route's own data.
fn scrub_timestamps(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for key in ["startedAt", "fetchedAt", "today"] {
                if let Some(x) = m.get_mut(key) {
                    if x.is_string() {
                        *x = json!(format!("<{}>", key));
                    }
                }
            }
            // the versions end with the day they were computed on
            for key in ["coreVersion", "dataVersion"] {
                if let Some(Value::String(s)) = m.get_mut(key) {
                    if let Some((head, _)) = s.rsplit_once('|') {
                        *s = format!("{head}|<today>");
                    }
                }
            }
            for x in m.values_mut() {
                scrub_timestamps(x);
            }
        }
        Value::Array(a) => {
            for x in a {
                scrub_timestamps(x);
            }
        }
        _ => {}
    }
}

/// Blanks the store's own path (this test's temp folder), at the top level
/// only: `path` is meaningful data on some routes (`WatchStatus`, `ScanReport`).
fn scrub(mut v: Value) -> Value {
    if let Some(b) = v.get_mut("body") {
        if let Some(p) = b.get_mut("path") {
            if p.is_string() {
                *p = json!("<db-path>");
            }
        }
        scrub_timestamps(b);
    }
    v
}

#[test]
fn test_routes_golden() {
    let _g = crate::tests_common::guard();
    std::env::set_var("BAGHOLDER_DRY_ORDERS", "1");
    // "browser" keeps `notifications_test` from reaching this machine's own notifications
    std::env::set_var(crate::notify::MODE_ENV, "browser");
    let app = isolated();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/routes.json");
    let got: Map<String, Value> = runtime().block_on(async {
        let mut m = Map::new();
        for (name, req) in cases(&app) {
            m.insert(name.to_string(), scrub(json_of(app.clone(), req).await));
        }
        m
    });
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(got)).unwrap() + "\n").unwrap();
        return;
    }
    let want: Map<String, Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or_default();
    let mut names: Vec<&String> = got.keys().chain(want.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        assert_eq!(got.get(name), want.get(name), "route {} answers differently than the golden pins", name);
    }
}

/// `POST /api/refresh`'s connected branch: the refresh goes through the one
/// session (`bagholder_wealthsimple::session`), here on a network that answers
/// Wealthsimple's token endpoint.
#[test]
fn test_refresh_route_reaches_a_connected_answer() {
    let _g = crate::tests_common::guard();
    std::env::set_var("BAGHOLDER_DRY_ORDERS", "1");
    struct Token(std::sync::Mutex<Vec<String>>);
    struct Shared(std::sync::Arc<Token>);
    impl bagholder_net::Transport for Shared {
        fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
            self.0 .0.lock().unwrap().push(ask.url.to_string());
            assert_eq!(ask.url, bagholder_wealthsimple::session::TOKEN_URL, "only the token endpoint is asked");
            Ok((200, ask.url.to_string(), vec![], br#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#.to_vec()))
        }
    }
    let asked = std::sync::Arc::new(Token(std::sync::Mutex::new(vec![])));
    let dir = std::env::temp_dir().join(format!("bh-routes-refresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let net = bagholder_net::Net::answered_by(std::sync::Arc::new(bagholder_net::SystemClock), std::sync::Arc::new(bagholder_net::Limiter::new()), Box::new(Shared(asked.clone())));
    let app = App::with_net(dir, PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net);
    bagholder_store::relabel::ensure(&app.open().unwrap()).unwrap();
    seed_refreshable_session(&app);
    let req = from_the_page(&app, Method::POST, "/api/refresh", None);
    let got = runtime().block_on(json_of(app.clone(), req));
    assert_eq!(got, json!({"status": 200, "body": {"ok": true, "error": "", "connected": true}}));
    assert_eq!(asked.0.lock().unwrap().len(), 1);
    assert_eq!(crate::session::load_session(&app).unwrap().refresh_token, "new-refresh");
}

/// `http::orders`'s routes, on the shared app the way `tests_orders.rs`
/// drives the functions behind them: `orders::seam` fakes Wealthsimple, so
/// nothing here reaches the network either, and every case that needs a
/// clean store wipes it first (as `tests_orders.rs`'s own `setup()` does --
/// this crate's server tests share one app and one store, so a test that
/// leaves rows behind would be read by whichever test runs next).
#[test]
fn test_orders_routes_golden() {
    use crate::orders::seam;
    let _g = crate::tests_common::guard();
    seam::reset();
    let app = crate::tests_common::app();
    let conn = app.open().unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    for t in ["orders", "brackets", "activities", "accounts", "securities"] {
        let _ = conn.execute(&format!("DELETE FROM \"{}\"", t), []);
    }
    *app.orders.refreshed_at.lock().unwrap() = String::new();
    app.state.lock().unwrap().connected = false;
    drop(conn);

    let req = |method: Method, uri: &str, body: Option<Value>| from_the_page(&app, method, uri, body);
    let get = |req: Request<Body>| runtime().block_on(json_of(app.clone(), req));

    // no session at all: the deterministic early branch of every mutation
    *seam::SESSION.lock().unwrap() = Some(None);
    assert_eq!(get(req(Method::GET, "/api/orders", None)), json!({"status": 200, "body": {"ok": true, "orders": [], "brackets": [], "live": false, "refreshedAt": ""}}));
    // `orders_payload`'s own `ok` (always true) is the one the page reads:
    // the flatten of the two documents keeps whichever field the second
    // (`OrdersDoc`) carries, exactly as the untyped `Map::extend` did
    assert_eq!(
        get(req(Method::POST, "/api/orders/refresh", None)),
        json!({"status": 200, "body": {"ok": true, "skipped": "no session", "orders": [], "brackets": [], "live": false, "refreshedAt": ""}})
    );
    assert_eq!(get(req(Method::POST, "/api/order/cancel", Some(json!({"id": "golden-no-such-order"})))), json!({"status": 200, "body": {"ok": false, "error": "No such order."}}));
    assert_eq!(get(req(Method::POST, "/api/order/modify", Some(json!({"id": "golden-no-such-order"})))), json!({"status": 200, "body": {"ok": false, "error": "No such order."}}));
    assert_eq!(get(req(Method::POST, "/api/bracket/adjust", Some(json!({"id": "golden-no-such-bracket", "leg": "sl"})))), json!({"status": 200, "body": {"ok": false, "error": "No such bracket."}}));
    assert_eq!(get(req(Method::POST, "/api/bracket/cancel", Some(json!({"id": "golden-no-such-bracket"})))), json!({"status": 200, "body": {"ok": false, "error": "No such bracket."}}));

    // a resting order, cancelled through the stand-in for Wealthsimple's cancel mutation
    let golden_order: so::Order = serde_json::from_value(json!({"id": "golden-order-1", "accountId": "acct-golden", "account": "Golden", "securityId": "sec-golden", "symbol": "GOLDEN", "currency": "USD", "side": "BUY", "type": "LIMIT", "quantity": 5, "limitPrice": 1.75, "tif": "DAY", "status": "sent", "source": "bagholder", "role": "entry"})).unwrap();
    so::typed::insert_order(&app.open().unwrap(), &golden_order, &crate::app::now_iso()).unwrap();
    *seam::SESSION.lock().unwrap() = Some(Some(bagholder_ws::session::Session { access_token: "tok".into(), ..Default::default() }));
    *seam::LIVE.lock().unwrap() = Some(true);
    seam::SPAWN_INLINE.store(true, Ordering::SeqCst);
    *seam::GQL.lock().unwrap() = Some(std::sync::Arc::new(|op: &str, _vars: &Value| match op {
        "SoOrdersOrderCancel" => Ok(json!({"orderServiceCancelOrder": {"externalId": "golden-order-1", "errors": []}})),
        "FetchSoOrdersExtendedOrder" => Ok(json!({"soOrdersExtendedOrder": {"status": "CANCELLED"}})),
        other => panic!("unexpected op {}", other),
    }));
    let cancelled = get(req(Method::POST, "/api/order/cancel", Some(json!({"id": "golden-order-1"}))));
    assert_eq!(cancelled, json!({"status": 200, "body": {"ok": true, "id": "golden-order-1", "status": "cancelling"}}));
    seam::reset();
}
