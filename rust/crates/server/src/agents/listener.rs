//! The agents' listener (`docs/plans/agents-from-anywhere.md`, §1): its own
//! router on its own loopback port, sharing no route with the page's. Its gate
//! takes only the public name in `Host` (421 otherwise), only a loopback peer (the
//! tunnel's connector runs on this machine), trusts no forwarded header for
//! anything, sends no CORS header, and bounds everything reachable before sign-in
//! across every caller together.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use bagholder_core::jiff::Timestamp;
use tower_http::catch_panic::CatchPanicLayer;

use super::tools::Caller;
use super::{api, oauth, secret, Ctx};

/// A bucket computed when it is touched, never refilled on a clock: `tokens` as
/// of `at`, up to `burst`, gaining `per_minute` a minute.
#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    tokens: f64,
    at: Timestamp,
}

impl Bucket {
    /// Take one at `now`: the seconds to wait when there is none.
    pub fn take(slot: &mut Option<Bucket>, now: Timestamp, burst: u32, per_minute: u32) -> Result<(), u64> {
        let b = slot.get_or_insert(Bucket { tokens: burst as f64, at: now });
        let gained = (now.duration_since(b.at).as_secs_f64().max(0.0)) * per_minute as f64 / 60.0;
        b.tokens = (b.tokens + gained).min(burst as f64);
        b.at = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - b.tokens) * 60.0 / per_minute as f64).ceil().max(1.0) as u64)
        }
    }
}

/// The buckets: one before sign-in for every caller together, one per agent.
#[derive(Default)]
pub struct Limits {
    before: Option<Bucket>,
    agents: BTreeMap<String, Option<Bucket>>,
}

/// An answer of `status` with an error in words, as JSON.
pub fn said(status: StatusCode, error: &str, description: &str) -> Response {
    (status, axum::Json(serde_json::json!({ "error": error, "error_description": description }))).into_response()
}

fn too_many(retry: u64) -> Response {
    let mut r = said(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", "Too many requests: ask again later.");
    r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(retry));
    r
}

/// The complete route table (§1): each path and the methods it answers, nothing
/// else. The router below is held to it by `tests`.
#[cfg_attr(not(test), allow(dead_code))]
pub const ROUTES: &[(&str, &[Method])] = &[
    ("/.well-known/oauth-protected-resource", &[Method::GET]),
    ("/.well-known/oauth-authorization-server", &[Method::GET]),
    ("/oauth/register", &[Method::POST]),
    ("/oauth/authorize", &[Method::GET]),
    ("/oauth/authorize/status", &[Method::GET]),
    ("/oauth/token", &[Method::POST]),
    ("/mcp", &[Method::POST, Method::GET]),
    ("/mcp/key", &[Method::POST, Method::GET]),
    ("/api/agents/openapi.json", &[Method::GET]),
    ("/api/agents/trades", &[Method::GET]),
    ("/api/agents/trade", &[Method::GET]),
    ("/api/agents/positions", &[Method::GET]),
    ("/api/agents/lists", &[Method::GET]),
    ("/agents/check/{value}", &[Method::GET]),
];

/// The listener's routes, the complete table (§1), behind its gate.
pub fn router(ctx: Arc<Ctx>) -> Router {
    let before = Router::new()
        .route("/.well-known/oauth-protected-resource", get(oauth::protected_resource))
        .route("/.well-known/oauth-authorization-server", get(oauth::authorization_server))
        .route("/oauth/register", post(oauth::register))
        .route("/oauth/authorize", get(oauth::authorize))
        .route("/oauth/authorize/status", get(oauth::status))
        .route("/oauth/token", post(oauth::token))
        .route("/agents/check/{value}", get(check))
        .route("/api/agents/openapi.json", get(api::openapi))
        .layer(DefaultBodyLimit::max(super::BODY_LIMIT))
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), before_sign_in));
    let mcp_service = || {
        let reader: Arc<dyn super::tools::Reader> = Arc::new(Served(ctx.clone()));
        let config = rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
            // `Host` is held by this listener's own gate, to the public name exactly
            .disable_allowed_hosts()
            .with_json_response(true);
        rmcp::transport::streamable_http_server::StreamableHttpService::new(
            move || Ok(super::tools::Tools::new(reader.clone())),
            Arc::new(rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default()),
            config,
        )
    };
    // each check runs for its route's own methods alone: any other method is 405, asked of no one
    use axum::middleware::from_fn_with_state as check_with;
    use axum::routing::post_service;
    let signed_in = Router::new().route("/mcp", post_service(mcp_service()).get_service(mcp_service()).route_layer(check_with(ctx.clone(), oauth_bearer)));
    let with_key = Router::new().route("/mcp/key", post_service(mcp_service()).get_service(mcp_service()).route_layer(check_with(ctx.clone(), mcp_key)));
    let api = Router::new()
        .route("/api/agents/trades", get(api::trades).route_layer(check_with(ctx.clone(), api_key)))
        .route("/api/agents/trade", get(api::trade).route_layer(check_with(ctx.clone(), api_key)))
        .route("/api/agents/positions", get(api::positions).route_layer(check_with(ctx.clone(), api_key)))
        .route("/api/agents/lists", get(api::lists).route_layer(check_with(ctx.clone(), api_key)));
    Router::new()
        .merge(before)
        .merge(signed_in)
        .merge(with_key)
        .merge(api)
        .fallback(|| async { said(StatusCode::NOT_FOUND, "not_found", "No such address.") })
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), gate))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(CatchPanicLayer::custom(|_: Box<dyn std::any::Any + Send>| said(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "Bagholder failed to answer.")))
        .with_state(ctx)
}

/// Serve until told to stop; then let what is in hand finish, for at most `DRAIN`.
pub async fn serve(listener: tokio::net::TcpListener, ctx: Arc<Ctx>, mut stop: tokio::sync::watch::Receiver<bool>) {
    const DRAIN: Duration = Duration::from_secs(5);
    let mut stop2 = stop.clone();
    let stopped = async move {
        while !*stop.borrow() {
            if stop.changed().await.is_err() {
                break;
            }
        }
    };
    let service = router(ctx).into_make_service_with_connect_info::<SocketAddr>();
    tokio::select! {
        _ = axum::serve(listener, service).with_graceful_shutdown(stopped) => {}
        _ = async {
            while !*stop2.borrow() {
                if stop2.changed().await.is_err() {
                    break;
                }
            }
            tokio::time::sleep(DRAIN).await
        } => {}
    }
}

/// A refusal kept in the log: of a caller no grant or key names, under no agent.
pub fn refused(ctx: &Ctx, agent: &str, why: &str) {
    let r = bagholder_book::agents::Read { agent: agent.to_string(), at: ctx.now(), tool: None, arguments: None, refusal: Some(why.to_string()) };
    if let Err(e) = ctx.store().and_then(|s| s.log(&r).map_err(|e| e.to_string())) {
        crate::app::log(&format!("bagholder: an agent's refusal could not be logged: {e}"));
    }
}

/// The gate: the public name, a loopback peer, and no body where none is taken.
async fn gate(State(ctx): State<Arc<Ctx>>, req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let loopback = match peer {
        Some(std::net::IpAddr::V4(ip)) => ip.is_loopback(),
        Some(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v| v.is_loopback()),
        None => false,
    };
    if !loopback {
        return said(StatusCode::FORBIDDEN, "forbidden", "Only the tunnel on this computer reaches this address.");
    }
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_ascii_lowercase();
    let want = ctx.received();
    if want.is_empty() || host != want {
        let c = ctx.clone();
        let why = format!("a request for another host ({host})");
        let _ = tokio::task::spawn_blocking(move || refused(&c, "", &why)).await;
        return said(StatusCode::MISDIRECTED_REQUEST, "misdirected_request", "This address answers for its own name alone.");
    }
    // the methods the table names, and no other (HEAD and OPTIONS among them)
    if req.method() != Method::GET && req.method() != Method::POST {
        return said(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "This address answers GET and POST alone.");
    }
    let has_body = req.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.trim() != "0") || req.headers().contains_key(header::TRANSFER_ENCODING);
    if req.method() == Method::GET && has_body {
        return said(StatusCode::PAYLOAD_TOO_LARGE, "invalid_request", "A GET takes no body.");
    }
    next.run(req).await
}

/// Everything before sign-in, every caller together: at most
/// `BEFORE_SIGN_IN_PER_MINUTE` a minute.
async fn before_sign_in(State(ctx): State<Arc<Ctx>>, req: Request, next: Next) -> Response {
    let now = ctx.now();
    let taken = {
        let mut l = ctx.limits.lock().unwrap_or_else(|e| e.into_inner());
        Bucket::take(&mut l.before, now, super::BEFORE_SIGN_IN_PER_MINUTE, super::BEFORE_SIGN_IN_PER_MINUTE)
    };
    match taken {
        Ok(()) => next.run(req).await,
        Err(retry) => {
            let c = ctx.clone();
            let _ = tokio::task::spawn_blocking(move || refused(&c, "", "over the rate before sign-in")).await;
            too_many(retry)
        }
    }
}

/// An agent's read taken from its bucket.
fn rate(ctx: &Ctx, agent: &str) -> Result<(), u64> {
    let now = ctx.now();
    let mut l = ctx.limits.lock().unwrap_or_else(|e| e.into_inner());
    Bucket::take(l.agents.entry(agent.to_string()).or_default(), now, super::READS_BURST, super::READS_PER_MINUTE)
}

/// The bearer a request carries.
pub fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim().to_string()).filter(|t| !t.is_empty())
}

/// Who a checked bearer is, or the answer refusing it.
async fn admitted(ctx: Arc<Ctx>, req: Request, next: Next, check: fn(&Ctx, Option<&str>) -> Result<String, Response>) -> Response {
    let token = bearer(req.headers());
    let c = ctx.clone();
    let who = tokio::task::spawn_blocking(move || check(&c, token.as_deref())).await.unwrap_or_else(|_| Err(said(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "Bagholder failed to answer.")));
    let agent = match who {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(retry) = rate(&ctx, &agent) {
        let c = ctx.clone();
        let a = agent.clone();
        let _ = tokio::task::spawn_blocking(move || refused(&c, &a, "over the rate")).await;
        return too_many(retry);
    }
    let mut req = req;
    req.extensions_mut().insert(Caller { agent });
    next.run(req).await
}

/// `/mcp`: an access token of a grant, for this address's `/mcp`.
async fn oauth_bearer(State(ctx): State<Arc<Ctx>>, req: Request, next: Next) -> Response {
    admitted(ctx, req, next, oauth::access).await
}

/// `/mcp/key`: a key made for MCP. No sign-in is offered: a client given a key is
/// never sent into one.
async fn mcp_key(State(ctx): State<Arc<Ctx>>, req: Request, next: Next) -> Response {
    admitted(ctx, req, next, |c, t| key(c, t, "mcp")).await
}

/// The JSON API: a key made for it.
async fn api_key(State(ctx): State<Arc<Ctx>>, req: Request, next: Next) -> Response {
    admitted(ctx, req, next, |c, t| key(c, t, "api")).await
}

/// A key of `way`, not withdrawn: its id.
fn key(ctx: &Ctx, token: Option<&str>, way: &str) -> Result<String, Response> {
    let refuse = |why: &str| {
        refused(ctx, "", why);
        let mut r = said(StatusCode::UNAUTHORIZED, "invalid_token", "No key, or not a key Bagholder made for this address.");
        r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        r
    };
    let Some(token) = token else { return Err(refuse("no key")) };
    let store = ctx.store().map_err(|e| said(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable", &e))?;
    let hash = secret::hash(token);
    match store.key_by_hash(&hash).map_err(|e| said(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable", &e.to_string()))? {
        Some(k) if k.withdrawn_at.is_none() && k.way == way => Ok(k.id),
        Some(k) => {
            refused(ctx, &k.id, if k.withdrawn_at.is_some() { "a key cut off" } else { "a key for the other way in" });
            let mut r = said(StatusCode::UNAUTHORIZED, "invalid_token", "This key was cut off, or is for the other way in.");
            r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            Err(r)
        }
        None => Err(refuse("a wrong key")),
    }
}

/// The reachability check's one-time value, answered only while the check is
/// open (§4.4), noting whether it came the Funnel way.
async fn check(State(ctx): State<Arc<Ctx>>, Path(value): Path<String>, headers: HeaderMap) -> Response {
    let funnel = headers.get("tailscale-funnel-request").and_then(|v| v.to_str().ok()).is_some_and(|v| v.trim() == "?1");
    let mut open = ctx.check.lock().unwrap_or_else(|e| e.into_inner());
    match open.as_mut() {
        Some(c) if secret::same(c.value.as_bytes(), value.as_bytes()) => {
            c.seen = true;
            c.funnel |= funnel;
            (StatusCode::OK, c.value.clone()).into_response()
        }
        _ => said(StatusCode::NOT_FOUND, "not_found", "No such address."),
    }
}

/// The tools, reading the server this listener belongs to, each read logged for
/// the agent that made it.
struct Served(Arc<Ctx>);

impl super::tools::Reader for Served {
    fn server(&self) -> Result<crate::mcp::ReadOnly, String> {
        (self.0.read)()
    }

    fn reading(&self, caller: Option<&Caller>, tool: &str, args: &serde_json::Value) {
        let Some(c) = caller else { return };
        let r = bagholder_book::agents::Read { agent: c.agent.clone(), at: self.0.now(), tool: Some(tool.to_string()), arguments: Some(args.to_string()), refusal: None };
        if let Err(e) = self.0.store().and_then(|s| s.log(&r).map_err(|e| e.to_string())) {
            crate::app::log(&format!("bagholder: an agent's read could not be logged: {e}"));
        }
    }
}
