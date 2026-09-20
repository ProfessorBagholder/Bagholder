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

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::book::{build_book, Book};
use crate::cashflow::build_cashflow;
use crate::clock::today_local;
use crate::fx::{apply_fx, Fx};
use crate::nav::{equity_series, Point};
use crate::positions::build_positions;
use crate::trades::{build_trades, Journal};
use crate::value::{field_s, get, norm_account_name, num};

/// What the model is built from, each part shared rather than copied so a base
/// can be assembled again from the parts that did not change.
#[derive(Clone)]
pub struct Inputs {
    pub activities: Arc<Vec<Value>>,
    pub securities: Arc<Vec<Value>>,
    pub fx: Arc<Fx>,
    pub benchmark: Arc<BTreeMap<String, f64>>,
    pub benchmarks: Arc<HashMap<String, BTreeMap<String, f64>>>,
    pub distributions: Arc<Map<String, Value>>,
    pub quotes: Arc<Map<String, Value>>,
    pub groups: Arc<Vec<Value>>,
    pub journal: Arc<Journal>,
    pub accounts: Arc<Vec<Value>>,
    pub balances: Arc<Vec<Value>>,
    pub margin: Arc<Vec<Value>>,
    pub nav: Arc<Vec<Value>>,
    pub nav_by_account: Arc<Map<String, Value>>,
    pub exposures: Arc<Map<String, Value>>,
    pub watchlist: Arc<Vec<Value>>,
    pub news: Arc<Vec<Value>>,
    pub universes: Arc<Map<String, Value>>,
    pub tiles: Arc<Option<Value>>,
    pub synced_at: String,
}

impl Inputs {
    /// The parts, from a whole snapshot and the whole market data.
    pub fn from_snapshot(snapshot: &Value, market: &Value, journal: &Journal) -> Inputs {
        Inputs {
            activities: Arc::new(arr(snapshot.get("activities"))),
            securities: Arc::new(arr(snapshot.get("securities"))),
            fx: Arc::new(fx_part(market.get("fx"))),
            benchmark: Arc::new(crate::nav::bench_map(market.get("benchmark"))),
            benchmarks: Arc::new(benchmarks_part(market.get("benchmarks"))),
            distributions: Arc::new(obj(market.get("distributions"))),
            quotes: Arc::new(obj(market.get("quotes"))),
            groups: Arc::new(arr(snapshot.get("tradeGroups"))),
            journal: Arc::new(journal.clone()),
            accounts: Arc::new(arr(snapshot.get("accounts"))),
            balances: Arc::new(arr(snapshot.get("balances"))),
            margin: Arc::new(arr(snapshot.get("margin"))),
            nav: Arc::new(arr(snapshot.get("navHistory"))),
            nav_by_account: Arc::new(obj(snapshot.get("navByAccount"))),
            exposures: Arc::new(obj(snapshot.get("exposures"))),
            watchlist: Arc::new(arr(snapshot.get("watchlist"))),
            news: Arc::new(arr(snapshot.get("news"))),
            universes: Arc::new(obj(snapshot.get("universes"))),
            tiles: Arc::new(snapshot.get("tiles").filter(|v| !v.is_null()).cloned()),
            synced_at: field_s(snapshot, "syncedAt"),
        }
    }
}

pub fn fx_part(v: Option<&Value>) -> Fx {
    obj(v).iter().filter_map(|(k, x)| x.as_f64().map(|f| (k.clone(), f))).collect()
}

pub fn benchmarks_part(v: Option<&Value>) -> HashMap<String, BTreeMap<String, f64>> {
    obj(v).into_iter().map(|(k, v)| (k, crate::nav::bench_map(Some(&v)))).collect()
}

/// The derived layers, each behind an `Arc` so an unchanged one is shared.
#[derive(Clone)]
pub struct Layers {
    pub book: Arc<Book>,
    pub trades: Arc<Vec<Value>>,
    pub cashflow: Arc<Vec<Value>>,
    pub positions: Arc<Vec<Value>>,
    pub equity: Arc<Vec<Point>>,
    pub equity_by_account: Arc<HashMap<String, Vec<Point>>>,
    pub accounts: Arc<Vec<Value>>,
}

pub fn book_layer(i: &Inputs, today: &str) -> Book {
    build_book(&i.activities, &i.securities, today)
}

/// The closed trades: the match's slices valued in CAD at the FX table's rates,
/// collapsed into trades, the journal joined on.
pub fn trades_layer(book: &Book, i: &Inputs) -> Vec<Value> {
    let mut closed = book.fifo.closed.clone();
    apply_fx(&mut closed, &i.fx);
    build_trades(&closed, &i.groups, &book.acts_by_id, &book.securities, &i.journal)
}

pub fn cashflow_layer(book: &Book, i: &Inputs) -> Vec<Value> {
    build_cashflow(&book.activities, &book.securities, &i.fx)
}

/// The open positions marked at the quotes: what a price tick rebuilds.
pub fn positions_layer(book: &Book, i: &Inputs, today: &str) -> Vec<Value> {
    build_positions(&book.fifo.open, &book.last_prices, &i.balances, &i.accounts, &book.securities, &i.journal, today, &i.quotes, &book.acts_by_id)
}

pub fn equity_layer(i: &Inputs) -> (Vec<Point>, HashMap<String, Vec<Point>>) {
    let by_account = i.nav_by_account.iter().map(|(nick, pts)| (norm_account_name(nick), equity_series(&arr(Some(pts))))).collect();
    (equity_series(&i.nav), by_account)
}

pub fn accounts_layer(i: &Inputs) -> Vec<Value> {
    i.accounts.iter().map(|acc| {
        let nick = ["nickname", "unifiedAccountType", "type"]
            .iter()
            .map(|k| field_s(acc, k))
            .find(|v| !v.is_empty())
            .map(|v| norm_account_name(&v))
            .unwrap_or_default();
        json!({
            "id": field_s(acc, "id"),
            "name": nick,
            "type": field_s(acc, "unifiedAccountType"),
            "currency": field_s(acc, "currency"),
            "status": field_s(acc, "status"),
            "nav": opt_num(get(acc, "netLiquidationValue")),
        })
    }).collect()
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

pub struct Base {
    pub today: String,
    pub synced_at: String,
    pub fx: Arc<Fx>,
    pub benchmark: Arc<BTreeMap<String, f64>>,
    pub benchmarks: Arc<HashMap<String, BTreeMap<String, f64>>>,
    pub distributions: Arc<Map<String, Value>>,
    pub quotes: Arc<Map<String, Value>>,
    pub fx_last: String,
    pub benchmark_last: String,
    pub book: Arc<Book>,
    pub trades: Arc<Vec<Value>>,
    pub positions: Arc<Vec<Value>>,
    pub cashflow: Arc<Vec<Value>>,
    pub equity: Arc<Vec<Point>>,
    pub equity_by_account: Arc<HashMap<String, Vec<Point>>>,
    pub accounts: Arc<Vec<Value>>,
    pub balances: Arc<Vec<Value>>,
    pub margin: Arc<Vec<Value>>,
    pub exposures: Arc<Map<String, Value>>,
    pub watchlist: Arc<Vec<Value>>,
    pub news: Arc<Vec<Value>>,
    pub universes: Arc<Map<String, Value>>,
    pub tiles: Arc<Option<Value>>,
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

/// A number that is absent rather than zero.
fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

impl Base {
    /// A base from its inputs and its layers: shares, copies nothing.
    pub fn assemble(today: &str, i: &Inputs, l: &Layers) -> Base {
        let objects = |rows: &Arc<Vec<Value>>| -> Arc<Vec<Value>> {
            if rows.iter().all(|r| r.is_object()) { rows.clone() } else { Arc::new(rows.iter().filter(|r| r.is_object()).cloned().collect()) }
        };
        let mut benchmarks = i.benchmarks.clone();
        if !benchmarks.contains_key("SP500") {
            Arc::make_mut(&mut benchmarks).insert("SP500".into(), (*i.benchmark).clone());
        }
        let universes = if i.universes.values().all(|v| v.as_array().map_or(false, |a| a.iter().all(|r| r.is_object()))) {
            i.universes.clone()
        } else {
            Arc::new(i.universes.iter().map(|(k, v)| (k.clone(), Value::Array(arr(Some(v)).into_iter().filter(|r| r.is_object()).collect()))).collect())
        };
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
            balances: objects(&i.balances),
            margin: objects(&i.margin),
            exposures: i.exposures.clone(),
            watchlist: objects(&i.watchlist),
            news: objects(&i.news),
            universes,
            // absent is not the same as an empty row: it means never saved
            tiles: i.tiles.clone(),
        }
    }
}

/// The whole base from scratch: every layer built, then assembled.
pub fn build_base(snapshot: &Value, market: &Value, journal: &Journal, today: Option<&str>) -> Base {
    let today = match today {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => today_local(),
    };
    let inputs = Inputs::from_snapshot(snapshot, market, journal);
    let layers = Layers::build(&inputs, &today);
    Base::assemble(&today, &inputs, &layers)
}
