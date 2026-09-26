//! The page's subscriptions to the figures (`docs/architecture.md` §13;
//! `docs/plans/stage-5-interface-and-running.md`, part A).
//!
//! The page subscribes to what it shows, one screen at a time: the book as a
//! whole (`book`), the Dashboard, the holdings, the trades, the Cashflow, the
//! Portfolio's exposure, the Markets tab, and one trade or holding it has open.
//! A subscription is built whole once, then brought forward by what the engine
//! reports moved (`Moved`), and only by that:
//!
//! - a subscription that reads nothing that moved is not looked at;
//! - a list of rows (holdings, trades, dividends) builds again only the rows
//!   whose entity moved, compares only those with what was sent, and sends the
//!   list's order when rows came, went or moved in it;
//! - a total over the rows (the holdings' value, the dashboard's statistics) is
//!   worked out again from the engine's figures, which is a sum, and compared.
//!
//! Long lists (the trades, the dividends) are sorted here, by the column the
//! page sorts by, and sent only as far down as the page has scrolled (`limit`).
//!
//! Each state sent carries its version (`version_of`): a hash of what it says.
//! A page opening again says the version it kept, and a subscription whose
//! state is still that is answered with its version alone.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use bagholder_diff::Diff;
use bagholder_engine::engine::{Entity, Moved};
use bagholder_engine::trades::{TradeFig, TradeKey};
use bagholder_engine::Engine;

use crate::wire::build::{self, Names};
use crate::wire::context::MarketBase;
use crate::wire::figures::*;

/// What a subscription is built from: the engine, what the broker calls things,
/// and the market's context.
pub struct Cx<'a> {
    pub engine: &'a Engine,
    pub names: &'a Names,
    pub base: &'a MarketBase,
}

/// The direction a list is sorted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Asc,
    Desc,
}

/// The column a list is sorted by, as the page's header names it.
#[derive(Clone, Debug, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Sort {
    pub key: String,
    pub dir: Dir,
}

/// What a subscription is asked with. Each kind reads the parameters it has.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// The page's filters: every kind but `book` and `trade:` reads them.
    pub filters: Option<crate::wire::filters::Filters>,
    /// The trades' or the dividends' order.
    pub sort: Option<Sort>,
    /// How many rows of a long list the page shows: as far as it has scrolled.
    pub limit: Option<usize>,
}

/// How many rows of a long list are sent before the page asks for more.
pub const FIRST_ROWS: usize = 100;

/// The subscription a key names, with its parameters: none for a key that is
/// not a view of the figures (the other documents, `docs::read`). A parameter
/// the key's kind does not know is refused.
pub fn open(key: &str, params: &Value) -> Option<Result<Box<dyn View>, String>> {
    let kind = key.split(':').next().unwrap_or(key);
    if !matches!(kind, "book" | "dashboard" | "positions" | "trades" | "cashflow" | "exposure" | "markets" | "trade") {
        return None;
    }
    let p: Params = match serde_json::from_value(params.clone()) {
        Ok(p) => p,
        Err(e) => return Some(Err(format!("{key}: {e}"))),
    };
    let filters = || -> Result<bagholder_engine::scope::Filters, String> { p.filters.clone().unwrap_or_default().to_engine() };
    Some((|| -> Result<Box<dyn View>, String> {
        Ok(match kind {
            "book" => Box::new(Whole::<BookDoc>::new(Reads::Book, |cx: &Cx| build::book_doc(cx.engine, cx.names))),
            "dashboard" => {
                let f = filters()?;
                let benchmark = f.benchmark.clone();
                Box::new(Whole::<DashboardDoc>::new(Reads::Dashboard(benchmark), move |cx: &Cx| build::dashboard_doc(cx.engine, &f)))
            }
            "exposure" => {
                let f = filters()?;
                Box::new(Whole::<ExposureDoc>::new(Reads::Holdings, move |cx: &Cx| {
                    let pf = cx.engine.portfolio(&f);
                    let c = build::context_of(cx.engine, cx.names, &build::links(cx.engine), &pf, cx.base);
                    ExposureDoc { sectors: c.sectors, regions: c.regions }
                }))
            }
            "markets" => {
                let f = filters()?;
                Box::new(Whole::<MarketsDoc>::new(Reads::Holdings, move |cx: &Cx| {
                    let pf = cx.engine.portfolio(&f);
                    MarketsDoc { markets: build::context_of(cx.engine, cx.names, &build::links(cx.engine), &pf, cx.base).markets }
                }))
            }
            "trade" => {
                let id = key.strip_prefix("trade:").unwrap_or_default().to_string();
                Box::new(Whole::<TradeDoc>::new(Reads::TradesAndHoldings, move |cx: &Cx| one_trade(cx, &id)))
            }
            "positions" => Box::new(Positions { filters: filters()?, sent: None }),
            "trades" => Box::new(Trades::new(filters()?, p.sort.clone().unwrap_or(Sort { key: "exitDate".into(), dir: Dir::Desc }), p.limit.unwrap_or(FIRST_ROWS))?),
            "cashflow" => Box::new(CashflowView::new(filters()?, p.sort.clone().unwrap_or(Sort { key: "date".into(), dir: Dir::Desc }), p.limit.unwrap_or(FIRST_ROWS))?),
            _ => unreachable!("matched above"),
        })
    })())
}

/// A subscription to part of the figures.
pub trait View: Send {
    /// Whether what moved (`moved`, and `base` for the market's context) can move it.
    fn reads(&self, moved: &Moved, base: bool) -> bool;
    /// Its whole state, kept as what was sent.
    fn snapshot(&mut self, cx: &Cx) -> Value;
    /// What turns what was sent into its state now, after `moved`: the operations
    /// (`bagholder_diff`), kept as what was sent.
    fn update(&mut self, cx: &Cx, moved: &Moved, base: bool) -> Vec<Value>;
    /// The version of what was last sent.
    fn version(&self) -> u64;
}

/// The version of a state: a hash of the JSON it is written as (FNV-1a), the same
/// for the same state on every run of every build.
pub fn version_of(v: &impl Serialize) -> u64 {
    version_of_bytes(&serde_json::to_vec(v).expect("a wire value is plain data"))
}

/// The version of an answer as written: FNV-1a over its bytes.
pub fn version_of_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// --- what a subscription reads ------------------------------------------------------

/// Which of the engine's entities a subscription's state is made of.
enum Reads {
    /// The record as a whole, the trades (their tags, years and instruments), the
    /// holdings that came or went, the accounts' values.
    Book,
    /// The trades, the accounts' values, the record (today), and one benchmark.
    Dashboard(String),
    /// The holdings (and the record, and the market's context).
    Holdings,
    /// The trades and the holdings.
    TradesAndHoldings,
}

impl Reads {
    fn reads(&self, moved: &Moved, base: bool) -> bool {
        moved.0.iter().any(|(e, fields)| match (self, e) {
            (_, Entity::Book) => true,
            (Reads::Book, Entity::Trade(_) | Entity::Equity(_)) => true,
            (Reads::Book, Entity::Position(..)) => fields.contains("*"),
            (Reads::Dashboard(_), Entity::Trade(_) | Entity::Equity(_)) => true,
            (Reads::Dashboard(b), Entity::Benchmark(k)) => k == b,
            (Reads::Holdings, Entity::Position(..) | Entity::Equity(_) | Entity::Trade(_)) => true,
            (Reads::TradesAndHoldings, Entity::Trade(_) | Entity::Position(..)) => true,
            _ => false,
        }) || (base && matches!(self, Reads::Holdings))
    }
}

// --- a state built whole -----------------------------------------------------------

/// A subscription whose state is a total over many entities (the dashboard's
/// statistics, the filters' choices): built again when anything it reads moved,
/// and compared field by field with what was sent.
struct Whole<T> {
    reads: Reads,
    build: Box<dyn Fn(&Cx) -> T + Send>,
    sent: Option<T>,
}

impl<T> Whole<T> {
    fn new(reads: Reads, build: impl Fn(&Cx) -> T + Send + 'static) -> Whole<T> {
        Whole { reads, build: Box::new(build), sent: None }
    }
}

impl<T: Diff + Serialize + Send> View for Whole<T> {
    fn reads(&self, moved: &Moved, base: bool) -> bool {
        self.reads.reads(moved, base)
    }
    fn snapshot(&mut self, cx: &Cx) -> Value {
        let now = (self.build)(cx);
        let v = serde_json::to_value(&now).expect("a wire value is plain data");
        self.sent = Some(now);
        v
    }
    fn update(&mut self, cx: &Cx, _moved: &Moved, _base: bool) -> Vec<Value> {
        let now = (self.build)(cx);
        let ops = match &self.sent {
            Some(was) => bagholder_diff::typed(was, &now),
            None => vec![json!(["set", [], serde_json::to_value(&now).expect("plain data")])],
        };
        self.sent = Some(now);
        ops
    }
    fn version(&self) -> u64 {
        self.sent.as_ref().map(version_of).unwrap_or(0)
    }
}

fn one_trade(cx: &Cx, id: &str) -> TradeDoc {
    let figs = cx.engine.figures();
    let inputs = cx.engine.inputs();
    let links = build::links(cx.engine);
    let trade = figs.trades.iter().find(|t| build::trade_wire_id(t) == id).map(|t| build::trade_row(inputs, cx.names, &links, t));
    let position = figs.positions.iter().find(|p| build::position_id(p) == id).map(|p| build::position_row(inputs, cx.names, &links, p));
    TradeDoc { id: id.to_string(), trade, position }
}

// --- lists of rows -----------------------------------------------------------------

/// The operations that turn the rows `was` (in its order) into `now`, where only
/// the rows named in `changed` can differ: those compared field by field, and
/// the order sent when it is not the same, carrying the rows the page has not seen.
fn rows_ops<T: Diff + Serialize>(path: &[&str], key: &str, was: &[(String, &T)], now: &[(String, &T)], changed: &BTreeSet<String>) -> Vec<Value> {
    let mut ops = Vec::new();
    let had: HashMap<&str, &T> = was.iter().map(|(k, r)| (k.as_str(), *r)).collect();
    let mut added = serde_json::Map::new();
    for (k, r) in now {
        match had.get(k.as_str()) {
            None => {
                added.insert(k.clone(), serde_json::to_value(r).expect("plain data"));
            }
            Some(old) if changed.contains(k) => {
                let mut p: Vec<Value> = path.iter().map(|s| json!(s)).collect();
                p.push(json!({"k": key, "v": k}));
                old.diff(r, &mut p, &mut ops);
            }
            Some(_) => {}
        }
    }
    let same_order = was.len() == now.len() && was.iter().zip(now).all(|(a, b)| a.0 == b.0);
    if !same_order {
        let order: Vec<&str> = now.iter().map(|(k, _)| k.as_str()).collect();
        ops.push(json!(["rows", path, key, order, added]));
    }
    ops
}

/// The wire ids of the holdings whose entity moved.
fn moved_positions(cx: &Cx, moved: &Moved) -> BTreeSet<String> {
    let keys: BTreeSet<_> = moved.0.keys().filter_map(|e| match e {
        Entity::Position(a, i, d) => Some((*a, *i, *d)),
        _ => None,
    }).collect();
    cx.engine.figures().positions.iter().filter(|p| keys.contains(&(p.account, p.instrument, p.direction))).map(build::position_id).collect()
}

/// The wire ids of the trades whose entity moved, and of an open trade whose
/// holding came or went (its link to it).
fn moved_trades(cx: &Cx, moved: &Moved) -> BTreeSet<String> {
    let keys: BTreeSet<&TradeKey> = moved.0.keys().filter_map(|e| match e {
        Entity::Trade(k) => Some(k),
        _ => None,
    }).collect();
    let held: BTreeSet<_> = moved.0.iter().filter(|(_, f)| f.contains("*")).filter_map(|(e, _)| match e {
        Entity::Position(a, i, _) => Some((*a, *i)),
        _ => None,
    }).collect();
    cx.engine.figures().trades.iter().filter(|t| keys.contains(&t.key) || held.contains(&(t.account, t.instrument))).map(build::trade_wire_id).collect()
}

/// The holdings under the filters, and their totals.
struct Positions {
    filters: bagholder_engine::scope::Filters,
    sent: Option<PositionsDoc>,
}

impl Positions {
    fn build(&self, cx: &Cx, keep: Option<(&PositionsDoc, &BTreeSet<String>)>) -> PositionsDoc {
        let pf = cx.engine.portfolio(&self.filters);
        let figs = cx.engine.figures();
        let inputs = cx.engine.inputs();
        let links = build::links(cx.engine);
        let kept: HashMap<&str, &Position> = keep.map(|(d, _)| d.positions.iter().map(|p| (p.id.as_str(), p)).collect()).unwrap_or_default();
        let positions = pf
            .positions
            .iter()
            .map(|i| &figs.positions[*i])
            .map(|p| {
                let id = build::position_id(p);
                match (kept.get(id.as_str()), keep) {
                    // a row whose holding did not move is the row that was sent
                    (Some(row), Some((_, changed))) if !changed.contains(&id) => (*row).clone(),
                    _ => build::position_row(inputs, cx.names, &links, p),
                }
            })
            .collect();
        PositionsDoc { portfolio: build::portfolio_totals(cx.engine, &pf), positions }
    }
}

impl View for Positions {
    fn reads(&self, moved: &Moved, _base: bool) -> bool {
        Reads::Holdings.reads(moved, false)
    }
    fn snapshot(&mut self, cx: &Cx) -> Value {
        let d = self.build(cx, None);
        let v = serde_json::to_value(&d).expect("plain data");
        self.sent = Some(d);
        v
    }
    fn update(&mut self, cx: &Cx, moved: &Moved, _base: bool) -> Vec<Value> {
        let Some(was) = self.sent.take() else {
            let v = self.snapshot(cx);
            return vec![json!(["set", [], v])];
        };
        // a change to the record renames and relinks every row: each is built again
        let changed: BTreeSet<String> = if moved.0.contains_key(&Entity::Book) { was.positions.iter().map(|p| p.id.clone()).chain(moved_positions(cx, moved)).collect() } else { moved_positions(cx, moved).into_iter().chain(moved_trades_links(cx, moved)).collect() };
        let now = self.build(cx, Some((&was, &changed)));
        let mut ops = bagholder_diff::typed_under(&["portfolio"], &was.portfolio, &now.portfolio);
        let w: Vec<(String, &Position)> = was.positions.iter().map(|p| (p.id.clone(), p)).collect();
        let n: Vec<(String, &Position)> = now.positions.iter().map(|p| (p.id.clone(), p)).collect();
        ops.extend(rows_ops(&["positions"], "id", &w, &n, &changed));
        self.sent = Some(now);
        ops
    }
    fn version(&self) -> u64 {
        self.sent.as_ref().map(version_of).unwrap_or(0)
    }
}

/// The holdings whose trade came or went (a holding shows its trade's id).
fn moved_trades_links(cx: &Cx, moved: &Moved) -> BTreeSet<String> {
    let trips: BTreeSet<_> = moved.0.iter().filter(|(_, f)| f.contains("*") || f.contains("trade")).filter_map(|(e, _)| match e {
        Entity::Trade(TradeKey::Trip(k)) => Some(k.clone()),
        _ => None,
    }).collect();
    cx.engine.figures().positions.iter().filter(|p| trips.contains(&p.key)).map(build::position_id).collect()
}

// --- sorting as the page sorts ----------------------------------------------------

/// A value to sort a row by: nothing (an empty cell, or a figure that waits),
/// a number, an exact decimal, or text.
#[derive(Clone, Debug, PartialEq)]
enum SortValue {
    None,
    Number(f64),
    Decimal(bagholder_core::Dec),
    Text(String),
}

impl SortValue {
    /// A row's field as JSON: a figure that waits (an object of gaps) has no value.
    fn of(v: Option<&Value>) -> SortValue {
        match v {
            None | Some(Value::Null) => SortValue::None,
            Some(Value::Number(n)) => n.as_f64().map(SortValue::Number).unwrap_or(SortValue::None),
            Some(Value::String(s)) => match bagholder_core::Dec::parse(s) {
                Ok(d) if s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') => SortValue::Decimal(d),
                _ => SortValue::Text(s.clone()),
            },
            Some(Value::Bool(b)) => SortValue::Text(b.to_string()),
            Some(Value::Object(_)) | Some(Value::Array(_)) => SortValue::None,
        }
    }
}

/// Two values' order as the page orders them (`web/src/lib/sort.svelte.ts`):
/// numbers and decimals by value, text as a person reads it (letters without
/// regard to case first), nothing after everything.
fn cmp_values(a: &SortValue, b: &SortValue) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    use SortValue::*;
    match (a, b) {
        (None, None) => Equal,
        (None, _) => Greater,
        (_, None) => Less,
        (Number(x), Number(y)) => x.partial_cmp(y).unwrap_or(Equal),
        (Decimal(x), Decimal(y)) => x.cmp(y),
        _ => {
            let (x, y) = (text(a), text(b));
            x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(&y))
        }
    }
}

fn text(v: &SortValue) -> String {
    match v {
        SortValue::None => String::new(),
        SortValue::Number(n) => n.to_string(),
        SortValue::Decimal(d) => d.to_string(),
        SortValue::Text(t) => t.clone(),
    }
}

/// The ids of `rows` in the page's order for `sort` (`sortRows`): a row with
/// nothing in the column sinks, whichever the direction; rows equal in it are
/// ordered by the further values each carries, ascending; a tie in all of them
/// keeps the rows' own order.
fn sorted(rows: &[(String, SortValue)], dir: Dir) -> Vec<String> {
    let rows: Vec<(String, Vec<SortValue>)> = rows.iter().map(|(k, v)| (k.clone(), vec![v.clone()])).collect();
    sorted_then(&rows, dir)
}

fn sorted_then(rows: &[(String, Vec<SortValue>)], dir: Dir) -> Vec<String> {
    let mut v: Vec<&(String, Vec<SortValue>)> = rows.iter().collect();
    v.sort_by(|(_, a), (_, b)| {
        let (x, y) = (&a[0], &b[0]);
        let c = if matches!(x, SortValue::None) || matches!(y, SortValue::None) {
            cmp_values(x, y)
        } else if dir == Dir::Desc {
            cmp_values(x, y).reverse()
        } else {
            cmp_values(x, y)
        };
        a.iter().zip(b.iter()).skip(1).fold(c, |c, (x, y)| c.then_with(|| cmp_values(x, y)))
    });
    v.into_iter().map(|(k, _)| k.clone()).collect()
}

/// What the Trades table sorts a trade by for a column (`Trades.svelte`,
/// `tradeSortValue`).
fn trade_sort_value(t: &Trade, key: &str) -> SortValue {
    match key {
        "tags" => {
            let mut tags = t.tags.clone();
            tags.sort();
            SortValue::Text(tags.first().cloned().unwrap_or_else(|| "\u{ffff}".into()))
        }
        "grade" => ["F", "C", "B", "A"].iter().position(|g| *g == t.grade).map(|i| SortValue::Number(i as f64)).unwrap_or(SortValue::None),
        "pnl" => SortValue::of(serde_json::to_value(&t.pnl_cad).ok().as_ref()),
        // newest activity first: an open trade by its latest fill, a closed one by its close
        "exitDate" => SortValue::Text(t.last_date.clone()),
        k => SortValue::of(serde_json::to_value(t).ok().as_ref().and_then(|v| v.get(k))),
    }
}

/// The trades under the filters: every one in scope held as its row, sorted as
/// the page sorts, the first `limit` sent.
struct Trades {
    filters: bagholder_engine::scope::Filters,
    sort: Sort,
    limit: usize,
    rows: HashMap<String, Trade>,
    keys: HashMap<String, SortValue>,
    order: Vec<String>,
    sent: Option<(usize, Vec<String>)>,
}

impl Trades {
    fn new(filters: bagholder_engine::scope::Filters, sort: Sort, limit: usize) -> Result<Trades, String> {
        const COLUMNS: [&str; 13] = ["entryDate", "exitDate", "symbol", "exchange", "qty", "entry", "exit", "currency", "pnl", "pnlPct", "holdDays", "grade", "tags"];
        if !COLUMNS.contains(&sort.key.as_str()) {
            return Err(format!("the trades have no column {:?} to sort by", sort.key));
        }
        Ok(Trades { filters, sort, limit, rows: HashMap::new(), keys: HashMap::new(), order: Vec::new(), sent: None })
    }

    /// The trades in scope now, each row built again where `changed` names it
    /// (all of them for `None`), and their order.
    fn refresh(&mut self, cx: &Cx, changed: Option<&BTreeSet<String>>) {
        let figs = cx.engine.figures();
        let inputs = cx.engine.inputs();
        let by_key: BTreeMap<&TradeKey, &TradeFig> = figs.trades.iter().map(|t| (&t.key, t)).collect();
        let links = build::links(cx.engine);
        let in_scope: Vec<&TradeFig> = cx.engine.trades_in_scope(&self.filters).iter().filter_map(|k| by_key.get(k).copied()).collect();
        let mut rows = HashMap::with_capacity(in_scope.len());
        let mut keys = HashMap::with_capacity(in_scope.len());
        let mut listed: Vec<(String, SortValue)> = Vec::with_capacity(in_scope.len());
        for t in in_scope {
            let id = build::trade_wire_id(t);
            let fresh = changed.is_none_or(|c| c.contains(&id)) || !self.rows.contains_key(&id);
            let (row, key) = if fresh {
                let row = build::trade_row(inputs, cx.names, &links, t);
                let key = trade_sort_value(&row, &self.sort.key);
                (row, key)
            } else {
                (self.rows.remove(&id).expect("held"), self.keys.remove(&id).expect("held"))
            };
            listed.push((id.clone(), key.clone()));
            rows.insert(id.clone(), row);
            keys.insert(id, key);
        }
        self.order = sorted(&listed, self.sort.dir);
        self.rows = rows;
        self.keys = keys;
    }

    fn window(&self) -> Vec<(String, &Trade)> {
        self.order.iter().take(self.limit).map(|id| (id.clone(), &self.rows[id])).collect()
    }

    fn doc(&self) -> TradesDoc {
        TradesDoc { total: self.order.len(), trades: self.window().into_iter().map(|(_, t)| t.clone()).collect() }
    }
}

impl View for Trades {
    fn reads(&self, moved: &Moved, _base: bool) -> bool {
        moved.0.iter().any(|(e, f)| matches!(e, Entity::Book | Entity::Trade(_)) || (matches!(e, Entity::Position(..)) && f.contains("*")))
    }
    fn snapshot(&mut self, cx: &Cx) -> Value {
        self.refresh(cx, None);
        let d = self.doc();
        self.sent = Some((d.total, self.order.iter().take(self.limit).cloned().collect()));
        serde_json::to_value(d).expect("plain data")
    }
    fn update(&mut self, cx: &Cx, moved: &Moved, _base: bool) -> Vec<Value> {
        let Some((total, was_ids)) = self.sent.take() else {
            let v = self.snapshot(cx);
            return vec![json!(["set", [], v])];
        };
        // what was sent, before the rows are built again
        let was_rows: Vec<(String, Trade)> = was_ids.iter().filter_map(|id| self.rows.get(id).map(|t| (id.clone(), t.clone()))).collect();
        let changed = if moved.0.contains_key(&Entity::Book) { None } else { Some(moved_trades(cx, moved)) };
        self.refresh(cx, changed.as_ref());
        let all: BTreeSet<String> = was_rows.iter().map(|(k, _)| k.clone()).collect();
        let changed = changed.unwrap_or(all);
        let now = self.window();
        let was: Vec<(String, &Trade)> = was_rows.iter().map(|(k, t)| (k.clone(), t)).collect();
        let mut ops = Vec::new();
        if self.order.len() != total {
            ops.push(json!(["set", ["total"], self.order.len()]));
        }
        ops.extend(rows_ops(&["trades"], "id", &was, &now, &changed));
        self.sent = Some((self.order.len(), now.iter().map(|(k, _)| k.clone()).collect()));
        ops
    }
    fn version(&self) -> u64 {
        version_of(&self.doc())
    }
}

/// Every trade under `filters`, in `sort`'s order: what an export writes.
pub fn all_trades(cx: &Cx, filters: bagholder_engine::scope::Filters, sort: Sort) -> Result<TradesDoc, String> {
    let mut t = Trades::new(filters, sort, usize::MAX)?;
    t.refresh(cx, None);
    Ok(t.doc())
}

/// What the Cashflow tab's dividends table sorts a row by (`Cashflow.svelte`).
fn cash_sort_value(r: &CashflowRow, key: &str) -> SortValue {
    SortValue::of(serde_json::to_value(r).ok().as_ref().and_then(|v| v.get(key)))
}

/// The Cashflow tab under the filters: its totals, holdings and months built
/// again when what they read moved, its dividends held as rows, sorted as the
/// page sorts, the first `limit` sent.
struct CashflowView {
    filters: bagholder_engine::scope::Filters,
    sort: Sort,
    limit: usize,
    sent: Option<CashflowDoc>,
}

impl CashflowView {
    fn new(filters: bagholder_engine::scope::Filters, sort: Sort, limit: usize) -> Result<CashflowView, String> {
        const COLUMNS: [&str; 7] = ["date", "symbol", "account", "qty", "per", "amount", "currency"];
        if !COLUMNS.contains(&sort.key.as_str()) {
            return Err(format!("the dividends have no column {:?} to sort by", sort.key));
        }
        Ok(CashflowView { filters, sort, limit, sent: None })
    }

    fn build(&self, cx: &Cx) -> CashflowDoc {
        let pf = cx.engine.portfolio(&self.filters);
        let mut c = build::cashflow_doc(cx.engine, &self.filters, &pf);
        // a day's payments by symbol, then account, whichever column is sorted
        let keyed: Vec<(String, Vec<SortValue>)> = c.rows.iter().map(|r| (r.id.clone(), vec![cash_sort_value(r, &self.sort.key), cash_sort_value(r, "symbol"), cash_sort_value(r, "account")])).collect();
        let order = sorted_then(&keyed, self.sort.dir);
        let total = c.rows.len();
        let mut by_id: HashMap<String, CashflowRow> = c.rows.drain(..).map(|r| (r.id.clone(), r)).collect();
        c.rows = order.iter().take(self.limit).filter_map(|id| by_id.remove(id)).collect();
        CashflowDoc { cashflow: c, rows_total: total }
    }
}

impl View for CashflowView {
    fn reads(&self, moved: &Moved, _base: bool) -> bool {
        moved.0.keys().any(|e| matches!(e, Entity::Book | Entity::CashRow(_) | Entity::Payer(_) | Entity::Position(..)))
    }
    fn snapshot(&mut self, cx: &Cx) -> Value {
        let d = self.build(cx);
        let v = serde_json::to_value(&d).expect("plain data");
        self.sent = Some(d);
        v
    }
    fn update(&mut self, cx: &Cx, _moved: &Moved, _base: bool) -> Vec<Value> {
        let now = self.build(cx);
        let ops = match &self.sent {
            Some(was) => bagholder_diff::typed(was, &now),
            None => vec![json!(["set", [], serde_json::to_value(&now).expect("plain data")])],
        };
        self.sent = Some(now);
        ops
    }
    fn version(&self) -> u64 {
        self.sent.as_ref().map(version_of).unwrap_or(0)
    }
}

/// A subscription's whole state, as a page opening it is sent it.
pub fn snapshot_of(cx: &Cx, key: &str, params: Value) -> Result<Value, String> {
    open(key, &params).ok_or_else(|| format!("{key} is not a view of the figures"))?.map(|mut v| v.snapshot(cx))
}

/// Each subscription's whole state under `filters`, every row of each list: what
/// a page opening every screen is sent. Keyed by the subscription.
#[cfg(test)]
pub fn every_view(cx: &Cx, filters: &crate::wire::filters::Filters) -> Result<serde_json::Map<String, Value>, String> {
    let all = json!({"filters": filters, "limit": usize::MAX >> 12});
    let filtered = json!({"filters": filters});
    let mut out = serde_json::Map::new();
    out.insert("book".into(), snapshot_of(cx, "book", json!({}))?);
    for key in ["dashboard", "positions", "exposure", "markets"] {
        out.insert(key.into(), snapshot_of(cx, key, filtered.clone())?);
    }
    for key in ["trades", "cashflow"] {
        out.insert(key.into(), snapshot_of(cx, key, all.clone())?);
    }
    Ok(out)
}
