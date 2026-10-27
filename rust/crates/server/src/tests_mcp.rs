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
    app: Arc<App>,
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
