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
//! machine; a body is never cut at a size. Routes that answer once are given
//! `ROUTE_TIMEOUT`; the three that stream are not.

mod admit;
pub(crate) mod agents;
pub(crate) mod assets;
mod error;
mod extract;
pub(crate) mod markets;
pub(crate) mod model;
mod notifications;
pub(crate) mod orders;
pub(crate) mod remote;
mod session;
pub mod stream;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

use crate::app::App;
pub use error::ApiError;
pub use extract::{Body, Params, Required};

/// What a route can reach. One field today: the app, which the modules behind
/// the routes still reach as a global. Routes take it from here so that the
/// global can be retired module by module without touching them again.
#[derive(Clone)]
pub struct AppState {
    pub app: Arc<App>,
}

/// `{"ok": true}`, or `{"ok": false, "error": "…"}`: a soft refusal (still
/// 200) that several routes across `login`, `session` and `update` answer in
/// this one shape.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
pub struct OkOr {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

impl OkOr {
    pub fn ok() -> OkOr {
        OkOr { ok: true, error: None }
    }
    pub fn err(e: impl Into<String>) -> OkOr {
        OkOr { ok: false, error: Some(e.into()) }
    }
}

/// What a JSON route answers: `Value` while a module is still untyped, the
/// route's own answer type once it is converted (stage 5d7d).
pub type Api<T> = Result<Json<T>, ApiError>;

/// The most an answer is read back into memory to tag it (`conditional`).
const ANSWER_LIMIT: usize = 128 * 1024 * 1024;
/// The longest a route that answers once may take: the slowest are a forced
/// re-read of a listing's filings and an order's round trip to Wealthsimple.
const ROUTE_TIMEOUT: Duration = Duration::from_secs(120);

/// One entry in the route table (`api_routes!`): how the page calls a route
/// and the TypeScript names of what it exchanges, for `routes.ts`. The names
/// are read from the handler's own signature (`Signature`), so the table cannot
/// say one thing while the handler takes or answers another. Read by
/// `tests_types.rs`, which only runs under `#[cfg(test)]`.
#[cfg_attr(not(test), allow(dead_code))]
pub struct RouteEntry {
    pub method: &'static str,
    pub path: &'static str,
    pub query: Option<String>,
    pub body: Option<String>,
    pub answer: String,
}

impl RouteEntry {
    /// The entry for `handler` at `method` `path`.
    pub fn of<M, H: Signature<M>>(_handler: H, method: &'static str, path: &'static str) -> RouteEntry {
        let (query, body, answer) = H::names();
        RouteEntry { method, path, query, body, answer }
    }
}

/// The TypeScript name of a type the page is sent or sends.
fn ts_name<T: ts_rs::TS>() -> String {
    T::ident(&ts_rs::Config::new())
}

/// What a route's answer is, whichever way the handler returns it.
pub trait Answered {
    type Answer: ts_rs::TS;
}
impl<A: ts_rs::TS> Answered for Api<A> {
    type Answer = A;
}
impl<A: ts_rs::TS> Answered for Result<(StatusCode, Json<A>), ApiError> {
    type Answer = A;
}

/// A handler's query, body and answer, read from its signature. `M` only tells
/// the shapes apart: a handler takes the state and at most one of a query or a body.
pub trait Signature<M> {
    fn names() -> (Option<String>, Option<String>, String);
}
impl<F, Fut> Signature<(State<AppState>,)> for F
where
    F: Fn(State<AppState>) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    fn names() -> (Option<String>, Option<String>, String) {
        (None, None, ts_name::<<Fut::Output as Answered>::Answer>())
    }
}
impl<F, Fut, Q: ts_rs::TS> Signature<(State<AppState>, Params<Q>)> for F
where
    F: Fn(State<AppState>, Params<Q>) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    fn names() -> (Option<String>, Option<String>, String) {
        (Some(ts_name::<Q>()), None, ts_name::<<Fut::Output as Answered>::Answer>())
    }
}
impl<F, Fut, B: ts_rs::TS> Signature<(State<AppState>, Required<B>)> for F
where
    F: Fn(State<AppState>, Required<B>) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    fn names() -> (Option<String>, Option<String>, String) {
        (None, Some(ts_name::<B>()), ts_name::<<Fut::Output as Answered>::Answer>())
    }
}
impl<F, Fut, B: ts_rs::TS> Signature<(State<AppState>, Body<B>)> for F
where
    F: Fn(State<AppState>, Body<B>) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    fn names() -> (Option<String>, Option<String>, String) {
        (None, Some(ts_name::<B>()), ts_name::<<Fut::Output as Answered>::Answer>())
    }
}

/// A route that takes a file as it is (`extract::Upload`): its query, and the body
/// the page sends as the file itself (`Blob`).
impl<F, Fut, Q: ts_rs::TS> Signature<(State<AppState>, Params<Q>, extract::Upload)> for F
where
    F: Fn(State<AppState>, Params<Q>, extract::Upload) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    fn names() -> (Option<String>, Option<String>, String) {
        (Some(ts_name::<Q>()), Some("Blob".to_string()), ts_name::<<Fut::Output as Answered>::Answer>())
    }
}

/// A route the agents' read client may call (`crate::mcp`): its path, the query it
/// takes and the answer it gives, made only from a handler whose signature takes
/// no body (`Readable`), so no route that is sent a body can be one. That the
/// route table declares it a `GET` is held by `crate::mcp`'s tests, for a route
/// with no body either way.
pub struct GetRoute<Q, A> {
    pub path: &'static str,
    _types: std::marker::PhantomData<fn(Q) -> A>,
}

/// The query of a route that takes none.
#[derive(Serialize)]
pub struct NoQuery {}

/// A handler that takes no body: the state alone, or the state and a query.
pub trait Readable<M> {
    type Query;
    type Answer;
}
impl<F, Fut> Readable<(State<AppState>,)> for F
where
    F: Fn(State<AppState>) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    type Query = NoQuery;
    type Answer = <Fut::Output as Answered>::Answer;
}
impl<F, Fut, Q> Readable<(State<AppState>, Params<Q>)> for F
where
    F: Fn(State<AppState>, Params<Q>) -> Fut,
    Fut: std::future::Future,
    Fut::Output: Answered,
{
    type Query = Q;
    type Answer = <Fut::Output as Answered>::Answer;
}

impl<Q, A> GetRoute<Q, A> {
    /// The route `handler` answers at `path`.
    pub fn of<M, H: Readable<M, Query = Q, Answer = A>>(_handler: H, path: &'static str) -> GetRoute<Q, A> {
        GetRoute { path, _types: std::marker::PhantomData }
    }
}

/// One module's routes, and the table entries it contributes.
pub struct Routed {
    pub router: Router<AppState>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub table: Vec<RouteEntry>,
}

/// Each route declared once, by method, path and handler: it adds the axum
/// route and the route's table entry, whose types come from the handler.
macro_rules! api_routes {
    ($($method:ident $path:literal => $handler:path);* $(;)?) => {{
        #[allow(unused_mut)]
        let mut router = axum::Router::new();
        #[allow(unused_mut)]
        let mut table = Vec::new();
        $(
            router = router.route($path, axum::routing::$method($handler));
            table.push($crate::http::RouteEntry::of($handler, stringify!($method), $path));
        )*
        $crate::http::Routed { router, table }
    }};
}
pub(crate) use api_routes;

/// The route table: every route `api_routes!` declared, from every module —
/// what `tests_types.rs` turns into `web/src/lib/generated/routes.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn route_table() -> Vec<RouteEntry> {
    let mut t = notifications::routes().table;
    t.extend(session::routes().table);
    t.extend(orders::routes().table);
    t.extend(markets::routes().table);
    t.extend(model::routes().table);
    t.extend(agents::routes().table);
    t.extend(remote::routes().table);
    t
}

/// Run synchronous work on the blocking pool. Work that panics is a 500; the
/// panic itself has been logged by the process's hook.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work).await.map_err(|_| ApiError::Internal)
}

/// Synchronous work that answers a JSON value.
pub async fn answer<T: Serialize + Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Api<T> {
    Ok(Json(blocking(work).await?))
}

/// Synchronous work on the book the notices are kept in (`notify::book`).
pub async fn with_notices<T: Serialize + Send + 'static>(state: &AppState, work: impl FnOnce(&bagholder_book::Book) -> rusqlite::Result<T> + Send + 'static) -> Api<T> {
    let app = state.app.clone();
    Ok(Json(blocking(move || crate::notify::book(&app).and_then(|book| work(&book))).await??))
}

pub fn router(state: AppState) -> Router {
    let once = Router::new()
        .merge(model::routes().router)
        .merge(markets::routes().router)
        .merge(orders::routes().router)
        .merge(session::routes().router)
        .merge(notifications::routes().router)
        .merge(agents::routes().router)
        .merge(remote::routes().router)
        .route("/api/events/watch", post(stream::watch))
        // GitHub sends the person back here, in its own tab, once Bagholder's GitHub app is made
        .route(crate::remote::MADE, get(remote::made))
        .route("/api/events/resync", post(stream::resync))
        .layer(axum::middleware::from_fn(conditional))
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
        // what the person sends is theirs, however large: never cut at a size; what keeps
        // memory bounded is one import at a time (`http::model`, `import_slot`)
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn_with_state(state.clone(), gate))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(CatchPanicLayer::custom(|_: Box<dyn std::any::Any + Send>| ApiError::Internal.into_response()))
        .with_state(state)
}

/// A read answered "unchanged" (RFC 9110 §13.1.2): every answer to a GET carries
/// its version as an entity tag, and a request naming the tag of what it holds is
/// answered 304 with no body. The browser keeps what it was sent (`no-cache`:
/// kept, and asked again each time with its tag), so a read asked for again when
/// nothing changed moves no data. The sign-in window's frames are never kept.
async fn conditional(req: Request, next: Next) -> Response {
    let get = req.method() == Method::GET && !req.uri().path().starts_with("/api/login");
    let held = req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string);
    let res = next.run(req).await;
    if !get || res.status() != StatusCode::OK {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, ANSWER_LIMIT).await else {
        return ApiError::Internal.into_response();
    };
    let tag = format!("\"{:016x}\"", crate::views::version_of_bytes(&bytes));
    let value = HeaderValue::from_str(&tag).expect("hex in quotes is a header value");
    parts.headers.insert(header::ETAG, value.clone());
    parts.headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    let matches = held.is_some_and(|h| h.split(',').any(|t| {
        let t = t.trim();
        t == "*" || t.trim_start_matches("W/") == tag
    }));
    if matches {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, axum::body::Body::empty());
    }
    Response::from_parts(parts, axum::body::Body::from(bytes))
}

async fn not_found() -> ApiError {
    ApiError::NotFound("not found".into())
}

/// A refusal, said in one sentence: a page for a browser opening the page, the
/// shape every route answers for anything else.
fn refused(why: &admit::Refused, port: u16, page: bool) -> Response {
    let said = why.said(port);
    if page {
        let body = format!("{}<main><div class=dim style='font-size:13px'>Bagholder</div><h1>Bagholder can’t open here</h1><p>{}</p></main>", crate::agents::oauth::HEAD, crate::agents::oauth::escape(&said));
        let mut r = (why.status(), axum::response::Html(body)).into_response();
        r.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        return r;
    }
    (why.status(), Json(OkOr { ok: false, error: Some(said) })).into_response()
}

/// The gate: loopback only unless the server was bound elsewhere on purpose, the
/// Host header has to name 127.0.0.1 and the port, or one name the person listed
/// for a proxy on this machine (a page on another origin that resolves a name of
/// its own to this machine names itself there), and a write has to come from the
/// page itself.
async fn gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // the page's address from anywhere: let in on Access's sign-in alone (`remote`)
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_string();
    if remote::is_remote(&state.app, &host) {
        return remote::gate(state.app.clone(), req, next).await;
    }
    let port = *state.app.port.lock().unwrap_or_else(|e| e.into_inner());
    // a browser opening a page gets a page saying why; anything else, the routes' shape
    let page = req.method() == Method::GET && !req.uri().path().starts_with("/api/");
    match admitted(&state.app, &req, port) {
        Ok(()) => next.run(req).await,
        Err(why) => refused(&why, port, page),
    }
}

/// The request, held against who the page answers (`admit`).
fn admitted(app: &App, req: &Request, port: u16) -> Result<(), admit::Refused> {
    let write = match *req.method() {
        Method::GET | Method::HEAD => false,
        Method::POST => true,
        Method::OPTIONS => return Err(admit::Refused::Method(StatusCode::FORBIDDEN)),
        _ => return Err(admit::Refused::Method(StatusCode::NOT_IMPLEMENTED)),
    };
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let head = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_lowercase();
    let forwarded = req.headers().get_all("x-forwarded-for").iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(",");
    let host = head("host");
    let listed = app.hosts.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let agents_name = |h: &str| app.agents.ctx().is_some_and(|c| c.sites().iter().any(|s| h == s.host || h == s.received()));
    let proxy_sees_visitors = app.proxy_sees_visitors.load(std::sync::atomic::Ordering::SeqCst);
    let server = admit::Server { bind_host: &app.bind_host, port, listed: &listed, agents_name: &agents_name, gateway: if app.bind_host == "127.0.0.1" { None } else { admit::docker_gateway() }, proxy_sees_visitors };
    let asked = admit::Asked { write, host: &host, peer, forwarded: &forwarded, from_the_page: head("sec-fetch-site") == "same-origin" || !head("x-bagholder").is_empty(), old_http: req.version() <= axum::http::Version::HTTP_10 };
    let answered = admit::admit(&server, &asked);
    if answered.is_ok() && admit::relayed_and_answered(&server, &asked) {
        relayed_said_once(app);
    }
    answered
}

/// Said once a run in the log: a proxy named a Docker gateway as its visitor, so
/// over IPv6 Bagholder can't tell the local network from the internet. True when
/// this call said it.
pub(crate) fn relayed_said_once(app: &App) -> bool {
    if app.relayed_said.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    crate::app::log("bagholder: your proxy gets visits through Docker's relay and names Docker's gateway as the visitor, so over IPv6 Bagholder can't tell your network from the internet and answers both. Give Docker IPv6 (README, Docker, IPv6), then set BAGHOLDER_PROXY_SEES_VISITORS=1.");
    true
}

pub(crate) fn peer_ok(bind_host: &str, peer: Option<std::net::IpAddr>) -> bool {
    if bind_host != "127.0.0.1" {
        return true; // bound beyond loopback on purpose (a container); the peer is its bridge
    }
    match peer {
        Some(std::net::IpAddr::V4(ip)) => ip.is_loopback(),
        Some(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
        None => false,
    }
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
