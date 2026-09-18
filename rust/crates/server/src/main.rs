//! The Bagholder server: the page and its assets, the model behind them, the
//! Wealthsimple session and sync, orders, and the market-data loops.
//!
//! The gate: loopback only unless it was bound elsewhere on purpose, the Host
//! header has to name 127.0.0.1 and the port, and a write has to come from the
//! page itself.

mod app;
mod feeds;
mod login;
mod notify;
mod orders;
mod session;
mod update;
mod versions;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tiny_http::{Header, Request, Response, Server};

use app::{app, f, log, s, spawn, truthy};

const PORTS: [u16; 3] = [8765, 8766, 8767];
const ACTIVITY_PULL_SEC: i64 = 24 * 60 * 60;

fn home_dir() -> PathBuf {
    let env = std::env::var("BAGHOLDER_HOME").unwrap_or_default();
    if !env.trim().is_empty() {
        return PathBuf::from(env.trim());
    }
    let base = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    Path::new(&base).join(".bagholder-rust")
}

/// Where the page and its assets are: beside the executable, or in a checkout
/// the executable was built in, or the working folder.
fn root_dir() -> PathBuf {
    let env = std::env::var("BAGHOLDER_ROOT").unwrap_or_default();
    if !env.trim().is_empty() {
        return PathBuf::from(env.trim());
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        for d in dir.ancestors().take(4) {
            if d.join("ledger.html").is_file() {
                return d.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

// --------------------------------------------------------------------------
// the gate
// --------------------------------------------------------------------------

fn header(req: &Request, name: &str) -> String {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default()
}

fn is_local(req: &Request) -> bool {
    if app().bind_host != "127.0.0.1" {
        // bound beyond loopback on purpose (a container); the peer is its bridge
        return true;
    }
    match req.remote_addr() {
        Some(addr) => {
            let ip = addr.ip().to_string();
            ip == "127.0.0.1" || ip == "::1" || ip == "::ffff:127.0.0.1"
        }
        None => false,
    }
}

fn host_ok(req: &Request) -> bool {
    let raw = header(req, "Host").trim().to_lowercase();
    if raw.is_empty() || raw.contains(',') {
        return false;
    }
    if app().bind_host != "127.0.0.1" {
        // a container's port may be published under another number; the name must still be 127.0.0.1
        let (name, port) = match raw.split_once(':') { Some(p) => p, None => return false };
        return name == "127.0.0.1" && !port.is_empty() && port.len() <= 5 && port.bytes().all(|c| c.is_ascii_digit());
    }
    raw == format!("127.0.0.1:{}", *app().port.lock().unwrap())
}

fn write_ok(req: &Request) -> bool {
    if header(req, "Sec-Fetch-Site").trim().to_lowercase() == "same-origin" {
        return true;
    }
    !header(req, "X-Bagholder").trim().is_empty()
}

fn gate(req: &Request, write: bool) -> bool {
    is_local(req) && host_ok(req) && !(write && !write_ok(req))
}

// --------------------------------------------------------------------------
// responses
// --------------------------------------------------------------------------

fn hdr(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn send(req: Request, code: u16, body: Vec<u8>, content_type: &str) {
    let response = Response::from_data(body)
        .with_status_code(code)
        .with_header(hdr("Content-Type", content_type))
        .with_header(hdr("Cache-Control", "no-store"))
        .with_header(hdr("X-Content-Type-Options", "nosniff"));
    let _ = req.respond(response);
}

fn send_json(req: Request, code: u16, body: &Value) {
    send(req, code, serde_json::to_vec(body).unwrap_or_default(), "application/json; charset=utf-8");
}

/// A body written as it is made: the headers, then each chunk flushed as soon
/// as the producer hands it over, until the producer is done or the client
/// goes. Chunked, so the response ends when the producer does: the server keeps
/// the connection for the next request, and a browser that waited on an
/// unterminated response would hold one of its few connections to this host.
fn stream<F: FnOnce(&mut dyn FnMut(&[u8]) -> bool) + Send + 'static>(req: Request, content_type: &str, produce: F) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nTransfer-Encoding: chunked\r\n\r\n",
        content_type
    );
    let mut w = req.into_writer();
    if w.write_all(head.as_bytes()).and_then(|_| w.flush()).is_err() {
        return;
    }
    let mut alive = true;
    {
        let mut write = |chunk: &[u8]| {
            if chunk.is_empty() {
                return true;
            }
            alive = w
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .and_then(|_| w.write_all(chunk))
                .and_then(|_| w.write_all(b"\r\n"))
                .and_then(|_| w.flush())
                .is_ok();
            alive
        };
        produce(&mut write);
    }
    if alive {
        let _ = w.write_all(b"0\r\n\r\n").and_then(|_| w.flush());
    }
}

fn query_of(url: &str) -> &str {
    url.split_once('?').map(|(_, q)| q).unwrap_or("")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1
            }
            b'%' if i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 => match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                Ok(v) => {
                    out.push(v);
                    i += 3
                }
                Err(_) => {
                    out.push(b[i]);
                    i += 1
                }
            },
            c => {
                out.push(c);
                i += 1
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// `parse_qs(q).get(name)[0]`, raw (blank values dropped, as parse_qs does).
fn first(query: &str, name: &str) -> String {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(k) == name && !v.is_empty() {
            return percent_decode(v);
        }
    }
    String::new()
}

/// Stripped, None when blank.
fn qp(query: &str, name: &str) -> Option<String> {
    let v = first(query, name).trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

fn yes(v: Option<String>) -> bool {
    matches!(v.as_deref(), Some("1") | Some("true") | Some("yes"))
}

fn static_file(name: &str) -> Option<Vec<u8>> {
    std::fs::read(app().root.join(name)).ok()
}

// --------------------------------------------------------------------------
// what the header shows
// --------------------------------------------------------------------------

fn status_payload() -> Value {
    let conn = app().open().ok();
    let (acts, accounts, synced) = conn.as_ref().and_then(|c| versions::status_counts(c).ok()).unwrap_or((0, 0, String::new()));
    let data_version = conn.as_ref().and_then(|c| versions::data_version(c).ok()).unwrap_or_default();
    let core_version = conn.as_ref().and_then(|c| versions::core_version(c).ok()).unwrap_or_default();
    let upd = update::update_status();
    let sess = session::load_session();
    let notify_status = conn.as_ref().and_then(|c| notify::status(c).ok()).unwrap_or(json!({}));
    let open_orders = orders::open_orders_count();
    let can_update = update::can_update(None);
    let off = update::updates_off();
    let st = app().state.lock().unwrap();
    let connected = st.connected && sess.as_ref().map(|x| truthy(x.get("access_token"))).unwrap_or(false);
    let email = if !st.email.is_empty() { st.email.clone() } else { sess.as_ref().map(|x| f(x, "email")).unwrap_or_default() };
    json!({
        "ok": true,
        "connected": connected,
        "email": email,
        "lastSync": if st.last_sync.is_empty() { synced } else { st.last_sync.clone() },
        "activityCount": acts,
        "accountCount": accounts,
        "capturing": st.capturing,
        "syncing": st.syncing,
        "listingsFilling": st.listings_filling,
        "syncStep": st.sync_step,
        "error": st.error,
        "dataVersion": format!("{}|{}", data_version, bagholder_model::clock::today_local()),
        // everything the model reads except the quotes: when this is unchanged but the
        // data version moved, only prices ticked, and the page fetches just the live figures
        "coreVersion": format!("{}|{}", core_version, bagholder_model::clock::today_local()),
        "summaryReady": bagholder_market::enrich::summary_status() == "ready",
        "protocol": app::PROTOCOL,
        "startedAt": app().started_at,
        "version": app::APP_VERSION,
        "latestVersion": s(upd.get("latest")),
        "updateAvailable": truthy(upd.get("updateAvailable")),
        "updateUrl": if off { update::image_page() } else { let u = s(upd.get("url")); if u.is_empty() { update::repo_url() } else { u } },
        "canUpdate": can_update,
        "updateBy": if off { "image" } else { "app" },
        "loginView": login::login_view(),
        "ordersLive": orders::orders_live(),
        "openOrders": open_orders,
        "updating": st.updating,
        "updateError": st.update_error,
        "notify": notify_status,
        "newsReading": feeds::news_reading(),
    })
}

// --------------------------------------------------------------------------
// routing
// --------------------------------------------------------------------------

fn handle(mut req: Request) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let query = query_of(&url).to_string();
    let method = req.method().as_str().to_uppercase();
    if method == "POST" {
        if !gate(&req, true) {
            return send_json(req, 403, &json!({"ok": false}));
        }
        let body = read_json(&mut req);
        return handle_post(req, &path, body);
    }
    if method != "GET" && method != "HEAD" {
        return send_json(req, if method == "OPTIONS" { 403 } else { 501 }, &json!({"ok": false}));
    }
    if !gate(&req, false) {
        return send_json(req, 403, &json!({"ok": false}));
    }
    handle_get(req, &path, &query);
}

fn read_json(req: &mut Request) -> Value {
    let n = req.body_length().unwrap_or(0);
    if n == 0 || n > 1_048_576 {
        return json!({});
    }
    let mut raw = Vec::with_capacity(n);
    let _ = req.as_reader().take(n as u64).read_to_end(&mut raw);
    if raw.is_empty() {
        return json!({});
    }
    serde_json::from_slice(&raw).unwrap_or_else(|_| json!({}))
}

fn handle_get(req: Request, path: &str, query: &str) {
    match path {
        "/api/login/stream" => stream(req, "multipart/x-mixed-replace; boundary=frame", |write| {
            login::login_stream(|chunk| write(chunk));
        }),
        "/api/login/frame" => match login::login_frame() {
            Some(data) => send(req, 200, data, "image/jpeg"),
            None => send(req, 204, vec![], "application/json; charset=utf-8"),
        },
        "/" | "/index.html" | "/ledger.html" | "/v2" | "/v2/" => match std::fs::read(feeds::ledger_path()) {
            Ok(data) => send(req, 200, data, "text/html; charset=utf-8"),
            Err(_) => send_json(req, 404, &json!({"ok": false, "error": "ledger.html missing"})),
        },
        "/api/order/quote" => {
            let v = orders::ticket_quote(&first(query, "symbol"), &first(query, "security"), &first(query, "account"), &first(query, "exchange"));
            send_json(req, 200, &v)
        }
        "/api/symbols/search" => send_json(req, 200, &bagholder_market::search::symbol_search(&app().db_path(), &first(query, "q"))),
        "/api/symbols/quote" => {
            // a glance at a listing the watchlist's add row offers: its price and day change, not stored
            let rec = json!({"symbol": qp(query, "symbol").unwrap_or_default(), "exchange": qp(query, "exchange").unwrap_or_default(),
                             "currency": qp(query, "currency").unwrap_or_default(), "kind": "Shares"});
            let mut out = json!({"ok": true, "price": null, "priceChange": null, "percentChange": null});
            if !f(&rec, "symbol").is_empty() {
                if let Ok(conn) = app().open() {
                    let (today, _, _) = bagholder_market::clock_now();
                    if let Some(Value::Object(q)) = bagholder_market::quotes::peek_quote(&conn, &rec, &today) {
                        for (k, v) in q {
                            out[k] = v;
                        }
                    }
                }
            }
            send_json(req, 200, &out)
        }
        "/api/filings" => {
            let symbol = match qp(query, "symbol") { Some(x) => x, None => return send_json(req, 400, &json!({"ok": false, "error": "symbol required"})) };
            let v = feeds::filings_payload(&symbol, yes(qp(query, "refresh")), qp(query, "name").as_deref(), qp(query, "exchange").as_deref(), qp(query, "currency").as_deref());
            send_json(req, 200, &v)
        }
        "/api/listing" => {
            // one listing's own page, held or not
            let g = |k| qp(query, k).unwrap_or_default();
            send_json(req, 200, &feeds::listing_payload(&g("symbol"), &g("exchange"), &g("currency"), &g("name")))
        }
        "/api/fear" => send_json(req, 200, &feeds::fear_payload(&qp(query, "index").unwrap_or_else(|| "stocks".into()))),
        "/api/shorts/feed" => send_json(req, 200, &feeds::shorts_feed()),
        "/api/shorts" => {
            let v = feeds::shorts_payload(&qp(query, "symbol").unwrap_or_default(), qp(query, "exchange").as_deref(), qp(query, "currency").as_deref(), yes(qp(query, "trend")));
            send_json(req, 200, &v)
        }
        "/api/news/symbol" => {
            let g = |k| qp(query, k).unwrap_or_default();
            send_json(req, 200, &feeds::news_symbol_payload(&g("symbol"), &g("exchange"), &g("currency")))
        }
        "/api/filings/feed" => send_json(req, 200, &feeds::filings_feed(&qp(query, "scope").unwrap_or_default(), 200)),
        "/api/filings/doc" => {
            let (symbol, id) = match (qp(query, "symbol"), qp(query, "id")) {
                (Some(a), Some(b)) => (a, b),
                _ => return send_json(req, 400, &json!({"ok": false, "error": "symbol and id required"})),
            };
            match feeds::filings_document(&symbol, &id) {
                Ok((data, ct)) => send(req, 200, data, if ct.is_empty() { "application/pdf" } else { &ct }),
                // this route is opened in a tab of its own, so a browser asking for a page is
                // answered with one: a raw JSON error is the app failing in front of the person
                Err(e) if header(&req, "Accept").contains("text/html") => {
                    let page = feeds::document_error_page(&symbol, &id, &e);
                    send(req, 502, page.into_bytes(), "text/html; charset=utf-8")
                }
                Err(e) => send_json(req, 502, &json!({"ok": false, "error": e})),
            }
        }
        "/api/filings/enrich" => {
            let (symbol, id) = match (qp(query, "symbol"), qp(query, "id")) {
                (Some(a), Some(b)) => (a, b),
                _ => return send_json(req, 400, &json!({"ok": false, "error": "symbol and id required"})),
            };
            send_json(req, 200, &feeds::filings_enrich(&symbol, &id))
        }
        "/api/orders" => send_json(req, 200, &guarded(|| orders::orders_payload(true))),
        "/api/notifications" => {
            let v = app().open().and_then(|conn| {
                Ok(json!({
                    "ok": true,
                    "settings": notify::status(&conn)?,
                    "kinds": notify::KINDS,
                    "rows": bagholder_store::feeds::list_notifications(&conn, 0, "", false, 50, true)?,
                    "unread": bagholder_store::feeds::unread_notifications(&conn)?,
                }))
            });
            match v {
                Ok(v) => send_json(req, 200, &v),
                Err(e) => store_failed(req, e),
            }
        }
        "/api/notifications/stream" => {
            let after = { let a = first(query, "after").trim().to_string(); if a.is_empty() { header(&req, "Last-Event-ID").trim().to_string() } else { a } };
            let after = if !after.is_empty() && after.bytes().all(|c| c.is_ascii_digit()) { after.parse::<i64>().ok() } else { None };
            stream(req, "text/event-stream; charset=utf-8", move |write| {
                notify::stream(after, |chunk| write(chunk.as_bytes()));
            })
        }
        "/api/status" => send_json(req, 200, &status_payload()),
        "/api/history" => send_json(req, 200, &feeds::history_payload(query)),
        "/lightweight-charts.js" => match static_file("lightweight-charts.js") {
            Some(data) => send(req, 200, data, "application/javascript; charset=utf-8"),
            None => send_json(req, 404, &json!({"ok": false, "error": "lightweight-charts.js missing"})),
        },
        "/favicon.png" | "/favicon.ico" => match static_file("favicon.png") {
            Some(data) => send(req, 200, data, "image/png"),
            None => send_json(req, 404, &json!({"ok": false, "error": "favicon missing"})),
        },
        "/api/watch" => match app().open().and_then(|c| bagholder_store::csvimport::status(&c)) {
            Ok(v) => send_json(req, 200, &v),
            Err(e) => store_failed(req, e),
        },
        "/api/data" => match app().open().and_then(|c| data_summary(&c)) {
            Ok(mut v) => {
                v["ok"] = json!(true);
                v["sessionPresent"] = json!(session::load_session().is_some());
                send_json(req, 200, &v)
            }
            Err(e) => store_failed(req, e),
        },
        "/api/model" => {
            if let (Ok(conn), Ok(base)) = (app().open(), app().base()) {
                let (today, now, _) = bagholder_market::clock_now();
                if bagholder_market::refresh::is_stale(&conn, &today, &bagholder_model::symbols_of::payer_symbols(&base)) {
                    app().kick("market", || {
                        feeds::refresh_market_data();
                    });
                } else {
                    let mut syms = bagholder_model::symbols_of::held_symbols(&base);
                    syms.extend(bagholder_model::markets::quote_symbols(&base));
                    let due = bagholder_market::quotes::quote_symbols_needing_refresh(&conn, &syms, now, bagholder_market::quotes::QUOTE_REFRESH_MINUTES).map(|v| !v.is_empty()).unwrap_or(false);
                    if due {
                        app().kick("quotes", || {
                            feeds::refresh_quotes();
                        });
                    }
                }
            }
            let filters = qp(query, "filters").and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            let trade = qp(query, "trade");
            let built = std::panic::catch_unwind(|| {
                app().base().map(|base| {
                    let full = bagholder_model::view::build_view(&base, filters.as_ref());
                    bagholder_model::view::slim(&full, trade.as_deref())
                })
            });
            match built {
                Ok(Ok(mut payload)) => {
                    if qp(query, "only").as_deref() == Some("live") {
                        // a quote tick moves only what is priced off the open positions; the
                        // closed trades, equity curve, KPIs and options lists are unchanged, so
                        // the page fetches just these sections instead of the whole book
                        let mut keys = vec!["ok", "today", "currency", "market", "positions", "positionsSummary", "portfolio"];
                        if qp(query, "markets").filter(|v| !v.is_empty()).is_some() {
                            keys.push("markets"); // the heatmap view is on screen and wants live tiles
                        }
                        let lean = if let Value::Object(full) = &payload {
                            let mut m = serde_json::Map::new();
                            for k in &keys {
                                if let Some(v) = full.get(*k) {
                                    m.insert((*k).to_string(), v.clone());
                                }
                            }
                            Some(Value::Object(m))
                        } else {
                            None
                        };
                        if let Some(l) = lean {
                            payload = l;
                        }
                    }
                    payload["status"] = status_payload();
                    send_json(req, 200, &payload)
                }
                Ok(Err(e)) => {
                    log(&format!("model failed: {}", e));
                    send_json(req, 500, &json!({"ok": false, "error": "model failed: OperationalError"}))
                }
                Err(_) => send_json(req, 500, &json!({"ok": false, "error": "model failed: Exception"})),
            }
        }
        "/api/trade" => {
            // the legs and fills of one trade or holding, fetched when its page opens
            let id = qp(query, "id").unwrap_or_default();
            let found = std::panic::catch_unwind(|| app().base().map(|base| bagholder_model::view::trade_detail(&base, &id)));
            match found {
                Ok(Ok(Some(mut d))) => {
                    d["ok"] = json!(true);
                    send_json(req, 200, &d)
                }
                Ok(Ok(None)) => send_json(req, 404, &json!({"ok": false, "error": "no such trade"})),
                Ok(Err(e)) => {
                    log(&format!("model failed: {}", e));
                    send_json(req, 500, &json!({"ok": false, "error": "model failed: OperationalError"}))
                }
                Err(_) => send_json(req, 500, &json!({"ok": false, "error": "model failed: Exception"})),
            }
        }
        "/api/book" => match app().open().and_then(|c| bagholder_store::snapshot::snapshot(&c, true)) {
            Ok(book) => {
                let or = |k: &str, d: Value| book.get(k).filter(|v| truthy(Some(v))).cloned().unwrap_or(d);
                send_json(req, 200, &json!({
                    "ok": true,
                    "activities": or("activities", json!([])),
                    "accounts": or("accounts", json!([])),
                    "balances": or("balances", json!([])),
                    "navHistory": or("navHistory", json!([])),
                    "navByAccount": or("navByAccount", json!({})),
                    "syncedAt": or("syncedAt", json!("")),
                    "tradeGroups": or("tradeGroups", json!([])),
                    "notes": or("notes", json!({})),
                    "securities": or("securities", json!([])),
                }))
            }
            Err(e) => store_failed(req, e),
        },
        _ => send_json(req, 404, &json!({"ok": false, "error": "not found"})),
    }
}

/// A route whose work panicked answers a 500 rather than dropping the
/// connection; the panic has been logged by the hook.
fn guarded<F: FnOnce() -> Value + std::panic::UnwindSafe>(work: F) -> Value {
    std::panic::catch_unwind(work).unwrap_or_else(|_| json!({"ok": false, "error": "internal error"}))
}

fn store_failed(req: Request, e: rusqlite::Error) {
    log(&format!("bagholder: {}", e));
    send_json(req, 500, &json!({"ok": false, "error": "store failed"}));
}

/// The row counts the Data & storage dialog shows before a wipe.
fn data_summary(conn: &rusqlite::Connection) -> rusqlite::Result<Value> {
    let count = |sql: &str| -> rusqlite::Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
    let journal_raw = bagholder_store::tables::get_meta(conn, "journal_v2", "")?;
    let journal_n = serde_json::from_str::<Value>(&journal_raw).ok().and_then(|v| v.as_object().map(|m| m.len())).unwrap_or(0);
    let (first_act, last_act): (Option<String>, Option<String>) =
        conn.query_row("SELECT MIN(transaction_date), MAX(transaction_date) FROM activities", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(json!({
        "path": app().db_path().display().to_string(),
        "activities": count("SELECT COUNT(*) FROM activities")?,
        "firstActivity": first_act.unwrap_or_default(),
        "lastActivity": last_act.unwrap_or_default(),
        "accounts": count("SELECT COUNT(*) FROM accounts")?,
        "balances": count("SELECT COUNT(*) FROM balances")?,
        "navDays": count("SELECT COUNT(*) FROM nav_history")?,
        "securities": count("SELECT COUNT(*) FROM securities")?,
        "journal": journal_n,
        "fxDays": count("SELECT COUNT(*) FROM fx_rates")?,
        "benchmarkDays": count("SELECT COUNT(*) FROM benchmark_prices")?,
        "filings": count("SELECT COUNT(*) FROM filings")?,
        "syncedAt": bagholder_store::tables::get_meta(conn, "synced_at", "")?,
    }))
}

fn ids_of(v: Option<&Value>) -> Option<Vec<i64>> {
    v.and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
}

fn handle_post(req: Request, path: &str, body: Value) {
    let body = if body.is_object() { body } else { json!({}) };
    let with_store = |req: Request, work: &dyn Fn(&rusqlite::Connection) -> rusqlite::Result<Value>| match app().open().and_then(|c| work(&c)) {
        Ok(v) => send_json(req, 200, &v),
        Err(e) => store_failed(req, e),
    };
    match path {
        "/api/login/start" => send_json(req, 200, &login::start_login_browser()),
        "/api/login/cancel" => send_json(req, 200, &login::cancel_login()),
        "/api/login/input" => send_json(req, 200, &login::login_input(&body)),
        "/api/update" => send_json(req, 200, &update::start_update()),
        "/api/capture" => send_json(req, 200, &session::capture_tokens(&body)),
        "/api/refresh" => send_json(req, 200, &session::refresh_now()),
        "/api/sync" => {
            if session::load_session().is_none() {
                return send_json(req, 200, &json!({"ok": false, "error": "not connected"}));
            }
            app().state.lock().unwrap().error.clear();
            spawn("bagholder-sync", || {
                feeds::sync_then_market();
            });
            send_json(req, 200, &json!({"ok": true, "syncing": true}))
        }
        "/api/data/clear" => {
            if app().state.lock().unwrap().syncing {
                return send_json(req, 409, &json!({"ok": false, "error": "A sync is running. Wait for it to finish."}));
            }
            let run = || -> rusqlite::Result<Value> {
                let conn = app().open()?;
                bagholder_store::admin::clear_synced_data(&conn, !truthy(body.get("journal")), !truthy(body.get("market")))?;
                data_summary(&conn)
            };
            match run() {
                Ok(mut summary) => {
                    if truthy(body.get("session")) {
                        session::delete_session();
                    }
                    {
                        let mut st = app().state.lock().unwrap();
                        st.last_sync.clear();
                        st.error.clear();
                    }
                    app().invalidate();
                    summary["ok"] = json!(true);
                    summary["sessionPresent"] = json!(session::load_session().is_some());
                    send_json(req, 200, &summary)
                }
                Err(e) => store_failed(req, e),
            }
        }
        "/api/order" => send_json(req, 200, &guarded(|| orders::place_order(&body))),
        "/api/order/cancel" => send_json(req, 200, &guarded(|| orders::cancel_order(&s(body.get("id"))))),
        "/api/order/modify" => send_json(req, 200, &guarded(|| orders::modify_order(&s(body.get("id")), body.get("quantity"), body.get("limitPrice")))),
        "/api/bracket/adjust" => send_json(req, 200, &guarded(|| {
            orders::adjust_bracket(&s(body.get("id")), &s(body.get("leg")), body.get("price"), body.get("trail"), truthy(body.get("remove")))
        })),
        "/api/bracket/cancel" => send_json(req, 200, &guarded(|| orders::cancel_bracket(&s(body.get("id"))))),
        "/api/notifications/settings" => with_store(req, &|conn| {
            notify::set_settings(conn, &body)?;
            Ok(json!({"ok": true, "settings": notify::status(conn)?}))
        }),
        "/api/notifications/test" => with_store(req, &|conn| {
            let row = notify::test_notification(conn);
            Ok(json!({"ok": row.is_some(), "id": row.as_ref().and_then(|r| r.get("id").cloned()).unwrap_or(json!(0))}))
        }),
        "/api/notifications/read" => with_store(req, &|conn| {
            let ids = ids_of(body.get("ids"));
            Ok(json!({"ok": true, "read": bagholder_store::feeds::mark_notifications_read(conn, ids.as_deref(), &app::now_iso())?}))
        }),
        "/api/notifications/clear" => with_store(req, &|conn| Ok(json!({"ok": true, "cleared": bagholder_store::feeds::clear_notifications(conn)?}))),
        "/api/notifications/seen" => with_store(req, &|conn| {
            let ids = ids_of(body.get("ids")).unwrap_or_default();
            Ok(json!({"ok": true, "seen": bagholder_store::feeds::mark_notifications_seen(conn, &ids, &app::now_iso())?}))
        }),
        "/api/orders/refresh" => send_json(req, 200, &guarded(|| {
            let mut r = orders::refresh_orders("");
            if let (Value::Object(m), Value::Object(p)) = (&mut r, orders::orders_payload(false)) {
                for (k, v) in p {
                    m.insert(k, v);
                }
            }
            r
        })),
        "/api/markets/refresh" => send_json(req, 200, &feeds::kick_universes()),
        "/api/watchlist/add" => send_json(req, 200, &feeds::watch_add(&body)),
        "/api/watchlist/remove" => send_json(req, 200, &feeds::watch_remove(&body)),
        "/api/tiles/set" => send_json(req, 200, &feeds::tiles_set(&body)),
        "/api/journal" => {
            let id = s(body.get("id")).trim().to_string();
            if id.is_empty() {
                return send_json(req, 400, &json!({"ok": false, "error": "id required"}));
            }
            let entry = json!({
                "thesis": body.get("thesis").cloned().unwrap_or(Value::Null),
                "tags": body.get("tags").cloned().unwrap_or(Value::Null),
                "grade": body.get("grade").cloned().unwrap_or(Value::Null),
            });
            with_store(req, &|conn| {
                let entries = bagholder_store::admin::save_journal_entry(conn, &id, Some(&entry))?;
                app().invalidate();
                Ok(json!({"ok": true, "journal": entries}))
            })
        }
        "/api/disconnect" => {
            session::delete_session();
            send_json(req, 200, &json!({"ok": true}))
        }
        "/api/book/append" => {
            let result = guarded(|| orders::append_manual(&body));
            app().invalidate();
            send_json(req, 200, &result)
        }
        "/api/import" => {
            let text = match body.get("text") {
                Some(Value::String(t)) if !bagholder_model::textrules::trim_space(t).is_empty() => t.clone(),
                _ => return send_json(req, 400, &json!({"ok": false, "error": "text required"})),
            };
            let name = { let n = s(body.get("name")); if n.is_empty() { "upload.csv".to_string() } else { n } };
            let conn = match app().open() { Ok(c) => c, Err(e) => return store_failed(req, e) };
            match bagholder_store::csvimport::import_text(&conn, &name, &text) {
                Ok(report) => {
                    if truthy(report.get("added")) {
                        app().invalidate();
                    }
                    send_json(req, 200, &report)
                }
                Err(e) => {
                    log(&format!("bagholder: import failed: {}", e));
                    send_json(req, 500, &json!({"ok": false, "error": e}))
                }
            }
        }
        "/api/watch" => {
            let run = || -> rusqlite::Result<(u16, Value)> {
                let conn = app().open()?;
                let set = bagholder_store::csvimport::set_watch_folder(&conn, &s(body.get("path")))?;
                if set.get("ok") != Some(&json!(true)) {
                    return Ok((400, set));
                }
                let mut result = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
                if truthy(result.get("added")) {
                    app().invalidate();
                }
                result["status"] = bagholder_store::csvimport::status(&conn)?;
                Ok((200, result))
            };
            match run() {
                Ok((code, v)) => send_json(req, code, &v),
                Err(e) => store_failed(req, e),
            }
        }
        "/api/watch/scan" => {
            let run = || -> rusqlite::Result<Value> {
                let conn = app().open()?;
                let mut result = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
                if truthy(result.get("ok")) && truthy(result.get("added")) {
                    app().invalidate();
                }
                result["status"] = bagholder_store::csvimport::status(&conn)?;
                Ok(result)
            };
            match run() {
                Ok(v) => {
                    let code = if truthy(v.get("ok")) { 200 } else { 400 };
                    send_json(req, code, &v)
                }
                Err(e) => store_failed(req, e),
            }
        }
        "/api/watch/clear" => with_store(req, &|conn| {
            bagholder_store::csvimport::clear_watch_folder(conn)?;
            bagholder_store::csvimport::status(conn)
        }),
        "/api/groups" => with_store(req, &|conn| Ok(json!({"ok": true, "groups": bagholder_store::tables::save_trade_groups(conn, body.get("groups"))?}))),
        "/api/notes" => with_store(req, &|conn| Ok(json!({"ok": true, "notes": bagholder_store::tables::save_trade_notes(conn, body.get("notes"))?}))),
        _ => send_json(req, 404, &json!({"ok": false, "error": "not found"})),
    }
}

// --------------------------------------------------------------------------
// start
// --------------------------------------------------------------------------

fn port_choices() -> Vec<u16> {
    let env = std::env::var("BAGHOLDER_PORT").unwrap_or_default();
    match env.trim().parse::<u16>() {
        Ok(p) if p >= 1024 && env.trim().bytes().all(|c| c.is_ascii_digit()) => vec![p],
        _ => PORTS.to_vec(),
    }
}

fn open_browser(url: &str) {
    let cmd: (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = std::process::Command::new(cmd.0).args(&cmd.1).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

fn serve() -> i32 {
    let home = home_dir();
    let _ = std::fs::create_dir_all(&home);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700));
    }
    let bind_host = { let b = std::env::var("BAGHOLDER_BIND").unwrap_or_default().trim().to_string(); if b.is_empty() { "127.0.0.1".to_string() } else { b } };
    let a = app::init(home, root_dir(), bind_host.clone());
    match a.open() {
        Ok(conn) => {
            if let Err(e) = bagholder_store::relabel::ensure(&conn) {
                log(&format!("bagholder: the store could not be prepared: {}", e));
                return 1;
            }
        }
        Err(e) => {
            log(&format!("bagholder: the store could not be opened: {}", e));
            return 1;
        }
    }
    session::boot_session();

    let mut bound = None;
    let mut last_err = String::new();
    for port in port_choices() {
        match Server::http(format!("{}:{}", bind_host, port)) {
            Ok(srv) => {
                bound = Some((srv, port));
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let (server, port) = match bound {
        Some(b) => b,
        None => {
            let ports: Vec<String> = port_choices().iter().map(|p| p.to_string()).collect();
            eprintln!("Could not bind {}:{} ({})", bind_host, ports.join("-"), last_err);
            return 1;
        }
    };
    *a.port.lock().unwrap() = port;

    spawn("bagholder-auto-sync", session::auto_sync_loop);
    spawn("bagholder-market", || {
        feeds::refresh_market_data();
    });
    spawn("bagholder-update-check", || {
        update::check_for_update();
    });
    spawn("bagholder-quote-loop", feeds::quote_loop);
    spawn("bagholder-portfolio-loop", session::portfolio_loop);
    spawn("bagholder-orders-loop", orders::orders_loop);
    spawn("bagholder-bracket-loop", orders::bracket_loop);
    spawn("bagholder-exposure-loop", feeds::exposure_loop);
    // rows added before the bare-ticker convention (Wealthsimple's `.TO` on a dual listing) take it now
    if let Ok(conn) = a.open() {
        for w in bagholder_store::feeds::list_watchlist(&conn).unwrap_or_default() {
            let sym = f(&w, "symbol");
            let bare = bagholder_model::venues::tmx_symbol(&sym);
            if !bare.is_empty() && bare != sym {
                let _ = bagholder_store::feeds::remove_watch(&conn, &sym, &f(&w, "exchange"));
                let _ = bagholder_store::feeds::add_watch(&conn, &bare, &f(&w, "exchange"), &f(&w, "name"), &f(&w, "currency"), &f(&w, "securityId"), &f(&w, "addedAt"));
            }
        }
    }
    spawn("bagholder-news-loop", feeds::news_loop);
    spawn("bagholder-universe-loop", feeds::universe_loop);
    spawn("bagholder-market-loop", feeds::market_loop);
    spawn("bagholder-archive", feeds::archive_loop);
    spawn("bagholder-watch", feeds::watch_loop);
    spawn("bagholder-filings-sweep", feeds::filings_sweep_loop);
    spawn("bagholder-disclosure-reader", feeds::disclosure_read_loop);
    spawn("bagholder-shorts-sweep", feeds::shorts_sweep_loop);
    spawn("bagholder-fear-sweep", feeds::fear_sweep_loop);

    let url = format!("http://127.0.0.1:{}", port);
    println!("Bagholder  {}", url);
    // a second instance run for verification must not open anyone's browser
    if std::env::var("BAGHOLDER_NO_BROWSER").unwrap_or_default().trim().is_empty() {
        open_browser(&url);
    }
    if a.state.lock().unwrap().connected {
        let due = a.open().ok().and_then(|c| bagholder_store::admin::activity_pull_due(&c, app::now_unix() as i64 - 0).ok()).unwrap_or(false);
        let _ = ACTIVITY_PULL_SEC;
        if due {
            spawn("bagholder-boot-sync", || {
                session::run_sync(true, true);
            });
        } else {
            spawn("bagholder-listings", || {
                if let Some(sess) = session::load_session() {
                    session::fill_listings(&sess, false);
                }
            });
        }
    }

    let server = std::sync::Arc::new(server);
    {
        let server = server.clone();
        spawn("bagholder-stop-watch", move || {
            while !app().wait(Duration::from_secs(3600)) {}
            server.unblock();
        });
    }
    for req in server.incoming_requests() {
        spawn("bagholder-request", move || handle(req));
    }
    a.exit_code.load(std::sync::atomic::Ordering::SeqCst)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("bagholder {}", app::APP_VERSION);
        return;
    }
    let child = std::env::var("BAGHOLDER_CHILD").map(|v| v == "1").unwrap_or(false);
    if child || update::updates_off() {
        // the supervisor exists to restart an updated server; a copy that never updates runs plain
        std::process::exit(serve());
    }
    std::process::exit(update::supervise(&home_dir(), update::UPDATE_HEALTHY_SEC));
}

#[cfg(test)]
mod tests {
    //! The page, the package contents and the protocol check.
    use std::path::PathBuf;

    /// The repository root: the shared page and data, with this workspace in rust/.
    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn page() -> String {
        std::fs::read_to_string(root().join("ledger.html")).unwrap()
    }

    const SHIPPED: [&str; 3] = ["ledger.html", "lightweight-charts.js", "favicon.png"];

    /// Where the Rust build keeps its data: ~/.bagholder-rust, or wherever BAGHOLDER_HOME says.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_the_default_folder_is_dot_bagholder_rust() {
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("BAGHOLDER_HOME").ok();
        std::env::remove_var("BAGHOLDER_HOME");
        let home = crate::home_dir();
        if let Some(v) = previous {
            std::env::set_var("BAGHOLDER_HOME", v);
        }
        assert_eq!(home.file_name().unwrap(), ".bagholder-rust");
    }

    #[test]
    fn test_bagholder_home_decides_where_the_folder_is() {
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("BAGHOLDER_HOME").ok();
        let d = std::env::temp_dir().join(format!("bh-home-{}-elsewhere", std::process::id()));
        std::env::set_var("BAGHOLDER_HOME", &d);
        let home = crate::home_dir();
        match previous {
            Some(v) => std::env::set_var("BAGHOLDER_HOME", v),
            None => std::env::remove_var("BAGHOLDER_HOME"),
        }
        assert_eq!(home, d);
    }

    #[test]
    fn test_every_script_on_the_page_parses() {
        let node = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("node")).find(|p| p.is_file()));
        let Some(node) = node else {
            eprintln!("skipped: node is needed to parse the page's script");
            return;
        };
        let html = page();
        let scripts: Vec<&str> = regex::Regex::new(r"(?s)<script>(.*?)</script>").unwrap().captures_iter(&html).map(|c| c.get(1).unwrap().as_str()).collect();
        assert!(!scripts.is_empty(), "the page carries its script inline");
        for (i, js) in scripts.iter().enumerate() {
            let path = std::env::temp_dir().join(format!("bagholder-page-{}-{}.js", std::process::id(), i));
            std::fs::write(&path, js).unwrap();
            let r = std::process::Command::new(&node).arg("--check").arg(&path).output().unwrap();
            let _ = std::fs::remove_file(&path);
            assert!(r.status.success(), "script {} does not parse:\n{}", i, String::from_utf8_lossy(&r.stderr).chars().take(2000).collect::<String>());
        }
    }

    #[test]
    fn test_no_title_attribute_anywhere_on_the_page() {
        let found: Vec<String> = regex::Regex::new(r#" title=\\?["']"#).unwrap().find_iter(&page()).map(|m| m.as_str().to_string()).collect();
        assert_eq!(found, Vec::<String>::new(), "nothing on the page gets a browser tooltip");
    }

    #[test]
    fn test_protocol_matches_page() {
        let html = page();
        let m = regex::Regex::new(r#"const PROTOCOL = "([^"]+)""#).unwrap().captures(&html).expect("the page names its protocol");
        assert_eq!(&m[1], crate::app::PROTOCOL);
        // status_payload()["protocol"] is app::PROTOCOL by construction (see status_payload)
    }

    #[test]
    fn test_the_image_carries_the_page_and_its_chart_library() {
        let docker = std::fs::read_to_string(root().join("rust/Dockerfile")).unwrap();
        let copy = regex::Regex::new(r"^COPY\s+(.*?)\s+\./\s*$").unwrap();
        let copied: Vec<String> = docker.lines().filter_map(|l| copy.captures(l.trim()).map(|c| c[1].to_string())).flat_map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        assert!(!copied.is_empty(), "the Dockerfile copies the app in");
        for needed in SHIPPED {
            assert!(copied.iter().any(|c| c == needed), "{} is served by the app", needed);
        }
    }

    #[test]
    fn test_nothing_the_image_needs_is_kept_out_of_it() {
        let ignored: Vec<String> = std::fs::read_to_string(root().join(".dockerignore")).unwrap().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        for needed in SHIPPED.iter().copied().chain(["rust", "rust/crates", "rust/Cargo.toml", "rust/Cargo.lock", "rust/rust-toolchain.toml", "rust/docker-entrypoint.sh"]) {
            assert!(!ignored.iter().any(|i| i == needed), "{} kept out of the image", needed);
        }
    }

    #[test]
    fn test_the_release_archive_carries_the_page_and_its_assets() {
        let out = match std::process::Command::new("git").arg("-C").arg(root()).arg("ls-files").output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            _ => {
                eprintln!("skipped: not a git checkout");
                return;
            }
        };
        let tracked: Vec<&str> = out.lines().collect();
        for needed in SHIPPED {
            assert!(tracked.contains(&needed), "{} untracked: the release archive would not carry it", needed);
        }
    }

    #[test]
    fn test_the_image_builds_this_workspace_from_the_repository_root() {
        let docker = std::fs::read_to_string(root().join("rust/Dockerfile")).unwrap();
        for line in ["COPY rust/rust-toolchain.toml rust/Cargo.toml rust/Cargo.lock ./", "COPY rust/crates crates", "COPY rust/docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint"] {
            assert!(docker.lines().any(|l| l.trim() == line), "rust/Dockerfile: {}", line);
        }
    }

    #[test]
    fn test_the_python_app_carries_the_same_version_and_protocol() {
        // both desktop apps are one product version and speak one protocol with the page
        let Ok(py) = std::fs::read_to_string(root().join("python/bagholder.py")) else {
            eprintln!("skipped: no python/ beside rust/");
            return;
        };
        let version = regex::Regex::new(r#"(?m)^APP_VERSION = "([^"]+)""#).unwrap().captures(&py).expect("APP_VERSION in python/bagholder.py");
        let protocol = regex::Regex::new(r#"(?m)^PROTOCOL = "([^"]+)""#).unwrap().captures(&py).expect("PROTOCOL in python/bagholder.py");
        assert_eq!(&version[1], crate::app::APP_VERSION);
        assert_eq!(&protocol[1], crate::app::PROTOCOL);
    }

    #[test]
    fn test_the_server_serves_what_ships_from_its_root() {
        // the Rust server reads these from its root at request time rather than embedding them
        let src = include_str!("main.rs");
        for needed in SHIPPED {
            assert!(root().join(needed).is_file(), "{} beside the workspace", needed);
        }
        assert!(src.contains("static_file(\"lightweight-charts.js\")") && src.contains("static_file(\"favicon.png\")") && src.contains("feeds::ledger_path()"));
    }
}

#[cfg(test)]
mod tests_common;

#[cfg(test)]
mod tests_misc;


#[cfg(test)]
mod tests_brackets;

#[cfg(test)]
mod tests_orders;
