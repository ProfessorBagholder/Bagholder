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

/// Every Wealthsimple security the book has met, as the earlier readers take
/// one: its id, what the book calls it now, its venue and currency, and for a
/// contract what it is on.
pub fn securities(app: &App) -> Result<Vec<bagholder_model::securities::Security>, String> {
    use bagholder_core::instrument::RefScheme;
    let f = app.figures.get().ok_or("the figures are not open")?;
    let book = f.book()?;
    let e = |e: bagholder_book::BookError| e.to_string();
    let ws = bagholder_core::Broker::named("wealthsimple");
    let ws_id = |i: bagholder_core::InstrumentId| -> Result<Option<String>, String> {
        Ok(book.instrument_refs(i).map_err(e)?.into_iter().find(|r| r.scheme == RefScheme::BrokerSecurity(ws.clone())).map(|r| r.value))
    };
    let mut out = Vec::new();
    for i in book.instruments().map_err(e)? {
        let Some(id) = ws_id(i.id)? else { continue };
        let name = book.names(i.id).map_err(e)?.last().cloned();
        let underlying_id = match book.option_terms(i.id).map_err(e)? {
            Some(t) => ws_id(t.underlying)?,
            None => None,
        };
        out.push(bagholder_model::securities::Security {
            id,
            symbol: name.as_ref().map(|n| n.symbol.clone()).unwrap_or_default(),
            name: name.as_ref().and_then(|n| n.name.clone()).unwrap_or_default(),
            primary_exchange: name.as_ref().and_then(|n| n.venue_name.clone()).unwrap_or_default(),
            primary_mic: name.as_ref().and_then(|n| n.venue_mic.clone()).unwrap_or_default(),
            currency: i.currency.as_str().to_string(),
            underlying_id: underlying_id.unwrap_or_default(),
        });
    }
    Ok(out)
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

    #[test]
    fn every_wealthsimple_security_the_book_has_met_is_listed_by_its_id_under_its_name() {
        use bagholder_core::instrument::RefScheme;
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let secs = super::securities(&app).unwrap();
        let book = app.figures.get().unwrap().book().unwrap();
        let ws = RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple"));
        let mut want: Vec<(String, String, String)> = vec![];
        for i in book.instruments().unwrap() {
            if let Some(r) = book.instrument_refs(i.id).unwrap().into_iter().find(|r| r.scheme == ws) {
                let symbol = book.names(i.id).unwrap().last().map(|n| n.symbol.clone()).unwrap_or_default();
                want.push((r.value, symbol, i.currency.as_str().to_string()));
            }
        }
        let got: Vec<(String, String, String)> = secs.iter().map(|s| (s.id.clone(), s.symbol.clone(), s.currency.clone())).collect();
        assert!(!got.is_empty());
        assert_eq!(got, want);
    }
}
