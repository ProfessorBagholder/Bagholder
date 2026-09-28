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
use crate::wire::figures::*;
use crate::wire::markets::{self, ExposureDoc, Following, Tables};

/// What a subscription is built from: the engine, what the broker calls things,
/// the market's context tables and what the person follows.
pub struct Cx<'a> {
    pub engine: &'a Engine,
    pub names: &'a Names,
    pub tables: &'a dyn Tables,
    pub following: &'a Following,
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
    /// The heatmap's universe: `holdings`, `watchlist`, `both`, `ca`, `us`, `intl`.
    pub universe: Option<String>,
    /// The heatmap's sizing: `value` or `equal`.
    pub size: Option<String>,
    /// The News card's scope: `all`, `holdings`, `watchlist`.
    pub scope: Option<String>,
    /// The News card's tab: `stories` or `releases`.
    pub kind: Option<String>,
    /// The News card's chip: a listing's bare ticker and its venue.
    pub symbol: Option<String>,
    pub exchange: Option<String>,
    /// What is typed in the News card's box.
    pub query: Option<String>,
}

/// How many rows of a long list are sent before the page asks for more.
pub const FIRST_ROWS: usize = 100;

/// The subscription a key names, with its parameters: none for a key that is
/// not a view of the figures (the other documents, `docs::read`). A parameter
/// the key's kind does not know is refused.
pub fn open(key: &str, params: &Value) -> Option<Result<Box<dyn View>, String>> {
    let kind = key.split(':').next().unwrap_or(key);
    if !matches!(kind, "book" | "dashboard" | "positions" | "trades" | "cashflow" | "exposure" | "markets" | "heatmap" | "headlines" | "trade") {
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
                    markets::exposure_doc(cx.engine, cx.names, &pf, cx.tables)
                }))
            }
            "markets" => {
                let f = filters()?;
                Box::new(Followers::new(false, move |cx: &Cx| {
                    let pf = cx.engine.portfolio(&f);
                    markets::markets_doc(cx.engine, &pf, cx.tables, cx.following)
                }))
            }
            "heatmap" => {
                let f = filters()?;
                let universe = p.universe.clone().unwrap_or_else(|| "holdings".into());
                let size = p.size.clone().unwrap_or_else(|| "value".into());
                if !matches!(universe.as_str(), "holdings" | "watchlist" | "both" | "ca" | "us" | "intl") {
                    return Err(format!("the heatmap has no universe {universe:?}"));
                }
                if !matches!(size.as_str(), "value" | "equal") {
                    return Err(format!("the heatmap has no sizing {size:?}"));
                }
                // only the holdings' tiles are sized and coloured by their figures
                let sized = matches!(universe.as_str(), "holdings" | "both");
                Box::new(Followers::new(sized, move |cx: &Cx| {
                    let pf = cx.engine.portfolio(&f);
                    markets::heatmap_doc(cx.engine, cx.names, &pf, cx.tables, cx.following, &universe, &size)
                }))
            }
            "headlines" => {
                let shown = crate::wire::news::Shown {
                    scope: p.scope.clone().unwrap_or_else(|| "all".into()),
                    kind: p.kind.clone().unwrap_or_else(|| "stories".into()),
                    symbol: p.symbol.clone().filter(|s| !s.trim().is_empty()),
                    exchange: p.exchange.clone(),
                    query: p.query.clone().unwrap_or_default(),
                };
                if !matches!(shown.scope.as_str(), "all" | "holdings" | "watchlist") {
                    return Err(format!("the news has no scope {:?}", shown.scope));
                }
                if !matches!(shown.kind.as_str(), "stories" | "releases") {
                    return Err(format!("the news has no tab {:?}", shown.kind));
                }
                let sort = p.sort.clone().unwrap_or(Sort { key: "when".into(), dir: Dir::Desc });
                if !matches!(sort.key.as_str(), "when" | "news" | "symbol" | "change") {
                    return Err(format!("the news has no column {:?} to sort by", sort.key));
                }
                Box::new(Headlines { filters: filters()?, shown, sort, limit: p.limit.unwrap_or(FIRST_ROWS), stories: None, followed: BTreeSet::new(), sent: None })
            }
            "trade" => {
                let id = key.strip_prefix("trade:").unwrap_or_default().to_string();
                Box::new(Whole::<TradeDoc>::new(Reads::TradesAndHoldings, move |cx: &Cx| one_trade(cx, &id)))
            }
            "positions" => Box::new(Positions { filters: filters()?, sent: None }),
            "trades" => Box::new(Trades::new(filters()?, p.sort.clone().unwrap_or(Sort { key: "activity".into(), dir: Dir::Desc }), p.limit.unwrap_or(FIRST_ROWS))?),
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
    /// holdings that came or went, the accounts' values as the broker states them.
    Book,
    /// The trades, the accounts' values, the record (today), and one benchmark.
    Dashboard(String),
    /// The holdings, the broker's cash and borrowing beside them (and the record,
    /// and the market's context).
    Holdings,
    /// The trades and the holdings.
    TradesAndHoldings,
}

impl Reads {
    fn reads(&self, moved: &Moved, base: bool) -> bool {
        moved.0.iter().any(|(e, fields)| match (self, e) {
            (_, Entity::Book) => true,
            (Reads::Book, Entity::Trade(_) | Entity::Equity(_) | Entity::Broker(_)) => true,
            (Reads::Book, Entity::Position(..)) => fields.contains("*"),
            (Reads::Dashboard(_), Entity::Trade(_) | Entity::Equity(_)) => true,
            (Reads::Dashboard(b), Entity::Benchmark(k)) => k == b,
            (Reads::Holdings, Entity::Position(..) | Entity::Equity(_) | Entity::Trade(_) | Entity::Broker(_)) => true,
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

/// A subscription to part of the Markets tab: built again when a holding came or
/// went, the record changed, a followed instrument's quote moved, or the market's
/// context did (what is followed, the tables it reads); with `holdings`, when any
/// holding's figures moved too (the heatmap sizes and colours them). Compared
/// field by field with what was sent.
struct Followers<T> {
    holdings: bool,
    build: Box<dyn Fn(&Cx) -> T + Send>,
    /// The instruments followed when it was last built, whose quotes it shows.
    followed: BTreeSet<bagholder_core::InstrumentId>,
    sent: Option<T>,
}

impl<T> Followers<T> {
    fn new(holdings: bool, build: impl Fn(&Cx) -> T + Send + 'static) -> Followers<T> {
        Followers { holdings, build: Box::new(build), followed: BTreeSet::new(), sent: None }
    }

    fn built(&mut self, cx: &Cx) -> T {
        self.followed = cx.following.watched.iter().chain(&cx.following.tiles).map(|f| f.id).collect();
        (self.build)(cx)
    }
}

impl<T: Diff + Serialize + Send> View for Followers<T> {
    fn reads(&self, moved: &Moved, base: bool) -> bool {
        base || moved.0.iter().any(|(e, fields)| match e {
            Entity::Book => true,
            Entity::Position(..) => self.holdings || fields.contains("*"),
            Entity::Quote(i) => self.followed.contains(i),
            _ => false,
        })
    }
    fn snapshot(&mut self, cx: &Cx) -> Value {
        let now = self.built(cx);
        let v = serde_json::to_value(&now).expect("a wire value is plain data");
        self.sent = Some(now);
        v
    }
    fn update(&mut self, cx: &Cx, _moved: &Moved, _base: bool) -> Vec<Value> {
        let now = self.built(cx);
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

/// The News card's list: every story in its scope, tab, chip and words, sorted by
/// its column, the first `limit` sent with how many there are. Built again when
/// the headlines or what is followed changed (the context), a holding moved (a
/// tag carries its day change), or a followed listing's quote moved.
struct Headlines {
    filters: bagholder_engine::scope::Filters,
    shown: crate::wire::news::Shown,
    sort: Sort,
    limit: usize,
    /// The stories, made once from the headlines they were made from.
    stories: Option<(usize, std::sync::Arc<Vec<crate::wire::news::Story>>)>,
    followed: BTreeSet<bagholder_core::InstrumentId>,
    sent: Option<crate::wire::news::HeadlinesDoc>,
}

impl Headlines {
    fn build(&mut self, cx: &Cx) -> crate::wire::news::HeadlinesDoc {
        use crate::wire::news::{self, Known};
        let rows = cx.tables.news();
        let ptr = std::sync::Arc::as_ptr(&rows) as usize;
        let stories = match &self.stories {
            Some((p, s)) if *p == ptr => s.clone(),
            _ => {
                let s = std::sync::Arc::new(news::stories(&rows));
                self.stories = Some((ptr, s.clone()));
                s
            }
        };
        // what the card knows of each listing: held (and as which holding), watched, its change
        let engine = cx.engine;
        let inputs = engine.inputs();
        let pf = engine.portfolio(&self.filters);
        let figs = engine.figures();
        let mut known: HashMap<(String, String), Known> = HashMap::new();
        for i in &pf.positions {
            let p = &figs.positions[*i];
            let s = build::shown(inputs, p.instrument);
            let k = known.entry(news::listing_key(&s.symbol, &s.exchange)).or_insert_with(|| Known { exchange: s.exchange.clone(), ..Known::default() });
            if k.held.is_none() {
                k.held = Some((build::position_id(p), p.mark.as_ref().ok().and_then(|m| m.change_pct).map(|c| c.to_f64() / 100.0)));
            }
        }
        for f in &cx.following.watched {
            let change = inputs.market.quotes.get(&f.id).and_then(|q| q.change_pct).map(|c| c.to_f64() / 100.0);
            let k = known.entry(news::listing_key(&f.symbol, &f.exchange)).or_insert_with(|| Known { exchange: f.exchange.clone(), ..Known::default() });
            k.watched.get_or_insert(change);
        }
        self.followed = cx.following.watched.iter().map(|f| f.id).collect();
        let (filed, filed_failed) = if self.shown.kind == "releases" {
            let chip = self.shown.symbol.as_deref().map(|s| (s, self.shown.exchange.as_deref().unwrap_or_default()));
            match cx.tables.filed_releases(&self.shown.scope, chip) {
                Ok(f) => (f, None),
                Err(e) => (Default::default(), Some(format!("The issuers' filed releases could not be read: {e}"))),
            }
        } else {
            (Default::default(), None)
        };
        let sort = news::NewsSort { key: self.sort.key.clone(), dir: self.sort.dir };
        let (all, chip) = news::headlines(&stories, &filed, &known, &self.shown, &sort);
        news::HeadlinesDoc { total: all.len(), items: all.into_iter().take(self.limit).collect(), chip, filed_failed }
    }
}

impl View for Headlines {
    fn reads(&self, moved: &Moved, base: bool) -> bool {
        base || moved.0.iter().any(|(e, _)| match e {
            Entity::Book | Entity::Position(..) => true,
            Entity::Quote(i) => self.followed.contains(i),
            _ => false,
        })
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
        // the list's own order, no column's: newest activity first, an open trade by its
        // latest fill, a closed one by its close
        "activity" => SortValue::Text(t.last_date.clone()),
        // the Close column: a closed trade by its close; an open one reads `Open`, after
        // every date (`~` sorts after any digit), the open ones by their latest fill
        "exitDate" => SortValue::Text(match &t.exit_date {
            Some(d) => d.clone(),
            None => format!("~{}", t.last_date),
        }),
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
        const COLUMNS: [&str; 14] = ["activity", "entryDate", "exitDate", "symbol", "exchange", "qty", "entry", "exit", "currency", "pnl", "pnlPct", "holdDays", "grade", "tags"];
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
        moved.0.keys().any(|e| matches!(e, Entity::Book | Entity::CashRow(_) | Entity::Payer(_) | Entity::Position(..) | Entity::Broker(_)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Feed, Want};
    use std::sync::Arc;

    /// A change as the page writes it into what it holds (`live.svelte.ts`, `applyOps`).
    fn apply(root: &mut Value, ops: &[Value]) {
        fn walk<'a>(mut at: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
            for s in path {
                at = match s {
                    Value::String(k) => at.get_mut(k.as_str())?,
                    Value::Object(step) => {
                        let (k, v) = (step["k"].as_str()?, step["v"].as_str()?);
                        at.as_array_mut()?.iter_mut().find(|r| r.get(k).and_then(|x| x.as_str().map(String::from).or_else(|| x.as_i64().map(|n| n.to_string()))).as_deref() == Some(v))?
                    }
                    _ => return None,
                };
            }
            Some(at)
        }
        for op in ops {
            let op = op.as_array().expect("an op is a list");
            let path = op[1].as_array().expect("a path").clone();
            match op[0].as_str().unwrap() {
                "rows" => {
                    let key = op[2].as_str().unwrap();
                    let order: Vec<&str> = op[3].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
                    let added = op[4].as_object().unwrap();
                    let list = walk(root, &path).and_then(|l| l.as_array_mut()).expect("the list the rows are in");
                    let text = |r: &Value| r.get(key).and_then(|x| x.as_str().map(String::from).or_else(|| x.as_i64().map(|n| n.to_string()))).unwrap_or_default();
                    let have: std::collections::HashMap<String, Value> = list.drain(..).map(|r| (text(&r), r)).collect();
                    *list = order.iter().map(|id| have.get(*id).cloned().or_else(|| added.get(*id).cloned()).expect("a row it has or is sent")).collect();
                }
                "set" if path.is_empty() => *root = op[2].clone(),
                "set" => {
                    let (last, parent) = path.split_last().unwrap();
                    let at = walk(root, parent).expect("the parent of what is set");
                    match last {
                        Value::String(k) => {
                            at.as_object_mut().expect("an object").insert(k.clone(), op[2].clone());
                        }
                        step => *walk(at, std::slice::from_ref(step)).expect("the row set") = op[2].clone(),
                    }
                }
                "del" => {
                    let (last, parent) = path.split_last().unwrap();
                    walk(root, parent).and_then(|p| p.as_object_mut()).expect("an object").remove(last.as_str().unwrap());
                }
                other => panic!("no op {other}"),
            }
        }
    }

    /// Every screen a page can show, each held as the page holds it.
    const VIEWS: [&str; 9] = ["book", "dashboard", "positions", "trades", "cashflow", "exposure", "markets", "heatmap", "headlines"];

    /// What each screen is asked with: every row, and the heatmap of what is held and watched.
    fn params_of(key: &str) -> Value {
        match key {
            k if k.starts_with("trade:") => serde_json::json!({}),
            "heatmap" => serde_json::json!({"limit": 1_000_000, "universe": "both"}),
            _ => serde_json::json!({"limit": 1_000_000}),
        }
    }

    struct Page {
        feed: Feed,
        held: std::collections::BTreeMap<String, Value>,
    }

    impl Page {
        /// What the stream sent, written into what the page holds; the docs that moved.
        fn step(&mut self) -> Vec<String> {
            self.step_ops().into_iter().map(|(doc, _)| doc).collect()
        }

        /// `step`, with the ops each doc was sent (a snapshot as one `set` of the whole).
        fn step_ops(&mut self) -> Vec<(String, Vec<Value>)> {
            let mut moved = Vec::new();
            for (name, m) in self.feed.step(&crate::status::status) {
                let doc = m["doc"].as_str().unwrap_or_default().to_string();
                let ops = match name {
                    "snapshot" => {
                        self.held.insert(doc.clone(), m["data"].clone());
                        vec![serde_json::json!(["set", [], m["data"]])]
                    }
                    "patch" => {
                        let ops = m["ops"].as_array().unwrap().clone();
                        apply(self.held.get_mut(&doc).expect("a patch to what is held"), &ops);
                        ops
                    }
                    _ => continue,
                };
                moved.push((doc, ops));
            }
            moved
        }
    }

    fn opened() -> (tempfile::TempDir, Arc<crate::app::App>, Page, String) {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = crate::app::App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let feed = Feed::open(app.clone());
        let mut docs: std::collections::BTreeMap<String, Want> = VIEWS.iter().map(|k| (k.to_string(), Want { params: params_of(k), have: None })).collect();
        // a holding's page too
        let held = app.figures.get().unwrap().read(|e| build::position_id(&e.figures().positions[0])).unwrap();
        docs.insert(format!("trade:{held}"), Want { params: serde_json::json!({}), have: None });
        assert!(app.events.watch(&app, feed.id(), docs));
        let mut page = Page { feed, held: Default::default() };
        let first = page.step();
        assert_eq!(first.len(), VIEWS.len() + 1, "every screen sent whole once: {first:?}");
        (home, app, page, held)
    }

    /// What a page holds of each screen is what a fresh build of it says.
    #[track_caller]
    fn same_as_fresh(app: &Arc<crate::app::App>, page: &Page, when: &str) {
        let f = app.figures.get().unwrap();
        let names = f.names().unwrap();
        let context = app.market_context().unwrap();
        let door = crate::wire::context::Door { built: &context, app: &app };
        for (key, held) in &page.held {
            let fresh = f.read(|e| snapshot_of(&Cx { engine: e, names: &names, tables: &door, following: &context.following }, key, params_of(key))).unwrap().unwrap();
            assert!(*held == fresh, "{when}: {key} as the page holds it is not what a fresh build says");
        }
    }

    /// For every kind of change, what the page is sent brings each screen it shows to
    /// exactly what a fresh build says, and a screen that reads nothing that moved is
    /// sent nothing (`docs/architecture.md` §13: the engine's report is what is sent).
    #[test]
    fn every_kind_of_change_brings_each_screen_to_a_fresh_build_and_touches_no_other() {
        let (_home, app, mut page, held) = opened();
        let f = app.figures.get().unwrap();
        let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        assert!(page.step().is_empty(), "nothing moved, nothing sent");

        // a quote for something held: the screens that show a price, and no other
        let (instrument, currency) = f.read(|e| {
            let p = &e.figures().positions[0];
            (p.instrument, e.inputs().ledger.instruments[&p.instrument].instrument.currency)
        }).unwrap();
        let price = bagholder_core::Money::new(bagholder_core::Dec::parse("12.34").unwrap(), currency);
        f.cache().unwrap().store_quote(&bagholder_sources::cache::StoredQuote { instrument, source: bagholder_core::SourceName::named("tmx"), price, change: None, change_pct: None, quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now }).unwrap();
        assert!(!f.price_changed(instrument).unwrap().is_empty());
        let moved = page.step();
        for untouched in ["book", "dashboard", "trades"] {
            assert!(!moved.contains(&untouched.to_string()), "a quote moved the {untouched}: {moved:?}");
        }
        assert!(moved.contains(&"positions".to_string()) && moved.contains(&format!("trade:{held}")), "{moved:?}");
        same_as_fresh(&app, &page, "a quote");

        // the payer's declared record
        let book = f.book().unwrap();
        let row = bagholder_book::facts::DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: "2025-11-03".parse().unwrap(), record_date: None, pay_date: None, amount: bagholder_core::Money::new(bagholder_core::Dec::parse("0.10").unwrap(), currency), reinvested: None };
        book.store_declared(instrument, &[row], &bagholder_core::SourceName::named("tmx"), now).unwrap();
        f.payer_changed(instrument).unwrap();
        page.step();
        same_as_fresh(&app, &page, "a declared distribution");

        // a rate the Bank published
        book.store_rates(bagholder_core::Currency::USD, &[("2025-11-18".parse().unwrap(), bagholder_core::Dec::parse("1.4012").unwrap())], ("2025-11-18".parse().unwrap(), "2025-11-18".parse().unwrap()), &bagholder_core::SourceName::named("bank-of-canada"), now).unwrap();
        f.rates_changed().unwrap();
        page.step();
        same_as_fresh(&app, &page, "a rate");

        // a journal written
        let trade = page.held["trades"]["trades"][0]["id"].as_str().unwrap().to_string();
        f.write_journal(&trade, &bagholder_core::journal::JournalEntry { thesis: "why".into(), grade: Some(bagholder_core::journal::Grade::A), tags: vec!["t".into()] }, now).unwrap();
        let moved = page.step();
        assert!(!moved.contains(&"positions".to_string()) || f.read(|e| e.figures().positions.iter().any(|p| p.trade.is_some_and(|t| t.to_string() == trade))).unwrap(), "a journal moved holdings that are not its trade's: {moved:?}");
        same_as_fresh(&app, &page, "a journal");

        // the broker states an account's cash
        let account = f.read(|e| e.figures().positions[0].account).unwrap();
        let connection = book.connections().unwrap()[0].id;
        let read = book.broker_read(connection, "cash", now).unwrap();
        book.store_cash(account, now, &[(bagholder_core::Currency::CAD, bagholder_core::Dec::parse("42.00").unwrap())].into_iter().collect(), &read).unwrap();
        f.broker_changed(account).unwrap();
        page.step();
        same_as_fresh(&app, &page, "the broker's cash");

        // the record changed: a row the broker no longer lists
        let removed = book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap()[0];
        book.mark_removed(removed, now).unwrap();
        f.record_changed(now).unwrap();
        page.step();
        same_as_fresh(&app, &page, "a record removed");

        // the next day
        f.clock_moved("2025-11-20T21:00:00Z".parse().unwrap()).unwrap();
        page.step();
        same_as_fresh(&app, &page, "the day turning");
    }

    /// The paths a doc's ops touch, as text: `watchlist/<id>/last`.
    fn touched(ops: &[Value]) -> Vec<String> {
        ops.iter()
            .map(|op| {
                op[1].as_array().unwrap().iter().map(|s| match s {
                    Value::String(k) => k.clone(),
                    Value::Object(step) => step["v"].as_str().unwrap_or_default().to_string(),
                    other => other.to_string(),
                }).collect::<Vec<_>>().join("/")
            })
            .collect()
    }

    fn quote(f: &crate::figures::Figures, instrument: bagholder_core::InstrumentId, price: &str, change: &str, pct: &str, at: &str) {
        let now: bagholder_core::jiff::Timestamp = at.parse().unwrap();
        let currency = f.book().unwrap().instrument(instrument).unwrap().currency;
        let d = |s: &str| bagholder_core::Dec::parse(s).unwrap();
        f.cache().unwrap().store_quote(&bagholder_sources::cache::StoredQuote { instrument, source: bagholder_core::SourceName::named("tmx"), price: bagholder_core::Money::new(d(price), currency), change: Some(d(change)), change_pct: Some(d(pct)), quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now }).unwrap();
        assert!(!f.price_changed(instrument).unwrap().is_empty(), "the quote moved something");
    }

    /// A watched listing's quote moving sends its watchlist row and its heatmap tile,
    /// and nothing else; a holding's sends its tile and its sector's block (the
    /// block's value and change are its tiles'), each equal to a fresh build.
    #[test]
    fn a_quote_moves_only_the_markets_rows_that_show_it() {
        let (_home, app, mut page, _) = opened();
        crate::following::ensure(&app).unwrap();
        let f = app.figures.get().unwrap();
        let book = f.book().unwrap();
        let named = crate::following::Named { symbol: "SHOP".into(), exchange: "TSX".into(), name: "Shopify Inc.".into(), currency: "CAD".into(), ..Default::default() };
        let shop = book.watch(&crate::following::draft(&book, &named).unwrap(), "2025-11-19T21:00:00Z".parse().unwrap()).unwrap();
        app.followed();
        page.step();
        same_as_fresh(&app, &page, "a listing watched");
        assert!(page.held["markets"]["watchlist"].as_array().unwrap().iter().any(|w| w["id"] == shop.to_string()));

        quote(f, shop, "101.50", "1.50", "1.5", "2025-11-19T21:01:00Z");
        let sent = page.step_ops();
        let docs: Vec<&str> = sent.iter().map(|(d, _)| d.as_str()).collect();
        assert!(docs.iter().all(|d| ["markets", "heatmap", "headlines"].contains(d)), "a watched quote reached {docs:?}");
        let markets = &sent.iter().find(|(d, _)| d == "markets").expect("the watchlist row moved").1;
        let id = shop.to_string();
        assert!(touched(markets).iter().all(|p| p.starts_with(&format!("watchlist/{id}/"))), "{:?}", touched(markets));
        let row = page.held["markets"]["watchlist"].as_array().unwrap().iter().find(|w| w["id"] == id).unwrap().clone();
        assert_eq!((row["last"].as_str(), row["change"].as_str()), (Some("101.5"), Some("1.5")));
        let heat = &sent.iter().find(|(d, _)| d == "heatmap").expect("its heatmap tile moved").1;
        // its tile, and its block's change (its tiles' weighted), in its one block
        let paths = touched(heat);
        let block = paths[0].split('/').take(2).collect::<Vec<_>>().join("/");
        let tile_key = page.held["heatmap"]["blocks"].as_array().unwrap().iter().flat_map(|b| b["tiles"].as_array().unwrap()).find(|t| t["symbol"] == "SHOP").unwrap()["key"].as_str().unwrap().to_string();
        assert!(paths.iter().all(|p| p.starts_with(&format!("{block}/tiles/{tile_key}/")) || *p == format!("{block}/percentChange")), "the watched tile and its block alone: {paths:?}");
        assert!(paths.iter().any(|p| p.starts_with(&format!("{block}/tiles/"))), "{paths:?}");
        same_as_fresh(&app, &page, "a watched quote");

        // a holding's quote: its tile and its block on the heatmap
        let held = f.read(|e| e.figures().positions[0].instrument).unwrap();
        quote(f, held, "12.34", "0.20", "1.65", "2025-11-19T21:02:00Z");
        let sent = page.step_ops();
        let heat = &sent.iter().find(|(d, _)| d == "heatmap").expect("the holding's tile moved").1;
        let paths = touched(heat);
        assert!(paths.iter().all(|p| p.starts_with("blocks/")), "{paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("/value") || p.ends_with("/percentChange")), "the block's own figures: {paths:?}");
        if let Some((_, m)) = sent.iter().find(|(d, _)| d == "markets") {
            panic!("a holding not watched moved the markets doc: {:?}", touched(m));
        }
        same_as_fresh(&app, &page, "a holding's quote");
    }

    /// A page opening again with the version it kept of a screen is told it is the
    /// same; one holding another version is sent the screen.
    #[test]
    fn a_screen_kept_at_its_version_is_answered_same_and_another_is_sent() {
        let (_home, app, mut page, _) = opened();
        let v = |d: &Value| format!("{:016x}", version_of(d));
        let feed = Feed::open(app.clone());
        let docs = [
            ("book".to_string(), Want { params: serde_json::json!({"limit": 1_000_000}), have: Some(v(&page.held["book"])) }),
            ("dashboard".to_string(), Want { params: serde_json::json!({"limit": 1_000_000}), have: Some("0000000000000000".into()) }),
        ];
        assert!(app.events.watch(&app, feed.id(), docs.into_iter().collect()));
        page.feed = feed;
        let said: Vec<(&str, String)> = page.feed.step(&crate::status::status).into_iter().map(|(n, m)| (n, m["doc"].as_str().unwrap_or_default().to_string())).collect();
        assert!(said.contains(&("same", "book".into())), "{said:?}");
        assert!(said.contains(&("snapshot", "dashboard".into())), "{said:?}");
    }

    /// A long list is sent as far as the page has scrolled, with how long it is.
    #[test]
    fn a_long_list_is_sent_as_far_as_the_page_has_scrolled() {
        let (_home, app, page, _) = opened();
        let all = page.held["trades"]["trades"].as_array().unwrap().len();
        assert!(all > 3, "a book with more trades than the window");
        let f = app.figures.get().unwrap();
        let names = f.names().unwrap();
        let context = app.market_context().unwrap();
        let door = crate::wire::context::Door { built: &context, app: &app };
        let three = f.read(|e| snapshot_of(&Cx { engine: e, names: &names, tables: &door, following: &context.following }, "trades", serde_json::json!({"limit": 3}))).unwrap().unwrap();
        assert_eq!(three["total"], serde_json::json!(all));
        assert_eq!(three["trades"].as_array().unwrap(), &page.held["trades"]["trades"].as_array().unwrap()[..3]);
        // the order is the page's: by symbol, ascending
        let by = f.read(|e| snapshot_of(&Cx { engine: e, names: &names, tables: &door, following: &context.following }, "trades", serde_json::json!({"limit": 1_000_000, "sort": {"key": "symbol", "dir": "asc"}}))).unwrap().unwrap();
        let symbols: Vec<String> = by["trades"].as_array().unwrap().iter().map(|t| t["symbol"].as_str().unwrap().to_lowercase()).collect();
        let mut sorted = symbols.clone();
        sorted.sort();
        assert_eq!(symbols, sorted);
        // a column the list does not have is refused
        assert!(open("trades", &serde_json::json!({"sort": {"key": "nope", "dir": "asc"}})).unwrap().is_err());
    }
}
