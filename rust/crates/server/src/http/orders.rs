//! Orders and brackets. Every route here reaches Wealthsimple, or would: with
//! `BAGHOLDER_DRY_ORDERS` set nothing is placed (`orders::orders_live`).

use axum::extract::State;
use serde::Deserialize;

use super::extract::text;
use super::{answer, api_routes, Api, AppState, Body, Params, Routed};
use crate::orders;

pub fn routes() -> Routed {
    api_routes! {
        get "/api/orders" => list;
        post "/api/order/cancel" => cancel;
        post "/api/order/modify" => modify;
        post "/api/bracket/adjust" => bracket_adjust;
        post "/api/bracket/cancel" => bracket_cancel;
        post "/api/orders/refresh" => refresh;
        get "/api/order/quote" => quote;
        post "/api/order/preview" => preview;
        post "/api/order" => place;
    }
}

async fn list(State(state): State<AppState>) -> Api<orders::OrdersDoc> {
    answer(move || orders::orders_payload(&state.app, true)).await
}

/// `POST /api/orders/refresh`: what changed, and the panel's rows besides,
/// the two documents merged into one object as they always were, with a
/// key both carry -- `ok` -- taken from `OrdersDoc`'s (always true), the
/// one the untyped `Map::extend` used to leave standing.
#[derive(serde::Serialize, ts_rs::TS)]
pub struct RefreshAndOrders {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    skipped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    read: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    added: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    failed: Option<i64>,
    orders: Vec<orders::OrderCard>,
    brackets: Vec<bagholder_store::orders::Bracket>,
    /// Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
    live: bool,
    #[serde(rename = "refreshedAt")]
    refreshed_at: String,
}

async fn refresh(State(state): State<AppState>) -> Api<RefreshAndOrders> {
    answer(move || {
        let r = orders::refresh_orders(&state.app, "");
        let doc = orders::orders_payload(&state.app, false);
        RefreshAndOrders {
            ok: doc.ok,
            skipped: r.skipped,
            read: r.read,
            added: r.added,
            failed: r.failed,
            orders: doc.orders,
            brackets: doc.brackets,
            live: doc.live,
            refreshed_at: doc.refreshed_at,
        }
    })
    .await
}

#[derive(Deserialize, ts_rs::TS)]
pub struct QuoteOf {
    #[serde(default, deserialize_with = "text")]
    symbol: String,
    #[serde(default, deserialize_with = "text")]
    security: String,
    #[serde(default, deserialize_with = "text")]
    account: String,
    #[serde(default, deserialize_with = "text")]
    exchange: String,
}

async fn quote(State(state): State<AppState>, Params(q): Params<QuoteOf>) -> Api<orders::TicketQuote> {
    answer(move || orders::ticket_quote(&state.app, &q.symbol, &q.security, &q.account, &q.exchange)).await
}

/// `POST /api/order/preview`: the ticket's figures, worked out exactly from what it
/// holds (`orders::preview`); nothing is sent anywhere.
async fn preview(State(_state): State<AppState>, Body(r): Body<orders::preview::PreviewRequest>) -> Api<orders::preview::Preview> {
    orders::preview::preview(&r).map(axum::Json).map_err(|orders::preview::Unread(why)| super::ApiError::BadRequest(why))
}

/// `POST /api/order`. The body is read as a ticket; `place_ticket` checks it field by
/// field and answers each refusal in the words the ticket shows.
async fn place(State(state): State<AppState>, Body(ticket): Body<orders::Ticket>) -> Api<orders::PlaceTicketAnswer> {
    answer(move || orders::place_ticket(&state.app, &ticket)).await
}

#[derive(Deserialize, Default, ts_rs::TS)]
pub struct Named {
    #[serde(default)]
    id: String,
}

async fn cancel(State(state): State<AppState>, Body(o): Body<Named>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::cancel_order(&state.app, &o.id)).await
}

/// A resting order's new size or limit. Either may be a number or the text the
/// person typed; read leniently into the number it names.
#[derive(Deserialize, Default, ts_rs::TS)]
#[serde(default)]
pub struct Modify {
    id: String,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    #[ts(type = "number | string | null")]
    quantity: Option<f64>,
    #[serde(rename = "limitPrice", deserialize_with = "bagholder_model::lenient::maybe_number")]
    #[ts(type = "number | string | null")]
    limit_price: Option<f64>,
}

async fn modify(State(state): State<AppState>, Body(m): Body<Modify>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::modify_order(&state.app, &m.id, m.quantity, m.limit_price)).await
}

/// One leg of a bracket moved, given a trail, or taken off.
#[derive(Deserialize, Default, ts_rs::TS)]
#[serde(default)]
pub struct Adjust {
    id: String,
    leg: String,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    #[ts(optional, type = "number | string | null")]
    price: Option<f64>,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    #[ts(optional, type = "number | string | null")]
    trail: Option<f64>,
    #[ts(optional)]
    remove: Option<bool>,
}

async fn bracket_adjust(State(state): State<AppState>, Body(a): Body<Adjust>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::adjust_bracket(&state.app, &a.id, &a.leg, a.price, a.trail, a.remove.unwrap_or(false))).await
}

async fn bracket_cancel(State(state): State<AppState>, Body(b): Body<Named>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::cancel_bracket(&state.app, &b.id)).await
}
