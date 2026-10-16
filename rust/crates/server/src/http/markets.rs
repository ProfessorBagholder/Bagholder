//! Listings and what is read about them from outside: search, a glance quote,
//! filings, short interest, news, fear and greed, chart history, the watchlist
//! and the Markets tab's tiles.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::extract::{flag, text, trimmed};
use super::{answer, api_routes, blocking, Api, ApiError, AppState, Body, Params, Routed};
use crate::{feeds, following};

pub fn routes() -> Routed {
    let mut routed = api_routes! {
        get "/api/symbols/search" => symbol_search;
        get "/api/symbols/quote" => symbol_quote;
        get "/api/listings" => listings;
        get "/api/listing" => listing;
        get "/api/filings" => filings;
        get "/api/filings/feed" => filings_feed;
        get "/api/filings/enrich" => filings_enrich;
        get "/api/news/symbol" => news_symbol;
        get "/api/fear" => fear;
        get "/api/shorts" => shorts;
        get "/api/shorts/feed" => shorts_feed;
        get "/api/history" => history;
        post "/api/watchlist/add" => watchlist_add;
        post "/api/watchlist/remove" => watchlist_remove;
        post "/api/tiles/set" => tiles_set;
    };
    // `filings/doc` answers a document (a PDF, or a page of it) rather than JSON
    routed.router = routed.router.route("/api/filings/doc", get(filings_doc));
    routed
}

/// One listing as the page writes it: its symbol and its venue, the venue in
/// words (`TSX`, `NYSE`) or as a code (`XTSE`, `US`), with its currency and name
/// where known.
#[derive(Deserialize, TS)]
#[ts(rename = "Listing")]
pub struct ListingText {
    #[serde(default, deserialize_with = "text")]
    symbol: String,
    #[serde(default, deserialize_with = "text")]
    exchange: String,
    #[serde(default, deserialize_with = "text")]
    currency: String,
    #[serde(default, deserialize_with = "text")]
    name: String,
}

/// One listing, as a route takes it: its key, made from its symbol and its venue.
/// A request that names no venue, or a venue no listing is keyed by, does not
/// parse: no route finds a listing by a bare ticker (plan
/// `market-data-keyed-by-listing`, Approach, 5; rule test 3).
#[derive(Deserialize)]
#[serde(try_from = "ListingText")]
pub struct Listing {
    key: bagholder_core::ListingKey,
    symbol: String,
    exchange: String,
    currency: String,
    name: String,
}

/// On the wire a listing is the text it parses from: written by hand rather than
/// with `ts(as)`, which cannot be flattened into the requests that carry one.
impl TS for Listing {
    type WithoutGenerics = Listing;
    type OptionInnerType = Listing;
    fn docs() -> Option<String> {
        ListingText::docs()
    }
    fn ident(cfg: &ts_rs::Config) -> String {
        ListingText::ident(cfg)
    }
    fn decl(cfg: &ts_rs::Config) -> String {
        ListingText::decl(cfg)
    }
    fn decl_concrete(cfg: &ts_rs::Config) -> String {
        ListingText::decl_concrete(cfg)
    }
    fn name(cfg: &ts_rs::Config) -> String {
        ListingText::name(cfg)
    }
    fn inline(cfg: &ts_rs::Config) -> String {
        ListingText::inline(cfg)
    }
    fn inline_flattened(cfg: &ts_rs::Config) -> String {
        ListingText::inline_flattened(cfg)
    }
    fn visit_dependencies(v: &mut impl ts_rs::TypeVisitor) {
        ListingText::visit_dependencies(v)
    }
    fn output_path() -> Option<std::path::PathBuf> {
        ListingText::output_path()
    }
}

impl TryFrom<ListingText> for Listing {
    type Error = String;
    fn try_from(t: ListingText) -> Result<Listing, String> {
        if t.symbol.is_empty() {
            return Err("symbol required".into());
        }
        let key = if t.exchange.is_empty() {
            return Err(format!("{} names no exchange: a ticker can be two companies on two exchanges", t.symbol));
        } else {
            bagholder_core::ListingKey::on(&t.symbol, &t.exchange).map_err(|e| e.to_string())?
        };
        Ok(Listing { key, symbol: t.symbol, exchange: t.exchange, currency: t.currency, name: t.name })
    }
}

#[derive(Deserialize, TS)]
pub struct Search {
    #[serde(default)]
    q: String,
}

/// `GET /api/symbols/search`: the exchanges' own directories -- a match
/// carries only what its source gave (`kind` and `rank` from the built-in
/// instrument list, neither from Nasdaq or TSX's own search).
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum SymbolSearchAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        matches: Vec<bagholder_model::wire::SymbolMatch>,
    },
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
        matches: Vec<bagholder_model::wire::SymbolMatch>,
    },
}

async fn symbol_search(State(state): State<AppState>, Params(s): Params<Search>) -> super::Api<SymbolSearchAnswer> {
    let app = state.app.clone();
    answer(move || match bagholder_market::search::symbol_search(&s.q) {
        Ok(matches) => SymbolSearchAnswer::Ok { ok: true, matches: not_held(&app, matches) },
        Err(error) => SymbolSearchAnswer::Err { ok: false, error, matches: vec![] },
    })
    .await
}

/// The key of a listing a search found: a coin and a directory instrument by
/// their kind, a share or fund by its symbol on the venue the source gave.
fn match_key(m: &bagholder_model::wire::SymbolMatch) -> Option<bagholder_core::ListingKey> {
    bagholder_model::input::Listing::new(m.symbol.as_str(), m.exchange.as_str(), m.currency.as_str(), m.kind.as_deref().unwrap_or("Shares")).key().ok()
}

/// The listing each book instrument is now, where its records name one.
fn held_listings(app: &std::sync::Arc<crate::app::App>) -> Option<Vec<(bagholder_core::ListingKey, bagholder_model::wire::SymbolMatch)>> {
    app.figures.get().and_then(|f| {
        f.read(|e| {
            e.inputs()
                .ledger
                .instruments
                .values()
                .filter_map(|i| {
                    let n = i.current_name()?;
                    let key = bagholder_core::ListingKey::of(i.instrument.kind, n).ok()?;
                    let exchange = n.venue_name.clone().or(n.venue_mic.clone()).unwrap_or_default();
                    Some((key, bagholder_model::wire::SymbolMatch { symbol: n.symbol.clone(), name: n.name.clone().unwrap_or_default(), exchange, currency: i.instrument.currency.as_str().to_string(), kind: None, rank: None }))
                })
                .collect()
        })
    })
}

/// The listings found, less any the book already holds (`SPEC.md` filters-24): one
/// listing is one key (`ListingKey`: the code in Canada, the market in the US), so
/// a listing the broker names on one NYSE venue and the directory on another is one
/// listing, never a second row.
fn not_held(app: &std::sync::Arc<crate::app::App>, matches: Vec<bagholder_model::wire::SymbolMatch>) -> Vec<bagholder_model::wire::SymbolMatch> {
    let Some(held) = held_listings(app) else { return matches };
    let held: std::collections::BTreeSet<_> = held.into_iter().map(|(k, _)| k).collect();
    matches.into_iter().filter(|m| !match_key(m).is_some_and(|k| held.contains(&k))).collect()
}

#[derive(Deserialize, TS)]
pub struct Resolve {
    #[serde(default, deserialize_with = "text")]
    symbol: String,
}

/// One listing a symbol given with no venue may be.
#[derive(Clone, Debug, Serialize, TS)]
pub struct ResolvedListing {
    /// The listing (`SYMBOL@VENUE`).
    pub listing: String,
    pub symbol: String,
    pub name: String,
    pub exchange: String,
    pub currency: String,
    pub held: bool,
}

/// `GET /api/listings`: every listing of a symbol given with no venue (`SPEC.md`
/// markets-60), each with its venue: the book's own, and those the exchanges'
/// directories list under exactly that symbol. The page opens one directly and
/// shows two or more as rows; nothing here reduces them to one.
#[derive(Clone, Debug, Serialize, TS)]
pub enum ResolveAnswer {
    /// The listings found; where the directories could not be asked, the book's own
    /// alone, with why the rest could not be read (`unread`, empty when they all answered).
    #[serde(untagged)]
    Ok {
        #[ts(type = "true")]
        ok: bool,
        listings: Vec<ResolvedListing>,
        unread: String,
    },
    /// A request that names no symbol.
    #[serde(untagged)]
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
}

/// The listings of `symbol`, the book's first: each key once.
pub(crate) fn resolve(held: Vec<(bagholder_core::ListingKey, bagholder_model::wire::SymbolMatch)>, found: Vec<bagholder_model::wire::SymbolMatch>, symbol: &str) -> Vec<ResolvedListing> {
    let want = bagholder_core::listing::spelling(symbol);
    let mut out: Vec<ResolvedListing> = Vec::new();
    let rows = held.into_iter().map(|(k, m)| (k, m, true)).chain(found.into_iter().filter_map(|m| Some((match_key(&m)?, m, false))));
    for (key, m, held) in rows {
        if key.symbol() != want || out.iter().any(|r| r.listing == key.to_string()) {
            continue;
        }
        out.push(ResolvedListing { listing: key.to_string(), symbol: m.symbol, name: m.name, exchange: m.exchange, currency: m.currency, held });
    }
    out
}

async fn listings(State(state): State<AppState>, Params(r): Params<Resolve>) -> super::Api<ResolveAnswer> {
    let app = state.app.clone();
    answer(move || {
        if r.symbol.is_empty() {
            return ResolveAnswer::Err { ok: false, error: "symbol required".into() };
        }
        let held = held_listings(&app).unwrap_or_default();
        match bagholder_market::search::symbol_search(&r.symbol) {
            Ok(found) => ResolveAnswer::Ok { ok: true, listings: resolve(held, found, &r.symbol), unread: String::new() },
            Err(unread) => ResolveAnswer::Ok { ok: true, listings: resolve(held, vec![], &r.symbol), unread },
        }
    })
    .await
}

/// `GET /api/symbols/quote`: a glance at a listing the watchlist's add row offers:
/// its price and day change, not stored; or why its source gave none.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum GlanceAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        price: crate::wire::Dec,
        /// The day's change in points, where the source states it.
        change: Option<crate::wire::Dec>,
        /// The day's change, as a fraction.
        #[serde(rename = "percentChange")]
        percent_change: Option<f64>,
    },
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
}

async fn symbol_quote(State(state): State<AppState>, Params(l): Params<Listing>) -> Api<GlanceAnswer> {
    let app = state.app;
    answer(move || match following::glance(&app, &l.symbol, &l.exchange, &l.currency) {
        Ok(g) => GlanceAnswer::Ok { ok: true, price: g.price, change: g.change, percent_change: g.percent_change },
        Err(e) => GlanceAnswer::Err { ok: false, error: e },
    })
    .await
}

/// `GET /api/listing`: one listing's own page, held or not.
async fn listing(State(state): State<AppState>, Params(l): Params<Listing>) -> Api<feeds::ListingAnswer> {
    answer(move || feeds::listing_payload(&state.app, &l.key, &l.exchange, &l.currency, &l.name)).await
}

#[derive(Deserialize, TS)]
pub struct Filings {
    #[serde(flatten)]
    #[ts(flatten)]
    listing: Listing,
    /// ask the sources again rather than answering from what is stored
    #[serde(default, deserialize_with = "flag")]
    refresh: bool,
}

async fn filings(State(state): State<AppState>, Params(q): Params<Filings>) -> Api<feeds::FilingsAnswer> {
    let l = q.listing;
    answer(move || feeds::filings_payload(&state.app, &l.key, q.refresh)).await
}

#[derive(Deserialize, TS)]
pub struct Scope {
    #[serde(default, deserialize_with = "text")]
    scope: String,
}

async fn filings_feed(State(state): State<AppState>, Params(s): Params<Scope>) -> Api<crate::feeds::FilingsFeed> {
    answer(move || feeds::filings_feed(&state.app, &s.scope, 200)).await
}

/// One stored filing, by its regulator and that regulator's own id for it.
#[derive(Deserialize, TS)]
pub struct Document {
    #[serde(default, deserialize_with = "trimmed")]
    source: Option<String>,
    #[serde(default, deserialize_with = "trimmed")]
    id: Option<String>,
}

impl Document {
    fn named(self) -> Result<(bagholder_store::feeds::Regulator, String), ApiError> {
        match (self.source.as_deref().and_then(bagholder_store::feeds::Regulator::parse), self.id) {
            (Some(source), Some(id)) => Ok((source, id)),
            _ => Err(ApiError::BadRequest("source (SEDAR+ or SEC) and id required".into())),
        }
    }
}

/// `GET /api/filings/doc`: the document itself, opened in a tab of its own.
async fn filings_doc(State(state): State<AppState>, Params(d): Params<Document>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (source, id) = d.named()?;
    let wants_page = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"));
    let read = blocking({
        let id = id.clone();
        let app = state.app.clone();
        move || feeds::filings_document(&app, source, &id)
    })
    .await?;
    match read {
        Ok((data, content_type)) => {
            let content_type = if content_type.is_empty() { "application/pdf".to_string() } else { content_type };
            Ok(([(header::CONTENT_TYPE, content_type)], data).into_response())
        }
        // a browser asking for a page is answered with one: a raw JSON error in a
        // tab of its own is the app failing in front of the person
        Err(why) if wants_page => {
            let page = feeds::document_error_page(&state.app, source, &id, &why);
            Ok((StatusCode::BAD_GATEWAY, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response())
        }
        Err(why) => Err(ApiError::Upstream(why)),
    }
}

async fn filings_enrich(State(state): State<AppState>, Params(d): Params<Document>) -> Api<feeds::EnrichAnswer> {
    let (source, id) = d.named()?;
    answer(move || feeds::filings_enrich(&state.app, source, &id)).await
}

#[derive(Deserialize, TS)]
pub struct Fear {
    #[serde(default, deserialize_with = "trimmed")]
    index: Option<String>,
}

async fn fear(State(state): State<AppState>, Params(q): Params<Fear>) -> Api<feeds::FearAnswer> {
    answer(move || feeds::fear_payload(&state.app, q.index.as_deref().unwrap_or("stocks")).into()).await
}

#[derive(Deserialize, TS)]
pub struct ShortsQuery {
    #[serde(flatten)]
    #[ts(flatten)]
    listing: Listing,
    /// carry the series too, for the listing's own page
    #[serde(default, deserialize_with = "flag")]
    trend: bool,
}

async fn shorts(State(state): State<AppState>, Params(q): Params<ShortsQuery>) -> Api<feeds::ShortsAnswer> {
    let l = q.listing;
    answer(move || feeds::shorts_payload(&state.app, &l.key, &l.exchange, q.trend).into()).await
}

async fn shorts_feed(State(state): State<AppState>) -> Api<crate::feeds::ShortsFeed> {
    answer(move || feeds::shorts_feed(&state.app)).await
}

async fn news_symbol(State(state): State<AppState>, Params(l): Params<Listing>) -> Api<feeds::NewsSymbolAnswer> {
    answer(move || feeds::news_symbol_payload(&state.app, &l.key, &l.exchange, &l.currency)).await
}

/// `GET /api/history`: a chart's bars. Its parameters are read by the history
/// module itself, which is also handed them by the documents (`history:<query>`).
async fn history(State(state): State<AppState>, Params(q): Params<feeds::HistoryQuery>) -> Api<feeds::HistoryAnswer> {
    answer(move || feeds::history_payload(&state.app, &q)).await
}

/// `POST /api/watchlist/add`: the listing to follow, found as `following` says:
/// by `id` when the page knows the instrument, else by what it is called.
#[derive(Deserialize, Serialize, Default, TS)]
#[serde(default)]
pub struct WatchlistBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) id: Option<String>,
    #[serde(deserialize_with = "text")]
    pub(crate) symbol: String,
    #[serde(deserialize_with = "text")]
    pub(crate) exchange: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) currency: Option<String>,
    #[serde(rename = "securityId", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) security_id: Option<String>,
}

async fn watchlist_add(State(state): State<AppState>, Body(body): Body<WatchlistBody>) -> Api<following::WatchlistAnswer> {
    let n = following::Named {
        instrument: body.id,
        symbol: body.symbol,
        exchange: body.exchange,
        name: body.name.unwrap_or_default(),
        currency: body.currency.unwrap_or_default(),
        security_id: body.security_id.unwrap_or_default(),
    };
    answer(move || following::watch(&state.app, &n)).await
}

/// `POST /api/watchlist/remove`: the instrument to stop following.
#[derive(Deserialize, Serialize, Default, TS)]
#[serde(default, deny_unknown_fields)]
pub struct WatchlistRemove {
    pub(crate) id: String,
}

async fn watchlist_remove(State(state): State<AppState>, Body(body): Body<WatchlistRemove>) -> Api<following::WatchlistAnswer> {
    answer(move || following::unwatch(&state.app, &body.id)).await
}

/// One tile of the row: an instrument of the directory, by its symbol and venue.
#[derive(Deserialize, Default, TS)]
#[serde(default, deny_unknown_fields)]
pub struct TileRef {
    symbol: String,
    exchange: String,
}

/// What `POST /api/tiles/set` accepts: the tile row, in order.
#[derive(Deserialize, Default, TS)]
#[serde(default, deny_unknown_fields)]
pub struct TilesSet {
    tiles: Vec<TileRef>,
}

async fn tiles_set(State(state): State<AppState>, Body(body): Body<TilesSet>) -> Api<following::TilesAnswer> {
    let tiles: Vec<(String, String)> = body.tiles.into_iter().map(|t| (t.symbol, t.exchange)).collect();
    answer(move || following::set_tiles(&state.app, &tiles)).await
}

#[cfg(test)]
mod listings_held {
    use bagholder_model::wire::SymbolMatch;

    /// A listing the book holds is not a second row of the search, whichever of the US
    /// market's exchanges the directory names it on; the same symbol on another market
    /// is its own listing, and stays (`SPEC.md` filters-24; brief 36, 1).
    #[test]
    fn a_listing_the_book_holds_is_left_out_on_any_exchange_of_its_market() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let f = app.figures.get().unwrap();
        let held: Vec<(String, String)> = f
            .read(|e| {
                e.inputs()
                    .ledger
                    .instruments
                    .values()
                    .filter_map(|i| {
                        let n = i.current_name()?;
                        let mic = bagholder_sources::venue::known_mic(n.venue_mic.as_deref(), n.venue_name.as_deref())?;
                        Some((crate::orders::bare_symbol(&n.symbol), mic.to_string()))
                    })
                    .collect()
            })
            .unwrap();
        let (us, _) = held.iter().find(|(_, m)| bagholder_sources::venue::listing_venue(m).as_deref() == Some("US")).expect("a US listing held").clone();
        let row = |symbol: &str, exchange: &str| SymbolMatch { symbol: symbol.into(), exchange: exchange.into(), currency: "USD".into(), ..SymbolMatch::default() };
        let kept = super::not_held(&app, vec![row(&us, "NYSE Arca"), row(&us, "NYSE American"), row(&us, "NASDAQ"), row(&us, "TSX"), row("ZZZZQ", "NYSE")]);
        let kept: Vec<(String, String)> = kept.into_iter().map(|m| (m.symbol, m.exchange)).collect();
        assert_eq!(kept, vec![(us.clone(), "TSX".to_string()), ("ZZZZQ".to_string(), "NYSE".to_string())]);
    }
}

#[cfg(test)]
mod resolver {
    use super::*;
    use bagholder_core::listing::{CANADA_MICS, US_MICS};
    use bagholder_model::wire::SymbolMatch;

    fn row(symbol: &str, exchange: &str) -> SymbolMatch {
        SymbolMatch { symbol: symbol.into(), name: format!("{symbol} on {exchange}"), exchange: exchange.into(), currency: "CAD".into(), kind: None, rank: None }
    }

    /// A symbol on k venues is k rows, each with its venue, whatever venues they
    /// are: one listing written two ways is one row, the book's first, another
    /// symbol is none, and nothing is ever reduced to one of several.
    #[test]
    fn a_symbol_resolves_to_every_listing_of_it_and_never_picks_one() {
        let venues: Vec<&str> = CANADA_MICS.iter().chain(US_MICS.iter()).copied().collect();
        for k in 1..=venues.len() {
            let on: Vec<&str> = venues.iter().copied().cycle().skip(k).take(k).collect();
            let mut found: Vec<SymbolMatch> = on.iter().map(|v| row("ZQ.B", v)).collect();
            // the same listings again, as another source spells them; and another symbol
            found.extend(on.iter().map(|v| row("zq-b", v)));
            found.push(row("ZQ", on[0]));
            let held_key = bagholder_core::ListingKey::on("ZQ.B", on[0]).unwrap();
            let got = resolve(vec![(held_key.clone(), row("ZQ.B", on[0]))], found, "zq/b");
            assert_eq!(got.first().map(|r| r.listing.clone()), Some(held_key.to_string()), "the book's own first");
            assert!(got[0].held && got[1..].iter().all(|r| !r.held));
            let mut keys: Vec<String> = got.iter().map(|r| r.listing.clone()).collect();
            keys.sort();
            let mut want: Vec<String> = on.iter().map(|v| bagholder_core::ListingKey::on("ZQ.B", v).unwrap().to_string()).collect();
            want.sort();
            want.dedup();
            assert_eq!(keys, want, "{k} venues: {on:?}");
        }
    }
}

#[cfg(test)]
mod resolver_book {
    /// The book's own listings answer the resolver, with no directory asked.
    #[test]
    fn the_books_listings_of_a_symbol_are_resolved_from_the_book() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let held = super::held_listings(&app).expect("the figures are open");
        let (key, m) = held.iter().find(|(k, _)| k.venue() != "CRYPTO").expect("a listed instrument").clone();
        let got = super::resolve(held.clone(), vec![], &m.symbol);
        assert!(got.iter().any(|r| r.listing == key.to_string() && r.held), "{got:?} from {held:?}");
    }
}
