//! Orders and brackets. Every route here reaches Wealthsimple, or would: with
//! `BAGHOLDER_DRY_ORDERS` set nothing is placed (`orders::orders_live`).

use axum::extract::State;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::Value;

use super::extract::text;
use super::{answer, Api, AppState, Body, Params};
use crate::orders;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/orders", get(list))
        .route("/api/orders/refresh", post(refresh))
        .route("/api/order/quote", get(quote))
        .route("/api/order", post(place))
        .route("/api/order/cancel", post(cancel))
        .route("/api/order/modify", post(modify))
        .route("/api/bracket/adjust", post(bracket_adjust))
        .route("/api/bracket/cancel", post(bracket_cancel))
        .route("/api/book/append", post(book_append))
}

async fn list(State(state): State<AppState>) -> Api<Value> {
    answer(move || orders::orders_payload(&state.app, true)).await
}

async fn refresh(State(state): State<AppState>) -> Api<Value> {
    answer(move || {
        let mut r = orders::refresh_orders(&state.app, "");
        if let (Value::Object(m), Value::Object(p)) = (&mut r, orders::orders_payload(&state.app, false)) {
            m.extend(p);
        }
        r
    })
    .await
}

#[derive(Deserialize)]
struct QuoteOf {
    #[serde(default, deserialize_with = "text")]
    symbol: String,
    #[serde(default, deserialize_with = "text")]
    security: String,
    #[serde(default, deserialize_with = "text")]
    account: String,
    #[serde(default, deserialize_with = "text")]
    exchange: String,
}

async fn quote(State(state): State<AppState>, Params(q): Params<QuoteOf>) -> Api<Value> {
    answer(move || orders::ticket_quote(&state.app, &q.symbol, &q.security, &q.account, &q.exchange)).await
}

/// `POST /api/order`. The body is read as a ticket; `place_ticket` checks it field by
/// field and answers each refusal in the words the ticket shows.
async fn place(State(state): State<AppState>, Body(ticket): Body<orders::Ticket>) -> Api<Value> {
    answer(move || orders::place_ticket(&state.app, &ticket)).await
}

#[derive(Deserialize, Default)]
struct Named {
    #[serde(default)]
    id: String,
}

async fn cancel(State(state): State<AppState>, Body(o): Body<Named>) -> Api<Value> {
    answer(move || orders::cancel_order(&state.app, &o.id)).await
}

/// A resting order's new size or limit. Either may be a number or the text the
/// person typed; `modify_order` reads both.
#[derive(Deserialize, Default)]
struct Modify {
    #[serde(default)]
    id: String,
    quantity: Option<Value>,
    #[serde(rename = "limitPrice")]
    limit_price: Option<Value>,
}

async fn modify(State(state): State<AppState>, Body(m): Body<Modify>) -> Api<Value> {
    answer(move || orders::modify_order(&state.app, &m.id, m.quantity.as_ref(), m.limit_price.as_ref())).await
}

/// One leg of a bracket moved, given a trail, or taken off.
#[derive(Deserialize, Default)]
struct Adjust {
    #[serde(default)]
    id: String,
    #[serde(default)]
    leg: String,
    price: Option<Value>,
    trail: Option<Value>,
    #[serde(default)]
    remove: bool,
}

async fn bracket_adjust(State(state): State<AppState>, Body(a): Body<Adjust>) -> Api<Value> {
    answer(move || orders::adjust_bracket(&state.app, &a.id, &a.leg, a.price.as_ref(), a.trail.as_ref(), a.remove)).await
}

async fn bracket_cancel(State(state): State<AppState>, Body(b): Body<Named>) -> Api<Value> {
    answer(move || orders::cancel_bracket(&state.app, &b.id)).await
}

/// `POST /api/book/append`: a fill the person enters by hand.
async fn book_append(State(state): State<AppState>, Body(body): Body<orders::BookAppend>) -> Api<Value> {
    answer(move || orders::append_manual(&state.app, &body)).await
}
