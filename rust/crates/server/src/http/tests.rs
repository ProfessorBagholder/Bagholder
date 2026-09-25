//! The server as a client meets it: the gate, the error shapes, the headers, the
//! typed requests, and the event stream. Requests go straight into the router
//! (`oneshot`): no socket, no port.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::{router, AppState};
use crate::tests_common::{app, app_ref};

/// The one app, with its store prepared as the server prepares it at start.
fn guard() -> std::sync::MutexGuard<'static, ()> {
    let g = crate::tests_common::guard();
    bagholder_store::relabel::ensure(&app_ref().open().unwrap()).unwrap();
    g
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

/// A request as the page on this machine makes it.
fn from_the_page(method: Method, uri: &str, body: Option<&str>) -> Request<Body> {
    let host = format!("127.0.0.1:{}", *app().port.lock().unwrap());
    let mut req = Request::builder().method(method).uri(uri).header(header::HOST, host).header("sec-fetch-site", "same-origin");
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    let mut req = req.body(Body::from(body.unwrap_or("").to_string())).unwrap();
    req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
    req
}

async fn send(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = router(AppState { app: app() }).oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    (parts.status, parts.headers, bytes.to_vec())
}

async fn json_of(req: Request<Body>) -> (StatusCode, Value) {
    let (code, _, body) = send(req).await;
    (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[test]
fn test_only_the_page_on_this_machine_gets_in() {
    let _g = guard();
    runtime().block_on(async {
        let ok = from_the_page(Method::GET, "/api/status", None);
        assert_eq!(json_of(ok).await.0, StatusCode::OK);

        let mut elsewhere = from_the_page(Method::GET, "/api/status", None);
        elsewhere.extensions_mut().insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 20], 50000))));
        assert_eq!(json_of(elsewhere).await, (StatusCode::FORBIDDEN, json!({"ok": false})), "a peer that is not this machine");

        let mut rebound = from_the_page(Method::GET, "/api/status", None);
        rebound.headers_mut().insert(header::HOST, "evil.example:8765".parse().unwrap());
        assert_eq!(json_of(rebound).await.0, StatusCode::FORBIDDEN, "a name of someone else's that resolves here");

        let mut forged = from_the_page(Method::POST, "/api/sync", None);
        forged.headers_mut().remove("sec-fetch-site");
        assert_eq!(json_of(forged).await.0, StatusCode::FORBIDDEN, "a write that does not come from the page");

        let mut scripted = from_the_page(Method::POST, "/api/events/watch", Some("{}"));
        scripted.headers_mut().remove("sec-fetch-site");
        scripted.headers_mut().insert("x-bagholder", "1".parse().unwrap());
        assert_eq!(json_of(scripted).await.0, StatusCode::OK, "a write with the app's own header");

        assert_eq!(json_of(from_the_page(Method::OPTIONS, "/api/status", None)).await.0, StatusCode::FORBIDDEN);
        assert_eq!(json_of(from_the_page(Method::PUT, "/api/status", None)).await.0, StatusCode::NOT_IMPLEMENTED);
    });
}

#[test]
fn test_every_answer_is_private_and_every_failure_has_one_shape() {
    let _g = guard();
    runtime().block_on(async {
        let (code, headers, _) = send(from_the_page(Method::GET, "/api/status", None)).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

        assert_eq!(json_of(from_the_page(Method::GET, "/api/nothing", None)).await, (StatusCode::NOT_FOUND, json!({"ok": false, "error": "not found"})));
        assert_eq!(json_of(from_the_page(Method::GET, "/api/trade?id=none-such", None)).await, (StatusCode::NOT_FOUND, json!({"ok": false, "error": "no such trade"})));
        assert_eq!(json_of(from_the_page(Method::GET, "/api/filings?symbol=%20", None)).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "symbol required"})));
        assert_eq!(json_of(from_the_page(Method::GET, "/api/filings/doc?symbol=QNC", None)).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "symbol and id required"})));
        assert_eq!(json_of(from_the_page(Method::POST, "/api/journal", Some(r#"{"thesis":"x"}"#))).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "id required"})));

        let (code, body) = json_of(from_the_page(Method::POST, "/api/journal", Some("{not json"))).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], json!(false));
        let (code, body) = json_of(from_the_page(Method::POST, "/api/notifications/read", Some(r#"{"ids":"all"}"#))).await;
        assert_eq!((code, &body["ok"]), (StatusCode::BAD_REQUEST, &json!(false)), "a field of the wrong type is refused, not ignored");
    });
}

#[test]
fn test_a_write_with_no_body_is_a_write_with_nothing_to_say() {
    let _g = guard();
    runtime().block_on(async {
        let (code, body) = json_of(from_the_page(Method::POST, "/api/notifications/read", None)).await;
        assert_eq!((code, &body["ok"]), (StatusCode::OK, &json!(true)));
        let (code, body) = json_of(from_the_page(Method::POST, "/api/journal", Some(r#"{"id":"t1","grade":"A","tags":["x"],"thesis":"why"}"#))).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["journal"]["t1"]["grade"], json!("A"));
    });
}

#[test]
fn test_hashed_files_are_kept_for_good_and_the_page_is_always_asked_for_again() {
    let _g = guard();
    runtime().block_on(async {
        let (code, _, _) = send(from_the_page(Method::GET, "/assets/../../Cargo.toml", None)).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "nothing outside the page's own folder");
        let (code, headers, _) = send(from_the_page(Method::GET, "/", None)).await;
        assert_eq!(code, StatusCode::OK, "the built page, or the legacy one where it has not been built");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
        assert!(headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
        // a hashed file, where the page has been built
        let dist = app().root.join("web/dist/assets");
        if let Some(name) = std::fs::read_dir(dist).ok().and_then(|mut d| d.next()).and_then(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()) {
            let (code, headers, _) = send(from_the_page(Method::GET, &format!("/assets/{}", name), None)).await;
            assert_eq!(code, StatusCode::OK);
            assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        }
    });
}

/// The frames of an event stream, as (event, data).
fn frames(text: &str) -> Vec<(String, Value)> {
    text.split("\n\n")
        .filter_map(|frame| {
            let event = frame.lines().find_map(|l| l.strip_prefix("event: "))?;
            let data = frame.lines().find_map(|l| l.strip_prefix("data: "))?;
            Some((event.to_string(), serde_json::from_str(data).ok()?))
        })
        .collect()
}

#[test]
fn test_the_stream_says_hello_sends_the_view_once_and_then_only_what_changed() {
    let _g = guard();
    runtime().block_on(async {
        assert_eq!(app().events.watchers(), 0);
        let res = router(AppState { app: app() }).oneshot(from_the_page(Method::GET, "/api/events", None)).await.unwrap();
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
        let mut body = res.into_body().into_data_stream();
        let mut text = String::new();
        let read = |text: &mut String, chunk: Option<Result<axum::body::Bytes, axum::Error>>| text.push_str(&String::from_utf8_lossy(&chunk.unwrap().unwrap()));
        while frames(&text).len() < 2 {
            read(&mut text, tokio::time::timeout(Duration::from_secs(10), body.next()).await.expect("the stream opens at once"));
        }
        let got = frames(&text);
        assert_eq!(got[0].0, "hello");
        assert!(got[0].1["id"].as_u64().unwrap() > 0);
        assert_eq!((got[1].0.as_str(), &got[1].1["doc"]), ("snapshot", &json!("model")));
        assert_eq!(got[1].1["data"]["status"]["ok"], json!(true), "the header's status rides with the view");
        assert_eq!(app().events.watchers(), 1, "someone is looking");

        // something the header shows changes: the page is sent that field, as a
        // patch -- never the view again
        app().state.lock().unwrap().sync_step = "Reading activity".into();
        let mut text = String::new();
        let told = |text: &str| frames(text).iter().any(|(_, data)| data["ops"].as_array().is_some_and(|ops| ops.contains(&json!(["set", ["status", "syncStep"], "Reading activity"]))));
        while !told(&text) {
            read(&mut text, tokio::time::timeout(Duration::from_secs(10), body.next()).await.expect("a change is told"));
        }
        assert!(frames(&text).iter().all(|(event, _)| event == "patch"), "after the first view, only changes");
        app().state.lock().unwrap().sync_step.clear();

        drop(body);
        for _ in 0..100 {
            if app().events.watchers() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(app().events.watchers(), 0, "the page went: nobody is looking");
    });
}

#[test]
fn test_a_new_notification_reaches_the_bell_as_one_row_inserted() {
    let _g = guard();
    std::env::set_var(crate::notify::MODE_ENV, "browser"); // never the system's own notifications from a test
    let conn = app_ref().open().unwrap();
    crate::notify::set_settings(&conn, &serde_json::from_value(json!({"fills": true})).unwrap()).unwrap();
    bagholder_store::feeds::clear_notifications(&conn).unwrap();
    let mut feed = crate::events::Feed::open(app(), None);
    assert!(app().events.watch(&app(), feed.id(), [("notifications".to_string(), json!({}))].into_iter().collect()));
    let first = feed.step(&crate::status::status);
    let bell = first.iter().find(|(_, data)| data["doc"] == "notifications").expect("the bell arrives whole once");
    assert_eq!((bell.0, &bell.1["data"]), ("snapshot", &json!({"rows": [], "unread": 0})));

    let row = crate::notify::emit(&app(), &conn, "fills", "order:9:filled", "Order filled · QNC", "Bought 5 at 1.75", None).expect("fills are on");
    let next = feed.step(&crate::status::status);
    let change = next.iter().find(|(_, data)| data["doc"] == "notifications").expect("the bell is told");
    assert_eq!(change.0, "patch");
    let ops = change.1["ops"].as_array().unwrap();
    assert!(ops.contains(&json!(["set", ["unread"], 1])), "{:?}", ops);
    let inserted = ops.iter().find(|op| op[0] == "rows").expect("a row inserted, not the list again");
    let added = inserted[4].as_object().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added.values().next().unwrap()["title"], json!(row.title));
}

/// `GET /api/book` answers exactly what `bagholder_store::book::book` builds,
/// as `Book`'s own JSON -- the route does nothing to it besides serializing it.
#[test]
fn test_the_book_route_is_the_store_books_own_json() {
    let _g = guard();
    let conn = app_ref().open().unwrap();
    let expected = serde_json::to_value(bagholder_store::book::book(&conn).unwrap()).unwrap();
    let (code, body) = runtime().block_on(json_of(from_the_page(Method::GET, "/api/book", None)));
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body, expected);
}
