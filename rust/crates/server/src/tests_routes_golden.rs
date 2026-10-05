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
use std::sync::Arc;

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
    // the figure path, on the recorded month, as the shared test app has it
    let book = app.home.join("figures");
    std::fs::create_dir_all(&book).unwrap();
    crate::tests_common::pulled_book(&book);
    let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
    let f = crate::figures::Figures::open(&book, now).unwrap();
    f.state_zone("America/Toronto", now).unwrap();
    app.set_figures(f);
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
    seed_refreshable_session(&app);
    let req = from_the_page(&app, Method::POST, "/api/refresh", None);
    let got = runtime().block_on(json_of(app.clone(), req));
    assert_eq!(got, json!({"status": 200, "body": {"ok": true, "error": "", "connected": true}}));
    assert_eq!(asked.0.lock().unwrap().len(), 1);
    assert_eq!(crate::session::load_session(&app).unwrap().unwrap().refresh_token, "new-refresh");
}

/// `http::orders`'s routes, on an app of their own with orders live against the fake
/// Wealthsimple of `tests_execution`: nothing here reaches the network.
#[test]
fn test_orders_routes_golden() {
    use crate::tests_execution::{fresh, order};
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    let req = |method: Method, uri: &str, body: Option<Value>| from_the_page(&app, method, uri, body);
    let get = |req: Request<Body>| runtime().block_on(json_of(app.clone(), req));

    // no session: nothing to read with
    let doc = get(req(Method::GET, "/api/orders", None));
    assert_eq!(doc, json!({"status": 200, "body": {"ok": true, "live": false, "refreshedAt": null, "orders": [], "brackets": [], "error": null}}));
    assert_eq!(get(req(Method::POST, "/api/orders/refresh", None))["body"]["read"], json!({"ok": false, "skipped": "no session", "read": 0, "failed": 0}));
    assert_eq!(get(req(Method::POST, "/api/order/cancel", Some(json!({"id": "golden-no-such-order"}))))["body"], json!({"ok": false, "error": "No such order."}));
    assert_eq!(get(req(Method::POST, "/api/order/modify", Some(json!({"id": "golden-no-such-order"}))))["body"], json!({"ok": false, "error": "No such order."}));
    assert_eq!(get(req(Method::POST, "/api/bracket/adjust", Some(json!({"id": "golden-no-such-bracket", "leg": "sl"}))))["body"], json!({"ok": false, "error": "No such bracket."}));
    assert_eq!(get(req(Method::POST, "/api/bracket/cancel", Some(json!({"id": "golden-no-such-bracket"}))))["body"], json!({"ok": false, "error": "No such bracket."}));
    assert_eq!(get(req(Method::POST, "/api/order/modify", Some(json!({"id": "x", "quantity": "five"}))))["body"], json!({"ok": false, "error": "No such order."}));

    // a resting order, cancelled through the gate
    app.set_orders_live(true);
    *app.orders.seam.session.lock().unwrap() = Some(bagholder_ws::session::Session { access_token: "tok".into(), ..Default::default() });
    let book = app.figures.get().unwrap().book().unwrap();
    crate::orders::gate::place(&app, &book, &order("golden-order-1", None), &bagholder_core::order::Asker::Person, bagholder_core::jiff::Timestamp::now()).unwrap().unwrap();
    let cancelled = get(req(Method::POST, "/api/order/cancel", Some(json!({"id": "golden-order-1"}))));
    assert_eq!(cancelled, json!({"status": 200, "body": {"ok": true, "id": "golden-order-1", "status": "cancelling"}}));
    assert_eq!(fake.0.lock().unwrap().cancels, vec!["golden-order-1".to_string()]);
    let asked: Vec<String> = book.order_log("golden-order-1").unwrap().into_iter().map(|l| format!("{} {}", l.asker.to_text(), l.event.kind())).collect();
    assert_eq!(asked, ["person written", "person accepted", "person cancel-asked"]);
}

/// Body limits per route, and one import at a time with its slot taken before its
/// body is read (`docs/plans/stage-money.md`, part D; brief 19, change 6).
#[test]
fn test_a_second_import_is_refused_before_its_body_is_read_and_each_route_has_its_own_body_limit() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _g = crate::tests_common::guard();
    let app = crate::tests_common::app();
    let rt = runtime();
    // an import while one runs: refused, and its body never read
    let read = Arc::new(AtomicBool::new(false));
    let r2 = read.clone();
    let body = Body::from_stream(futures_util::stream::once(async move {
        r2.store(true, Ordering::SeqCst);
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(br#"{"name":"a.csv","text":"x","account":""}"#))
    }));
    crate::http::model::IMPORTING.store(true, Ordering::SeqCst);
    let mut req = from_the_page(&app, Method::POST, "/api/import", None);
    *req.body_mut() = body;
    req.headers_mut().insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    let got = rt.block_on(json_of(app.clone(), req));
    crate::http::model::IMPORTING.store(false, Ordering::SeqCst);
    assert_eq!(got, json!({"status": 409, "body": {"ok": false, "error": crate::http::model::IMPORT_BUSY}}));
    assert!(!read.load(Ordering::SeqCst), "the refused import's body was never read");
    // a body over a JSON route's limit is refused; the import takes a file's worth
    let big = "x".repeat(2 * 1024 * 1024);
    let got = rt.block_on(json_of(app.clone(), from_the_page(&app, Method::POST, "/api/journal", Some(json!({"key": "k", "note": big.clone()})))));
    assert_eq!(got["status"], json!(413), "{got}");
    let got = rt.block_on(json_of(app.clone(), from_the_page(&app, Method::POST, "/api/import", Some(json!({"name": "a.csv", "text": big, "account": ""})))));
    assert!(!got.to_string().contains("length limit"), "an import of two megabytes is read: {got}");
    // past the import's own limit: told in words, as too large
    let huge = "x".repeat(crate::http::IMPORT_BODY_LIMIT + 1);
    let got = rt.block_on(json_of(app.clone(), from_the_page(&app, Method::POST, "/api/import", Some(json!({"name": "a.csv", "text": huge, "account": ""})))));
    assert_eq!(got, json!({"status": 413, "body": {"ok": false, "error": "The request is larger than this action takes."}}));
    assert!(!crate::http::model::IMPORTING.load(Ordering::SeqCst), "the slot is given back once the answer has gone");
}

/// Every write to the book reaches the stream: the bus is told after each one has
/// landed, by the book's own commits (`docs/plans/stage-money.md`, part E).
#[test]
fn test_every_write_to_the_book_reaches_the_stream() {
    let _g = crate::tests_common::guard();
    let app = crate::tests_common::app();
    let rt = runtime();
    let call = |method: Method, uri: &str, body: Option<Value>| rt.block_on(json_of(app.clone(), from_the_page(&app, method, uri, body)));
    let trades = call(Method::GET, "/api/figures/trades", None);
    let trade = trades["body"]["trades"].as_array().and_then(|t| t.first()).and_then(|t| t["id"].as_str()).unwrap_or_else(|| panic!("a trade in the made-up book: {trades}")).to_string();
    let stamp = || app.events.stamp()[crate::events::Source::Store as usize];
    let watched = std::env::temp_dir().join(format!("bh-watched-{}", std::process::id()));
    std::fs::create_dir_all(&watched).unwrap();
    let writes: Vec<(&str, Method, &str, Option<Value>)> = vec![
        ("a journal note and grade", Method::POST, "/api/journal", Some(json!({"id": trade, "thesis": "a note", "grade": "B", "tags": ["setup"]}))),
        ("a trade typed in", Method::POST, "/api/entries", Some(json!({"entry": "trade", "account": "", "instrument": null, "symbol": "ZZWRITE", "currency": "CAD", "day": "2026-09-01", "side": "BUY", "quantity": "10", "price": "1.50", "fee": "0"}))),
        ("an import", Method::POST, "/api/import", Some(json!({"name": "a.csv", "account": "", "text": "transaction_date,activity_type,activity_sub_type,account_id,symbol,currency,quantity,unit_price,net_cash_amount\n2026-09-02,Trade,BUY,,ZZIMPORT,CAD,5,2.00,-10\n"}))),
        ("the tiles chosen", Method::POST, "/api/tiles/set", Some(json!({"tiles": []}))),
        ("the notification settings", Method::POST, "/api/notifications/settings", Some(json!({"fills": true}))),
        ("a folder watched", Method::POST, "/api/watch", Some(json!({"path": watched.to_string_lossy(), "account": ""}))),
        ("the watched folder cleared", Method::POST, "/api/watch/clear", None),
    ];
    for (what, method, uri, body) in writes {
        let before = stamp();
        let got = call(method, uri, body);
        assert_eq!(got["status"], json!(200), "{what}: {got}");
        assert!(stamp() > before, "{what} reached no stream: {got}");
    }
}
