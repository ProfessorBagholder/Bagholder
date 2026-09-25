//! The routes that do not answer once: the page's event stream, the
//! notifications stream, and the sign-in window's frames.

use std::convert::Infallible;

use axum::body::{Body as ResponseBody, Bytes};
use axum::http::{header, HeaderMap};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::extract::trimmed;
use super::{blocking, Api, AppState, Body, Params};
use crate::events::{self, Feed};

#[derive(Deserialize)]
pub struct EventsQuery {
    /// the page's filters, as the JSON it keeps them in
    #[serde(default, deserialize_with = "trimmed")]
    filters: Option<String>,
    /// the trade whose page is open, when one is
    #[serde(default, deserialize_with = "trimmed")]
    trade: Option<String>,
    /// the time zone of the page's browser (IANA): the person's days are in it
    #[serde(default, deserialize_with = "trimmed")]
    zone: Option<String>,
}

/// `GET /api/events`: the page's data once, then only what changes in it. A task,
/// not a thread: it sleeps on the bell between changes, and only the comparison
/// (`Feed::step`) runs on a blocking thread. When the page goes, the stream is
/// dropped and the `Feed` with it, which is what tells the background work that
/// nobody is looking any more.
pub async fn events(axum::extract::State(state): axum::extract::State<AppState>, Params(q): Params<EventsQuery>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let filters = q.filters.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let app = state.app;
    if let Some(zone) = q.zone {
        // the zone of the browser in use: kept, and "today" follows it
        let a = app.clone();
        let _ = blocking(move || {
            if let Some(f) = a.figures.get() {
                if let Err(e) = f.state_zone(&zone, bagholder_core::jiff::Timestamp::now()) {
                    crate::app::log(&format!("bagholder: the zone the page stated ({zone}) was not taken: {e}"));
                }
            }
        })
        .await;
    }
    let feed = Feed::open(app.clone(), filters, q.trade);
    let hello = feed.hello();
    let changes = stream::unfold((Some(feed), app.events.subscribe(), true), move |(feed, mut rx, first)| {
    let value = app.clone();
    async move {
        let mut feed = feed?;
        if !first {
            events::changed(&mut rx).await;
        }
        loop {
            if value.stopping() {
                return None;
            }
            // mark what has been seen before looking, so a change made while
            // looking is not missed
            rx.borrow_and_update();
            let (back, messages) = blocking(move || {
                let messages = feed.step(&crate::status::status);
                (feed, messages)
            })
            .await
            .ok()?;
            feed = back;
            if !messages.is_empty() {
                return Some((messages, (Some(feed), rx, false)));
            }
            events::changed(&mut rx).await;
        }
    }
    });
    let messages = stream::iter([vec![hello]]).chain(changes).flat_map(stream::iter);
    Sse::new(messages.map(|(name, data)| Ok(Event::default().event(name).data(data.to_string()))))
        .keep_alive(KeepAlive::new().interval(events::KEEPALIVE).text("ping"))
}

#[derive(Deserialize, Default)]
pub struct Watch {
    /// the stream this page holds, from its `hello`
    #[serde(default)]
    id: u64,
    /// what the page is showing now beyond the model: document key -> its parameters
    #[serde(default)]
    docs: Map<String, Value>,
}

/// The stream watch's own answer -- just whether the id was one still open.
#[derive(serde::Serialize, ts_rs::TS)]
pub struct WatchAck {
    pub ok: bool,
}

/// `POST /api/events/watch`
pub async fn watch(axum::extract::State(state): axum::extract::State<AppState>, Body(w): Body<Watch>) -> Api<WatchAck> {
    let app = state.app;
    let ok = blocking(move || app.events.watch(&app, w.id, w.docs.into_iter().collect())).await?;
    Ok(Json(WatchAck { ok }))
}

/// A body written by a synchronous producer on a thread of its own, for the two
/// streams whose sources are blocking by nature (the notification wait and the
/// sign-in browser's frames). `write` answers false once the reader has gone.
fn produced(name: &str, produce: impl FnOnce(&mut dyn FnMut(&[u8]) -> bool) + Send + 'static) -> ResponseBody {
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(8);
    crate::app::spawn(name, move || produce(&mut |chunk: &[u8]| chunk.is_empty() || tx.blocking_send(Bytes::copy_from_slice(chunk)).is_ok()));
    ResponseBody::from_stream(stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|chunk| (Ok::<_, Infallible>(chunk), rx)) }))
}

#[derive(Deserialize)]
pub struct After {
    #[serde(default, deserialize_with = "trimmed")]
    after: Option<String>,
}

/// `GET /api/notifications/stream`: every notification made from now on (or after
/// `after` / `Last-Event-ID`), each once. Folded into `/api/events` in stage 6.
pub async fn notifications(axum::extract::State(state): axum::extract::State<AppState>, Params(q): Params<After>, headers: HeaderMap) -> Response {
    let named = q.after.or_else(|| headers.get("last-event-id").and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string()));
    let after = named.filter(|a| !a.is_empty() && a.bytes().all(|c| c.is_ascii_digit())).and_then(|a| a.parse::<i64>().ok());
    let app = state.app;
    let body = produced("bagholder-notify-stream", move |write| crate::notify::stream(&app, after, |text| write(text.as_bytes())));
    ([(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")], body).into_response()
}

/// `GET /api/login/stream`: the sign-in window as a multipart JPEG stream.
pub async fn login(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let app = state.app;
    let body = produced("bagholder-login-stream", move |write| crate::login::login_stream(&app, |chunk| write(chunk)));
    ([(header::CONTENT_TYPE, "multipart/x-mixed-replace; boundary=frame")], body).into_response()
}
