//! The base: everything a view is computed from.
//!
//! It is built in layers, each from its own inputs and nothing else, so that a
//! change rebuilds the layers that read it and leaves the rest standing
//! (docs/architecture.md, rule 3):
//!
//! | layer       | reads                                                        |
//! |-------------|--------------------------------------------------------------|
//! | book        | activities, securities, the day (the FIFO match)             |
//! | trades      | book, FX, saved groups, journal                              |
//! | cashflow    | book, FX                                                     |
//! | positions   | book, balances, accounts, journal, quotes, the day           |
//! | equity      | NAV history                                                  |
//! | accounts    | accounts                                                     |
//!
//! A quote tick therefore rebuilds `positions` alone; the match, the closed
//! trades, the cashflow and the equity curve are the same `Arc`s as before.
//! `build_base` composes every layer from scratch -- what the shared cases and
//! the wire snapshots run -- and a cache composes the same functions, reusing
//! what has not moved, so the two cannot disagree.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::activity::RawActivity;
use crate::book::{build_book, Book};
use crate::cashflow::build_cashflow;
use crate::clock::today_local;
use crate::exposure::Exposures;
use crate::fx::{apply_fx, Fx};
use crate::input::{AccountRow, BalanceRow, Distribution, Journal, MarginRow, NewsRow, Quotes, TileRef, TradeGroup, UniverseRow, WatchRow};
use crate::lenient;
use crate::nav::{equity_series, NavRow, Point};
use crate::positions::build_positions;
use crate::securities::{Securities, Security};
use crate::trades::build_trades;
use crate::value::{field_s, norm_account_name};
use crate::wire::{Account, CashflowRow, Ordered, Position, Trade};

/// What the model is built from, each part shared rather than copied so a base
/// can be assembled again from the parts that did not change. Every part is read
/// into its types once, when it is set (`lenient`), and never looked up by name
/// again.
#[derive(Clone, Default)]
pub struct Inputs {
    pub activities: Arc<Vec<RawActivity>>,
    pub securities: Arc<Vec<Security>>,
    pub fx: Arc<Fx>,
    pub benchmark: Arc<BTreeMap<String, f64>>,
    pub benchmarks: Arc<HashMap<String, BTreeMap<String, f64>>>,
    pub distributions: Arc<HashMap<String, Vec<Distribution>>>,
    pub quotes: Arc<Quotes>,
    pub groups: Arc<Vec<TradeGroup>>,
    pub journal: Arc<Journal>,
    pub accounts: Arc<Vec<AccountRow>>,
    pub balances: Arc<Vec<BalanceRow>>,
    pub margin: Arc<Vec<MarginRow>>,
    pub nav: Arc<Vec<NavRow>>,
    pub nav_by_account: Arc<HashMap<String, Vec<NavRow>>>,
    pub exposures: Arc<Exposures>,
    pub watchlist: Arc<Vec<WatchRow>>,
    pub news: Arc<Vec<NewsRow>>,
    /// In the order the store gives them, which is the order the page is sent them in.
    pub universes: Arc<Ordered<Vec<UniverseRow>>>,
    /// Never saved is not the same as saved empty: the first shows the default row.
    pub tiles: Arc<Option<Vec<TileRef>>>,
    pub synced_at: String,
}

/// The object rows of a JSON list, each read as `T`.
fn rows<T: serde::de::DeserializeOwned>(list: &[Value]) -> Arc<Vec<T>> {
    Arc::new(list.iter().filter(|r| r.is_object()).filter_map(|r| T::deserialize(r).ok()).collect())
}

/// The object values of a JSON map, each read as `T`.
fn keyed<T: serde::de::DeserializeOwned>(map: &Map<String, Value>) -> impl Iterator<Item = (String, T)> + '_ {
    map.iter().filter_map(|(k, v)| Some((k.clone(), T::deserialize(v).ok()?)))
}

impl Inputs {
    /// The parts, from a whole snapshot and the whole market data.
    pub fn from_snapshot(snapshot: &Value, market: &Value, journal: &Map<String, Value>) -> Inputs {
        let mut i = Inputs { fx: Arc::new(fx_part(market.get("fx"))), synced_at: field_s(snapshot, "syncedAt"), ..Inputs::default() };
        i.set_benchmarks(market.get("benchmark"), market.get("benchmarks"));
        i.set_distributions(&obj(market.get("distributions")));
        i.set_quotes(&obj(market.get("quotes")));
        i.set_activities(&arr(snapshot.get("activities")));
        i.set_securities(&arr(snapshot.get("securities")));
        i.set_groups(&arr(snapshot.get("tradeGroups")));
        i.set_journal(journal);
        i.set_accounts(&arr(snapshot.get("accounts")));
        i.set_balances(&arr(snapshot.get("balances")));
        i.set_margin(&arr(snapshot.get("margin")));
        i.set_nav(&arr(snapshot.get("navHistory")), &obj(snapshot.get("navByAccount")));
        i.set_exposures(&obj(snapshot.get("exposures")));
        i.set_watchlist(&arr(snapshot.get("watchlist")));
        i.set_news(&arr(snapshot.get("news")));
        i.set_universes(&obj(snapshot.get("universes")));
        i.set_tiles(snapshot.get("tiles"));
        i
    }

    pub fn set_activities(&mut self, list: &[Value]) {
        self.activities = rows(list);
    }
    pub fn set_securities(&mut self, list: &[Value]) {
        self.securities = rows(list);
    }
    pub fn set_benchmarks(&mut self, benchmark: Option<&Value>, benchmarks: Option<&Value>) {
        self.benchmark = Arc::new(crate::nav::bench_map(benchmark));
        self.benchmarks = Arc::new(obj(benchmarks).iter().map(|(k, v)| (k.clone(), crate::nav::bench_map(Some(v)))).collect());
    }
    pub fn set_distributions(&mut self, by_symbol: &Map<String, Value>) {
        self.distributions = Arc::new(by_symbol.iter().map(|(k, v)| (k.clone(), lenient::rows(v))).collect());
    }
    pub fn set_quotes(&mut self, by_key: &Map<String, Value>) {
        self.quotes = Arc::new(keyed(by_key).collect());
    }
    pub fn set_groups(&mut self, list: &[Value]) {
        self.groups = rows(list);
    }
    pub fn set_journal(&mut self, journal: &Map<String, Value>) {
        self.journal = Arc::new(crate::input::journal_from(journal));
    }
    pub fn set_accounts(&mut self, list: &[Value]) {
        // an account is read whatever it is: a row that is not an object is an account with nothing known
        self.accounts = Arc::new(list.iter().map(|r| AccountRow::deserialize(r).unwrap_or_default()).collect());
    }
    pub fn set_balances(&mut self, list: &[Value]) {
        self.balances = rows(list);
    }
    pub fn set_margin(&mut self, list: &[Value]) {
        self.margin = rows(list);
    }
    pub fn set_nav(&mut self, all: &[Value], by_account: &Map<String, Value>) {
        self.nav = rows(all);
        self.nav_by_account = Arc::new(by_account.iter().map(|(k, v)| (k.clone(), lenient::rows(v))).collect());
    }
    pub fn set_exposures(&mut self, by_key: &Map<String, Value>) {
        self.exposures = Arc::new(keyed(by_key).collect());
    }
    pub fn set_watchlist(&mut self, list: &[Value]) {
        self.watchlist = rows(list);
    }
    pub fn set_news(&mut self, list: &[Value]) {
        self.news = rows(list);
    }
    pub fn set_universes(&mut self, by_key: &Map<String, Value>) {
        self.universes = Arc::new(Ordered(by_key.iter().map(|(k, v)| (k.clone(), lenient::rows(v))).collect()));
    }
    pub fn set_tiles(&mut self, saved: Option<&Value>) {
        self.tiles = Arc::new(saved.filter(|v| !v.is_null()).map(|v| lenient::rows(v)));
    }
}

use serde::Deserialize;

pub fn fx_part(v: Option<&Value>) -> Fx {
    obj(v).iter().filter_map(|(k, x)| x.as_f64().map(|f| (k.clone(), f))).collect()
}

/// The derived layers, each behind an `Arc` so an unchanged one is shared.
#[derive(Clone)]
pub struct Layers {
    pub book: Arc<Book>,
    pub trades: Arc<Vec<Trade>>,
    pub cashflow: Arc<Vec<CashflowRow>>,
    pub positions: Arc<Vec<Position>>,
    pub equity: Arc<Vec<Point>>,
    pub equity_by_account: Arc<HashMap<String, Vec<Point>>>,
    pub accounts: Arc<Vec<Account>>,
}

pub fn book_layer(i: &Inputs, today: &str) -> Book {
    build_book(&i.activities, Securities::new(&i.securities), today)
}

/// The closed trades: the match's slices valued in CAD at the FX table's rates,
/// collapsed into trades, the journal joined on.
pub fn trades_layer(book: &Book, i: &Inputs) -> Vec<Trade> {
    let mut closed = book.fifo.closed.clone();
    apply_fx(&mut closed, &i.fx);
    build_trades(&closed, &i.groups, book, &i.journal)
}

pub fn cashflow_layer(book: &Book, i: &Inputs) -> Vec<CashflowRow> {
    build_cashflow(&book.activities, &book.securities, &i.fx)
}

/// The open positions marked at the quotes: what a price tick rebuilds.
pub fn positions_layer(book: &Book, i: &Inputs, today: &str) -> Vec<Position> {
    build_positions(book, &i.balances, &i.accounts, &i.journal, today, &i.quotes)
}

pub fn equity_layer(i: &Inputs) -> (Vec<Point>, HashMap<String, Vec<Point>>) {
    let by_account = i.nav_by_account.iter().map(|(name, days)| (norm_account_name(name), equity_series(days))).collect();
    (equity_series(&i.nav), by_account)
}

pub fn accounts_layer(i: &Inputs) -> Vec<Account> {
    i.accounts.iter().map(|a| Account { id: a.id.clone(), name: a.name(), kind: a.unified_account_type.clone(), currency: a.currency.clone(), status: a.status.clone(), nav: a.net_liquidation_value }).collect()
}

impl Layers {
    /// Every layer, from scratch.
    pub fn build(i: &Inputs, today: &str) -> Layers {
        let book = book_layer(i, today);
        let (equity, equity_by_account) = equity_layer(i);
        Layers {
            trades: Arc::new(trades_layer(&book, i)),
            cashflow: Arc::new(cashflow_layer(&book, i)),
            positions: Arc::new(positions_layer(&book, i, today)),
            equity: Arc::new(equity),
            equity_by_account: Arc::new(equity_by_account),
            accounts: Arc::new(accounts_layer(i)),
            book: Arc::new(book),
        }
    }
}

/// Everything a view is computed from: the inputs a view reads directly and the
/// layers derived from the rest, all shared.
pub struct Base {
    pub today: String,
    pub synced_at: String,
    pub fx: Arc<Fx>,
    pub benchmark: Arc<BTreeMap<String, f64>>,
    pub benchmarks: Arc<HashMap<String, BTreeMap<String, f64>>>,
    pub distributions: Arc<HashMap<String, Vec<Distribution>>>,
    pub quotes: Arc<Quotes>,
    pub fx_last: String,
    pub benchmark_last: String,
    pub book: Arc<Book>,
    pub trades: Arc<Vec<Trade>>,
    pub positions: Arc<Vec<Position>>,
    pub cashflow: Arc<Vec<CashflowRow>>,
    pub equity: Arc<Vec<Point>>,
    pub equity_by_account: Arc<HashMap<String, Vec<Point>>>,
    pub accounts: Arc<Vec<Account>>,
    pub balances: Arc<Vec<BalanceRow>>,
    pub margin: Arc<Vec<MarginRow>>,
    pub exposures: Arc<Exposures>,
    pub watchlist: Arc<Vec<WatchRow>>,
    pub news: Arc<Vec<NewsRow>>,
    pub universes: Arc<Ordered<Vec<UniverseRow>>>,
    pub tiles: Arc<Option<Vec<TileRef>>>,
    /// The cash rows Wealthsimple lists as securities: id -> currency.
    pub cash_currencies: HashMap<String, String>,
    pub activity_count: usize,
}

fn obj(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

fn arr(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        _ => vec![],
    }
}

impl Base {
    /// A base from its inputs and its layers: shares, copies nothing.
    pub fn assemble(today: &str, i: &Inputs, l: &Layers) -> Base {
        let mut benchmarks = i.benchmarks.clone();
        if !benchmarks.contains_key("SP500") {
            Arc::make_mut(&mut benchmarks).insert("SP500".into(), (*i.benchmark).clone());
        }
        Base {
            today: today.to_string(),
            synced_at: i.synced_at.clone(),
            fx_last: i.fx.keys().max().cloned().unwrap_or_default(),
            benchmark_last: i.benchmark.keys().max().cloned().unwrap_or_default(),
            fx: i.fx.clone(),
            benchmark: i.benchmark.clone(),
            benchmarks,
            distributions: i.distributions.clone(),
            quotes: i.quotes.clone(),
            cash_currencies: l.book.securities.cash_currencies(),
            activity_count: l.book.raw_count,
            book: l.book.clone(),
            trades: l.trades.clone(),
            positions: l.positions.clone(),
            cashflow: l.cashflow.clone(),
            equity: l.equity.clone(),
            equity_by_account: l.equity_by_account.clone(),
            accounts: l.accounts.clone(),
            balances: i.balances.clone(),
            margin: i.margin.clone(),
            exposures: i.exposures.clone(),
            watchlist: i.watchlist.clone(),
            news: i.news.clone(),
            universes: i.universes.clone(),
            tiles: i.tiles.clone(),
        }
    }
}

/// The whole base from scratch: every layer built, then assembled.
pub fn build_base(snapshot: &Value, market: &Value, journal: &Map<String, Value>, today: Option<&str>) -> Base {
    let today = match today {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => today_local(),
    };
    let inputs = Inputs::from_snapshot(snapshot, market, journal);
    let layers = Layers::build(&inputs, &today);
    Base::assemble(&today, &inputs, &layers)
}
