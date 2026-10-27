//! Sign-in for MCP at `/mcp` (`docs/plans/agents-from-anywhere.md`, §2.1, §3),
//! as the MCP authorization specification (2026-07-28) has it: protected-resource
//! metadata (RFC 9728), authorization-server metadata (RFC 8414), Client ID
//! Metadata Documents first and Dynamic Client Registration (RFC 7591) beside
//! them, PKCE with S256 alone, tokens bound to `/mcp` (RFC 8707), and refresh
//! tokens rotated on every use (RFC 9700 §4.14.2).
//!
//! Nothing reachable here grants anything by itself. The public page shows who is
//! asking and a code, and waits; the person allows it on their own page, on the
//! main listener (`approve`). Only then does the agent's page get its code.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};

use super::listener::{refused, said};
use super::{fetch, secret, Ctx};
use bagholder_book::agents::{Client, NewRequest};
use bagholder_core::jiff::Timestamp;

fn unavailable(e: impl ToString) -> Response {
    said(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable", &e.to_string())
}

/// What runs on a blocking thread (the store, a fetch), its panic an answer.
async fn blocking(work: impl FnOnce() -> Response + Send + 'static) -> Response {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| said(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "Bagholder failed to answer."))
}

// ---- metadata ----

/// RFC 9728: the resource `/mcp`, signed in to at this address.
pub async fn protected_resource(State(ctx): State<Arc<Ctx>>) -> Response {
    axum::Json(json!({
        "resource": ctx.resource(),
        "authorization_servers": [ctx.base()],
        "bearer_methods_supported": ["header"],
        "resource_name": "Bagholder",
    }))
    .into_response()
}

/// RFC 8414.
pub async fn authorization_server(State(ctx): State<Arc<Ctx>>) -> Response {
    let base = ctx.base();
    axum::Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    }))
    .into_response()
}

/// The `WWW-Authenticate` that sends a client with no good token to sign in.
fn challenge(ctx: &Ctx, error: Option<&str>) -> Response {
    let mut r = said(StatusCode::UNAUTHORIZED, error.unwrap_or("unauthorized"), "Sign in to Bagholder first.");
    let mut v = format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"", ctx.base());
    if let Some(e) = error {
        v.push_str(&format!(", error=\"{e}\""));
    }
    if let Ok(h) = HeaderValue::from_str(&v) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, h);
    }
    r
}

/// An access token of a grant still standing, for this address's `/mcp`: the grant's id.
pub fn access(ctx: &Ctx, token: Option<&str>) -> Result<String, Response> {
    let Some(token) = token else { return Err(challenge(ctx, None)) };
    let store = ctx.store().map_err(unavailable)?;
    let Some(t) = store.token(&secret::hash(token)).map_err(unavailable)? else {
        super::listener::stranger(ctx, "with a wrong token");
        return Err(challenge(ctx, Some("invalid_token")));
    };
    let why = if t.refresh {
        Some("a refresh token sent as an access token")
    } else if t.expires_at <= ctx.now() {
        Some("an expired token")
    } else if t.grant.withdrawn_at.is_some() {
        Some("a token of a grant cut off")
    } else if t.grant.resource != ctx.resource() {
        Some("a token for another resource")
    } else {
        None
    };
    match why {
        Some(why) => {
            refused(ctx, &t.grant.id, why);
            Err(challenge(ctx, Some("invalid_token")))
        }
        None => Ok(t.grant.id),
    }
}

// ---- redirect URIs and clients ----

/// A redirect URI an agent may be sent back to: https, and never this machine
/// (brief 73, 3: a cloud agent never redirects here, and a local one uses stdio).
pub fn redirect_ok(uri: &str) -> Result<(), String> {
    let (host, _) = fetch::https_host(uri)?;
    if uri.contains('#') {
        return Err(format!("{uri} carries a fragment"));
    }
    let loopback = host == "localhost" || host.ends_with(".localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback() || matches!(ip, std::net::IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())));
    if loopback {
        return Err(format!("{uri} redirects to this machine, which a cloud agent never does"));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct Registration {
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

/// RFC 7591: a client registered with its exact redirect URIs. It is a public
/// client (no secret); never approved, it goes with its request.
pub async fn register(State(ctx): State<Arc<Ctx>>, body: axum::body::Bytes) -> Response {
    let reg: Registration = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return said(StatusCode::BAD_REQUEST, "invalid_client_metadata", &format!("Not a registration: {e}")),
    };
    if reg.redirect_uris.is_empty() {
        return said(StatusCode::BAD_REQUEST, "invalid_redirect_uri", "A client needs a redirect URI.");
    }
    if let Some(e) = reg.redirect_uris.iter().find_map(|u| redirect_ok(u).err()) {
        return said(StatusCode::BAD_REQUEST, "invalid_redirect_uri", &e);
    }
    let name: String = reg.client_name.unwrap_or_default().chars().filter(|c| !c.is_control()).take(80).collect();
    let name = if name.trim().is_empty() { "An agent".to_string() } else { name.trim().to_string() };
    blocking(move || {
        let id = secret::id("bhc_");
        let now = ctx.now();
        let c = Client { id: id.clone(), metadata: false, name: name.clone(), redirect_uris: reg.redirect_uris.clone(), made_at: now, approved: false };
        if let Err(e) = ctx.store().and_then(|s| s.keep_client(&c, super::UNAPPROVED_KEPT).map_err(|e| e.to_string())) {
            return unavailable(e);
        }
        (
            StatusCode::CREATED,
            axum::Json(json!({
                "client_id": id,
                "client_id_issued_at": now.as_second(),
                "client_name": name,
                "redirect_uris": reg.redirect_uris,
                "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
            })),
        )
            .into_response()
    })
    .await
}

/// Who is asking, as both pages show it (brief 73, 4).
struct Asking {
    client: Client,
    /// The host shown as who it is: the metadata document's, or the redirect's.
    who: String,
}

/// The client `client_id` names, and that `redirect_uri` is exactly one of its own.
fn asking(ctx: &Ctx, client_id: &str, redirect_uri: &str) -> Result<Asking, String> {
    redirect_ok(redirect_uri)?;
    let store = ctx.store()?;
    let client = if client_id.starts_with("https://") {
        let doc = fetch::metadata(client_id, ctx.outside.resolve.as_ref(), ctx.outside.get.as_ref())?;
        if doc.get("client_id").and_then(Value::as_str) != Some(client_id) {
            return Err(format!("the metadata document at {client_id} names another client_id"));
        }
        let uris: Vec<String> = doc.get("redirect_uris").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let name: String = doc.get("client_name").and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(80).collect();
        // kept only once the person allows it (`approve::answer`): until then it
        // lives in its sign-in, so a refused or unanswered one stores nothing
        Client { id: client_id.to_string(), metadata: true, name, redirect_uris: uris, made_at: ctx.now(), approved: false }
    } else {
        store.client(client_id).map_err(|e| e.to_string())?.filter(|c| !c.metadata).ok_or_else(|| format!("no client {client_id} is registered here"))?
    };
    // exactly, character for character: no prefix, no pattern
    if !client.redirect_uris.iter().any(|u| u == redirect_uri) {
        return Err(format!("{redirect_uri} is not a redirect URI of this client"));
    }
    let who = if client.metadata { fetch::https_host(client_id)?.0 } else { fetch::https_host(redirect_uri)?.0 };
    Ok(Asking { client, who })
}

#[derive(Deserialize, Default)]
pub struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
}

/// A page that says what is wrong, sent to no one: a request whose client or
/// redirect cannot be trusted is never redirected.
fn error_page(status: StatusCode, error: &str, why: &str) -> Response {
    let body = format!("<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Bagholder</title><body style='font:16px system-ui;margin:40px auto;max-width:520px;padding:0 16px'><h1 style='font-size:20px'>This sign-in can't go ahead</h1><p>{}</p><p style='opacity:.6'>{}</p>", escape(why), escape(error));
    let mut r = (status, Html(body)).into_response();
    page_headers(r.headers_mut());
    r
}

fn page_headers(h: &mut HeaderMap) {
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
}

/// Text for a page, escaped.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// `/oauth/authorize`: the public page. A waiting request is recorded, and the
/// page shows who is asking and the code, and waits for the person's answer on
/// their computer. It has nothing that grants anything.
pub async fn authorize(State(ctx): State<Arc<Ctx>>, Query(q): Query<AuthorizeQuery>) -> Response {
    blocking(move || {
        let (Some(client_id), Some(redirect_uri)) = (q.client_id.as_deref(), q.redirect_uri.as_deref()) else {
            return error_page(StatusCode::BAD_REQUEST, "invalid_request", "The agent did not say who it is, or where to go back to.");
        };
        let asking = match asking(&ctx, client_id, redirect_uri) {
            Ok(a) => a,
            Err(e) => return error_page(StatusCode::BAD_REQUEST, "invalid_client", &e),
        };
        if q.response_type.as_deref() != Some("code") {
            return error_page(StatusCode::BAD_REQUEST, "unsupported_response_type", "Only the authorization code flow is offered.");
        }
        let challenge = q.code_challenge.unwrap_or_default();
        if q.code_challenge_method.as_deref() != Some("S256") || !(43..=128).contains(&challenge.len()) {
            return error_page(StatusCode::BAD_REQUEST, "invalid_request", "PKCE with S256 is required.");
        }
        let resource = q.resource.unwrap_or_else(|| ctx.resource());
        if resource != ctx.resource() {
            return error_page(StatusCode::BAD_REQUEST, "invalid_target", &format!("This address signs in to {} alone.", ctx.resource()));
        }
        let now = ctx.now();
        let store = match ctx.store() {
            Ok(s) => s,
            Err(e) => return unavailable(e),
        };
        if let Err(e) = super::end_expired(&store, now) {
            return unavailable(e);
        }
        let (id, code) = (secret::request_id(), secret::code());
        let r = NewRequest { id: &id, code: &code, client_id: &asking.client.id, client_name: &asking.client.name, who: &asking.who, redirect_uri, state: q.state.as_deref(), challenge: &challenge, resource: &resource };
        match store.add_request(&r, now, super::PENDING_KEPT) {
            Ok(true) => {}
            Ok(false) => {
                let mut r = error_page(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", "Too many sign-ins are waiting on Bagholder. Try again once one is answered.");
                r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(60));
                return r;
            }
            Err(e) => return unavailable(e),
        }
        let mut r = Html(waiting_page(&asking, &code, &id)).into_response();
        page_headers(r.headers_mut());
        r
    })
    .await
}

fn waiting_page(a: &Asking, code: &str, id: &str) -> String {
    let name = if a.client.metadata {
        if a.client.name.is_empty() { String::new() } else { format!(" <span style='opacity:.7'>({})</span>", escape(&a.client.name)) }
    } else {
        format!(" <span style='opacity:.7'>(“{}”, name given by the agent, not checked)</span>", escape(&a.client.name))
    };
    #[expect(clippy::disallowed_methods, reason = "a string always serializes")]
    let id_json = serde_json::to_string(id).unwrap_or_default();
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Bagholder</title>\
<body style='font:16px system-ui;margin:40px auto;max-width:520px;padding:0 16px'>\
<h1 style='font-size:20px'>Approve this in Bagholder on your computer</h1>\
<p><b>{who}</b>{name} is asking to read your trade history.</p>\
<p>Its code is</p><p style='font:600 32px ui-monospace,monospace;letter-spacing:.2em'>{code}</p>\
<p id=s style='opacity:.7'>Waiting for your answer in Bagholder…</p>\
<script>(function(){{var id={id};function ask(){{fetch('/oauth/authorize/status?request='+id).then(function(r){{return r.json()}}).then(function(j){{if(j.redirect){{location.replace(j.redirect)}}else if(j.state==='waiting'){{setTimeout(ask,2000)}}else{{document.getElementById('s').textContent='This sign-in has ended. Start again from your agent.'}}}}).catch(function(){{setTimeout(ask,4000)}})}}ask()}})()</script>",
        who = escape(&a.who),
        name = name,
        code = escape(code),
        id = id_json,
    )
}

#[derive(Deserialize)]
pub struct StatusQuery {
    request: Option<String>,
}

/// A redirect back to the agent, with `params` and this address as the issuer.
fn back(redirect: &str, params: &[(&str, &str)], state: Option<&str>, iss: &str) -> String {
    let mut q: Vec<(&str, &str)> = params.to_vec();
    if let Some(s) = state {
        q.push(("state", s));
    }
    q.push(("iss", iss));
    #[expect(clippy::disallowed_methods, reason = "pairs of strings always encode")]
    let qs = serde_urlencoded::to_string(&q).unwrap_or_default();
    format!("{redirect}{}{qs}", if redirect.contains('?') { "&" } else { "?" })
}

/// Whether a request has ended by now, kept or not: one with no code handed after
/// `REQUEST_LIFE`, one whose code was handed after `CODE_LIFE`.
pub fn ended(r: &bagholder_book::agents::Request, now: Timestamp) -> bool {
    match r.code_at {
        None => r.made_at < now - super::REQUEST_LIFE,
        Some(at) => at < now - super::CODE_LIFE,
    }
}

/// `/oauth/authorize/status?request={id}`: the request this id names alone (brief
/// 73, 5): waiting, or the redirect back to the agent once the person answered. A
/// poll naming a request takes nothing from the allowance before sign-in, so the
/// waiting page never uses it up, and is answered at most once a `POLL_EVERY`; one
/// naming none takes a call, as any stranger's. An answer ends the request: a
/// denial once its page has read it, an allowed one's code is handed once.
pub async fn status(State(ctx): State<Arc<Ctx>>, Query(q): Query<StatusQuery>) -> Response {
    blocking(move || {
        let not = || super::listener::spend_before_sign_in(&ctx).err().unwrap_or_else(|| said(StatusCode::NOT_FOUND, "not_found", "No such sign-in."));
        // an id is 32 hex digits: anything else, the code among them, names nothing
        let Some(id) = q.request.filter(|i| i.len() == 32 && i.bytes().all(|b| b.is_ascii_hexdigit())) else { return not() };
        let now = ctx.now();
        let store = match ctx.store() {
            Ok(s) => s,
            Err(e) => return unavailable(e),
        };
        let r = match store.request(&id) {
            Ok(Some(r)) if !ended(&r, now) => r,
            Ok(_) => return not(),
            Err(e) => return unavailable(e),
        };
        if let Err(too_soon) = super::listener::poll(&ctx, &r.id, now) {
            return too_soon;
        }
        let iss = ctx.base();
        match r.answer {
            None => axum::Json(json!({ "state": "waiting" })).into_response(),
            Some(false) => match store.drop_request(&r.id) {
                Ok(()) => axum::Json(json!({ "state": "denied", "redirect": back(&r.redirect_uri, &[("error", "access_denied")], r.state.as_deref(), &iss) })).into_response(),
                Err(e) => unavailable(e),
            },
            Some(true) => {
                let code = secret::secret("bhcode_");
                match store.hand_code(&r.id, &secret::hash(&code), now) {
                    Ok(true) => axum::Json(json!({ "state": "allowed", "redirect": back(&r.redirect_uri, &[("code", &code)], r.state.as_deref(), &iss) })).into_response(),
                    Ok(false) => axum::Json(json!({ "state": "done" })).into_response(),
                    Err(e) => unavailable(e),
                }
            }
        }
    })
    .await
}

fn grant_error(error: &str, why: &str) -> Response {
    said(StatusCode::BAD_REQUEST, error, why)
}

/// The tokens a grant is given: an access token for an hour, a refresh token to
/// be used once.
fn tokens(ctx: &Ctx, store: &bagholder_book::agents::AgentsBook, grant: &str) -> Response {
    let now = ctx.now();
    let (access, refresh) = (secret::secret("bha_"), secret::secret("bhr_"));
    let made = store.atomically(|| {
        store.add_token(&secret::hash(&access), false, grant, now + super::ACCESS_LIFE)?;
        store.add_token(&secret::hash(&refresh), true, grant, now + super::REFRESH_LIFE)
    });
    match made {
        Ok(()) => axum::Json(json!({ "access_token": access, "token_type": "Bearer", "expires_in": super::ACCESS_LIFE.as_secs(), "refresh_token": refresh })).into_response(),
        Err(e) => unavailable(e),
    }
}

/// `/oauth/token`: a code exchanged once, with its verifier, for the redirect and
/// the resource it was asked with; or a refresh token used once.
pub async fn token(State(ctx): State<Arc<Ctx>>, body: axum::body::Bytes) -> Response {
    let form: std::collections::BTreeMap<String, String> = match serde_urlencoded::from_bytes(&body) {
        Ok(f) => f,
        Err(e) => return grant_error("invalid_request", &format!("Not a token request: {e}")),
    };
    blocking(move || {
        let get = |k: &str| form.get(k).map(String::as_str);
        let now = ctx.now();
        let store = match ctx.store() {
            Ok(s) => s,
            Err(e) => return unavailable(e),
        };
        match get("grant_type") {
            Some("authorization_code") => {
                let (Some(code), Some(verifier)) = (get("code"), get("code_verifier")) else { return grant_error("invalid_request", "A code and its verifier are needed.") };
                let r = match store.request_by_code(&secret::hash(code)) {
                    Ok(Some(r)) => r,
                    Ok(None) => return grant_error("invalid_grant", "No such code."),
                    Err(e) => return unavailable(e),
                };
                let fresh = r.code_at.is_some_and(|at| at + super::CODE_LIFE > now);
                let fits = r.answer == Some(true)
                    && fresh
                    && get("client_id") == Some(r.client_id.as_str())
                    && get("redirect_uri") == Some(r.redirect_uri.as_str())
                    && get("resource").is_none_or(|x| x == r.resource)
                    && r.resource == ctx.resource()
                    && secret::pkce_ok(verifier, &r.challenge);
                if !fits {
                    return grant_error("invalid_grant", "The code does not fit this request.");
                }
                match store.use_code(&r.id) {
                    Ok(true) => {}
                    Ok(false) => return grant_error("invalid_grant", "The code was used already."),
                    Err(e) => return unavailable(e),
                }
                let name = if r.client_name.is_empty() { r.who.clone() } else { r.client_name.clone() };
                let grant = secret::id("bhg_");
                if let Err(e) = store.add_grant(&grant, &r.client_id, &name, &r.who, &r.resource, now).and_then(|()| store.drop_request(&r.id)) {
                    return unavailable(e);
                }
                tokens(&ctx, &store, &grant)
            }
            Some("refresh_token") => {
                let Some(refresh) = get("refresh_token") else { return grant_error("invalid_request", "A refresh token is needed.") };
                let hash = secret::hash(refresh);
                let t = match store.token(&hash) {
                    Ok(Some(t)) if t.refresh => t,
                    Ok(_) => return grant_error("invalid_grant", "No such refresh token."),
                    Err(e) => return unavailable(e),
                };
                if t.grant.withdrawn_at.is_some() || t.expires_at <= now || get("client_id") != Some(t.grant.client_id.as_str()) || get("resource").is_some_and(|x| x != t.grant.resource) {
                    return grant_error("invalid_grant", "This refresh token does not stand.");
                }
                match store.use_refresh(&hash, now) {
                    Ok(true) => tokens(&ctx, &store, &t.grant.id),
                    Ok(false) => {
                        // used a second time: someone else holds it, and the grant ends
                        if let Err(e) = store.withdraw_grant(&t.grant.id, now) {
                            return unavailable(e);
                        }
                        refused(&ctx, &t.grant.id, "a refresh token used twice: the grant was ended");
                        grant_error("invalid_grant", "This refresh token was used before, so the grant has ended.")
                    }
                    Err(e) => unavailable(e),
                }
            }
            _ => grant_error("unsupported_grant_type", "Only authorization_code and refresh_token are offered."),
        }
    })
    .await
}
