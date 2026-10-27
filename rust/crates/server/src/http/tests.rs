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

/// The one app.
fn guard() -> std::sync::MutexGuard<'static, ()> {
    crate::tests_common::guard()
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

/// Names listed for a proxy on this machine (`BAGHOLDER_HOSTS`), emptied again
/// whatever the test does.
struct Listed;

impl Listed {
    fn names(names: &[&str]) -> Listed {
        *app().hosts.lock().unwrap() = names.iter().map(|n| n.to_string()).collect();
        Listed
    }
}

impl Drop for Listed {
    fn drop(&mut self) {
        app().hosts.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// A request as the page makes it through a proxy on this machine that names it
/// `host`, from `peer`.
fn through(method: Method, uri: &str, body: Option<&str>, host: &str, peer: [u8; 4]) -> Request<Body> {
    let mut req = from_the_page(method, uri, body);
    req.headers_mut().insert(header::HOST, host.parse().unwrap());
    req.extensions_mut().insert(ConnectInfo(SocketAddr::from((peer, 50000))));
    req
}

/// A name listed for a proxy in front is answered as 127.0.0.1 and the port are,
/// from this machine alone, exactly as listed; nothing listed, nothing more is.
#[test]
fn test_a_name_listed_for_a_proxy_is_answered_from_this_machine_alone() {
    let _g = guard();
    let local = [127, 0, 0, 1];
    runtime().block_on(async {
        // nothing listed: the proxy's name is refused, as any other
        assert_eq!(json_of(through(Method::GET, "/api/status", None, "dev.example.com", local)).await.0, StatusCode::FORBIDDEN);
        let _listed = Listed::names(&["dev.example.com"]);
        // listed, from this machine: a read, and a write from the page
        assert_eq!(json_of(through(Method::GET, "/api/status", None, "dev.example.com", local)).await.0, StatusCode::OK);
        assert_eq!(json_of(through(Method::POST, "/api/events/watch", Some("{}"), "dev.example.com", local)).await.0, StatusCode::OK);
        // the Host header is folded before the check
        assert_eq!(json_of(through(Method::GET, "/api/status", None, "DEV.example.com", local)).await.0, StatusCode::OK);
        // the same name from a peer that is not this machine
        assert_eq!(json_of(through(Method::GET, "/api/status", None, "dev.example.com", [192, 168, 1, 20])).await.0, StatusCode::FORBIDDEN);
        // near misses: a port the list does not name, a longer name ending in the listed one, another name
        for host in ["dev.example.com:443", "evil-dev.example.com", "other.example.com"] {
            assert_eq!(json_of(through(Method::GET, "/api/status", None, host, local)).await.0, StatusCode::FORBIDDEN, "{host}");
        }
        // 127.0.0.1 and the port as ever
        assert_eq!(json_of(from_the_page(Method::GET, "/api/status", None)).await.0, StatusCode::OK);
    });
}

/// The agents' public name is never answered by the page, even listed for a proxy:
/// the page's Allow and Deny cannot be reached through the agents' tunnel.
#[test]
fn test_the_agents_name_is_never_the_page_s_even_listed() {
    let _g = guard();
    let local = [127, 0, 0, 1];
    runtime().block_on(async {
        let _listed = Listed::names(&["agents.example.com"]);
        assert_eq!(json_of(through(Method::GET, "/api/status", None, "agents.example.com", local)).await.0, StatusCode::OK, "listed, and no agents' address yet");
        assert_eq!(json_of(from_the_page(Method::POST, "/api/agents/switch", Some(r#"{"on":true}"#))).await.0, StatusCode::OK);
        assert_eq!(json_of(from_the_page(Method::POST, "/api/agents/tunnel", Some(r#"{"provider":"own","name":"agents.example.com"}"#))).await.0, StatusCode::OK);
        assert_eq!(json_of(through(Method::GET, "/api/status", None, "agents.example.com", local)).await.0, StatusCode::FORBIDDEN, "the agents' name, though listed");
        assert_eq!(json_of(from_the_page(Method::GET, "/api/status", None)).await.0, StatusCode::OK);
        assert_eq!(json_of(from_the_page(Method::POST, "/api/data/clear", Some(r#"{"kinds":["agents"]}"#))).await.0, StatusCode::OK);
    });
}

/// A public name that is this machine (an address, `localhost`) is refused as the
/// agents' name or as the Host a proxy sends, and the page stays reachable: taken,
/// the page's own gate would refuse the page (brief 80 §4.1).
#[test]
fn test_an_agents_name_that_is_this_machine_is_refused_and_the_page_stays_reachable() {
    let _g = guard();
    let port = *app().port.lock().unwrap();
    runtime().block_on(async {
        assert_eq!(json_of(from_the_page(Method::POST, "/api/agents/switch", Some(r#"{"on":true}"#))).await.0, StatusCode::OK);
        for name in [format!("127.0.0.1:{port}"), "localhost".into(), "app.localhost".into(), "[::1]".into(), "10.0.0.1".into()] {
            let (_, said) = json_of(from_the_page(Method::POST, "/api/agents/tunnel", Some(&serde_json::json!({ "provider": "own", "name": name }).to_string()))).await;
            assert_eq!(said["ok"], false, "{name}: {said}");
            let (_, said) = json_of(from_the_page(Method::POST, "/api/agents/tunnel", Some(&serde_json::json!({ "provider": "own", "name": "agents.example.com", "sendsHost": name }).to_string()))).await;
            assert_eq!(said["ok"], false, "{name} as the Host sent: {said}");
            assert_eq!(json_of(from_the_page(Method::GET, "/api/status", None)).await.0, StatusCode::OK);
        }
        assert_eq!(json_of(from_the_page(Method::POST, "/api/data/clear", Some(r#"{"kinds":["agents"]}"#))).await.0, StatusCode::OK);
    });
}

/// `BAGHOLDER_HOSTS` read: names lowercased, a port kept; an entry with a space,
/// an empty one, or one that is no host name refuses the start, naming it.
#[test]
fn test_the_names_listed_are_read_exactly_or_the_start_is_refused() {
    use crate::app::parse_hosts;
    assert_eq!(parse_hosts(""), Ok(vec![]));
    assert_eq!(parse_hosts("Dev.Example.com,app.example.com:8443"), Ok(vec!["dev.example.com".to_string(), "app.example.com:8443".to_string()]));
    for (text, named) in [("dev.example.com, app.example.com", "\" app.example.com\""), ("dev.example.com,,app.example.com", "\"\""), ("dev.example.com,", "\"\""), ("dev example.com", "\"dev example.com\""), ("dev.example.com:", "\"dev.example.com:\""), ("dev.example.com:99999", "\"dev.example.com:99999\""), ("*.example.com", "\"*.example.com\""), ("-dev.example.com", "\"-dev.example.com\"")] {
        let e = parse_hosts(text).unwrap_err();
        assert!(e.starts_with("BAGHOLDER_HOSTS lists ") && e.contains(named), "{text}: {e}");
    }
}

#[test]
fn test_every_answer_is_private_and_every_failure_has_one_shape() {
    let _g = guard();
    runtime().block_on(async {
        let (code, headers, _) = send(from_the_page(Method::GET, "/api/status", None)).await;
        assert_eq!(code, StatusCode::OK);
        // a read is kept by this browser alone, and asked again each time with its tag
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-cache");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        // a failure and a write are never kept
        let (_, headers, _) = send(from_the_page(Method::GET, "/api/nothing", None)).await;
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let (_, headers, _) = send(from_the_page(Method::POST, "/api/journal", None)).await;
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");

        assert_eq!(json_of(from_the_page(Method::GET, "/api/nothing", None)).await, (StatusCode::NOT_FOUND, json!({"ok": false, "error": "not found"})));
        assert_eq!(json_of(from_the_page(Method::GET, "/api/figures/detail?id=none-such", None)).await, (StatusCode::NOT_FOUND, json!({"ok": false, "error": "no such trade or holding"})));
        assert_eq!(json_of(from_the_page(Method::GET, "/api/filings?symbol=%20", None)).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "symbol required"})));
        // a listing is named by its venue too: a ticker can be two companies on two exchanges
        assert_eq!(json_of(from_the_page(Method::GET, "/api/filings?symbol=QNC", None)).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "QNC names no exchange: a ticker can be two companies on two exchanges"})));
        assert_eq!(json_of(from_the_page(Method::GET, "/api/filings/doc?symbol=QNC", None)).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "source (SEDAR+ or SEC) and id required"})));
        assert_eq!(json_of(from_the_page(Method::POST, "/api/journal", None)).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "id required"})));
        let (code, body) = json_of(from_the_page(Method::POST, "/api/journal", Some(r#"{"id":"x","thesis":"x"}"#))).await;
        assert_eq!((code, &body["ok"]), (StatusCode::BAD_REQUEST, &json!(false)), "the page sends the whole entry");
        let (code, body) = json_of(from_the_page(Method::POST, "/api/journal", Some(r#"{"id":"no-such","thesis":"","grade":"","tags":[]}"#))).await;
        assert_eq!((code, body), (StatusCode::NOT_FOUND, json!({"ok": false, "error": "no trade no-such"})));

        // an answer about a payee is for a payee asked about, naming a debt it may be
        assert_eq!(json_of(from_the_page(Method::POST, "/api/debts/answer", Some(r#"{"payee":" ","debt":null}"#))).await, (StatusCode::BAD_REQUEST, json!({"ok": false, "error": "payee required"})));
        assert_eq!(
            json_of(from_the_page(Method::POST, "/api/debts/answer", Some(r#"{"payee":"debit:no one","debt":null}"#))).await,
            (StatusCode::NOT_FOUND, json!({"ok": false, "error": "no payment waits for an answer about debit:no one"}))
        );

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
        // a trade of the figures, by its id: its journal kept in the book, and the figures' trade carrying it
        let a = app();
        let f = a.figures.get().unwrap();
        let book = f.book().unwrap();
        let names = f.names().unwrap();
        let context = a.market_context().unwrap();
        let door = crate::wire::context::Door { built: &context, app: &a };
        let trades = |f: &crate::figures::Figures| f.read(|e| crate::views::snapshot_of(&crate::views::Cx { engine: e, names: &names, tables: &door, following: &context.following }, "trades", json!({"limit": 100000})).unwrap()).unwrap();
        let doc = trades(f);
        let id = doc["trades"][0]["id"].as_str().unwrap().to_string();
        let (code, body) = json_of(from_the_page(Method::POST, "/api/journal", Some(&format!(r#"{{"id":"{id}","grade":"A","tags":["x", " "],"thesis":"why"}}"#)))).await;
        assert_eq!((code, body), (StatusCode::OK, json!({"ok": true})));
        let subject = bagholder_core::journal::JournalSubject::Trade(bagholder_core::TradeId::parse(&id).unwrap());
        let kept = book.journal(subject).unwrap().unwrap();
        assert_eq!((kept.thesis.as_str(), kept.grade.map(|g| g.as_str()), kept.tags), ("why", Some("A"), vec!["x".to_string()]));
        let doc = trades(f);
        let t = doc["trades"].as_array().unwrap().iter().find(|t| t["id"] == json!(id)).unwrap();
        assert_eq!((t["grade"].as_str(), t["thesis"].as_str()), (Some("A"), Some("why")));
        let (code, _) = json_of(from_the_page(Method::POST, "/api/journal", Some(&format!(r#"{{"id":"{id}","grade":"Z","tags":[],"thesis":""}}"#)))).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "a grade that is not one");
    });
}

#[test]
fn test_hashed_files_are_kept_for_good_and_the_page_is_always_asked_for_again() {
    let _g = guard();
    runtime().block_on(async {
        let (code, _, _) = send(from_the_page(Method::GET, "/assets/../../Cargo.toml", None)).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "nothing outside the page's own folder");
        let (code, headers, _) = send(from_the_page(Method::GET, "/", None)).await;
        if super::assets::built_page(&app().root) {
            assert_eq!(code, StatusCode::OK, "the built page");
            assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
            assert!(headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
        } else {
            assert_eq!(code, StatusCode::NOT_FOUND, "a checkout where the page has not been built has none to serve");
        }
        // the icon, from the built page or the page's sources
        let (code, _, _) = send(from_the_page(Method::GET, "/favicon.png", None)).await;
        assert_eq!(code, StatusCode::OK);
        for gone in ["/v2", "/ledger.html", "/lightweight-charts.js"] {
            let (code, _, _) = send(from_the_page(Method::GET, gone, None)).await;
            assert_eq!(code, StatusCode::NOT_FOUND, "{gone}");
        }
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
        while frames(&text).is_empty() {
            read(&mut text, tokio::time::timeout(Duration::from_secs(10), body.next()).await.expect("the stream opens at once"));
        }
        let got = frames(&text);
        assert_eq!(got[0].0, "hello");
        let id = got[0].1["id"].as_u64().unwrap();
        assert!(id > 0);
        assert_eq!(got.len(), 1, "nothing is sent that the page has not asked for");
        // the page says what it shows: the header's status
        let (code, _) = json_of(from_the_page(Method::POST, "/api/events/watch", Some(&json!({"id": id, "docs": {"status": {}}}).to_string()))).await;
        assert_eq!(code, StatusCode::OK);
        while frames(&text).len() < 2 {
            read(&mut text, tokio::time::timeout(Duration::from_secs(10), body.next()).await.expect("what the page shows is sent"));
        }
        let got = frames(&text);
        assert_eq!((got[1].0.as_str(), &got[1].1["doc"]), ("snapshot", &json!("status")));
        assert_eq!(got[1].1["data"]["ok"], json!(true));
        assert_eq!(app().events.watchers(), 1, "someone is looking");

        // something the header shows changes: the page is sent that field, as a
        // patch -- never the view again
        app().state.lock().unwrap().sync_step = "Reading activity".into();
        let mut text = String::new();
        let told = |text: &str| frames(text).iter().any(|(_, data)| data["ops"].as_array().is_some_and(|ops| ops.contains(&json!(["set", ["syncStep"], "Reading activity"]))));
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
    let conn = crate::notify::book(app_ref()).unwrap();
    crate::notify::set_settings(&conn, &serde_json::from_value(json!({"fills": true})).unwrap()).unwrap();
    bagholder_market::stored::feeds::clear_notifications(conn.notices()).unwrap();
    let mut feed = crate::events::Feed::open(app());
    assert!(app().events.watch(&app(), feed.id(), [("notifications".to_string(), crate::events::Want { params: json!({}), have: None })].into_iter().collect()));
    let first = feed.step(&crate::status::status);
    let bell = first.iter().find(|(_, data)| data["doc"] == "notifications").expect("the bell arrives whole once");
    assert_eq!((bell.0, &bell.1["data"]), ("snapshot", &json!({"rows": [], "unread": 0})));

    let row = crate::notify::emit(&app(), &conn, "fills", "order:9:filled", "Order filled · QNC", "Bought 5 at 1.75", None).unwrap().expect("fills are on");
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


/// A read asked for again with the tag of what it holds is answered 304, with no
/// body; asked with another tag, it is sent whole with its tag.
#[test]
fn test_a_read_the_page_holds_is_answered_unchanged() {
    let _g = guard();
    runtime().block_on(async {
        let (code, headers, body) = send(from_the_page(Method::GET, "/api/figures/detail?id=nothing-by-this-id", None)).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "a failure is never tagged: {}", String::from_utf8_lossy(&body));
        assert!(headers.get(header::ETAG).is_none());
        let (code, headers, body) = send(from_the_page(Method::GET, "/api/watch", None)).await;
        assert_eq!(code, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let tag = headers[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-cache");
        let mut again = from_the_page(Method::GET, "/api/watch", None);
        again.headers_mut().insert(header::IF_NONE_MATCH, tag.parse().unwrap());
        let (code, headers, body) = send(again).await;
        assert_eq!((code, body.len()), (StatusCode::NOT_MODIFIED, 0));
        assert_eq!(headers[header::ETAG].to_str().unwrap(), tag);
        let mut other = from_the_page(Method::GET, "/api/watch", None);
        other.headers_mut().insert(header::IF_NONE_MATCH, "\"0000000000000000\"".parse().unwrap());
        let (code, _, body) = send(other).await;
        assert_eq!(code, StatusCode::OK);
        assert!(!body.is_empty());
    });
}
