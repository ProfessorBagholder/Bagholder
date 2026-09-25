//! The market around the book (`docs/plans/stage-3c-switch.md`, §4): the
//! Markets tab (holdings heatmap, watchlist, news, universes, tiles) and the
//! Portfolio's sectors and regions. Their readers and their types stay the old
//! ones until stage 5 moves each behind the source contract; what changes here is
//! only that they are given the engine's holdings, by the engine's ids, in place
//! of the old model's. Their values are the old types' numbers: sizes a heatmap
//! and a donut draw, never an amount the page adds.

use bagholder_model::activity::Kind;
use bagholder_model::base::Base;
use bagholder_model::wire::{ExposureSlice, Markets, Mark, Position as OldPosition};

use super::figures::Position;
use super::Fig;

/// The context as the page draws it.
#[derive(Clone, Debug)]
pub struct Context {
    pub markets: Markets,
    pub sectors: Vec<ExposureSlice>,
    pub regions: Vec<ExposureSlice>,
}

fn number(f: &Fig<super::Dec>) -> f64 {
    match f {
        Fig::Stated(d) => d.0.to_f64(),
        Fig::Waits { .. } => 0.0,
    }
}

/// A holding as the old readers take one: what they read of it.
fn old(p: &Position) -> OldPosition {
    OldPosition {
        id: p.id.clone(),
        symbol: p.symbol.clone(),
        underlying: p.underlying.clone(),
        name: p.name.clone(),
        exchange: p.exchange.clone(),
        kind: Kind::parse(&p.kind).unwrap_or(Kind::Shares),
        account: p.account.clone(),
        account_id: p.account_id.clone(),
        currency: p.currency.clone(),
        security_id: p.security.clone(),
        short: p.short,
        qty: 0.0,
        mult: 0,
        avg: 0.0,
        cost: 0.0,
        fees: 0.0,
        last: 0.0,
        price_source: Mark::Quote,
        price_change: None,
        // the old readers take a percentage
        percent_change: p.percent_change.map(|f| f * 100.0),
        day_change: None,
        // a holding with no value states none: it is sized by nothing
        mv: number(&p.mv),
        unreal: 0.0,
        unreal_pct: None,
        held: 0,
        opened: p.opened.clone(),
        ws_qty: None,
        rt: None,
        lots: Vec::new(),
        fills: None,
        grade: String::new(),
        thesis: String::new(),
        tags: Vec::new(),
        alloc: 0.0,
    }
}

/// The largest first, up to `cap`, the rest folded into `Other (n)`, and what is
/// not classified last.
fn folded(rows: Vec<ExposureSlice>, cap: usize) -> Vec<ExposureSlice> {
    let (unclassified, mut known): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| r.name == bagholder_model::exposure::UNCLASSIFIED);
    let rest = if known.len() > cap { known.split_off(cap) } else { Vec::new() };
    if !rest.is_empty() {
        known.push(ExposureSlice { name: format!("Other ({})", rest.len()), value: rest.iter().map(|r| r.value).sum(), share: rest.iter().map(|r| r.share).sum() });
    }
    known.extend(unclassified);
    known
}

/// The context for the holdings in scope.
pub fn context(base: &Base, positions: &[Position]) -> Context {
    let olds: Vec<OldPosition> = positions.iter().map(old).collect();
    let refs: Vec<&OldPosition> = olds.iter().collect();
    let cad = |amount: f64, currency: &str| bagholder_model::fx::to_cad(&base.fx, amount, currency, &base.today);
    let (sectors, regions) = bagholder_model::exposure::exposure_slices(&refs, &base.exposures, &cad);
    Context { markets: bagholder_model::markets::markets_view(base, &refs), sectors: folded(sectors, 12), regions: folded(regions, 10) }
}
