//! The market's context as the earlier readers are given it until stage 5
//! (`bagholder_model::context::MarketBase`): the engine's holdings and round
//! trips, and the context tables the earlier store keeps. Built again only when
//! the figures or one of those tables moved, or the day turned.

use std::sync::{Arc, Mutex};

use bagholder_model::context::{MarketBase, Traded};
use bagholder_store::{gens, rows};

use crate::app::App;
use crate::figures::Figures;

/// The earlier store's tables the context reads.
const TABLES: [&str; 6] = ["quotes", "exposures", "watchlist", "news", "tiles", "universes"];

#[derive(Default)]
pub struct MarketContext {
    built: Mutex<Option<(String, Arc<MarketBase>)>>,
}

impl MarketContext {
    pub fn new() -> MarketContext {
        MarketContext::default()
    }

    /// The context now: the one built last when nothing it reads has moved.
    pub fn get(&self, app: &App) -> Result<Arc<MarketBase>, String> {
        let f = app.figures.get().ok_or("the figures are not open")?;
        let conn = app.open().map_err(|e| e.to_string())?;
        let today = f.read(|e| e.inputs().clock.today.to_string()).ok_or("the figures are not built yet")?;
        let key = format!("{}|{}|{}", f.version(), today, gens::key(&gens::all(&conn).map_err(|e| e.to_string())?, &TABLES));
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, b)) = built.as_ref() {
            if *k == key {
                return Ok(b.clone());
            }
        }
        let base = Arc::new(build(f, &conn, today)?);
        *built = Some((key, base.clone()));
        Ok(base)
    }
}

fn build(f: &Figures, conn: &rusqlite::Connection, today: String) -> Result<MarketBase, String> {
    let names = f.names()?;
    let (positions, traded) = f
        .read(|e| {
            let inputs = e.inputs();
            let figs = e.figures();
            let positions: Vec<_> = figs
                .positions
                .iter()
                .map(|p| {
                    let shown = crate::wire::build::position(inputs, &names, p, String::new());
                    crate::wire::context::old(&shown, p.market_cad.as_ref().ok().map(|m| m.amount.to_f64()))
                })
                .collect();
            let traded: Vec<Traded> = figs
                .trades
                .iter()
                .map(|t| {
                    let shown = crate::wire::build::trade(inputs, &names, t, None);
                    Traded {
                        kind: bagholder_model::activity::Kind::parse(&shown.kind).unwrap_or(bagholder_model::activity::Kind::Shares),
                        symbol: shown.symbol,
                        exchange: shown.exchange,
                        currency: shown.currency,
                        entry_date: shown.entry_date,
                        exit_date: shown.exit_date.unwrap_or_default(),
                    }
                })
                .collect();
            (positions, traded)
        })
        .ok_or("the figures are not built yet")?;
    let e = |e: rusqlite::Error| e.to_string();
    Ok(MarketBase {
        today,
        positions: Arc::new(positions),
        traded: Arc::new(traded),
        quotes: Arc::new(rows::quotes(conn).map_err(e)?),
        exposures: Arc::new(rows::exposures(conn).map_err(e)?),
        watchlist: Arc::new(rows::watchlist(conn).map_err(e)?),
        news: Arc::new(rows::news(conn).map_err(e)?),
        tiles: Arc::new(rows::tiles(conn).map_err(e)?),
        universes: Arc::new(rows::universes(conn).map_err(e)?),
    })
}

/// The context of the earlier store's tables alone, holding nothing: for a test
/// of what the readers make of those tables.
#[cfg(test)]
pub fn tables_only(conn: &rusqlite::Connection, today: &str) -> MarketBase {
    MarketBase {
        today: today.to_string(),
        positions: Arc::new(vec![]),
        traded: Arc::new(vec![]),
        quotes: Arc::new(rows::quotes(conn).unwrap()),
        exposures: Arc::new(rows::exposures(conn).unwrap()),
        watchlist: Arc::new(rows::watchlist(conn).unwrap()),
        news: Arc::new(rows::news(conn).unwrap()),
        tiles: Arc::new(rows::tiles(conn).unwrap()),
        universes: Arc::new(rows::universes(conn).unwrap()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_context_holds_the_engine_s_holdings_valued_in_cad_in_their_own_currency_and_is_built_once_per_change() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let f = app.figures.get().unwrap();
        let a = app.market_base().unwrap();
        let want: Vec<(String, f64)> = f
            .read(|e| e.figures().positions.iter().map(|p| (p.currency.as_str().to_string(), p.market_cad.as_ref().map(|m| m.amount.to_f64()).unwrap_or(0.0))).collect())
            .unwrap();
        let got: Vec<(String, f64)> = a.positions.iter().map(|p| (p.currency.clone(), p.mv)).collect();
        assert!(!got.is_empty());
        assert_eq!(got, want);
        // nothing moved: the same context, not a new one
        assert!(std::sync::Arc::ptr_eq(&a, &app.market_base().unwrap()));
    }
}
