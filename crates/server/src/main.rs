//! The Bagholder server.
//!
//! Ported from `bagholder.py`. What is here so far is the read side: the page
//! and its assets, the model behind them, and the status the header shows.
//! The Wealthsimple client, the sync and the order routes are still Python's.
//!
//! The gate is the same one the Python server applies: loopback only unless it
//! was bound elsewhere on purpose, the Host header has to name 127.0.0.1 and
//! the port, and a write has to come from the page itself.

mod notify;
mod versions;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use tiny_http::{Header, Request, Response, Server};

use bagholder_model::base::Base;
use bagholder_model::value::field_s;

/// `bagholder.APP_VERSION`.
const APP_VERSION: &str = "1.42.0";
/// `bagholder.PROTOCOL`: bumped whenever the page and the server change
/// together, so the page says to restart rather than degrading quietly.
const PROTOCOL: &str = "2026-09-16.1";

const DEFAULT_PORT: u16 = 8765;

struct Cache {
    version: String,
    base: Option<Base>,
}

struct App {
    home: PathBuf,
    root: PathBuf,
    bind_host: String,
    port: u16,
    started_at: String,
    cache: Mutex<Cache>,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("BAGHOLDER_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&base).join(".bagholder")
}

impl App {
    fn db_path(&self) -> PathBuf {
        self.home.join("bagholder.db")
    }

    fn open(&self) -> rusqlite::Result<rusqlite::Connection> {
        let conn = rusqlite::Connection::open(self.db_path())?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        bagholder_store::relabel::ensure(&conn)?;
        Ok(conn)
    }

    /// `model.base_model`: today is part of the key, because a YTD tile and
    /// the current year's return must roll over at midnight even when nothing
    /// in the database has changed.
    fn base_for(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        let today = bagholder_model::clock::today_local();
        let version = format!("{}|{}", versions::data_version(conn)?, today);
        {
            let cache = self.cache.lock().unwrap();
            if cache.base.is_some() && cache.version == version {
                return Ok(());
            }
        }
        let snapshot = bagholder_store::snapshot::snapshot(conn, true)?;
        let market = bagholder_store::market::market_data(conn)?;
        let journal = bagholder_store::snapshot::journal(conn)?;
        let base = bagholder_model::base::build_base(&snapshot, &market, &journal, Some(&today));
        let mut cache = self.cache.lock().unwrap();
        cache.version = version;
        cache.base = Some(base);
        Ok(())
    }

    fn status_payload(&self, conn: &rusqlite::Connection) -> rusqlite::Result<Value> {
        let (acts, accounts, synced) = versions::status_counts(conn)?;
        let today = bagholder_model::clock::today_local();
        Ok(json!({
            "ok": true,
            // the Wealthsimple session is still Python's, so nothing here is connected yet
            "connected": false,
            "email": "",
            "lastSync": synced,
            "activityCount": acts,
            "accountCount": accounts,
            "capturing": false,
            "syncing": false,
            "listingsFilling": false,
            "syncStep": "",
            "error": "",
            "dataVersion": format!("{}|{}", versions::data_version(conn)?, today),
            "summaryReady": false,
            "protocol": PROTOCOL,
            "startedAt": self.started_at,
            "version": APP_VERSION,
            "latestVersion": "",
            "updateAvailable": false,
            "updateUrl": "https://github.com/ProfessorBagholder/Bagholder",
            "canUpdate": false,
            "updateBy": "app",
            "loginView": "",
            "ordersLive": false,
            "openOrders": 0,
            "updating": "",
            "updateError": "",
            "notify": json!({}),
        }))
    }
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

/// `Handler._local`.
fn is_local(app: &App, req: &Request) -> bool {
    if app.bind_host != "127.0.0.1" {
        // bound beyond loopback on purpose (a container); the peer is its bridge
        return true;
    }
    match req.remote_addr() {
        Some(addr) => {
            let ip = addr.ip().to_string();
            ip == "127.0.0.1" || ip == "::1"
        }
        None => false,
    }
}

/// `Handler._host_ok`: the name must be 127.0.0.1 and the port this server's,
/// so a page served from anywhere else cannot reach the API.
fn host_ok(app: &App, req: &Request) -> bool {
    let raw = header(req, "Host").trim().to_lowercase();
    if raw.is_empty() || raw.contains(',') {
        return false;
    }
    if app.bind_host != "127.0.0.1" {
        // a container's port may be published under another number
        let (name, port) = match raw.split_once(':') { Some(p) => p, None => return false };
        return name == "127.0.0.1" && !port.is_empty() && port.len() <= 5 && port.bytes().all(|c| c.is_ascii_digit());
    }
    raw == format!("127.0.0.1:{}", app.port)
}

/// `Handler._write_ok`.
fn write_ok(req: &Request) -> bool {
    let site = header(req, "Sec-Fetch-Site").trim().to_lowercase();
    if site == "same-origin" {
        return true;
    }
    !header(req, "X-Bagholder").trim().is_empty()
}

fn gate(app: &App, req: &Request, write: bool) -> bool {
    if !is_local(app, req) || !host_ok(app, req) {
        return false;
    }
    !(write && !write_ok(req))
}

// --------------------------------------------------------------------------
// responses
// --------------------------------------------------------------------------

fn hdr(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

/// `Handler._send`.
fn send(req: Request, code: u16, body: Vec<u8>, content_type: &str) {
    let response = Response::from_data(body)
        .with_status_code(code)
        .with_header(hdr("Content-Type", content_type))
        .with_header(hdr("Cache-Control", "no-store"))
        .with_header(hdr("X-Content-Type-Options", "nosniff"))
        .with_header(hdr("Referrer-Policy", "no-referrer"));
    let _ = req.respond(response);
}

fn send_json(req: Request, code: u16, body: &Value) {
    send(req, code, serde_json::to_vec(body).unwrap_or_default(), "application/json; charset=utf-8");
}

fn forbidden(req: Request) {
    send_json(req, 403, &json!({"ok": false}));
}

fn not_found(req: Request) {
    send_json(req, 404, &json!({"ok": false, "error": "not found"}));
}

fn query_of(url: &str) -> &str {
    url.split_once('?').map(|(_, q)| q).unwrap_or("")
}

fn path_of(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// One parameter out of a query string, percent-decoded.
fn query_param(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(k) == name {
            let v = percent_decode(v);
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => { out.push(b' '); i += 1 }
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => { out.push(v); i += 3 }
                    Err(_) => { out.push(b[i]); i += 1 }
                }
            }
            c => { out.push(c); i += 1 }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// `bagholder._model_filters`: an unreadable filter object is no filter, not
/// an error.
fn model_filters(query: &str) -> Option<Value> {
    let raw = query_param(query, "filters")?;
    serde_json::from_str(&raw).ok()
}

fn static_file(app: &App, name: &str) -> Option<Vec<u8>> {
    std::fs::read(app.root.join(name)).ok()
}

// --------------------------------------------------------------------------
// routing
// --------------------------------------------------------------------------

fn handle(app: &App, req: Request) {
    let url = req.url().to_string();
    let path = path_of(&url).to_string();
    let query = query_of(&url).to_string();
    let method = req.method().as_str().to_string();

    if method == "POST" {
        if !gate(app, &req, true) {
            forbidden(req);
            return;
        }
        handle_post(app, req, &path);
        return;
    }
    if method != "GET" {
        send_json(req, 404, &json!({"ok": false, "error": "not found"}));
        return;
    }
    if !gate(app, &req, false) {
        forbidden(req);
        return;
    }

    match path.as_str() {
        "/" | "/index.html" | "/ledger.html" | "/v2" | "/v2/" => match static_file(app, "ledger.html") {
            Some(data) => send(req, 200, data, "text/html; charset=utf-8"),
            None => send_json(req, 404, &json!({"ok": false, "error": "ledger.html missing"})),
        },
        "/lightweight-charts.js" => match static_file(app, "lightweight-charts.js") {
            Some(data) => send(req, 200, data, "text/javascript; charset=utf-8"),
            None => not_found(req),
        },
        "/favicon.png" => match static_file(app, "favicon.png") {
            Some(data) => send(req, 200, data, "image/png"),
            None => not_found(req),
        },
        "/api/status" => {
            let conn = match app.open() { Ok(c) => c, Err(e) => return fail(req, e) };
            match app.status_payload(&conn) {
                Ok(p) => send_json(req, 200, &p),
                Err(e) => fail(req, e),
            }
        }
        "/api/model" => {
            let conn = match app.open() { Ok(c) => c, Err(e) => return fail(req, e) };
            if let Err(e) = app.base_for(&conn) {
                return fail(req, e);
            }
            let status = match app.status_payload(&conn) { Ok(s) => s, Err(e) => return fail(req, e) };
            let cache = app.cache.lock().unwrap();
            let base = cache.base.as_ref().expect("base built above");
            let full = bagholder_model::view::build_view(base, model_filters(&query).as_ref());
            // the legs and fills of the one trade the page has open, and no
            // other: sending every leg on every poll is most of the payload
            let detail = query_param(&query, "trade");
            let mut payload = bagholder_model::view::slim(&full, detail.as_deref());
            if let Value::Object(m) = &mut payload {
                m.insert("status".into(), status);
            }
            drop(cache);
            send_json(req, 200, &payload);
        }
        "/api/trade" => {
            // the legs and fills of one trade or holding, fetched when its page opens
            let conn = match app.open() { Ok(c) => c, Err(e) => return fail(req, e) };
            if let Err(e) = app.base_for(&conn) {
                return fail(req, e);
            }
            let id = query_param(&query, "trade").or_else(|| query_param(&query, "id"));
            let cache = app.cache.lock().unwrap();
            let base = cache.base.as_ref().expect("base built above");
            let found = id.as_deref().and_then(|i| bagholder_model::view::trade_detail(base, i));
            drop(cache);
            match found {
                Some(mut d) => {
                    if let Value::Object(m) = &mut d {
                        m.insert("ok".into(), json!(true));
                    }
                    send_json(req, 200, &d)
                }
                None => send_json(req, 404, &json!({"ok": false, "error": "no such trade"})),
            }
        }
        "/api/orders" => with_conn(app, req, |conn| {
            let securities = bagholder_store::admin::list_securities(conn)?;
            let mut orders = bagholder_store::orders::list_orders(conn, 200)?;
            for o in orders.iter_mut() {
                let sid = field_s(o, "securityId");
                let exch = securities
                    .iter()
                    .find(|s| field_s(s, "id") == sid)
                    .map(|s| field_s(s, "primaryExchange"))
                    .unwrap_or_default();
                if let Value::Object(m) = o {
                    m.insert("exchange".into(), json!(exch));
                }
            }
            Ok(json!({
                "ok": true,
                "orders": orders,
                "brackets": bagholder_store::orders::list_brackets(conn, &[])?,
                // orders are not placed from here yet; see the README
                "live": false,
                "refreshedAt": "",
            }))
        }),
        "/api/notifications" => with_conn(app, req, |conn| {
            Ok(json!({
                "ok": true,
                "settings": notify::status(conn)?,
                "kinds": notify::KINDS,
                "rows": bagholder_store::feeds::list_notifications(conn, 0, "", false, 50, true)?,
                "unread": bagholder_store::feeds::unread_notifications(conn)?,
            }))
        }),
        "/api/filings" => {
            let sym = query_param(&query, "symbol").unwrap_or_default();
            with_conn(app, req, move |conn| {
                Ok(json!({
                    "ok": true,
                    "symbol": bagholder_store::feeds::filing_key(&sym),
                    "filings": bagholder_store::feeds::filings_for(conn, &sym)?,
                    "fetchedAt": bagholder_store::feeds::filings_fetched_for(conn, &sym)?,
                    "profileNo": bagholder_store::feeds::sedar_profile(conn, &sym)?,
                }))
            })
        }
        "/api/shorts" => {
            let sym = query_param(&query, "symbol").unwrap_or_default();
            let ex = query_param(&query, "exchange").unwrap_or_default();
            with_conn(app, req, move |conn| {
                match bagholder_store::feeds::shorts_for(conn, &sym, &ex)? {
                    Some(row) => Ok(json!({"ok": true, "shorts": row})),
                    None => Ok(json!({"ok": false, "error": "nothing stored for that listing"})),
                }
            })
        }
        "/api/shorts/feed" => with_conn(app, req, |conn| {
            Ok(json!({"ok": true, "rows": bagholder_store::feeds::all_shorts(conn)?}))
        }),
        "/api/news/symbol" => {
            let sym = query_param(&query, "symbol").unwrap_or_default();
            let ex = query_param(&query, "exchange").unwrap_or_default();
            with_conn(app, req, move |conn| {
                Ok(json!({
                    "ok": true,
                    "ids": bagholder_store::feeds::news_ids(conn, &sym, &ex)?,
                    "fetchedAt": bagholder_store::feeds::news_fetched_at(conn)?,
                }))
            })
        }
        "/api/fear" => {
            let which = query_param(&query, "index").unwrap_or_else(|| "stocks".into()).to_lowercase();
            with_conn(app, req, move |conn| {
                // reading a fresh one needs fear.py, which is not ported; what
                // has already been read is answered from the store
                match bagholder_store::feeds::gauge(conn, &which)? {
                    Some(g) if !g.get("score").map(|s| s.is_null()).unwrap_or(true) => {
                        Ok(json!({"ok": true, "gauge": g}))
                    }
                    _ => Ok(json!({"ok": false, "error": "the index did not answer"})),
                }
            })
        }
        "/api/data" => {
            let conn = match app.open() { Ok(c) => c, Err(e) => return fail(req, e) };
            match data_summary(&conn, &app.db_path().display().to_string()) {
                Ok(mut s) => {
                    if let Value::Object(m) = &mut s {
                        m.insert("ok".into(), json!(true));
                        // the Wealthsimple session is still Python's
                        m.insert("sessionPresent".into(), json!(false));
                    }
                    send_json(req, 200, &s)
                }
                Err(e) => fail(req, e),
            }
        }
        _ => not_found(req),
    }
}

/// `store.data_summary`: the row counts the Data & storage dialog shows before
/// a wipe.
fn data_summary(conn: &rusqlite::Connection, path: &str) -> rusqlite::Result<Value> {
    let count = |sql: &str| -> rusqlite::Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
    let journal_raw = bagholder_store::tables::get_meta(conn, "journal_v2", "")?;
    let journal_n = serde_json::from_str::<Value>(&journal_raw)
        .ok()
        .and_then(|v| v.as_object().map(|m| m.len()))
        .unwrap_or(0);
    let (first, last): (Option<String>, Option<String>) = conn.query_row(
        "SELECT MIN(transaction_date), MAX(transaction_date) FROM activities",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(json!({
        "path": path,
        "activities": count("SELECT COUNT(*) FROM activities")?,
        "firstActivity": first.unwrap_or_default(),
        "lastActivity": last.unwrap_or_default(),
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

/// Opens the store, runs one reader, and answers with what it returned.
fn with_conn<F>(app: &App, req: Request, f: F)
where
    F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<Value>,
{
    let conn = match app.open() { Ok(c) => c, Err(e) => return fail(req, e) };
    match f(&conn) {
        Ok(v) => send_json(req, 200, &v),
        Err(e) => fail(req, e),
    }
}

/// The write routes that only touch the store. Anything that would reach
/// Wealthsimple is not here: the session, the sync and the order routes are
/// still Python's, and they answer 404 rather than pretending.
fn handle_post(app: &App, mut req: Request, path: &str) {
    let mut body = Vec::new();
    let _ = std::io::Read::read_to_end(req.as_reader(), &mut body);
    let doc: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let now = now_iso();

    match path {
        "/api/journal" => {
            let id = field_s(&doc, "id").trim().to_string();
            if id.is_empty() {
                send_json(req, 400, &json!({"ok": false, "error": "id required"}));
                return;
            }
            let entry = json!({
                "thesis": doc.get("thesis").cloned().unwrap_or(Value::Null),
                "tags": doc.get("tags").cloned().unwrap_or(Value::Null),
                "grade": doc.get("grade").cloned().unwrap_or(Value::Null),
            });
            with_conn(app, req, move |conn| {
                let entries = bagholder_store::admin::save_journal_entry(conn, &id, Some(&entry))?;
                Ok(json!({"ok": true, "journal": entries}))
            });
        }
        "/api/groups" => with_conn(app, req, move |conn| {
            let groups = bagholder_store::tables::save_trade_groups(conn, doc.get("groups"))?;
            Ok(json!({"ok": true, "groups": groups}))
        }),
        "/api/notes" => with_conn(app, req, move |conn| {
            let notes = bagholder_store::tables::save_trade_notes(conn, doc.get("notes"))?;
            Ok(json!({"ok": true, "notes": notes}))
        }),
        "/api/watchlist/add" => {
            // the bare ticker: Wealthsimple's `.TO` on a dual listing is not
            // the app's convention
            let sym = bagholder_model::venues::tmx_symbol(&field_s(&doc, "symbol"));
            if sym.is_empty() {
                send_json(req, 200, &json!({"ok": false, "error": "symbol required"}));
                return;
            }
            let ex = field_s(&doc, "exchange");
            let inst = bagholder_model::instruments::find(&sym, &ex);
            let name = inst.map(|i| i.name.to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| field_s(&doc, "name"));
            let ccy = inst.map(|i| i.currency.to_string()).filter(|c| !c.is_empty()).unwrap_or_else(|| field_s(&doc, "currency"));
            let sid = field_s(&doc, "securityId");
            with_conn(app, req, move |conn| {
                bagholder_store::feeds::add_watch(conn, &sym, &ex, &name, &ccy, &sid, &now)?;
                Ok(json!({"ok": true, "watchlist": bagholder_store::feeds::list_watchlist(conn)?}))
            });
        }
        "/api/watchlist/remove" => {
            let raw = field_s(&doc, "symbol");
            let sym = bagholder_model::venues::tmx_symbol(&raw);
            if sym.is_empty() {
                send_json(req, 200, &json!({"ok": false, "error": "symbol required"}));
                return;
            }
            let ex = field_s(&doc, "exchange");
            with_conn(app, req, move |conn| {
                bagholder_store::feeds::remove_watch(conn, &sym, &ex)?;
                // a row kept under Wealthsimple's own form of the ticker
                bagholder_store::feeds::remove_watch(conn, raw.trim(), &ex)?;
                bagholder_store::feeds::forget_news(conn, &sym, &ex)?;
                Ok(json!({"ok": true, "watchlist": bagholder_store::feeds::list_watchlist(conn)?}))
            });
        }
        "/api/tiles/set" => {
            // only instruments the directory knows, twelve at most
            let mut rows: Vec<Value> = Vec::new();
            let mut seen: Vec<&str> = Vec::new();
            if let Some(list) = doc.get("tiles").and_then(|v| v.as_array()) {
                for r in list {
                    if let Some(i) = bagholder_model::instruments::find(&field_s(r, "symbol"), &field_s(r, "exchange")) {
                        if !seen.contains(&i.symbol) {
                            seen.push(i.symbol);
                            rows.push(json!({"symbol": i.symbol, "exchange": i.exchange}));
                        }
                    }
                }
            }
            if rows.len() > 12 {
                send_json(req, 200, &json!({"ok": false, "error": "at most 12 tiles"}));
                return;
            }
            with_conn(app, req, move |conn| {
                let tiles = bagholder_store::admin::save_tiles(conn, &rows)?;
                Ok(json!({"ok": true, "tiles": tiles}))
            });
        }
        "/api/notifications/read" => {
            let ids: Option<Vec<i64>> = doc.get("ids").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect());
            with_conn(app, req, move |conn| {
                let n = bagholder_store::feeds::mark_notifications_read(conn, ids.as_deref(), &now)?;
                Ok(json!({"ok": true, "read": n}))
            });
        }
        "/api/notifications/seen" => {
            let ids: Vec<i64> = doc.get("ids").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            with_conn(app, req, move |conn| {
                let n = bagholder_store::feeds::mark_notifications_seen(conn, &ids, &now)?;
                Ok(json!({"ok": true, "seen": n}))
            });
        }
        "/api/notifications/clear" => with_conn(app, req, |conn| {
            Ok(json!({"ok": true, "cleared": bagholder_store::feeds::clear_notifications(conn)?}))
        }),
        "/api/notifications/settings" => with_conn(app, req, move |conn| {
            let saved = notify::save_settings(conn, &doc)?;
            Ok(json!({"ok": true, "settings": saved}))
        }),
        _ => send_json(req, 404, &json!({"ok": false, "error": "not ported"})),
    }
}

fn fail(req: Request, e: rusqlite::Error) {
    eprintln!("bagholder: {}", e);
    send_json(req, 500, &json!({"ok": false, "error": "store failed"}));
}

fn main() {
    let home = home_dir();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let bind_host = env_or("BAGHOLDER_BIND", "127.0.0.1");
    let port: u16 = env_or("BAGHOLDER_PORT", &DEFAULT_PORT.to_string()).parse().unwrap_or(DEFAULT_PORT);
    let started_at = now_iso();

    let app = App {
        home,
        root,
        bind_host: bind_host.clone(),
        port,
        started_at,
        cache: Mutex::new(Cache { version: String::new(), base: None }),
    };

    let addr = format!("{}:{}", bind_host, port);
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bagholder: cannot bind {}: {}", addr, e);
            std::process::exit(1);
        }
    };
    eprintln!("bagholder {} on http://{}/  (db: {})", APP_VERSION, addr, app.db_path().display());

    let app = std::sync::Arc::new(app);
    for req in server.incoming_requests() {
        let app = app.clone();
        std::thread::spawn(move || handle(&app, req));
    }
}

fn now_iso() -> String {
    // the started-at stamp the page compares across a restart
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Kept so the reader type is used even before the write routes land.
#[allow(dead_code)]
fn body_of(req: &mut Request) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = std::io::Read::read_to_end(req.as_reader(), &mut buf);
    let _ = Cursor::new(&buf);
    buf
}
