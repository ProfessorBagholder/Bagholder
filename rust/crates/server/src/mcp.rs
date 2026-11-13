//! `bagholder mcp`: the trade history read by an AI agent on this machine, over
//! MCP on standard input and output (`docs/plans/agents-read-the-trade-history.md`).
//!
//! The adapter holds no data of its own. It reads the server already running on
//! the data folder, through that server's own `GET` routes, so what an agent is
//! told is what the page shows. It can only read: the one request it makes is a
//! `GET` of a route its read client names (`ReadOnly`), a `GetRoute` is made only
//! from a handler that takes no body, and the tests hold every route it names to
//! the route table's `GET`s. It never starts a server or opens the book, and
//! holds the data folder's lock for no longer than it takes to see whether a
//! server holds it.

use std::path::{Path, PathBuf};

use rmcp::ServiceExt;
use serde::Serialize;
use serde_json::Value;

#[cfg(test)]
pub use crate::agents::tools::TradesArgs;

use crate::http::model::{TradeQuery, ViewQuery};
use crate::http::{GetRoute, NoQuery};

/// How the server holding a data folder stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// It answers at this address.
    Running(String),
    /// It holds the folder but has not said where it answers yet.
    Starting,
    /// No server holds the folder.
    NotRunning,
}

/// The server holding `home`, read from its lock (`app::hold_home`): a server is
/// running only while the lock is held, which this tries without waiting and, if
/// it gets it, lets go of at once. A pid can be reused; a held lock cannot be
/// left behind. The address is the one the server wrote once it had bound
/// (`app::note_home_port`); one that has not written it yet is starting.
pub fn find(home: &Path) -> Found {
    use std::io::Read;
    let path = home.join(crate::app::HOME_LOCK_FILE);
    // never made: a folder no server has held
    let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { return Found::NotRunning };
    match f.try_lock() {
        // no server holds it: let go at once, so a server starting now takes it
        Ok(()) => {
            drop(f);
            return Found::NotRunning;
        }
        Err(std::fs::TryLockError::WouldBlock) => {}
        Err(std::fs::TryLockError::Error(_)) => return Found::NotRunning,
    }
    let mut text = String::new();
    if f.read_to_string(&mut text).is_err() {
        return Found::Starting;
    }
    match text.split_once(", ").map(|(_, at)| at.trim()) {
        Some(at) if at.starts_with("http://127.0.0.1:") => Found::Running(at.to_string()),
        _ => Found::Starting,
    }
}

/// The routes the adapter reads: each a `GET` of the server's interface, with
/// the query it takes and the answer it gives. The only way to a request.
pub struct ReadOnly {
    base: String,
}

/// The routes `ReadOnly` reads, by path: what the tests hold to the route table.
#[cfg(test)]
pub fn routes() -> Vec<&'static str> {
    let mut all = vec![status().path, view().path, detail().path, transactions().path];
    all.extend(page_reads().into_iter().map(|(path, _)| path));
    all
}

/// The page's other reads an agent is given (`docs/plans/agents-read-what-the-page-shows.md`),
/// each by its path, as the page asks it: what `ReadOnly::page` reads.
fn page_reads() -> Vec<(&'static str, &'static str)> {
    use crate::http::{markets as m, model, notifications, orders};
    vec![
        (GetRoute::of(m::news_symbol, "/api/news/symbol").path, "news_symbol"),
        (GetRoute::of(m::filings, "/api/filings").path, "filings"),
        (GetRoute::of(m::filings_feed, "/api/filings/feed").path, "filings_feed"),
        (GetRoute::of(m::filings_enrich, "/api/filings/enrich").path, "filings_enrich"),
        (GetRoute::of(m::filings_text, "/api/filings/text").path, "filings_text"),
        (GetRoute::of(m::shorts, "/api/shorts").path, "shorts"),
        (GetRoute::of(m::shorts_feed, "/api/shorts/feed").path, "shorts_feed"),
        (GetRoute::of(m::options, "/api/options").path, "options"),
        (GetRoute::of(model::tax, "/api/tax").path, "tax"),
        (GetRoute::of(m::option_contract, "/api/options/contract").path, "option_contract"),
        (GetRoute::of(m::fear, "/api/fear").path, "fear"),
        (GetRoute::of(m::history, "/api/history").path, "history"),
        (GetRoute::of(m::symbol_quote, "/api/symbols/quote").path, "symbol_quote"),
        (GetRoute::of(m::listing, "/api/listing").path, "listing"),
        (GetRoute::of(m::listings, "/api/listings").path, "listings"),
        (GetRoute::of(m::symbol_search, "/api/symbols/search").path, "symbol_search"),
        (GetRoute::of(orders::list, "/api/orders").path, "orders"),
        (GetRoute::of(orders::quote, "/api/order/quote").path, "order_quote"),
        (GetRoute::of(orders::listing_named, "/api/order/listing").path, "order_quote"),
        (GetRoute::of(notifications::list, "/api/notifications").path, "notifications"),
        (GetRoute::of(model::figures_details, "/api/figures/details").path, "figures_details"),
    ]
}

fn status() -> GetRoute<NoQuery, crate::status::StatusAnswer> {
    GetRoute::of(crate::http::model::status, "/api/status")
}

fn view() -> GetRoute<ViewQuery, crate::http::model::ViewAnswer> {
    GetRoute::of(crate::http::model::view, "/api/view")
}

fn detail() -> GetRoute<TradeQuery, crate::wire::figures::Detail> {
    GetRoute::of(crate::http::model::figures_detail, "/api/figures/detail")
}

fn transactions() -> GetRoute<crate::http::model::TransactionsQuery, crate::http::model::TransactionsAnswer> {
    GetRoute::of(crate::http::model::transactions, "/api/transactions")
}

impl ReadOnly {
    /// The server at `base`, once its status answers: an address in a lock is
    /// trusted only once the server there says so.
    pub fn at(base: &str) -> Result<ReadOnly, String> {
        let r = ReadOnly { base: base.to_string() };
        r.read(&status(), &NoQuery {})?;
        Ok(r)
    }

    /// The server this process runs, at its own address: the agents' listener's
    /// reads (`agents`), which need no lock to find it.
    pub fn own(base: &str) -> ReadOnly {
        ReadOnly { base: base.to_string() }
    }

    /// The one request: a `GET` of `route` with `query`, its answer as the server
    /// sent it.
    fn read<Q: Serialize, A>(&self, route: &GetRoute<Q, A>, query: &Q) -> Result<Value, String> {
        self.get(route.path, query)
    }

    /// A `GET` of `path`, one of the routes above, with `query`.
    fn get(&self, path: &str, query: &impl Serialize) -> Result<Value, String> {
        let q = serde_urlencoded::to_string(query).map_err(|e| format!("the query could not be written: {e}"))?;
        let url = if q.is_empty() { format!("{}{}", self.base, path) } else { format!("{}{}?{q}", self.base, path) };
        let r = bagholder_net::client::request("GET", &url, &[("Accept", "application/json")], None, std::time::Duration::from_secs(120)).map_err(|e| format!("Bagholder did not answer: {e}"))?;
        let body: Value = serde_json::from_slice(&r.body).map_err(|e| format!("Bagholder's answer could not be read: {e}"))?;
        if r.status != 200 {
            let said = body.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("status {}", r.status));
            return Err(said);
        }
        Ok(body)
    }

    /// One of the page's other reads (`page_reads`), by its path, with the query
    /// the page sends it: a `GET`, of a route the tests hold to the route table.
    pub fn page(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        let Some((path, _)) = page_reads().into_iter().find(|(p, _)| *p == path) else { return Err(format!("{path} is not a read an agent is given")) };
        let q: Vec<(&str, &str)> = query.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (*k, v.as_str())).collect();
        self.get(path, &q)
    }

    /// The status the page's header shows.
    pub fn status(&self) -> Result<Value, String> {
        self.read(&status(), &NoQuery {})
    }

    /// One screen's document, as a page subscribing to it is first sent it.
    pub fn view(&self, key: &str, params: Value) -> Result<Value, String> {
        self.read(&view(), &ViewQuery { key: Some(key.to_string()), params: Some(params.to_string()) })
    }

    /// A trade's or holding's fills.
    pub fn detail(&self, id: &str) -> Result<Value, String> {
        self.read(&detail(), &TradeQuery { id: Some(id.to_string()) })
    }

    pub fn transactions(&self, q: &crate::http::model::TransactionsQuery) -> Result<Value, String> {
        self.read(&transactions(), q)
    }
}

/// The server holding a data folder, read by the tools on this machine: found
/// again for each read, so a server started after the agent is found.
struct Local {
    home: PathBuf,
}

impl crate::agents::tools::Reader for Local {
    fn server(&self) -> Result<ReadOnly, String> {
        match find(&self.home) {
            Found::Running(at) => ReadOnly::at(&at),
            Found::Starting => Err("Bagholder is starting and is not answering yet: ask again in a moment.".to_string()),
            Found::NotRunning => Err(format!("Bagholder is not running on this machine (its data folder {}): start it, then ask again.", self.home.display())),
        }
    }
}

/// The tools on this machine's data folder, as `bagholder mcp` serves them: for
/// the tests, which read them without MCP's framing.
#[cfg(test)]
pub struct Agents {
    local: Local,
}

#[cfg(test)]
impl Agents {
    pub fn new(home: PathBuf) -> Agents {
        Agents { local: Local { home } }
    }

    /// A page of the Trades list (`tools::trades_page`).
    pub fn trades_page(&self, a: TradesArgs) -> Result<Value, String> {
        use crate::agents::tools::Reader;
        crate::agents::tools::trades_page(&self.local.server()?, a)
    }

    /// One trade, with its executions (`tools::one_trade`).
    pub fn one_trade(&self, id: &str) -> Result<Value, String> {
        use crate::agents::tools::Reader;
        crate::agents::tools::one_trade(&self.local.server()?, id)
    }
}

/// `bagholder mcp`: serve the tools on standard input and output until the
/// client closes them.
pub fn cli(home: PathBuf) -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("bagholder mcp: {e}");
            return 1;
        }
    };
    let served = rt.block_on(async move {
        let tools = crate::agents::tools::Tools::new(std::sync::Arc::new(Local { home }));
        let service = tools.serve(rmcp::transport::stdio()).await.map_err(|e| e.to_string())?;
        service.waiting().await.map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    });
    match served {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("bagholder mcp: {e}");
            1
        }
    }
}
