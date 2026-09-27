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

/// One listing, as the page names it. Only the symbol is ever required.
#[derive(Deserialize, TS)]
pub struct Listing {
    #[serde(default, deserialize_with = "text")]
    symbol: String,
    #[serde(default, deserialize_with = "text")]
    exchange: String,
    #[serde(default, deserialize_with = "text")]
    currency: String,
    #[serde(default, deserialize_with = "text")]
    name: String,
}

fn some(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
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
    let pool = state.app.cache_pool()?;
    answer(move || match bagholder_market::search::symbol_search(&pool, &s.q) {
        Ok(matches) => SymbolSearchAnswer::Ok { ok: true, matches },
        Err(error) => SymbolSearchAnswer::Err { ok: false, error, matches: vec![] },
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
    answer(move || feeds::listing_payload(&state.app, &l.symbol, &l.exchange, &l.currency, &l.name)).await
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
    if l.symbol.is_empty() {
        return Err(ApiError::BadRequest("symbol required".into()));
    }
    answer(move || feeds::filings_payload(&state.app, &l.symbol, q.refresh, some(&l.name), some(&l.exchange), some(&l.currency))).await
}

#[derive(Deserialize, TS)]
pub struct Scope {
    #[serde(default, deserialize_with = "text")]
    scope: String,
}

async fn filings_feed(State(state): State<AppState>, Params(s): Params<Scope>) -> Api<crate::feeds::FilingsFeed> {
    answer(move || feeds::filings_feed(&state.app, &s.scope, 200)).await
}

/// One stored filing of one listing.
#[derive(Deserialize, TS)]
pub struct Document {
    #[serde(default, deserialize_with = "trimmed")]
    symbol: Option<String>,
    #[serde(default, deserialize_with = "trimmed")]
    id: Option<String>,
}

impl Document {
    fn named(self) -> Result<(String, String), ApiError> {
        match (self.symbol, self.id) {
            (Some(symbol), Some(id)) => Ok((symbol, id)),
            _ => Err(ApiError::BadRequest("symbol and id required".into())),
        }
    }
}

/// `GET /api/filings/doc`: the document itself, opened in a tab of its own.
async fn filings_doc(State(state): State<AppState>, Params(d): Params<Document>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (symbol, id) = d.named()?;
    let wants_page = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"));
    let read = blocking({
        let (symbol, id) = (symbol.clone(), id.clone());
        let app = state.app.clone();
        move || feeds::filings_document(&app, &symbol, &id)
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
            let page = feeds::document_error_page(&state.app, &symbol, &id, &why);
            Ok((StatusCode::BAD_GATEWAY, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response())
        }
        Err(why) => Err(ApiError::Upstream(why)),
    }
}

async fn filings_enrich(State(state): State<AppState>, Params(d): Params<Document>) -> Api<feeds::EnrichAnswer> {
    let (symbol, id) = d.named()?;
    answer(move || feeds::filings_enrich(&state.app, &symbol, &id)).await
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
    answer(move || feeds::shorts_payload(&state.app, &l.symbol, some(&l.exchange), some(&l.currency), q.trend).into()).await
}

async fn shorts_feed(State(state): State<AppState>) -> Api<crate::feeds::ShortsFeed> {
    answer(move || feeds::shorts_feed(&state.app)).await
}

async fn news_symbol(State(state): State<AppState>, Params(l): Params<Listing>) -> Api<feeds::NewsSymbolAnswer> {
    answer(move || feeds::news_symbol_payload(&state.app, &l.symbol, &l.exchange, &l.currency)).await
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
