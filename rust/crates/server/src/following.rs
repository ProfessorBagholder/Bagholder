//! What the person follows, kept in the book (`docs/plans/stage-5-interface-and-running.md`,
//! A2): the watched listings and the Markets tab's tile row.
//!
//! A row is an instrument of the book, found as `docs/architecture.md` §5 finds
//! any instrument, in this order: the instrument the page names; the broker's own
//! security id; for an index, a future, a rate or a pair, the app's own directory;
//! a listing of the book whose name now is that symbol on that venue (the lookup
//! ⌘K opens a listing by, nothing merged); else a new instrument, identified by a
//! `listing` reference only a watched row carries.

use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use bagholder_book::watched::{ListingDraft, ListingName};
use bagholder_book::Book;
use bagholder_core::directory;
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Broker, Currency, InstrumentId};
use bagholder_sources::contract::Listing;
use bagholder_sources::venue;

use crate::app::App;

/// The tile row until the person changes it: `SPX NDX DJI VIX GOLD BITCOIN`.
pub const DEFAULT_TILES: [(&str, &str); 6] = [("SPX", "Index"), ("NDX", "Index"), ("DJI", "Index"), ("VIX", "Index"), ("GC", "COMEX"), ("BTCUSD", "FX")];
pub const TILES_MAX: usize = 12;

/// A listing as the page or the earlier store names it.
#[derive(Clone, Debug, Default)]
pub struct Named {
    /// The instrument, where the page knows it (a holding's, a book listing's).
    pub instrument: Option<String>,
    pub symbol: String,
    /// The venue in words (`TSX-V`, `NASDAQ`, `Index`, `CRYPTO`).
    pub exchange: String,
    pub name: String,
    pub currency: String,
    /// The broker's own security id, where the page or the earlier store has one.
    pub security_id: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn wealthsimple(id: &str) -> Reference {
    Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), id)
}

/// The instrument of the book whose name now is `symbol` on the venue `mic` (or,
/// for a coin, `symbol`), of `kind` in `currency`, or none. A ticker on a venue
/// names one listing at a time: where several instruments were last seen under it
/// (a listing replaced by a new security, as a consolidation does), it is the one
/// seen under it last, and of two seen last the same day, the one first seen
/// later; an instrument no record names (one only followed) comes after any a
/// record names. Two the records cannot tell apart are refused.
fn named_now(book: &Book, symbol: &str, mic: Option<&str>, kind: InstrumentKind, currency: Currency) -> Result<Option<InstrumentId>, String> {
    let mut found: Vec<(Seen, InstrumentId)> = Vec::new();
    for i in book.instruments().map_err(err)? {
        if i.kind != kind || i.currency != currency {
            continue;
        }
        let Some(n) = book.current_name(i.id).map_err(err)? else { continue };
        if venue::root(&n.symbol) != symbol {
            continue;
        }
        let at = n.venue_mic.as_deref().or_else(|| n.venue_name.as_deref().and_then(venue::mic_of));
        if kind == InstrumentKind::Crypto || at == mic {
            let names = book.names(i.id).map_err(err)?;
            let under: Vec<_> = names.iter().filter(|x| venue::root(&x.symbol) == symbol).collect();
            let seen = under.iter().map(|x| x.last_seen).max().zip(under.iter().map(|x| x.first_seen).min());
            found.push((seen, i.id));
        }
    }
    current(symbol, found)
}

/// When an instrument was seen under a name: the last day, then the first.
type Seen = Option<(bagholder_core::jiff::civil::Date, bagholder_core::jiff::civil::Date)>;

/// Of the instruments called `symbol`, the one it names now (`named_now`).
fn current(symbol: &str, mut found: Vec<(Seen, InstrumentId)>) -> Result<Option<InstrumentId>, String> {
    found.sort_by(|a, b| b.0.cmp(&a.0));
    match found.as_slice() {
        [] => Ok(None),
        [(_, one)] => Ok(Some(*one)),
        [(a, one), (b, _), ..] if a != b => Ok(Some(*one)),
        _ => Err(format!("{} instruments of the book are called {symbol} and the records do not tell which it is now", found.len())),
    }
}

/// The draft of the listing `n` names, found as the module's header says.
pub fn draft(book: &Book, n: &Named) -> Result<ListingDraft, String> {
    let symbol = venue::root(&n.symbol);
    if symbol.is_empty() {
        return Err("a symbol is required".into());
    }
    let name = |venue_mic: Option<&str>, venue_name: &str, words: &str| ListingName {
        symbol: symbol.clone(),
        venue_mic: venue_mic.map(str::to_string),
        venue_name: Some(venue_name.to_string()).filter(|v| !v.is_empty()),
        name: Some(words.trim().to_string()).filter(|v| !v.is_empty()),
    };
    // the instrument the page names
    if let Some(id) = n.instrument.as_deref().filter(|s| !s.is_empty()) {
        let id = InstrumentId::parse(id).map_err(err)?;
        let held = book.instrument(id).map_err(err)?;
        let now = book.current_name(id).map_err(err)?;
        let mic = now.as_ref().and_then(|x| x.venue_mic.clone()).or_else(|| venue::mic_of(&n.exchange).map(str::to_string));
        return Ok(ListingDraft { found: Some(id), kind: held.kind, currency: held.currency, refs: vec![], name: name(mic.as_deref(), &n.exchange, &n.name) });
    }
    // an index, a future, a rate or a pair of the directory
    if let Some(e) = directory::find(&symbol, &n.exchange) {
        return Ok(ListingDraft {
            found: None,
            kind: e.instrument_kind(),
            currency: Currency::parse(e.currency).map_err(err)?,
            refs: vec![Reference::new(RefScheme::Directory, e.key()), Reference::new(RefScheme::Yahoo, e.yahoo)],
            name: ListingName { symbol: e.symbol.into(), venue_mic: None, venue_name: Some(e.exchange.into()), name: Some(e.name.into()) },
        });
    }
    let security = Some(n.security_id.trim()).filter(|s| !s.is_empty()).map(wealthsimple);
    // a coin, quoted as its USD pair whatever currency the book holds it in
    if n.exchange.trim().eq_ignore_ascii_case("CRYPTO") {
        let found = named_now(book, &symbol, None, InstrumentKind::Crypto, Currency::USD)?;
        let mut refs = vec![Reference::new(RefScheme::Listing, format!("{symbol}@CRYPTO"))];
        refs.extend(security);
        return Ok(ListingDraft { found, kind: InstrumentKind::Crypto, currency: Currency::USD, refs, name: name(None, "Crypto", &n.name) });
    }
    let mic = venue::mic_of(&n.exchange).ok_or_else(|| format!("{} names no venue the app knows ({})", symbol, n.exchange))?;
    let currency = match Currency::parse(n.currency.trim()) {
        Ok(c) => c,
        Err(_) => venue::currency_of(mic).ok_or_else(|| format!("{symbol} on {} states no currency", n.exchange))?,
    };
    // the broker's id finds its instrument in the book; else the name it has now
    let found = match &security {
        Some(r) if book.instrument_by_ref(r).map_err(err)?.is_some() => None,
        _ => named_now(book, &symbol, Some(mic), InstrumentKind::Security, currency)?,
    };
    let mut refs = vec![Reference::new(RefScheme::Listing, format!("{symbol}@{mic}"))];
    refs.extend(security);
    Ok(ListingDraft { found, kind: InstrumentKind::Security, currency, refs, name: name(Some(mic), &n.exchange, &n.name) })
}

/// The draft of a tile: an instrument of the directory only.
fn tile_draft(book: &Book, symbol: &str, exchange: &str) -> Result<ListingDraft, String> {
    if directory::find(symbol, exchange).is_none() {
        return Err(format!("{symbol} is not an instrument of the directory"));
    }
    draft(book, &Named { symbol: symbol.into(), exchange: exchange.into(), ..Named::default() })
}

fn book(app: &App) -> Result<Book, String> {
    app.figures.get().ok_or("the figures are not open")?.book()
}

/// The earlier store's watchlist carried into the book once, and the tile row set
/// to the default six when it was never chosen (a first start, or after Clear data).
pub fn ensure(app: &Arc<App>) -> Result<(), String> {
    let b = book(app)?;
    let now = Timestamp::now();
    if !b.following_carried().map_err(err)? {
        let conn = app.open().map_err(err)?;
        let rows = bagholder_store::feeds::list_watchlist(&conn).map_err(err)?;
        let mut watched = Vec::new();
        for w in &rows {
            let n = Named { instrument: None, symbol: w.symbol.clone(), exchange: w.exchange.clone(), name: w.name.clone(), currency: w.currency.clone(), security_id: w.security_id.clone() };
            let d = draft(&b, &n).map_err(|e| format!("the watched {} could not be carried into the book: {e}", w.symbol))?;
            // a row the earlier store kept without its day counts from now
            let added = match w.added_at.trim() {
                "" => now,
                t => t.parse::<Timestamp>().map_err(|e| format!("the watched {} was added at {t:?}: {e}", w.symbol))?,
            };
            watched.push((d, added));
        }
        let tiles = match bagholder_store::rows::tiles(&conn).map_err(err)? {
            Some(saved) => Some(saved.iter().filter(|t| directory::find(&t.symbol, &t.exchange).is_some()).map(|t| tile_draft(&b, &t.symbol, &t.exchange)).collect::<Result<Vec<_>, String>>()?),
            None => None,
        };
        b.carry_following(&watched, tiles.as_deref(), now).map_err(err)?;
    }
    if b.tiles().map_err(err)?.is_none() {
        let defaults = DEFAULT_TILES.iter().map(|(s, e)| tile_draft(&b, s, e)).collect::<Result<Vec<_>, String>>()?;
        b.set_tiles(&defaults, now).map_err(err)?;
    }
    app.followed();
    Ok(())
}

/// `ensure`, its failure said in the header until it succeeds: the rows are not
/// carried until they all can be, and nothing is lost meanwhile.
pub fn open(app: &Arc<App>) {
    match ensure(app) {
        Ok(()) => crate::feeds::feed_answered(app, "following"),
        Err(e) => {
            crate::app::log(&format!("bagholder: the watchlist and tiles: {e}"));
            crate::feeds::feed_failed(app, "following", format!("The watchlist could not be carried into the book: {e}"));
        }
    }
}

/// Every instrument followed: the watched, then the tiles not among them.
pub fn followed(b: &Book) -> Result<Vec<InstrumentId>, String> {
    let mut out: Vec<InstrumentId> = b.watched().map_err(err)?.into_iter().map(|w| w.instrument).collect();
    for t in b.tiles().map_err(err)?.unwrap_or_default() {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

/// The followed instruments as the sources ask for them.
pub fn listings(b: &Book) -> Result<Vec<Listing>, String> {
    let mut out = Vec::new();
    for id in followed(b)? {
        if let Some(l) = crate::read_sources::listing(b, id)? {
            out.push(l);
        }
    }
    Ok(out)
}

/// Read the quotes of `ids` now, each applied: a listing just followed shows its
/// price moments later (`SPEC.md` §4, Watchlist), not at the next minute.
fn quote_now(app: &Arc<App>, ids: Vec<InstrumentId>) {
    let a = app.clone();
    crate::app::spawn("follow-quote", move || {
        let Some(f) = a.figures.get() else { return };
        let run = || -> Result<(), String> {
            let b = f.book()?;
            let cache = f.cache()?;
            let bank = crate::figures::bank_zone()?;
            let ctx = bagholder_sources::read::Ctx { book: &b, cache: &cache, net: &a.net, now: Timestamp::now(), bank: &bank };
            let mut ls = Vec::new();
            for id in &ids {
                ls.extend(crate::read_sources::listing(&b, *id)?);
            }
            bagholder_sources::quotes::read_quotes(&ctx, &ls).map_err(err)?;
            for id in &ids {
                f.price_changed(*id)?;
            }
            Ok(())
        };
        if let Err(e) = run() {
            crate::app::log(&format!("bagholder: a followed listing's quote: {e}"));
        }
        a.events.signal();
    });
}

/// `POST /api/watchlist/add`, `POST /api/watchlist/remove`: refused, or the
/// instrument the row is.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum WatchlistAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        /// The instrument followed or dropped.
        id: String,
    },
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
}

/// `POST /api/tiles/set`: refused, or done.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum TilesAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
    },
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
}

fn refused(e: String) -> WatchlistAnswer {
    WatchlistAnswer::Err { ok: false, error: e }
}

/// The listing `n` names followed in the book: its instrument.
fn follow(app: &Arc<App>, n: &Named) -> Result<InstrumentId, String> {
    let b = book(app)?;
    let d = draft(&b, n)?;
    let id = b.watch(&d, Timestamp::now()).map_err(err)?;
    app.followed();
    Ok(id)
}

/// Follow a listing: its instrument, and its quote and its sector read at once.
pub fn watch(app: &Arc<App>, n: &Named) -> WatchlistAnswer {
    match follow(app, n) {
        Ok(id) => {
            quote_now(app, vec![id]);
            crate::feeds::read_sector(app, n);
            WatchlistAnswer::Ok { ok: true, id: id.to_string() }
        }
        Err(e) => refused(e),
    }
}

/// Stop following an instrument, and forget the news read for it alone.
pub fn unwatch(app: &Arc<App>, id: &str) -> WatchlistAnswer {
    let run = || -> Result<InstrumentId, String> {
        if id.trim().is_empty() {
            return Err("the listing to stop watching is required".into());
        }
        let id = InstrumentId::parse(id).map_err(err)?;
        let b = book(app)?;
        let name = b.current_name(id).map_err(err)?;
        if !b.unwatch(id).map_err(err)? {
            return Err("that listing is not watched".into());
        }
        app.followed();
        if let Some(n) = name {
            crate::feeds::forget_news(app, &n.symbol, n.venue_name.as_deref().unwrap_or_default())?;
        }
        Ok(id)
    };
    match run() {
        Ok(id) => WatchlistAnswer::Ok { ok: true, id: id.to_string() },
        Err(e) => refused(e),
    }
}

/// The tile row is `tiles`, in order: instruments of the directory only, twelve
/// at most, each once.
pub fn set_tiles(app: &Arc<App>, tiles: &[(String, String)]) -> TilesAnswer {
    let run = || -> Result<Vec<InstrumentId>, String> {
        if tiles.len() > TILES_MAX {
            return Err(format!("at most {TILES_MAX} tiles"));
        }
        let b = book(app)?;
        let drafts = tiles.iter().map(|(s, e)| tile_draft(&b, s, e)).collect::<Result<Vec<_>, String>>()?;
        let ids = b.set_tiles(&drafts, Timestamp::now()).map_err(err)?;
        app.followed();
        Ok(ids)
    };
    match run() {
        Ok(ids) => {
            quote_now(app, ids);
            TilesAnswer::Ok { ok: true }
        }
        Err(e) => TilesAnswer::Err { ok: false, error: e },
    }
}

/// How long a glance is remembered (`SPEC.md` §4 Markets, Watchlist: a minute's
/// memory, nothing stored).
pub const GLANCE_MEMORY: std::time::Duration = std::time::Duration::from_secs(60);

/// A listing's price and day change for a glance, as the wire carries them.
#[derive(Clone, Debug, PartialEq)]
pub struct Glanced {
    pub price: crate::wire::Dec,
    pub change: Option<crate::wire::Dec>,
    /// As a fraction.
    pub percent_change: Option<f64>,
}

/// What a listing named by the page is, for a glance.
fn glance_of(symbol: &str, exchange: &str, currency: &str) -> Result<bagholder_sources::quotes::GlanceOf, String> {
    let symbol = venue::root(symbol);
    if symbol.is_empty() {
        return Err("a symbol is required".into());
    }
    if let Some(e) = directory::find(&symbol, exchange) {
        return Ok(bagholder_sources::quotes::GlanceOf { kind: e.instrument_kind(), currency: Currency::parse(e.currency).map_err(err)?, symbol, venue_mic: None, yahoo: Some(e.yahoo.into()) });
    }
    if exchange.trim().eq_ignore_ascii_case("CRYPTO") {
        // in the currency the page names, US dollars where it names none
        let currency = if currency.trim().is_empty() { Currency::USD } else { Currency::parse(currency.trim()).map_err(err)? };
        return Ok(bagholder_sources::quotes::GlanceOf { kind: InstrumentKind::Crypto, currency, symbol, venue_mic: None, yahoo: None });
    }
    // a venue the app does not know: the listing follows its currency
    let mic = venue::mic_of(exchange);
    let currency = match (Currency::parse(currency.trim()), mic.and_then(venue::currency_of)) {
        (Ok(c), _) => c,
        (Err(_), Some(c)) => c,
        (Err(_), None) => return Err(format!("{symbol} on {exchange} states no currency")),
    };
    Ok(bagholder_sources::quotes::GlanceOf { kind: InstrumentKind::Security, currency, symbol, venue_mic: mic.map(str::to_string), yahoo: None })
}

/// A listing's price and day change, read for a glance and remembered for a
/// minute; why there is none, when its source did not answer.
pub fn glance(app: &Arc<App>, symbol: &str, exchange: &str, currency: &str) -> Result<Glanced, String> {
    type Memory = std::collections::HashMap<String, (std::time::Instant, Result<Glanced, String>)>;
    static MEMORY: std::sync::OnceLock<std::sync::Mutex<Memory>> = std::sync::OnceLock::new();
    let of = glance_of(symbol, exchange, currency)?;
    let key = format!("{}@{}|{}", of.symbol, exchange.trim().to_uppercase(), of.currency);
    let memory = MEMORY.get_or_init(Default::default);
    if let Some((at, g)) = memory.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if at.elapsed() < GLANCE_MEMORY {
            return g.clone();
        }
    }
    // the Bank's rates, for a coin's close kept in another currency than its own
    let rates = match (of.kind, app.figures.get()) {
        (InstrumentKind::Crypto, Some(f)) => f.book()?.rates().map_err(err)?,
        _ => Default::default(),
    };
    let got = match bagholder_sources::quotes::glance(&app.net, Timestamp::now(), &of, &rates) {
        bagholder_sources::outcome::Outcome::Answered(g) => Ok(Glanced {
            price: crate::wire::Dec(g.price.amount),
            change: g.change.map(crate::wire::Dec),
            percent_change: g.change_pct.map(|c| c.to_f64() / 100.0),
        }),
        other => Err(format!("{}'s quote: {} ({})", of.symbol, other.kind(), other.detail())),
    };
    memory.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (std::time::Instant::now(), got.clone()));
    got
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Every request is written down and answered 503: nothing reaches a source.
    struct Refused(Arc<Mutex<Vec<String>>>);
    impl bagholder_net::Transport for Refused {
        fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
            self.0.lock().unwrap().push(ask.url.to_string());
            Ok((503, ask.url.to_string(), vec![], Vec::new()))
        }
    }

    /// An app of its own on the recorded month, its figures built, its network refused.
    fn opened() -> (tempfile::TempDir, Arc<App>, Arc<Mutex<Vec<String>>>) {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let net = bagholder_net::Net::answered_by(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()), Box::new(Refused(asked.clone())));
        let app = App::with_net(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net);
        bagholder_store::relabel::ensure(&app.open().unwrap()).unwrap();
        let now: Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        (home, app, asked)
    }

    fn named(symbol: &str, exchange: &str) -> Named {
        Named { symbol: symbol.into(), exchange: exchange.into(), ..Named::default() }
    }

    fn symbols(b: &Book, ids: &[InstrumentId]) -> Vec<String> {
        ids.iter().map(|i| b.current_name(*i).unwrap().unwrap().symbol).collect()
    }

    fn watched(b: &Book) -> Vec<InstrumentId> {
        b.watched().unwrap().into_iter().map(|w| w.instrument).collect()
    }

    #[test]
    fn the_earlier_store_s_rows_are_carried_into_the_book_once_in_their_order() {
        let (_home, app, _) = opened();
        let conn = app.open().unwrap();
        for (i, (s, e, c)) in [("QNC", "TSX-V", "CAD"), ("AAPL", "NASDAQ", "USD"), ("SPX", "Index", ""), ("BTC", "CRYPTO", "USD")].iter().enumerate() {
            bagholder_store::feeds::add_watch(&conn, s, e, &format!("{s} named"), c, "", &format!("2026-09-0{}T14:00:00Z", i + 1)).unwrap();
        }
        let saved = [("VIX", "Index"), ("GC", "COMEX")].map(|(s, e)| bagholder_model::input::TileRef { symbol: s.into(), exchange: e.into() });
        bagholder_store::admin::save_tiles(&conn, &saved).unwrap();
        ensure(&app).unwrap();
        let b = book(&app).unwrap();
        // newest first, as the earlier store listed them oldest first
        assert_eq!(symbols(&b, &watched(&b)), ["BTC", "SPX", "AAPL", "QNC"]);
        assert_eq!(symbols(&b, &b.tiles().unwrap().unwrap()), ["VIX", "GC"], "the tiles chosen before, in their order");
        // carried once: a row the earlier store gains later is not carried
        bagholder_store::feeds::add_watch(&conn, "SHOP", "TSX", "", "CAD", "", "2026-09-10T14:00:00Z").unwrap();
        ensure(&app).unwrap();
        assert_eq!(watched(&b).len(), 4);
    }

    #[test]
    fn a_watchlist_that_cannot_be_read_is_said_and_nothing_is_carried_until_it_can_be() {
        let (_home, app, _) = opened();
        let conn = app.open().unwrap();
        bagholder_store::feeds::add_watch(&conn, "QNC", "TSX-V", "QNC named", "CAD", "", "2026-09-01T14:00:00Z").unwrap();
        conn.execute("ALTER TABLE watchlist RENAME TO watchlist_away", []).unwrap();
        open(&app);
        let said = crate::status::status(&app).error;
        assert!(said.contains("The watchlist could not be carried into the book"), "{said}");
        let b = book(&app).unwrap();
        assert!(watched(&b).is_empty() && !b.following_carried().unwrap(), "no row is carried until they all can be");
        conn.execute("ALTER TABLE watchlist_away RENAME TO watchlist", []).unwrap();
        open(&app);
        assert!(!crate::status::status(&app).error.contains("watchlist"), "the next good read takes the failure away");
        assert_eq!(symbols(&b, &watched(&b)), ["QNC"]);
    }

    #[test]
    fn a_book_that_never_chose_its_tiles_has_the_default_six_and_one_chosen_empty_stays_empty() {
        let (_home, app, _) = opened();
        ensure(&app).unwrap();
        let b = book(&app).unwrap();
        assert_eq!(symbols(&b, &b.tiles().unwrap().unwrap()), ["SPX", "NDX", "DJI", "VIX", "GC", "BTCUSD"]);
        assert!(matches!(set_tiles(&app, &[]), TilesAnswer::Ok { .. }));
        ensure(&app).unwrap();
        assert_eq!(b.tiles().unwrap(), Some(vec![]));
    }

    #[test]
    fn a_refused_tile_row_changes_nothing() {
        let (_home, app, _) = opened();
        ensure(&app).unwrap();
        let b = book(&app).unwrap();
        let before = b.tiles().unwrap();
        let thirteen: Vec<(String, String)> = ["SPX", "NDX", "IXIC", "DJI", "RUT", "VIX", "TSX", "FTSE", "DAX", "N225", "HSI", "STOXX50E", "DXY"].iter().map(|s| (s.to_string(), "Index".to_string())).collect();
        assert!(matches!(set_tiles(&app, &thirteen), TilesAnswer::Err { .. }), "at most twelve");
        assert!(matches!(set_tiles(&app, &[("SPX".into(), "Index".into()), ("SHOP".into(), "TSX".into())]), TilesAnswer::Err { .. }), "a listing is not a tile");
        assert_eq!(b.tiles().unwrap(), before);
    }

    #[test]
    fn an_index_both_a_tile_and_watched_is_one_instrument_and_quoted_through_yahoo() {
        let (_home, app, asked) = opened();
        ensure(&app).unwrap();
        let b = book(&app).unwrap();
        let spx = b.tiles().unwrap().unwrap()[0];
        let WatchlistAnswer::Ok { id, .. } = watch(&app, &named("spx", "index")) else { panic!("refused") };
        assert_eq!(id, spx.to_string());
        assert_eq!(followed(&b).unwrap().iter().filter(|i| **i == spx).count(), 1);
        let l = listings(&b).unwrap();
        let spx_listing = l.iter().find(|l| l.id == spx).unwrap();
        assert_eq!(spx_listing.routes.get(&RefScheme::Yahoo), Some(&vec!["^GSPC".to_string()]));
        // the quote read at once goes to Yahoo's chart for the directory's code
        let start = std::time::Instant::now();
        while !asked.lock().unwrap().iter().any(|u| u.contains("%5EGSPC") || u.contains("^GSPC")) {
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "asked: {:?}", asked.lock().unwrap());
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_listing_watched_twice_is_one_row_and_a_held_listing_watched_is_the_holding() {
        let (_home, app, _) = opened();
        ensure(&app).unwrap();
        let b = book(&app).unwrap();
        // `follow`, the book's half of `watch`: a listing's sector is read by a reader
        // on the machine's own network, which a test must not reach
        let a = follow(&app, &named("QNC", "TSX-V")).unwrap().to_string();
        let again = follow(&app, &named("qnc", "TSXV")).unwrap().to_string();
        assert_eq!(a, again);
        assert_eq!(watched(&b).len(), 1);
        // a held share, named as the page names it by symbol and venue, is that instrument
        let held = b
            .instruments()
            .unwrap()
            .into_iter()
            .filter(|i| i.kind == InstrumentKind::Security)
            .find_map(|i| b.current_name(i.id).unwrap().filter(|n| n.venue_mic.is_some()).map(|n| (i, n)))
            .expect("the recorded month holds a share named on its venue");
        let (i, n) = held;
        let id = follow(&app, &Named { currency: i.currency.to_string(), ..named(&n.symbol, n.venue_mic.as_deref().unwrap()) }).unwrap();
        assert_eq!(id, i.id);
        let by_id = follow(&app, &Named { instrument: Some(i.id.to_string()), ..named(&n.symbol, "") }).unwrap();
        assert_eq!(by_id, i.id, "named by the page's instrument");
        assert_eq!(watched(&b).len(), 2);
        assert!(matches!(unwatch(&app, &a), WatchlistAnswer::Ok { .. }));
        assert!(matches!(unwatch(&app, &a), WatchlistAnswer::Err { .. }), "a row not watched is not removed twice");
    }

    /// A ticker on a venue names one listing at a time: the instrument seen under it
    /// last (a successor security after a consolidation), the later-started of two
    /// last seen the same day, any a record names before one only followed.
    #[test]
    fn of_two_instruments_called_the_same_the_one_seen_under_it_last_is_the_listing() {
        let day = |s: &str| s.parse::<bagholder_core::jiff::civil::Date>().unwrap();
        let id = |n: u8| InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap();
        let old = (Some((day("2026-06-03"), day("2025-12-30"))), id(1));
        let new = (Some((day("2026-09-21"), day("2026-06-10"))), id(2));
        assert_eq!(current("CH", vec![old, new]), Ok(Some(id(2))));
        assert_eq!(current("CH", vec![new, old]), Ok(Some(id(2))));
        let same_day_later_start = (Some((day("2026-06-03"), day("2026-06-03"))), id(3));
        assert_eq!(current("CH", vec![old, same_day_later_start]), Ok(Some(id(3))));
        assert_eq!(current("CH", vec![(None, id(4)), old]), Ok(Some(id(1))), "a record's instrument before one only followed");
        assert!(current("CH", vec![old, (old.0, id(5))]).is_err(), "two the records cannot tell apart");
        assert!(current("CH", vec![(None, id(4)), (None, id(6))]).is_err());
        assert_eq!(current("CH", vec![]), Ok(None));
    }

    #[test]
    fn a_listing_on_no_venue_the_app_knows_is_refused() {
        let (_home, app, _) = opened();
        let b = book(&app).unwrap();
        assert!(draft(&b, &named("ABC", "MOON")).is_err());
        assert!(draft(&b, &named("  ", "TSX")).is_err());
        let d = draft(&b, &named("SHOP", "TSX")).unwrap();
        assert_eq!((d.kind, d.currency), (InstrumentKind::Security, Currency::CAD), "the venue states the currency when the page does not");
        assert_eq!(d.refs[0], Reference::new(RefScheme::Listing, "SHOP@XTSE"));
    }

    #[test]
    fn a_glance_at_a_listing_on_an_unknown_venue_follows_its_currency_and_a_coin_keeps_its_own() {
        // a venue the app does not know, with the currency the page states
        let g = glance_of("ABC", "OTC", "USD").unwrap();
        assert_eq!((g.kind, g.currency, g.venue_mic), (InstrumentKind::Security, Currency::USD, None));
        assert!(glance_of("ABC", "OTC", "").is_err(), "no venue known and no currency: nothing says where it trades");
        // the old words for a venue still name it
        assert_eq!(glance_of("ONE", "TSX Venture Exchange", "").unwrap().venue_mic.as_deref(), Some("XTSX"));
        // a coin in the currency the page names, US dollars where it names none
        assert_eq!(glance_of("BTC", "CRYPTO", "CAD").unwrap().currency, Currency::CAD);
        assert_eq!(glance_of("BTC", "CRYPTO", "").unwrap().currency, Currency::USD);
    }
}
