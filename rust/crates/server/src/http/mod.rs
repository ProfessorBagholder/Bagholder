//! The HTTP server: an axum router on tokio (docs/architecture.md, "Process and
//! HTTP").
//!
//! Every route is a function with a typed request -- `Params<T>` for a query,
//! `Body<T>` for a JSON body -- and answers `Result<_, ApiError>`, so a bad
//! request, a missing thing, a failed store and a failed upstream each have one
//! status code and one shape, decided in one place. The work behind a route is
//! synchronous (SQLite, the market and Wealthsimple clients), so a handler hands
//! it to the blocking pool (`blocking`) and the runtime's own threads only ever
//! move bytes.
//!
//! Around the routes, outermost first: a panic becomes a 500 rather than a
//! dropped connection; every answer is `no-store` and `nosniff` unless it says
//! otherwise; the gate (`gate`) turns away anything that is not the page on this
//! machine; a body is at most `BODY_LIMIT`. Routes that answer once are given
//! `ROUTE_TIMEOUT`; the three that stream are not.

mod assets;
mod error;
mod extract;
mod markets;
mod model;
mod notifications;
mod orders;
mod session;
mod stream;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

use crate::app::App;
pub use error::ApiError;
pub use extract::{Body, Params};

/// What a route can reach. One field today: the app, which the modules behind
/// the routes still reach as a global. Routes take it from here so that the
/// global can be retired module by module without touching them again.
#[derive(Clone)]
pub struct AppState {
    pub app: Arc<App>,
}

/// What a JSON route answers.
pub type Api = Result<Json<Value>, ApiError>;

/// The most a request body may be. The page's largest is a CSV import.
const BODY_LIMIT: usize = 32 * 1024 * 1024;
/// The longest a route that answers once may take: the slowest are a forced
/// re-read of a listing's filings and an order's round trip to Wealthsimple.
const ROUTE_TIMEOUT: Duration = Duration::from_secs(120);

/// Run synchronous work on the blocking pool. Work that panics is a 500; the
/// panic itself has been logged by the process's hook.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work).await.map_err(|_| ApiError::Internal)
}

/// Synchronous work that answers a JSON value.
pub async fn answer(work: impl FnOnce() -> Value + Send + 'static) -> Api {
    Ok(Json(blocking(work).await?))
}

/// Synchronous work on a connection to the store.
pub async fn with_store(state: &AppState, work: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<Value> + Send + 'static) -> Api {
    let app = state.app.clone();
    Ok(Json(blocking(move || app.open().and_then(|conn| work(&conn))).await??))
}

pub fn router(state: AppState) -> Router {
    let once = Router::new()
        .merge(model::routes())
        .merge(markets::routes())
        .merge(orders::routes())
        .merge(session::routes())
        .merge(notifications::routes())
        .route("/api/events/watch", post(stream::watch))
        .layer(TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, ROUTE_TIMEOUT));
    let streams = Router::new()
        .route("/api/events", get(stream::events))
        .route("/api/notifications/stream", get(stream::notifications))
        .route("/api/login/stream", get(stream::login));
    Router::new()
        .merge(once)
        .merge(streams)
        .merge(assets::routes())
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(axum::middleware::from_fn_with_state(state.clone(), gate))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(CatchPanicLayer::custom(|_: Box<dyn std::any::Any + Send>| ApiError::Internal.into_response()))
        .with_state(state)
}

async fn not_found() -> ApiError {
    ApiError::NotFound("not found".into())
}

fn refused(code: StatusCode) -> Response {
    (code, Json(json!({"ok": false}))).into_response()
}

/// The gate: loopback only unless the server was bound elsewhere on purpose, the
/// Host header has to name 127.0.0.1 and the port (a page on another origin that
/// resolves a name of its own to this machine names itself there), and a write
/// has to come from the page itself.
async fn gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    match admitted(&state.app, &req) {
        Ok(()) => next.run(req).await,
        Err(code) => refused(code),
    }
}

fn admitted(app: &App, req: &Request) -> Result<(), StatusCode> {
    let write = match *req.method() {
        Method::GET | Method::HEAD => false,
        Method::POST => true,
        Method::OPTIONS => return Err(StatusCode::FORBIDDEN),
        _ => return Err(StatusCode::NOT_IMPLEMENTED),
    };
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let head = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_lowercase();
    let port = *app.port.lock().unwrap_or_else(|e| e.into_inner());
    let from_the_page = || head("sec-fetch-site") == "same-origin" || !head("x-bagholder").is_empty();
    if peer_ok(&app.bind_host, peer) && host_ok(&app.bind_host, port, &head("host")) && (!write || from_the_page()) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn peer_ok(bind_host: &str, peer: Option<std::net::IpAddr>) -> bool {
    if bind_host != "127.0.0.1" {
        return true; // bound beyond loopback on purpose (a container); the peer is its bridge
    }
    match peer {
        Some(std::net::IpAddr::V4(ip)) => ip.is_loopback(),
        Some(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
        None => false,
    }
}

fn host_ok(bind_host: &str, port: u16, host: &str) -> bool {
    if host.is_empty() || host.contains(',') {
        return false;
    }
    if bind_host != "127.0.0.1" {
        // a container's port may be published under another number; the name must still be 127.0.0.1
        return match host.split_once(':') {
            Some((name, p)) => name == "127.0.0.1" && !p.is_empty() && p.len() <= 5 && p.bytes().all(|c| c.is_ascii_digit()),
            None => false,
        };
    }
    host == format!("127.0.0.1:{}", port)
}

/// Serve until the app is told to stop; then let the requests in hand finish,
/// for at most `DRAIN`.
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> std::io::Result<()> {
    const DRAIN: Duration = Duration::from_secs(5);
    let app = state.app.clone();
    let stopped = || {
        let app = app.clone();
        async move {
            let mut rx = app.events.subscribe();
            while !app.stopping() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
    };
    let service = router(state).into_make_service_with_connect_info::<SocketAddr>();
    tokio::select! {
        served = axum::serve(listener, service).with_graceful_shutdown(stopped()) => served,
        _ = async { stopped().await; tokio::time::sleep(DRAIN).await } => Ok(()),
    }
}

#[cfg(test)]
mod tests;
