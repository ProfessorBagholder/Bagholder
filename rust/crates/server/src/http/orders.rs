//! Orders and brackets. Every route here reaches Wealthsimple, or would: with
//! `BAGHOLDER_DRY_ORDERS` set nothing is placed (`orders::orders_live`).

use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::{Map, Value};

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

async fn list() -> Api {
    answer(|| orders::orders_payload(true)).await
}

async fn refresh() -> Api {
    answer(|| {
        let mut r = orders::refresh_orders("");
        if let (Value::Object(m), Value::Object(p)) = (&mut r, orders::orders_payload(false)) {
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

async fn quote(Params(q): Params<QuoteOf>) -> Api {
    answer(move || orders::ticket_quote(&q.symbol, &q.security, &q.account, &q.exchange)).await
}

/// `POST /api/order`. The ticket is validated field by field by `place_order`,
/// which answers each refusal in words; it takes the ticket as it was sent until
/// the order types are typed with the rest of the domain (stage 5).
async fn place(Body(ticket): Body<Map<String, Value>>) -> Api {
    answer(move || orders::place_order(&Value::Object(ticket))).await
}

#[derive(Deserialize, Default)]
struct Named {
    #[serde(default)]
    id: String,
}

async fn cancel(Body(o): Body<Named>) -> Api {
    answer(move || orders::cancel_order(&o.id)).await
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

async fn modify(Body(m): Body<Modify>) -> Api {
    answer(move || orders::modify_order(&m.id, m.quantity.as_ref(), m.limit_price.as_ref())).await
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

async fn bracket_adjust(Body(a): Body<Adjust>) -> Api {
    answer(move || orders::adjust_bracket(&a.id, &a.leg, a.price.as_ref(), a.trail.as_ref(), a.remove)).await
}

async fn bracket_cancel(Body(b): Body<Named>) -> Api {
    answer(move || orders::cancel_bracket(&b.id)).await
}

/// `POST /api/book/append`: a fill the person enters by hand.
async fn book_append(Body(row): Body<Map<String, Value>>) -> Api {
    answer(move || orders::append_manual(&Value::Object(row))).await
}
