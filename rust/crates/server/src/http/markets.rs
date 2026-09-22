//! Listings and what is read about them from outside: search, a glance quote,
//! filings, short interest, news, fear and greed, chart history, the watchlist
//! and the Markets tab's tiles.

use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::extract::{flag, text, trimmed};
use super::{answer, blocking, Api, ApiError, AppState, Body, Params};
use crate::feeds;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/symbols/search", get(symbol_search))
        .route("/api/symbols/quote", get(symbol_quote))
        .route("/api/listing", get(listing))
        .route("/api/filings", get(filings))
        .route("/api/filings/feed", get(filings_feed))
        .route("/api/filings/doc", get(filings_doc))
        .route("/api/filings/enrich", get(filings_enrich))
        .route("/api/fear", get(fear))
        .route("/api/shorts", get(shorts))
        .route("/api/shorts/feed", get(shorts_feed))
        .route("/api/news/symbol", get(news_symbol))
        .route("/api/history", get(history))
        .route("/api/markets/refresh", post(markets_refresh))
        .route("/api/watchlist/add", post(watchlist_add))
        .route("/api/watchlist/remove", post(watchlist_remove))
        .route("/api/tiles/set", post(tiles_set))
}

/// One listing, as the page names it. Only the symbol is ever required.
#[derive(Deserialize)]
struct Listing {
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

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
}

async fn symbol_search(State(state): State<AppState>, Params(s): Params<Search>) -> Api {
    let path = state.app.db_path();
    answer(move || bagholder_market::search::symbol_search(&path, &s.q)).await
}

/// `GET /api/symbols/quote`: a glance at a listing the watchlist's add row offers:
/// its price and day change, not stored.
async fn symbol_quote(State(state): State<AppState>, Params(l): Params<Listing>) -> Api {
    let app = state.app;
    answer(move || {
        let mut out = json!({"ok": true, "price": null, "priceChange": null, "percentChange": null});
        if l.symbol.is_empty() {
            return out;
        }
        let rec = json!({"symbol": l.symbol, "exchange": l.exchange, "currency": l.currency, "kind": "Shares"});
        if let Ok(conn) = app.open() {
            let (today, _, _) = bagholder_market::clock_now();
            if let Some(Value::Object(q)) = bagholder_market::quotes::peek_quote(&conn, &rec, &today) {
                for (k, v) in q {
                    out[k] = v;
                }
            }
        }
        out
    })
    .await
}

/// `GET /api/listing`: one listing's own page, held or not.
async fn listing(State(state): State<AppState>, Params(l): Params<Listing>) -> Api {
    answer(move || feeds::listing_payload(&state.app, &l.symbol, &l.exchange, &l.currency, &l.name)).await
}

#[derive(Deserialize)]
struct Filings {
    #[serde(flatten)]
    listing: Listing,
    /// ask the sources again rather than answering from what is stored
    #[serde(default, deserialize_with = "flag")]
    refresh: bool,
}

async fn filings(State(state): State<AppState>, Params(q): Params<Filings>) -> Api {
    let l = q.listing;
    if l.symbol.is_empty() {
        return Err(ApiError::BadRequest("symbol required".into()));
    }
    answer(move || feeds::filings_payload(&state.app, &l.symbol, q.refresh, some(&l.name), some(&l.exchange), some(&l.currency))).await
}

#[derive(Deserialize)]
struct Scope {
    #[serde(default, deserialize_with = "text")]
    scope: String,
}

async fn filings_feed(State(state): State<AppState>, Params(s): Params<Scope>) -> Api {
    answer(move || feeds::filings_feed(&state.app, &s.scope, 200)).await
}

/// One stored filing of one listing.
#[derive(Deserialize)]
struct Document {
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

async fn filings_enrich(State(state): State<AppState>, Params(d): Params<Document>) -> Api {
    let (symbol, id) = d.named()?;
    answer(move || feeds::filings_enrich(&state.app, &symbol, &id)).await
}

#[derive(Deserialize)]
struct Fear {
    #[serde(default, deserialize_with = "trimmed")]
    index: Option<String>,
}

async fn fear(State(state): State<AppState>, Params(q): Params<Fear>) -> Api {
    answer(move || feeds::fear_payload(&state.app, q.index.as_deref().unwrap_or("stocks"))).await
}

#[derive(Deserialize)]
struct Shorts {
    #[serde(flatten)]
    listing: Listing,
    /// carry the series too, for the listing's own page
    #[serde(default, deserialize_with = "flag")]
    trend: bool,
}

async fn shorts(State(state): State<AppState>, Params(q): Params<Shorts>) -> Api {
    let l = q.listing;
    answer(move || feeds::shorts_payload(&state.app, &l.symbol, some(&l.exchange), some(&l.currency), q.trend)).await
}

async fn shorts_feed(State(state): State<AppState>) -> Api {
    answer(move || feeds::shorts_feed(&state.app)).await
}

async fn news_symbol(State(state): State<AppState>, Params(l): Params<Listing>) -> Api {
    answer(move || feeds::news_symbol_payload(&state.app, &l.symbol, &l.exchange, &l.currency)).await
}

/// `GET /api/history`: a chart's bars. Its parameters are read by the history
/// module itself, which is also handed them by the documents (`history:<query>`).
async fn history(State(state): State<AppState>, RawQuery(query): RawQuery) -> Api {
    answer(move || feeds::history_payload(&state.app, query.as_deref().unwrap_or(""))).await
}

async fn markets_refresh() -> Api {
    answer(feeds::kick_universes).await
}

// The three writes below hand their body to the module that owns the rows; it
// becomes a typed request with the typed watchlist and tiles (stage 5).

async fn watchlist_add(State(state): State<AppState>, Body(body): Body<Map<String, Value>>) -> Api {
    answer(move || feeds::watch_add(&state.app, &Value::Object(body))).await
}

async fn watchlist_remove(State(state): State<AppState>, Body(body): Body<Map<String, Value>>) -> Api {
    answer(move || feeds::watch_remove(&state.app, &Value::Object(body))).await
}

async fn tiles_set(State(state): State<AppState>, Body(body): Body<Map<String, Value>>) -> Api {
    answer(move || feeds::tiles_set(&state.app, &Value::Object(body))).await
}
