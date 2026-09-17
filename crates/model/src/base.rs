//! `build_base`: everything the view is computed from, in one object.
//!
//! It depends on the snapshot, the market data and the day, so one filter
//! change re-runs `build_view` alone.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

use crate::book::{build_book, Book};
use crate::cashflow::build_cashflow;
use crate::clock::today_local;
use crate::fx::{apply_fx, Fx};
use crate::nav::{equity_series, Point};
use crate::positions::build_positions;
use crate::trades::{build_trades, last_fill_prices, Journal};
use crate::value::{field_s, get, norm_account_name, num};

pub struct Base {
    pub today: String,
    pub synced_at: String,
    pub fx: Fx,
    pub benchmark: BTreeMap<String, f64>,
    pub benchmarks: HashMap<String, BTreeMap<String, f64>>,
    pub distributions: Map<String, Value>,
    pub quotes: Map<String, Value>,
    pub fx_last: String,
    pub benchmark_last: String,
    pub book: Book,
    pub trades: Vec<Value>,
    pub positions: Vec<Value>,
    pub cashflow: Vec<Value>,
    pub equity: Vec<Point>,
    pub equity_by_account: HashMap<String, Vec<Point>>,
    pub accounts: Vec<Value>,
    pub balances: Vec<Value>,
    pub margin: Vec<Value>,
    pub exposures: Map<String, Value>,
    pub watchlist: Vec<Value>,
    pub news: Vec<Value>,
    pub universes: Map<String, Value>,
    pub tiles: Option<Value>,
    pub cash_currencies: HashMap<String, String>,
    pub activity_count: usize,
    pub last_prices: Map<String, Value>,
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

fn fx_map(v: Option<&Value>) -> Fx {
    obj(v).iter().filter_map(|(k, x)| x.as_f64().map(|f| (k.clone(), f))).collect()
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

pub fn build_base(snapshot: &Value, market: &Value, journal: &Journal, today: Option<&str>) -> Base {
    let today = match today {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => today_local(),
    };
    let fx = fx_map(market.get("fx"));
    let bench = crate::nav::bench_map(market.get("benchmark"));

    let mut benchmarks: HashMap<String, BTreeMap<String, f64>> = HashMap::new();
    for (k, v) in obj(market.get("benchmarks")) {
        benchmarks.insert(k, crate::nav::bench_map(Some(&v)));
    }
    benchmarks.entry("SP500".into()).or_insert_with(|| bench.clone());

    let mut book = build_book(snapshot, &today);
    apply_fx(&mut book.fifo.closed, &fx);

    let saved = arr(snapshot.get("tradeGroups"));
    let trades = build_trades(&book.fifo.closed, &saved, &book.acts_by_id, &book.securities, journal);
    let last_prices = last_fill_prices(&book.activities);
    let quotes = obj(market.get("quotes"));
    let positions = build_positions(
        &book.fifo.open,
        &last_prices,
        &arr(snapshot.get("balances")),
        &arr(snapshot.get("accounts")),
        &book.securities,
        journal,
        &today,
        &quotes,
        &book.acts_by_id,
    );
    let cashflow = build_cashflow(&book.activities, &book.securities, &fx);
    let equity = equity_series(&arr(snapshot.get("navHistory")));

    let mut equity_by_account = HashMap::new();
    for (nick, pts) in obj(snapshot.get("navByAccount")) {
        equity_by_account.insert(norm_account_name(&nick), equity_series(&arr(Some(&pts))));
    }

    let mut accounts = Vec::new();
    for acc in arr(snapshot.get("accounts")) {
        let nick = ["nickname", "unifiedAccountType", "type"]
            .iter()
            .map(|k| field_s(&acc, k))
            .find(|v| !v.is_empty())
            .map(|v| norm_account_name(&v))
            .unwrap_or_default();
        accounts.push(json!({
            "id": field_s(&acc, "id"),
            "name": nick,
            "type": field_s(&acc, "unifiedAccountType"),
            "currency": field_s(&acc, "currency"),
            "status": field_s(&acc, "status"),
            "nav": opt_num(get(&acc, "netLiquidationValue")),
        }));
    }

    let fx_last = fx.keys().max().cloned().unwrap_or_default();
    let benchmark_last = bench.keys().max().cloned().unwrap_or_default();
    let cash_currencies = book.securities.cash_currencies();
    let activity_count = book.raw_count;

    Base {
        today,
        synced_at: field_s(snapshot, "syncedAt"),
        fx,
        benchmark: bench,
        benchmarks,
        distributions: obj(market.get("distributions")),
        quotes,
        fx_last,
        benchmark_last,
        book,
        trades,
        positions,
        cashflow,
        equity,
        equity_by_account,
        accounts,
        balances: arr(snapshot.get("balances")).into_iter().filter(|b| b.is_object()).collect(),
        margin: arr(snapshot.get("margin")).into_iter().filter(|m| m.is_object()).collect(),
        exposures: obj(snapshot.get("exposures")),
        watchlist: arr(snapshot.get("watchlist")).into_iter().filter(|w| w.is_object()).collect(),
        news: arr(snapshot.get("news")).into_iter().filter(|n| n.is_object()).collect(),
        universes: {
            let mut m = Map::new();
            for (k, v) in obj(snapshot.get("universes")) {
                m.insert(k, Value::Array(arr(Some(&v)).into_iter().filter(|r| r.is_object()).collect()));
            }
            m
        },
        // absent is not the same as an empty row: it means never saved
        tiles: snapshot.get("tiles").filter(|v| !v.is_null()).cloned(),
        cash_currencies,
        activity_count,
        last_prices,
    }
}
