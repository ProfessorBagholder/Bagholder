//! The market's context: what the earlier readers are given until stage 6 moves
//! them (`bagholder_model::context::MarketBase`: the holdings and round trips, and
//! the earlier readers' tables), and what the person follows as the book
//! holds it (`wire::markets::Following`).
//!
//! Built again only when something it reads moved: a holding came or went, a
//! record or a round trip changed (the engine's report says so), what the person
//! follows changed, one of the earlier readers' tables it reads was written, or
//! the day turned. A quote moves none of these, so a price tick never rebuilds it.
//! The earlier readers' tables are the market cache's (`App::cache`).

use std::sync::{Arc, Mutex};

use bagholder_book::Book;
use bagholder_core::directory;
use bagholder_core::instrument::{InstrumentKind, RefScheme};
use bagholder_core::InstrumentId;
use bagholder_engine::engine::{Entity, Moved};
use bagholder_model::context::{Held, MarketBase, Traded};
use bagholder_model::input::{TileRef, WatchRow};
use bagholder_sources::venue;
use bagholder_store::{gens, rows};

use crate::app::App;
use crate::figures::{Figures, Since};
use crate::wire::markets::{Followed, Following};

/// The earlier readers' tables the context reads, in the market cache.
const TABLES: [&str; 3] = ["exposures", "news", "universes"];
/// The issuers' filed documents, read on demand by the News card's Releases.
const FILED: [&str; 1] = ["filings"];

/// The context, built.
pub struct Built {
    pub base: Arc<MarketBase>,
    pub following: Arc<Following>,
    /// The headlines, as the figure path reads them: made once per context.
    pub news: std::sync::OnceLock<Arc<Vec<crate::wire::news::NewsRow>>>,
    /// The filed releases asked for, by the scope or chip that asked.
    pub filed: Mutex<std::collections::BTreeMap<String, Arc<Vec<crate::wire::news::FiledRelease>>>>,
}

struct Kept {
    /// The figures' version it was built at, and brought forward to.
    at: u64,
    /// The day, what the person follows, the earlier readers' tables.
    key: String,
    /// The filed documents' generation.
    filed: String,
    built: Arc<Built>,
}

#[derive(Default)]
pub struct MarketContext {
    kept: Mutex<Option<Kept>>,
}

/// Whether what moved can move the context: a holding that came or went, a
/// round trip, the record as a whole. A holding's figures moving (a quote) cannot.
pub(crate) fn moves_it(m: &Moved) -> bool {
    m.0.iter().any(|(e, fields)| match e {
        Entity::Book | Entity::Trade(_) => true,
        Entity::Position(..) => fields.contains("*"),
        _ => false,
    })
}

impl MarketContext {
    pub fn new() -> MarketContext {
        MarketContext::default()
    }

    /// The context now: the one built last when nothing it reads has moved.
    pub fn get(&self, app: &App) -> Result<Arc<Built>, String> {
        let f = app.figures.get().ok_or("the figures are not open")?;
        let conn = app.cache().map_err(|e| e.to_string())?;
        let today = f.read(|e| e.inputs().clock.today.to_string()).ok_or("the figures are not built yet")?;
        let all = gens::all(&conn).map_err(|e| e.to_string())?;
        let key = format!("{}|{}|{}", today, app.following_version(), gens::key(&all, &TABLES));
        let filed = gens::key(&all, &FILED);
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        let now_at = f.version();
        if let Some(k) = kept.as_mut() {
            if k.key == key {
                let same = match f.moved_since(k.at) {
                    Since::Nothing => true,
                    Since::Moved(m) => !moves_it(&m),
                    Since::Everything => false,
                };
                if same {
                    k.at = now_at;
                    if k.filed != filed {
                        // only the filed documents moved: the same context, its filed releases asked again
                        let b = &k.built;
                        let news = std::sync::OnceLock::new();
                        if let Some(n) = b.news.get() {
                            news.set(n.clone()).expect("a cell just made is empty");
                        }
                        k.built = Arc::new(Built { base: b.base.clone(), following: b.following.clone(), news, filed: Mutex::new(Default::default()) });
                        k.filed = filed;
                    }
                    return Ok(k.built.clone());
                }
            }
        }
        let following = following(&f.book()?)?;
        let built = Arc::new(Built { base: Arc::new(build(f, &following, &conn, today)?), following: Arc::new(following), news: Default::default(), filed: Mutex::new(Default::default()) });
        *kept = Some(Kept { at: now_at, key, filed, built: built.clone() });
        Ok(built)
    }
}

/// An instrument the person follows, as the book holds it.
fn followed(book: &Book, id: InstrumentId) -> Result<Followed, String> {
    let e = |e: bagholder_book::BookError| e.to_string();
    let i = book.instrument(id).map_err(e)?;
    let name = book.current_name(id).map_err(e)?;
    let entry = book.instrument_refs(id).map_err(e)?.into_iter().find(|r| r.scheme == RefScheme::Directory).and_then(|r| directory::INSTRUMENTS.iter().find(|x| x.key() == r.value));
    let symbol = name.as_ref().map(|n| venue::root(&n.symbol)).unwrap_or_default();
    let venue_name = name.as_ref().and_then(|n| n.venue_name.clone());
    let mic = name.as_ref().and_then(|n| n.venue_mic.clone().or_else(|| n.venue_name.as_deref().and_then(venue::mic_of).map(str::to_string)));
    let exchange = if i.kind == InstrumentKind::Crypto { "Crypto".to_string() } else { venue_name.or_else(|| mic.clone()).unwrap_or_default() };
    Ok(Followed {
        id,
        kind: i.kind,
        currency: i.currency,
        symbol,
        exchange,
        mic,
        name: name.and_then(|n| n.name).unwrap_or_default(),
        directory: entry,
    })
}

/// What the person follows, as the book holds it.
pub fn following(book: &Book) -> Result<Following, String> {
    let watched = book.watched().map_err(|e| e.to_string())?.into_iter().map(|w| followed(book, w.instrument)).collect::<Result<Vec<_>, String>>()?;
    let tiles = book.tiles().map_err(|e| e.to_string())?.unwrap_or_default().into_iter().map(|id| followed(book, id)).collect::<Result<Vec<_>, String>>()?;
    Ok(Following { watched, tiles })
}

fn build(f: &Figures, following: &Following, conn: &rusqlite::Connection, today: String) -> Result<MarketBase, String> {
    let names = f.names()?;
    let (positions, traded) = f
        .read(|e| {
            let inputs = e.inputs();
            let figs = e.figures();
            let positions: Vec<Held> = figs
                .positions
                .iter()
                .map(|p| {
                    let shown = crate::wire::build::position(inputs, &names, p, String::new());
                    Held {
                        id: shown.id,
                        symbol: shown.symbol,
                        underlying: shown.underlying,
                        name: shown.name,
                        exchange: shown.exchange,
                        kind: bagholder_model::activity::Kind::parse(&shown.kind).unwrap_or(bagholder_model::activity::Kind::Shares),
                        account: shown.account,
                        account_id: shown.account_id,
                        currency: shown.currency,
                        security_id: shown.security,
                        short: shown.short,
                        opened: shown.opened,
                    }
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
    let watch_row = |x: &Followed| WatchRow { symbol: x.symbol.clone(), exchange: x.exchange.clone(), name: x.name.clone(), currency: x.currency.as_str().to_string() };
    let e = |e: rusqlite::Error| e.to_string();
    Ok(MarketBase {
        today,
        positions: Arc::new(positions),
        traded: Arc::new(traded),
        exposures: Arc::new(rows::exposures(conn).map_err(e)?),
        watchlist: Arc::new(following.watched.iter().map(watch_row).collect()),
        news: Arc::new(rows::news(conn).map_err(e)?),
        tiles: Arc::new(Some(following.tiles.iter().map(|x| TileRef { symbol: x.symbol.clone(), exchange: x.exchange.clone() }).collect())),
        universes: Arc::new(rows::universes(conn).map_err(e)?),
    })
}

/// Every Wealthsimple security the book has met, as the earlier readers take
/// one: its id, what the book calls it now, its venue and currency, and for a
/// contract what it is on.
pub fn securities(app: &App) -> Result<Vec<bagholder_model::securities::Security>, String> {
    let f = app.figures.get().ok_or("the figures are not open")?;
    let book = f.book()?;
    let e = |e: bagholder_book::BookError| e.to_string();
    let ws = bagholder_core::Broker::named("wealthsimple");
    let ws_id = |i: InstrumentId| -> Result<Option<String>, String> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_context_holds_the_engine_s_holdings_and_a_quote_does_not_rebuild_it() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let f = app.figures.get().unwrap();
        let a = app.market_context().unwrap();
        let want: Vec<String> = f.read(|e| e.figures().positions.iter().map(crate::wire::build::position_id).collect()).unwrap();
        let got: Vec<String> = a.base.positions.iter().map(|p| p.id.clone()).collect();
        assert!(!got.is_empty());
        assert_eq!(got, want);
        // nothing moved: the same context, not a new one
        assert!(std::sync::Arc::ptr_eq(&a, &app.market_context().unwrap()));
        // a quote moves a holding's figures, and nothing the context reads
        let held = f.read(|e| e.figures().positions[0].instrument).unwrap();
        let cache = f.cache().unwrap();
        let cur = f.read(|e| e.figures().positions[0].currency).unwrap();
        let t: bagholder_core::jiff::Timestamp = "2025-11-19T21:10:00Z".parse().unwrap();
        cache
            .store_quote(&bagholder_sources::cache::StoredQuote { instrument: held, source: bagholder_core::SourceName::named("tmx"), price: bagholder_core::Money::new(bagholder_core::Dec::parse("123.45").unwrap(), cur), change: None, change_pct: None, quoted_at: t, allowance: Default::default(), received_at: t })
            .unwrap();
        let moved = f.price_changed(held).unwrap();
        assert!(!moved.is_empty(), "the quote moved the holding");
        assert!(std::sync::Arc::ptr_eq(&a, &app.market_context().unwrap()), "a quote tick does not rebuild the context");
        // what the person follows changing does
        app.followed();
        assert!(!std::sync::Arc::ptr_eq(&a, &app.market_context().unwrap()));
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
