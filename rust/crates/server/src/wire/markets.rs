//! The Markets tab and the Portfolio's exposure (`docs/plans/stage-5-interface-and-running.md`,
//! A2): the tile row, the watchlist, the heatmap and the sectors and regions,
//! built from the engine's holdings (their values in CAD), the market cache's
//! quotes and what the person follows, in exact decimals.
//!
//! What the earlier store still keeps of the market's context (the classifier's
//! records, the universes, the news; stage 6 moves them) is asked through
//! [`Tables`], which `context.rs` answers from it: nothing here reads that store.
//!
//! Everything the page used to work out from amounts is worked out here: a
//! sector block's value and its value-weighted day change, the folded
//! `Other (N)` tiles, the donuts' slices. The page lays the treemap out and
//! formats; it does no money arithmetic.

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

use bagholder_core::directory::{self, Entry};
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::{Currency, InstrumentId, Money, Rounding};
use bagholder_engine::input::Quote;
use bagholder_engine::positions::PositionFig;
use bagholder_engine::scope::Portfolio;
use bagholder_engine::Engine;

use super::build::{self, Names};
use super::figures::Slice;
use super::{Dec, Fig};

/// What a holding or a listing with no classifier's record is filed under.
pub const UNCLASSIFIED: &str = "Not classified";
/// A coin's sector, and a watched coin's.
pub const DIGITAL_ASSETS: &str = "Digital assets";

// --------------------------------------------------------------------------
// what the market's context tables say
// --------------------------------------------------------------------------

/// What a classifier's record says a security is exposed to: weights (fractions
/// of one) by sector, their names the Portfolio's, and by country, in the
/// record's order (which decides a tie for the dominant sector).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub sectors: Vec<(String, bagholder_core::Dec)>,
    pub countries: Vec<(String, bagholder_core::Dec)>,
}

impl Record {
    /// The sector it gives most weight to (the first of equals), or
    /// `Not classified`.
    pub fn dominant_sector(&self) -> String {
        let mut best: Option<&(String, bagholder_core::Dec)> = None;
        for s in &self.sectors {
            if s.1 > bagholder_core::Dec::ZERO && best.is_none_or(|b| s.1 > b.1) {
                best = Some(s);
            }
        }
        best.map(|b| b.0.clone()).unwrap_or_else(|| UNCLASSIFIED.to_string())
    }
}

/// One constituent of a market universe, as its source states it.
#[derive(Clone, Debug, PartialEq)]
pub struct UniverseRow {
    pub symbol: String,
    pub name: String,
    /// Its index weight or its market cap: what its tile is sized by.
    pub value: bagholder_core::Dec,
    /// The day's move, as a fraction.
    pub percent_change: Option<f64>,
    pub sector: String,
    pub country: String,
}

/// The market's context tables, as the builders ask of them.
pub trait Tables {
    /// A holding's record, by its broker's security id (a contract's by its
    /// underlying's share record, on the venue its currency suggests first).
    fn holding(&self, kind: InstrumentKind, security: &str, underlying: &str, currency: Currency) -> Option<Record>;
    /// A listing's record, by its symbol and venue as the watchlist names it.
    fn listing(&self, symbol: &str, exchange: &str, currency: Currency) -> Option<Record>;
    /// A market universe's constituents, in the source's order: `ca`, `us`, `intl`.
    fn universe(&self, key: &str) -> Vec<UniverseRow>;
    /// Every headline the news readers kept.
    fn news(&self) -> std::sync::Arc<Vec<super::news::NewsRow>>;
    /// The issuers' filed news releases: every listing's in the News card's
    /// `scope`, or the chip's listing's alone.
    fn filed_releases(&self, scope: &str, chip: Option<(&str, &str)>) -> Result<std::sync::Arc<Vec<super::news::FiledRelease>>, String>;
}

// --------------------------------------------------------------------------
// what the person follows
// --------------------------------------------------------------------------

/// An instrument the person follows, as the book holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Followed {
    pub id: InstrumentId,
    pub kind: InstrumentKind,
    pub currency: Currency,
    /// Its symbol now, bare (`QNC`, never `QNC.TO`).
    pub symbol: String,
    /// Its venue in words (`TSX-V`, `Index`, `CME`, `Crypto`).
    pub exchange: String,
    /// Its venue's market identifier code, where one is known.
    pub mic: Option<String>,
    pub name: String,
    /// Its entry in the app's directory, for an index, a future, a rate or a pair.
    pub directory: Option<&'static Entry>,
}

/// What the person follows: the watched listings, newest first, and the tile row
/// in its order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Following {
    pub watched: Vec<Followed>,
    pub tiles: Vec<Followed>,
}

// --------------------------------------------------------------------------
// the wire
// --------------------------------------------------------------------------

/// One tile of the Markets tab's row: an instrument of the directory.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct MarketTile {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    /// What people call it (`GOLD`, `10Y`), else its symbol.
    pub label: String,
    pub name: String,
    pub kind: String,
    pub last: Option<Dec>,
    /// The day's change in points.
    pub change: Option<Dec>,
    /// The day's change, as a fraction.
    pub percent_change: Option<f64>,
    /// Its prices' scale.
    pub decimals: u32,
    /// Quoted as 100 minus the rate it settles against: the tile shows the rate.
    pub priced_as_rate: bool,
    /// That rate, 100 − the price; none for any other instrument, or before a quote.
    pub rate: Option<Dec>,
    /// The rate's move, against the price's; none where the price's is not known.
    pub rate_change: Option<Dec>,
}

/// A watched listing.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct WatchItem {
    /// The instrument, which removing it names.
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub name: String,
    /// What its prices are in.
    pub currency: String,
    pub last: Option<Dec>,
    /// The day's change in points.
    pub change: Option<Dec>,
    /// The day's change, as a fraction.
    pub percent_change: Option<f64>,
    pub sector: String,
    /// `Shares`, `Crypto`, or the directory's kind (`Index`, `Future`, …).
    pub kind: String,
    /// The holding it is, where the book holds it: a click opens that page.
    pub position_id: Option<String>,
}

/// An instrument of the directory, for the tile picker and ⌘K.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = key)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    /// `SYMBOL@VENUE`.
    pub key: String,
    pub symbol: String,
    pub label: String,
    pub name: String,
    pub exchange: String,
    pub kind: String,
    pub aliases: Vec<String>,
}

/// The Markets tab's tile row and watchlist, and the directory the tile picker
/// offers.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct MarketsDoc {
    pub tiles: Vec<MarketTile>,
    pub watchlist: Vec<WatchItem>,
    pub directory: Vec<DirectoryEntry>,
}

/// One tile of a heatmap.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = key)]
#[serde(rename_all = "camelCase")]
pub struct HeatTile {
    /// Its own in the heatmap: its symbol, told apart from another of the same by
    /// its venue, a folded remainder by its sector. A tile keeps it from one
    /// universe or sizing to the next, so it travels.
    pub key: String,
    /// The holding it is, where it is one: a click opens that page.
    pub id: Option<String>,
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub name: String,
    /// What it is sized by: its value in CAD, its index weight or market cap, or
    /// one under `Equal`.
    pub value: Dec,
    /// The day's change, as a fraction; for `Other (N)`, its tiles' value-weighted.
    pub percent_change: Option<f64>,
    /// Several small tiles folded into one, which opens nothing.
    pub other: bool,
}

/// One sector's block.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = label)]
#[serde(rename_all = "camelCase")]
pub struct HeatBlock {
    pub label: String,
    /// Σ its tiles' values.
    pub value: Dec,
    /// Its tiles' value-weighted day change, as a fraction.
    pub percent_change: Option<f64>,
    pub tiles: Vec<HeatTile>,
}

/// How many tiles each universe has: the slideshow passes over an empty one.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct HeatCounts {
    pub holdings: usize,
    pub watchlist: usize,
    pub both: usize,
    pub ca: usize,
    pub us: usize,
    pub intl: usize,
}

/// The heatmap for the universe and the sizing the page shows.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapDoc {
    pub universe: String,
    pub size: String,
    pub blocks: Vec<HeatBlock>,
    pub counts: HeatCounts,
}

/// The Portfolio's sectors and regions.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct ExposureDoc {
    pub sectors: Vec<Slice>,
    pub regions: Vec<Slice>,
}

// --------------------------------------------------------------------------
// the builders
// --------------------------------------------------------------------------

fn pct(q: &Quote) -> Option<f64> {
    q.change_pct.map(|c| c.to_f64() / 100.0)
}

/// The holding a followed instrument is, among `positions`: its own instrument,
/// else a holding called the same on the same venue now (a lookup, as ⌘K's).
fn held_as(engine: &Engine, positions: &[&PositionFig], f: &Followed) -> Option<String> {
    let inputs = engine.inputs();
    let by_id = positions.iter().find(|p| p.instrument == f.id);
    let by_name = || {
        positions.iter().find(|p| {
            let s = build::shown(inputs, p.instrument);
            if bagholder_sources::venue::root(&s.symbol) != f.symbol {
                return false;
            }
            match (p.kind, f.kind) {
                (InstrumentKind::Crypto, InstrumentKind::Crypto) => true,
                (InstrumentKind::Security, InstrumentKind::Security) => {
                    let mic = inputs.ledger.instruments.get(&p.instrument).and_then(|i| i.current_name()).and_then(|n| n.venue_mic.clone().or_else(|| n.venue_name.as_deref().and_then(bagholder_sources::venue::mic_of).map(str::to_string)));
                    mic.is_some() && mic == f.mic
                }
                _ => false,
            }
        })
    };
    by_id.or_else(by_name).map(|p| build::position_id(p))
}

fn tile(engine: &Engine, f: &Followed) -> Option<MarketTile> {
    let e = f.directory?;
    let q = engine.inputs().market.quotes.get(&f.id);
    let last = q.map(|q| q.price.amount);
    let change = q.and_then(|q| q.change);
    let rated = e.priced_as_rate();
    Some(MarketTile {
        id: f.id.to_string(),
        symbol: e.symbol.into(),
        exchange: e.exchange.into(),
        label: directory::label(e.symbol),
        name: e.name.into(),
        kind: e.kind.into(),
        last: last.map(Dec),
        change: change.map(Dec),
        percent_change: q.and_then(pct),
        decimals: e.decimals(),
        priced_as_rate: rated,
        rate: rate_of(last, rated).map(Dec),
        rate_change: change.filter(|_| rated).map(|c| Dec(c.neg())),
    })
}

/// The rate a contract quoted as 100 minus it settles against: 100 − the price.
fn rate_of(last: Option<bagholder_core::Dec>, rated: bool) -> Option<bagholder_core::Dec> {
    last.filter(|_| rated).and_then(|p| bagholder_core::Dec::from_int(100).checked_sub(p).ok())
}

fn watch_item(engine: &Engine, t: &dyn Tables, positions: &[&PositionFig], f: &Followed) -> WatchItem {
    let q = engine.inputs().market.quotes.get(&f.id);
    let (exchange, sector, kind) = match (f.directory, f.kind) {
        (Some(e), _) => (e.exchange.to_string(), directory::kind_label(e.kind), e.kind.to_string()),
        (None, InstrumentKind::Crypto) => ("Crypto".to_string(), DIGITAL_ASSETS.to_string(), "Crypto".to_string()),
        _ => (f.exchange.clone(), t.listing(&f.symbol, &f.exchange, f.currency).map(|r| r.dominant_sector()).unwrap_or_else(|| UNCLASSIFIED.to_string()), "Shares".to_string()),
    };
    WatchItem {
        id: f.id.to_string(),
        symbol: f.symbol.clone(),
        exchange,
        name: f.name.clone(),
        currency: f.currency.as_str().into(),
        last: q.map(|q| Dec(q.price.amount)),
        change: q.and_then(|q| q.change).map(Dec),
        percent_change: q.and_then(pct),
        sector,
        kind,
        position_id: held_as(engine, positions, f),
    }
}

/// The directory, as the tile picker lists it.
pub fn directory_rows() -> Vec<DirectoryEntry> {
    directory::INSTRUMENTS
        .iter()
        .map(|e| DirectoryEntry { key: e.key(), symbol: e.symbol.into(), label: directory::label(e.symbol), name: e.name.into(), exchange: e.exchange.into(), kind: e.kind.into(), aliases: e.aliases.iter().map(|a| a.to_string()).collect() })
        .collect()
}

fn in_scope<'a>(engine: &'a Engine, pf: &Portfolio) -> Vec<&'a PositionFig> {
    let figs = engine.figures();
    pf.positions.iter().map(|i| &figs.positions[*i]).collect()
}

/// The tile row, the watchlist and the directory, the holdings in `pf` telling
/// which watched listings are held.
pub fn markets_doc(engine: &Engine, pf: &Portfolio, t: &dyn Tables, following: &Following) -> MarketsDoc {
    let positions = in_scope(engine, pf);
    MarketsDoc {
        tiles: following.tiles.iter().filter_map(|f| tile(engine, f)).collect(),
        watchlist: following.watched.iter().map(|f| watch_item(engine, t, &positions, f)).collect(),
        directory: directory_rows(),
    }
}

/// A tile before it is grouped: what it is sized by is a decimal, or none where
/// a holding's value waits (it takes the smallest holding's size).
struct Raw {
    id: Option<String>,
    symbol: String,
    exchange: String,
    currency: String,
    name: String,
    value: Option<bagholder_core::Dec>,
    percent_change: Option<f64>,
    sector: String,
}

/// The sector a holding sits under: a coin under `Digital assets`, anything else
/// under its record's dominant sector.
fn holding_sector(engine: &Engine, names: &Names, t: &dyn Tables, p: &PositionFig) -> String {
    if p.kind == InstrumentKind::Crypto {
        return DIGITAL_ASSETS.to_string();
    }
    let inputs = engine.inputs();
    let security = names.security.get(&p.instrument).cloned().unwrap_or_default();
    let underlying = build::shown(inputs, bagholder_engine::scope::underlying_of(inputs, p.instrument)).symbol;
    t.holding(p.kind, &security, &underlying, p.currency).map(|r| r.dominant_sector()).unwrap_or_else(|| UNCLASSIFIED.to_string())
}

/// One tile per symbol held, its value in CAD summed over the accounts holding
/// it; a holding whose value waits is still one, at the smallest holding's size.
fn held_tiles(engine: &Engine, names: &Names, t: &dyn Tables, positions: &[&PositionFig]) -> Vec<Raw> {
    let inputs = engine.inputs();
    let mut out: Vec<Raw> = Vec::new();
    for p in positions {
        let value = p.market_cad.as_ref().ok().map(|m| m.amount);
        if value.is_some_and(|v| v <= bagholder_core::Dec::ZERO) {
            continue;
        }
        let s = build::shown(inputs, p.instrument);
        let symbol = bagholder_sources::venue::root(&s.symbol);
        if let Some(known) = out.iter_mut().find(|r| r.symbol == symbol && r.exchange.eq_ignore_ascii_case(&s.exchange)) {
            known.value = match (known.value, value) {
                (Some(a), Some(b)) => Some(add(a, b)),
                (a, b) => a.or(b),
            };
            continue;
        }
        out.push(Raw {
            id: Some(build::position_id(p)),
            symbol,
            exchange: s.exchange,
            currency: p.currency.as_str().into(),
            name: s.name,
            value,
            percent_change: p.mark.as_ref().ok().and_then(|m| m.change_pct).map(|c| c.to_f64() / 100.0),
            sector: holding_sector(engine, names, t, p),
        });
    }
    out
}

/// The smallest holding's size, or one where none is valued.
fn smallest(held: &[Raw]) -> bagholder_core::Dec {
    held.iter().filter_map(|r| r.value).min().unwrap_or(bagholder_core::Dec::ONE)
}

fn universe_tiles(t: &dyn Tables, key: &str) -> Vec<Raw> {
    let (exchange, currency) = match key {
        "ca" => ("TSX", "CAD"),
        _ => ("", "USD"),
    };
    t.universe(key)
        .into_iter()
        .map(|r| Raw {
            id: None,
            symbol: r.symbol,
            exchange: exchange.into(),
            currency: currency.into(),
            name: r.name,
            value: Some(r.value),
            percent_change: r.percent_change,
            sector: if r.sector.is_empty() { UNCLASSIFIED.to_string() } else { r.sector },
        })
        .collect()
}

/// The value-weighted mean of the tiles' day changes, over those that have one.
fn weighted(tiles: &[(bagholder_core::Dec, Option<f64>)]) -> Option<f64> {
    let (mut sum, mut weight) = (0.0_f64, 0.0_f64);
    for (v, c) in tiles {
        if let Some(c) = c {
            let v = v.to_f64();
            sum += v * c;
            weight += v;
        }
    }
    (weight != 0.0).then(|| sum / weight)
}

/// `a + b`, exact where it fits in `Dec`'s digits, else rounded once at the last
/// place that holds it: a dust coin's value, worth a fraction of a cent to many
/// places, beside a five-figure holding (`add_to_fit`).
fn add(a: bagholder_core::Dec, b: bagholder_core::Dec) -> bagholder_core::Dec {
    // beyond `Dec`'s range altogether a heatmap's size is its larger part
    a.add_to_fit(b).unwrap_or(a.max(b))
}

/// The tiles grouped by sector, in the order each sector is first met, each
/// block's small tiles (under 1.5% of its value) folded into one `Other (N)` when
/// there are two or more, every tile given its key.
fn blocks(tiles: Vec<Raw>, equal: bool) -> Vec<HeatBlock> {
    let fallback = smallest(&tiles);
    let mut order: Vec<String> = Vec::new();
    let mut by: BTreeMap<String, Vec<Raw>> = BTreeMap::new();
    for t in tiles {
        if !by.contains_key(&t.sector) {
            order.push(t.sector.clone());
        }
        by.entry(t.sector.clone()).or_default().push(t);
    }
    let mut taken: std::collections::BTreeSet<String> = Default::default();
    let mut key = |base: String, exchange: &str, i: usize| {
        let mut k = base;
        if taken.contains(&k) {
            k = format!("{k}|{exchange}");
        }
        if taken.contains(&k) {
            k = format!("{k}|{i}");
        }
        taken.insert(k.clone());
        k
    };
    let mut out = Vec::new();
    let mut n = 0usize;
    for label in order {
        let rows = by.remove(&label).unwrap_or_default();
        let sized: Vec<(Raw, bagholder_core::Dec)> = rows.into_iter().map(|r| {
            let v = if equal { bagholder_core::Dec::ONE } else { r.value.unwrap_or(fallback) };
            (r, v)
        }).collect();
        let value = sized.iter().fold(bagholder_core::Dec::ZERO, |a, (_, v)| add(a, *v));
        let change = weighted(&sized.iter().map(|(r, v)| (*v, r.percent_change)).collect::<Vec<_>>());
        let floor = value.mul_to_fit(bagholder_core::Dec::new(15, 3).expect("0.015")).unwrap_or(bagholder_core::Dec::ZERO);
        let small: Vec<usize> = sized.iter().enumerate().filter(|(_, (_, v))| *v < floor).map(|(i, _)| i).collect();
        let fold = small.len() > 1;
        let mut tiles = Vec::new();
        let mut folded: Vec<(bagholder_core::Dec, Option<f64>)> = Vec::new();
        for (i, (r, v)) in sized.into_iter().enumerate() {
            n += 1;
            if fold && small.contains(&i) {
                folded.push((v, r.percent_change));
                continue;
            }
            tiles.push(HeatTile { key: key(r.symbol.clone(), &r.exchange, n), id: r.id, symbol: r.symbol, exchange: r.exchange, currency: r.currency, name: r.name, value: Dec(v), percent_change: r.percent_change, other: false });
        }
        if fold {
            let v = folded.iter().fold(bagholder_core::Dec::ZERO, |a, (x, _)| add(a, *x));
            let symbol = format!("Other ({})", folded.len());
            tiles.push(HeatTile { key: key(format!("other|{label}"), "", n), id: None, symbol, exchange: String::new(), currency: String::new(), name: String::new(), value: Dec(v), percent_change: weighted(&folded), other: true });
        }
        out.push(HeatBlock { label, value: Dec(value), percent_change: change, tiles });
    }
    out
}

/// The watched listings not held, each at `size`.
fn watched_tiles(engine: &Engine, t: &dyn Tables, positions: &[&PositionFig], following: &Following, size: bagholder_core::Dec) -> Vec<Raw> {
    following
        .watched
        .iter()
        .map(|f| watch_item(engine, t, positions, f))
        .filter(|w| w.position_id.is_none())
        .map(|w| Raw { id: None, symbol: w.symbol, exchange: w.exchange, currency: w.currency, name: w.name, value: Some(size), percent_change: w.percent_change, sector: w.sector })
        .collect()
}

/// The heatmap of `universe` (`holdings`, `watchlist`, `both`, `ca`, `us`,
/// `intl`) sized by `size` (`value` or `equal`), the holdings those in `pf`.
pub fn heatmap_doc(engine: &Engine, names: &Names, pf: &Portfolio, t: &dyn Tables, following: &Following, universe: &str, size: &str) -> HeatmapDoc {
    let positions = in_scope(engine, pf);
    let held = held_tiles(engine, names, t, &positions);
    let least = smallest(&held);
    let watched = watched_tiles(engine, t, &positions, following, bagholder_core::Dec::ONE);
    let counts = HeatCounts {
        holdings: held.len(),
        watchlist: watched.len(),
        both: held.len() + watched.len(),
        ca: t.universe("ca").len(),
        us: t.universe("us").len(),
        intl: t.universe("intl").len(),
    };
    let tiles: Vec<Raw> = match universe {
        "holdings" => held,
        "watchlist" => watched,
        "both" => held.into_iter().chain(watched_tiles(engine, t, &positions, following, least)).collect(),
        "ca" | "us" | "intl" => universe_tiles(t, universe),
        _ => Vec::new(),
    };
    HeatmapDoc { universe: universe.into(), size: size.into(), blocks: blocks(tiles, size == "equal"), counts }
}

/// Spread amounts by name, in the order each name is first met.
#[derive(Default)]
struct Spread {
    order: Vec<String>,
    total: BTreeMap<String, bagholder_core::Dec>,
    unclassified: bagholder_core::Dec,
}

impl Spread {
    fn add(&mut self, name: &str, amount: bagholder_core::Dec) {
        if !self.total.contains_key(name) {
            self.order.push(name.to_string());
        }
        let t = self.total.entry(name.to_string()).or_insert(bagholder_core::Dec::ZERO);
        *t = add(*t, amount);
    }

    /// The slices, largest first, the rest past `cap` folded into `Other (n)`,
    /// `Not classified` last: each with its share of `all`.
    fn slices(self, all: bagholder_core::Dec, cap: usize) -> Vec<Slice> {
        let share = |v: bagholder_core::Dec| if all.is_zero() { 0.0 } else { v.to_f64() / all.to_f64() };
        let mut rows: Vec<(String, bagholder_core::Dec)> = self.order.iter().filter_map(|n| self.total.get(n).copied().filter(|v| v.is_positive()).map(|v| (n.clone(), v))).collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        let rest = if rows.len() > cap { rows.split_off(cap) } else { Vec::new() };
        let mut out: Vec<Slice> = rows.into_iter().map(|(label, v)| Slice { label, value: Fig::Stated(Dec(v)), share: share(v), id: None }).collect();
        if !rest.is_empty() {
            let v = rest.iter().fold(bagholder_core::Dec::ZERO, |a, (_, x)| add(a, *x));
            out.push(Slice { label: format!("Other ({})", rest.len()), value: Fig::Stated(Dec(v)), share: share(v), id: None });
        }
        // what no record covers, to the cent
        if self.unclassified > bagholder_core::Dec::new(5, 3).expect("0.005") {
            out.push(Slice { label: UNCLASSIFIED.into(), value: Fig::Stated(Dec(self.unclassified)), share: share(self.unclassified), id: None });
        }
        out
    }
}

/// `v × w`, to the cent.
fn part(v: bagholder_core::Dec, w: bagholder_core::Dec) -> bagholder_core::Dec {
    v.mul_to_fit(w).map(|x| x.round(2, Rounding::HalfEven)).unwrap_or(bagholder_core::Dec::ZERO)
}

/// The Portfolio's sectors (twelve at most, the rest folded) and regions (ten):
/// each holding worth something spread by its record's weights, a fund looked
/// through to what it holds, a coin its own sector and no country's, whatever no
/// record covers named `Not classified`.
pub fn exposure_doc(engine: &Engine, names: &Names, pf: &Portfolio, t: &dyn Tables) -> ExposureDoc {
    let figs = engine.figures();
    let inputs = engine.inputs();
    let (mut sectors, mut countries) = (Spread::default(), Spread::default());
    let mut total = bagholder_core::Dec::ZERO;
    for a in &pf.allocation {
        let p = &pf.held(figs.positions)[a.position];
        let v: Money = a.value;
        if !v.amount.is_positive() {
            continue;
        }
        total = add(total, v.amount);
        let record = if p.kind == InstrumentKind::Crypto {
            Record { sectors: vec![(DIGITAL_ASSETS.to_string(), bagholder_core::Dec::ONE)], countries: vec![] }
        } else {
            let security = names.security.get(&p.instrument).cloned().unwrap_or_default();
            let underlying = build::shown(inputs, bagholder_engine::scope::underlying_of(inputs, p.instrument)).symbol;
            t.holding(p.kind, &security, &underlying, p.currency).unwrap_or_default()
        };
        for (name, w) in &record.sectors {
            sectors.add(name, part(v.amount, *w));
        }
        for (name, w) in &record.countries {
            countries.add(name, part(v.amount, *w));
        }
        let uncovered = |weights: &[(String, bagholder_core::Dec)]| {
            let covered = weights.iter().fold(bagholder_core::Dec::ZERO, |a, (_, w)| add(a, *w)).min(bagholder_core::Dec::ONE);
            part(v.amount, bagholder_core::Dec::ONE.checked_sub(covered).unwrap_or(bagholder_core::Dec::ZERO).max(bagholder_core::Dec::ZERO))
        };
        sectors.unclassified = add(sectors.unclassified, uncovered(&record.sectors));
        countries.unclassified = add(countries.unclassified, uncovered(&record.countries));
    }
    ExposureDoc { sectors: sectors.slices(total, 12), regions: countries.slices(total, 10) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> bagholder_core::Dec {
        bagholder_core::Dec::parse(s).unwrap()
    }

    fn raw(symbol: &str, sector: &str, value: Option<&str>, pct: Option<f64>) -> Raw {
        Raw { id: None, symbol: symbol.into(), exchange: "TSX".into(), currency: "CAD".into(), name: symbol.into(), value: value.map(d), percent_change: pct, sector: sector.into() }
    }

    /// Each sector a block in the order first met, its value the sum of its tiles'
    /// and its change their value-weighted change; the tiles under 1.5% of their
    /// block folded into one `Other (N)` when there are two or more, keyed by the
    /// sector, its change theirs weighted.
    #[test]
    fn a_heatmap_is_blocks_of_sectors_with_the_small_tiles_folded() {
        let tiles = vec![
            raw("BIG", "Energy", Some("1000"), Some(0.02)),
            raw("S1", "Energy", Some("10"), Some(0.01)),
            raw("S2", "Energy", Some("5"), None),
            raw("S3", "Energy", Some("5"), Some(-0.03)),
            raw("TECH", "Technology", Some("300"), Some(-0.01)),
            raw("TINY", "Technology", Some("1"), Some(0.05)),
        ];
        let b = blocks(tiles, false);
        assert_eq!(b.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["Energy", "Technology"]);
        let energy = &b[0];
        assert_eq!(energy.value, Dec(d("1020")));
        // (1000 × 0.02 + 10 × 0.01 + 5 × −0.03) / 1015: a tile with no change weighs nothing
        let want = (1000.0 * 0.02 + 10.0 * 0.01 + 5.0 * -0.03) / 1015.0;
        assert!((energy.percent_change.unwrap() - want).abs() < 1e-12);
        let keys: Vec<&str> = energy.tiles.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["BIG", "other|Energy"]);
        let other = &energy.tiles[1];
        assert_eq!((other.symbol.as_str(), other.value, other.other, other.id.as_deref()), ("Other (3)", Dec(d("20")), true, None));
        assert!((other.percent_change.unwrap() - (10.0 * 0.01 + 5.0 * -0.03) / 15.0).abs() < 1e-12);
        // one small tile alone is not folded
        assert_eq!(b[1].tiles.iter().map(|t| t.symbol.as_str()).collect::<Vec<_>>(), ["TECH", "TINY"]);
    }

    /// A dust coin beside a large holding: the block is the tiles' sum to the
    /// last place that holds it, never the larger part alone.
    #[test]
    fn a_block_holding_dust_and_a_large_holding_is_their_sum() {
        let b = blocks(vec![raw("BIG", "Digital assets", Some("33649.99"), None), raw("DUST", "Digital assets", Some("0.000000264188636702736309916"), None)], false);
        assert_eq!(b[0].value.0.round(8, bagholder_core::Rounding::HalfEven), d("33649.99000026"));
    }

    #[test]
    fn equal_sizing_gives_every_tile_one_and_a_tile_with_no_value_takes_the_smallest() {
        let tiles = vec![raw("A", "Energy", Some("900"), Some(0.01)), raw("B", "Energy", Some("50"), None), raw("C", "Energy", None, None)];
        let b = blocks(tiles, false);
        assert_eq!(b[0].tiles.iter().map(|t| t.value).collect::<Vec<_>>(), [Dec(d("900")), Dec(d("50")), Dec(d("50"))]);
        let tiles = vec![raw("A", "Energy", Some("900"), Some(0.01)), raw("B", "Energy", Some("50"), None)];
        let b = blocks(tiles, true);
        assert_eq!((b[0].value, b[0].tiles.iter().map(|t| t.value).collect::<Vec<_>>()), (Dec(d("2")), vec![Dec(d("1")), Dec(d("1"))]));
    }

    #[test]
    fn a_symbol_met_twice_is_told_apart_by_its_venue() {
        let mut other = raw("ABC", "Energy", Some("100"), None);
        other.exchange = "NYSE".into();
        let b = blocks(vec![raw("ABC", "Energy", Some("100"), None), other], false);
        assert_eq!(b[0].tiles.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["ABC", "ABC|NYSE"]);
    }

    /// The Portfolio's slices: largest first, those past the cap folded into
    /// `Other (n)`, what no record covers last, each with its share of the whole.
    #[test]
    fn exposure_slices_are_largest_first_the_rest_folded_and_the_unclassified_last() {
        let mut sp = Spread::default();
        for (name, v) in [("Energy", "300"), ("Tech", "500"), ("Utilities", "100"), ("Energy", "100"), ("Materials", "50")] {
            sp.add(name, d(v));
        }
        sp.unclassified = d("50");
        let all = d("1100");
        let got: Vec<(String, Fig<Dec>, f64)> = sp.slices(all, 2).into_iter().map(|x| (x.label, x.value, x.share)).collect();
        assert_eq!(got, vec![
            ("Tech".to_string(), Fig::Stated(Dec(d("500"))), 500.0 / 1100.0),
            ("Energy".to_string(), Fig::Stated(Dec(d("400"))), 400.0 / 1100.0),
            ("Other (2)".to_string(), Fig::Stated(Dec(d("150"))), 150.0 / 1100.0),
            ("Not classified".to_string(), Fig::Stated(Dec(d("50"))), 50.0 / 1100.0),
        ]);
    }

    #[test]
    fn a_contract_quoted_as_100_minus_a_rate_shows_the_rate() {
        assert_eq!(rate_of(Some(d("95.965")), true), Some(d("4.035")));
        assert_eq!(rate_of(Some(d("95.965")), false), None);
        assert_eq!(rate_of(None, true), None);
    }
}
