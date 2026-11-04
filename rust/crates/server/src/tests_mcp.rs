//! `bagholder mcp` reads the server holding its data folder, only reads, and
//! answers what the page answers (`docs/plans/agents-read-the-trade-history.md`).

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::{App, HOME_LOCK_FILE};
use crate::capacity::build;
use bagholder_wealthsimple::generated::Size;
use crate::http::{route_table, AppState};
use crate::mcp::{find, routes, Agents, Found, ReadOnly, TradesArgs};

/// A server on `home`'s book, answering on loopback, holding the folder's lock as
/// `app::hold_home` and `app::note_home_port` leave it.
pub(crate) struct Served {
    pub(crate) port: u16,
    pub(crate) app: Arc<App>,
    _lock: std::fs::File,
    _rt: tokio::runtime::Runtime,
}

fn served(home: &Path) -> Served {
    served_for(home)
}

/// A server on `home`'s book, for the agents' tests too.
pub(crate) fn served_for(home: &Path) -> Served {
    crate::tests_common::home();
    let app = App::new(home.to_path_buf(), std::path::PathBuf::from("."), "127.0.0.1".into());
    app.set_figures(crate::figures::Figures::open(home, bagholder_core::jiff::Timestamp::now()).unwrap());
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    *app.port.lock().unwrap() = port;
    rt.spawn(crate::http::serve(listener, AppState { app: app.clone() }));
    let mut lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(home.join(HOME_LOCK_FILE)).unwrap();
    lock.try_lock().unwrap();
    write!(lock, "pid {}, http://127.0.0.1:{port}", std::process::id()).unwrap();
    Served { port, app, _lock: lock, _rt: rt }
}

/// A generated book: accounts, currencies, listings, open and closed trades.
fn book(seed: u64) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    build(d.path(), Size { accounts: 4, instruments: 10, trades: 40, months: 4, seed, deep: 0 }, 2).unwrap();
    d
}

fn every_trade(a: &Agents, size: usize) -> (usize, Vec<Value>) {
    let mut out = Vec::new();
    let mut offset = Some(0);
    let mut total = 0;
    while let Some(o) = offset {
        let page = a.trades_page(TradesArgs { offset: Some(o), size: Some(size), ..TradesArgs::default() }).unwrap();
        total = page["total"].as_u64().unwrap() as usize;
        out.extend(page["trades"].as_array().unwrap().iter().cloned());
        offset = page["next"].as_u64().map(|n| n as usize);
    }
    (total, out)
}

/// Every route the read client names is a `GET` of the route table: a tool that
/// wrote would need a route the client cannot name.
#[test]
fn every_route_the_read_client_names_is_a_get() {
    let table = route_table();
    for path in routes() {
        assert!(table.iter().any(|e| e.method == "get" && e.path == path), "{path} is not a GET of the route table");
    }
}

/// Every read the page makes is an agent's too (`docs/decisions.md` 2026-11-02;
/// `docs/plans/agents-read-what-the-page-shows.md`): each `GET` of the route table
/// is one the read client names and a tool reads, or is on the short list below of
/// what is no reading of the book or the market; each document the page
/// subscribes to is one a tool asks for. A page read added without its agent tool
/// fails here.
#[test]
fn every_read_the_page_makes_is_an_agent_tool() {
    // not data an agent reads, each with why
    const NOT_DATA: [(&str, &str); 14] = [
        ("/api/agents/log", "the agents' own panel: what agents read, for the person"),
        ("/api/figures/trades", "the CSV export: the trades tool's rows, whole"),
        ("/api/watch", "the CSV import's folder: where the person's files come in, not what the book holds"),
        ("/api/filings/doc", "a filing's document as the page opens it: answered to agents as `filing_text`"),
        ("/api/events", "the page's live feed of the documents the tools read"),
        ("/api/notifications/stream", "the page's live feed of the notifications the `notifications` tool reads"),
        ("/api/login/stream", "the sign-in browser's frames, live: the page itself"),
        ("/api/login/frame", "the sign-in browser's frame: the page itself"),
        ("/", "the page itself"),
        ("/index.html", "the page itself"),
        ("/assets/{*path}", "the page's own files"),
        ("/favicon.png", "the page's own files"),
        ("/favicon.ico", "the page's own files"),
        ("/sw.js", "the page's own files"),
    ];
    let tools = include_str!("agents/tools.rs");
    let named = routes();
    for e in route_table().iter().filter(|e| e.method == "get") {
        if NOT_DATA.iter().any(|(p, _)| *p == e.path) {
            continue;
        }
        assert!(named.contains(&e.path), "{} is a page read the agents' read client does not name", e.path);
        let read = matches!(e.path, "/api/view" | "/api/status" | "/api/figures/detail" | "/api/transactions") || tools.contains(&format!("r.page(\"{}\"", e.path));
        assert!(read, "{} is named and no agent tool reads it", e.path);
    }
    // and each `GET` a router adds by hand, outside the route table: held to the same rule
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/http");
    let by_hand = regex::Regex::new(r#"\.route\("([^"]+)",\s*(?:axum::routing::)?get\("#).unwrap();
    let mut found = 0;
    for f in std::fs::read_dir(&dir).unwrap() {
        let f = f.unwrap().path();
        if f.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        for c in by_hand.captures_iter(&src) {
            let path = &c[1];
            found += 1;
            if NOT_DATA.iter().any(|(p, _)| *p == path) {
                continue;
            }
            assert!(named.contains(&path), "{path} ({}) is a GET added by hand that the agents' read client does not name", f.display());
            assert!(tools.contains(&format!("r.page(\"{path}\"")) || matches!(path, "/api/view" | "/api/status" | "/api/figures/detail" | "/api/transactions"), "{path} is named and no agent tool reads it");
        }
    }
    assert!(found >= 10, "the scan finds the routes added by hand: {found}");
    // every document the page subscribes to, as `views` builds them
    let views = include_str!("views.rs");
    let list = regex::Regex::new(r#"if !matches!\(kind, ([^)]*)\)"#).unwrap();
    let kinds: Vec<String> = regex::Regex::new(r#""([a-z]+)""#).unwrap().captures_iter(&list.captures(views).expect("the view kinds")[1]).map(|c| c[1].to_string()).collect();
    assert!(kinds.len() >= 10, "{kinds:?}");
    for k in kinds {
        let asked = if k == "trade" { tools.contains("format!(\"trade:{id}\")") } else { tools.contains(&format!("r.view(\"{k}\"")) };
        assert!(asked, "the page's {k} document is one no agent tool asks for");
    }
}

/// Every `POST` the page sends has its place (`docs/decisions.md` 2026-11-02): a
/// write, listed with what it changes; a step of the set-up of the person's own
/// access, which an agent is never given; the page's own plumbing; or a read, whose
/// `GET` of the same path the read client names and a tool reads, so a read the page
/// makes by `POST` is an agent's too. A `POST` added with no place here fails.
#[test]
fn every_post_the_page_sends_is_a_write_or_a_read_an_agent_has() {
    const ORDER: &str = "an order sent to Wealthsimple: an agent places none (SPEC agents-6)";
    const SIGN_IN: &str = "the Wealthsimple sign-in";
    const ENTRY: &str = "what the person enters, written to the book";
    const IMPORT: &str = "a file import: its rows written to the book";
    const MARKS: &str = "the notifications' marks and settings, written, or a test one sent";
    const SET_UP: &str = "the set-up of the person's own access (agents' tunnels, keys and sign-ins; Open from anywhere): steps on their own accounts, never the book or the market";
    const WRITES: &[(&str, &str)] = &[
        ("/api/order", ORDER),
        ("/api/order/cancel", ORDER),
        ("/api/order/modify", ORDER),
        ("/api/bracket/adjust", ORDER),
        ("/api/bracket/cancel", ORDER),
        ("/api/orders/refresh", "the orders read back from Wealthsimple now and written: the `orders` tool reads them"),
        ("/api/capture", SIGN_IN),
        ("/api/login/start", SIGN_IN),
        ("/api/login/input", SIGN_IN),
        ("/api/login/cancel", SIGN_IN),
        ("/api/disconnect", SIGN_IN),
        ("/api/sync", "Wealthsimple read now and the book written: the tools read what it brings"),
        ("/api/refresh", "the sources read now: the tools read what they bring"),
        ("/api/update", "a release installed"),
        ("/api/data/clear", "what the book holds, deleted"),
        ("/api/entries", ENTRY),
        ("/api/journal", ENTRY),
        ("/api/debts/answer", ENTRY),
        ("/api/tiles/set", ENTRY),
        ("/api/watchlist/add", ENTRY),
        ("/api/watchlist/remove", ENTRY),
        ("/api/import", IMPORT),
        ("/api/import/stop", IMPORT),
        ("/api/import/told", IMPORT),
        ("/api/watch", IMPORT),
        ("/api/watch/scan", IMPORT),
        ("/api/watch/clear", IMPORT),
        ("/api/notifications/read", MARKS),
        ("/api/notifications/seen", MARKS),
        ("/api/notifications/clear", MARKS),
        ("/api/notifications/settings", MARKS),
        ("/api/notifications/test", MARKS),
        ("/api/push/subscribe", MARKS),
        ("/api/push/unsubscribe", MARKS),
        ("/api/agents/check", SET_UP),
        ("/api/agents/cloudflare/sign-in", SET_UP),
        ("/api/agents/codes/resume", SET_UP),
        ("/api/agents/cut-off", SET_UP),
        ("/api/agents/install", SET_UP),
        ("/api/agents/keys", SET_UP),
        ("/api/agents/ngrok/token", SET_UP),
        ("/api/agents/rename", SET_UP),
        ("/api/agents/requests/deny", SET_UP),
        ("/api/agents/switch", SET_UP),
        ("/api/agents/tunnel", SET_UP),
        ("/api/agents/tunnel/look", SET_UP),
        ("/api/agents/tunnel/remove", SET_UP),
        ("/api/agents/tunnel/start", SET_UP),
        ("/api/agents/tunnel/stop", SET_UP),
        ("/api/remote/check", SET_UP),
        ("/api/remote/github", SET_UP),
        ("/api/remote/github/login", SET_UP),
        ("/api/remote/remove", SET_UP),
        ("/api/remote/set-up", SET_UP),
        ("/api/remote/switch", SET_UP),
        ("/api/remote/team", SET_UP),
        ("/api/remote/token", SET_UP),
    ];
    const NOT_DATA: &[(&str, &str)] = &[
        ("/api/events/watch", "the page naming the documents its live feed carries"),
        ("/api/events/resync", "the page asking its live feed for every document again"),
        ("/api/order/preview", "the ticket's arithmetic over the draft and the figures the page sends it: every figure it rests on is `order_quote`'s, and an agent drafts no order, placing none (SPEC agents-6)"),
    ];
    // a read the page makes by `POST`: the agents' is its `GET` of the same path
    const READ_BY_GET: &[(&str, &str)] = &[("/api/order/listing", "a listing named by symbol and venue is found by the GET, which writes nothing; the POST also opens the book's own instruments, and may record the id found on one")];
    let mut posts: Vec<String> = route_table().iter().filter(|e| e.method == "post").map(|e| e.path.to_string()).collect();
    // and each `POST` a router adds by hand, outside the route table
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/http");
    // (a `POST` on its own, or after the path's `GET`)
    let by_hand = regex::Regex::new(r#"\.route\("([^"]+)",\s*(?:(?:axum::routing::)?get\([^)]*\)\.)?(?:axum::routing::)?post\("#).unwrap();
    for f in std::fs::read_dir(&dir).unwrap() {
        let f = f.unwrap().path();
        if f.extension().is_some_and(|e| e == "rs") {
            posts.extend(by_hand.captures_iter(&std::fs::read_to_string(&f).unwrap()).map(|c| c[1].to_string()));
        }
    }
    assert!(posts.len() >= 55, "the POSTs are found: {}", posts.len());
    let (named, tools) = (routes(), include_str!("agents/tools.rs"));
    for path in &posts {
        let placed = WRITES.iter().chain(NOT_DATA).any(|(p, _)| p == path);
        let read = READ_BY_GET.iter().any(|(p, _)| p == path);
        assert!(placed != read, "{path} is a POST with no place, or two: a write, the page's own, or a read whose GET an agent tool reads");
        if read {
            assert!(named.contains(&path.as_str()), "{path}'s GET is not one the agents' read client names");
            assert!(tools.contains(&format!("r.page(\"{path}\"")), "{path}'s GET is named and no agent tool reads it");
        }
    }
    for (p, _) in WRITES.iter().chain(NOT_DATA).chain(READ_BY_GET) {
        assert!(posts.iter().any(|q| q == p), "{p} is listed and is no POST the page sends");
    }
}

/// A path the read client does not list is refused by name, before any request
/// is made: here no server answers at all, so a request made would say so instead.
#[test]
fn a_page_read_not_listed_is_refused_before_any_request() {
    let r = ReadOnly::own("http://127.0.0.1:1");
    for path in ["/api/figures/trades", "/api/watch", "/api/filings/doc"] {
        let said = r.page(path, &[]).unwrap_err();
        assert_eq!(said, format!("{path} is not a read an agent is given"));
    }
}

/// The adapter makes one request, a `GET`, through the one client; nothing else
/// in its module can send one.
#[test]
fn the_adapter_sends_one_request_a_get() {
    let src = include_str!("mcp.rs");
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert_eq!(code.matches("client::request(").count(), 1, "one request");
    assert!(code.contains("client::request(\"GET\""), "and it is a GET");
    for word in ["\"POST\"", "\"PUT\"", "\"PATCH\"", "\"DELETE\"", "client::send", "client::download", "client::leave", "reqwest", "hyper::", "TcpStream", "browser::Session", "Book::open", "hold_home"] {
        assert!(!code.contains(word), "the adapter's module names {word}");
    }
}

/// Which server: one holding the folder's lock answers at the address it wrote;
/// one still starting (a pid, no address) is not guessed at; a lock no process
/// holds is not trusted, whatever it says; a folder with no lock file is no
/// server, and none is made. The lock is left free either way.
#[test]
fn the_server_is_the_one_holding_the_folders_lock() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(HOME_LOCK_FILE);
    assert_eq!(find(home.path()), Found::NotRunning);
    assert!(!path.exists(), "no lock file is made");
    // written by a server that is gone: an address, and no lock held
    std::fs::write(&path, "pid 1, http://127.0.0.1:9").unwrap();
    assert_eq!(find(home.path()), Found::NotRunning, "a lock no process holds is not trusted");
    let mut held = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
    held.try_lock().unwrap();
    held.set_len(0).unwrap();
    write!(held, "pid {}", std::process::id()).unwrap();
    assert_eq!(find(home.path()), Found::Starting, "a pid and no address yet");
    held.set_len(0).unwrap();
    use std::io::Seek;
    held.rewind().unwrap();
    write!(held, "pid {}, http://127.0.0.1:8123", std::process::id()).unwrap();
    assert_eq!(find(home.path()), Found::Running("http://127.0.0.1:8123".into()));
    drop(held);
    // and nothing the adapter did holds it now
    let f = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
    assert!(f.try_lock().is_ok(), "the folder's lock is left free");
}

/// With no server, each tool says so in words, and makes nothing.
#[test]
fn with_no_server_each_tool_says_bagholder_is_not_running() {
    let home = tempfile::tempdir().unwrap();
    let a = Agents::new(home.path().to_path_buf());
    let said = a.trades_page(TradesArgs::default()).unwrap_err();
    assert!(said.contains("not running"), "{said}");
    assert!(a.one_trade("x").unwrap_err().contains("not running"));
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0, "nothing made in the folder");
}

/// The holdings an agent reads are the page's own, field for field, less the net
/// worth's daily history: the Portfolio chart's series, read by no agent asking for
/// the holdings, and nearly all of the answer's size.
#[test]
fn the_holdings_are_the_page_s_without_the_net_worth_s_history() {
    let home = book(3);
    let s = served(home.path());
    let read = ReadOnly::at(&format!("http://127.0.0.1:{}", s.port)).unwrap();
    let page = read.view("positions", json!({ "filters": null })).unwrap();
    assert!(page.pointer("/portfolio/netWorth/history").and_then(Value::as_array).is_some_and(|h| !h.is_empty()), "the page's answer has a history, so the test shows its removal");
    let mine = crate::agents::tools::positions(&read, crate::agents::tools::PositionsArgs::default()).unwrap();
    assert!(mine.pointer("/portfolio/netWorth/history").is_none());
    let mut want = page.clone();
    want.pointer_mut("/portfolio/netWorth").unwrap().as_object_mut().unwrap().remove("history");
    assert_eq!(mine, want, "everything else, field for field");
}

/// Two servers on two folders: each adapter reads its own folder's book; each
/// page of trades is the page's own answer for the same filters, field for
/// field; paging returns every trade once, with the Trades list's total; each
/// trade carries its listing, one per instrument.
#[test]
fn each_adapter_reads_its_own_folders_book_as_the_page_does() {
    let (one, two) = (book(1), book(2));
    let (s1, s2) = (served(one.path()), served(two.path()));
    // one ticker on two exchanges, each a listing of its own
    let symbol = "TWOV".to_string();
    let f = s1.app.figures.get().unwrap();
    for (exchange, currency) in [("XTSE", "CAD"), ("XNAS", "USD")] {
        for (day, side) in [("2025-06-02", "buy"), ("2025-06-03", "sell")] {
            let req = crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: symbol.clone(), exchange: exchange.into(), currency: currency.into(), day: day.into(), side: side.into(), quantity: "1".into(), price: "10".into(), fee: "0".into() };
            crate::entries::enter(f, &req, bagholder_core::jiff::Timestamp::now()).unwrap();
        }
    }
    for (home, s, other) in [(one.path(), &s1, &s2), (two.path(), &s2, &s1)] {
        let a = Agents::new(home.to_path_buf());
        let (total, rows) = every_trade(&a, 7);
        // the page's own answer, all of it at once
        let page = ReadOnly::at(&format!("http://127.0.0.1:{}", s.port)).unwrap().view("trades", json!({ "filters": null, "limit": total.max(1) })).unwrap();
        assert_eq!(total as u64, page["total"].as_u64().unwrap(), "the Trades list's total");
        let theirs = page["trades"].as_array().unwrap();
        assert_eq!(rows.len(), total, "every trade, once");
        let ids: std::collections::BTreeSet<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), total, "no trade twice");
        for (mine, page_row) in rows.iter().zip(theirs) {
            let mut m = mine.clone();
            let listing = m.as_object_mut().unwrap().remove("listing");
            assert_eq!(&m, page_row, "field for field");
            let want = bagholder_core::ListingKey::on(page_row["symbol"].as_str().unwrap(), page_row["exchange"].as_str().unwrap()).unwrap().to_string();
            assert_eq!(listing, Some(json!(want)), "its listing");
        }
        // one listing per instrument, and two for a ticker on two venues
        let mut by_instrument: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
        for r in &rows {
            let (i, l) = (r["instrument"].as_str().unwrap(), r["listing"].as_str().unwrap());
            assert_eq!(*by_instrument.entry(i).or_insert(l), l, "an instrument is one listing");
        }
        if home == one.path() {
            let of_ticker: std::collections::BTreeSet<&str> = rows.iter().filter(|r| r["symbol"] == json!(symbol)).map(|r| r["listing"].as_str().unwrap()).collect();
            assert_eq!(of_ticker.len(), 2, "{symbol} on two venues is two listings: {of_ticker:?}");
        }
        // not the other folder's book
        let other_ids: std::collections::BTreeSet<String> = {
            let o = ReadOnly::at(&format!("http://127.0.0.1:{}", other.port)).unwrap().view("trades", json!({ "limit": 100000 })).unwrap();
            o["trades"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
        };
        assert!(ids.iter().all(|i| !other_ids.contains(*i)), "an adapter read the other folder's book");
        // one trade, with its fills, and the holdings, as the page has them
        let first = rows[0]["id"].as_str().unwrap();
        let t = a.one_trade(first).unwrap();
        assert_eq!(t["trade"]["id"].as_str(), Some(first), "{t}");
        assert!(t["fills"].as_array().is_some_and(|f| !f.is_empty()), "its executions");
        let _ = &s.app;
    }
}
