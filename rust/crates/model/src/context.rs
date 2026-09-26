//! What the market readers are given until stage 5 moves each behind the source
//! contract (`docs/plans/stage-3c-switch.md` §8): the holdings and what was
//! traded, as the engine states them, and the tables of the market's context the
//! earlier store keeps (quotes of watched listings and tiles, the watchlist, news,
//! exposures, the tiles chosen). Nothing here is a figure: a holding's value is
//! its size on the heatmap and the donuts, already in CAD.

use std::sync::Arc;

use crate::activity::Kind;
use crate::exposure::Exposures;
use crate::input::{NewsRow, Quotes, TileRef, UniverseRow, WatchRow};
use crate::wire::{Ordered, Position};

/// A round trip as the market readers ask of one: what it traded, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Traded {
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub kind: Kind,
    pub entry_date: String,
    /// The day it went flat; empty while it is open.
    pub exit_date: String,
}

#[derive(Clone, Debug)]
pub struct MarketBase {
    pub today: String,
    /// Every holding, its value (`mv`) in CAD; its currency is the instrument's,
    /// which says what venue an option's underlying is read on.
    pub positions: Arc<Vec<Position>>,
    pub traded: Arc<Vec<Traded>>,
    pub quotes: Arc<Quotes>,
    pub exposures: Arc<Exposures>,
    pub watchlist: Arc<Vec<WatchRow>>,
    pub news: Arc<Vec<NewsRow>>,
    pub tiles: Arc<Option<Vec<TileRef>>>,
    pub universes: Arc<Ordered<Vec<UniverseRow>>>,
}

impl MarketBase {
    /// The context of an earlier model's base, for the comparison with it: its
    /// holdings valued in CAD at its own rates.
    pub fn of_base(b: &crate::base::Base) -> MarketBase {
        let positions = b
            .positions
            .iter()
            .map(|p| Position { mv: crate::fx::to_cad(&b.fx, p.mv, &p.currency, &b.today), ..p.clone() })
            .collect();
        let traded = b
            .trades
            .iter()
            .map(|t| Traded { symbol: t.symbol.clone(), exchange: t.exchange.clone(), currency: t.currency.clone(), kind: t.kind, entry_date: t.entry_date.clone(), exit_date: t.exit_date.clone() })
            .collect();
        MarketBase {
            today: b.today.clone(),
            positions: Arc::new(positions),
            traded: Arc::new(traded),
            quotes: b.quotes.clone(),
            exposures: b.exposures.clone(),
            watchlist: b.watchlist.clone(),
            news: b.news.clone(),
            tiles: b.tiles.clone(),
            universes: b.universes.clone(),
        }
    }
}
