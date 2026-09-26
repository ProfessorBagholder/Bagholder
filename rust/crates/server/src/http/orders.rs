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
    answer(move || {
        orders::kick_orders_refresh(&state.app);
        orders::orders_doc(&state.app)
    })
    .await
}

/// `POST /api/orders/refresh`: what the read found, and the panel's cards after it.
#[derive(serde::Serialize, ts_rs::TS)]
pub struct RefreshAndOrders {
    read: orders::RefreshOrdersAnswer,
    orders: orders::OrdersDoc,
}

async fn refresh(State(state): State<AppState>) -> Api<RefreshAndOrders> {
    answer(move || {
        let read = orders::refresh_orders(&state.app);
        RefreshAndOrders { read, orders: orders::orders_doc(&state.app) }
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
/// holds and the figures' rate for its currency (`orders::preview`); nothing is sent
/// anywhere.
async fn preview(State(state): State<AppState>, Body(r): Body<orders::preview::PreviewRequest>) -> Api<orders::preview::Preview> {
    let app = state.app;
    super::blocking(move || orders::preview::preview_for(&app, &r)).await?.map(axum::Json)
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

/// A resting order's new size or limit, each a number or the decimal text typed.
#[derive(Deserialize, Default, ts_rs::TS)]
#[serde(default, deny_unknown_fields)]
pub struct Modify {
    id: String,
    quantity: orders::PageDec,
    #[serde(rename = "limitPrice")]
    limit_price: orders::PageDec,
}

async fn modify(State(state): State<AppState>, Body(m): Body<Modify>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::modify_order(&state.app, &m.id, &m.quantity, &m.limit_price)).await
}

/// One leg of a bracket moved, given a trail, or taken off.
#[derive(Deserialize, Default, ts_rs::TS)]
#[serde(default, deny_unknown_fields)]
pub struct Adjust {
    id: String,
    leg: String,
    price: orders::PageDec,
    trail: orders::PageDec,
    #[ts(optional)]
    remove: Option<bool>,
}

async fn bracket_adjust(State(state): State<AppState>, Body(a): Body<Adjust>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::adjust_bracket(&state.app, &a.id, &a.leg, &a.price, &a.trail, a.remove.unwrap_or(false))).await
}

async fn bracket_cancel(State(state): State<AppState>, Body(b): Body<Named>) -> Api<orders::OrderActionAnswer> {
    answer(move || orders::cancel_bracket(&state.app, &b.id)).await
}
