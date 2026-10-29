//! Sign-in for MCP at `/mcp` (`docs/plans/agents-from-anywhere.md`, §2.1, §3),
//! as the MCP authorization specification (2026-07-28) has it: protected-resource
//! metadata (RFC 9728), authorization-server metadata (RFC 8414), Client ID
//! Metadata Documents first and Dynamic Client Registration (RFC 7591) beside
//! them, PKCE with S256 alone, tokens bound to `/mcp` (RFC 8707), and refresh
//! tokens rotated on every use (RFC 9700 §4.14.2).
//!
//! Nothing reachable here grants anything by itself. The public page shows who is
//! asking and asks for the code Bagholder shows the person on their own page, on
//! the main listener (`approve`); typed right, the agent's page gets its code. The
//! code's bounds are what make that safe (`enter_code`; brief 81 §2).

use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};

use super::listener::{refused, said};
use super::{fetch, secret, Ctx, Site};
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
pub async fn protected_resource(Extension(site): Extension<Site>) -> Response {
    axum::Json(json!({
        "resource": site.resource(),
        "authorization_servers": [site.base()],
        "bearer_methods_supported": ["header"],
        "resource_name": "Bagholder",
    }))
    .into_response()
}

/// RFC 8414.
pub async fn authorization_server(Extension(site): Extension<Site>) -> Response {
    let base = site.base();
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
fn challenge(site: &Site, error: Option<&str>) -> Response {
    let mut r = said(StatusCode::UNAUTHORIZED, error.unwrap_or("unauthorized"), "Sign in to Bagholder first.");
    let mut v = format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"", site.base());
    if let Some(e) = error {
        v.push_str(&format!(", error=\"{e}\""));
    }
    if let Ok(h) = HeaderValue::from_str(&v) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, h);
    }
    r
}

/// An access token of a grant still standing, for this address's `/mcp`: the grant's id.
pub fn access(ctx: &Ctx, site: &Site, token: Option<&str>) -> Result<String, Response> {
    let Some(token) = token else { return Err(challenge(site, None)) };
    let store = ctx.store().map_err(unavailable)?;
    let Some(t) = store.token(&secret::hash(token)).map_err(unavailable)? else {
        super::listener::stranger(ctx, "with a wrong token");
        return Err(challenge(site, Some("invalid_token")));
    };
    // an access token run out is the agent's sign to refresh it (RFC 6750 §3.1),
    // routine every hour: refused, never logged as the agent's doing
    if !t.refresh && t.expires_at <= ctx.now() {
        return Err(challenge(site, Some("invalid_token")));
    }
    let why = if t.refresh {
        Some("a refresh token sent as an access token")
    } else if t.grant.withdrawn_at.is_some() {
        Some("a token of a grant cut off")
    } else if t.grant.resource != site.resource() {
        Some("a token for another resource")
    } else {
        None
    };
    match why {
        Some(why) => {
            refused(ctx, &t.grant.id, why);
            Err(challenge(site, Some("invalid_token")))
        }
        None => Ok(t.grant.id),
    }
}

// ---- redirect URIs and clients ----

/// A desktop app's loopback redirect (RFC 8252 §7.3; the MCP specification's
/// "All redirect URIs MUST be either localhost or use HTTPS"; brief 81 §1):
/// `http://localhost`, `http://127.0.0.1` or `http://[::1]`, written exactly so,
/// with any port, then a path. Every other spelling of this machine (a name under
/// `localhost`, another address, a number form, a trailing dot, a percent escape,
/// a backslash, a user part) is none. Its host, and the URI without its port, which
/// is what an exact match compares (the port may vary, §7.3).
pub fn loopback(uri: &str) -> Option<(&'static str, String)> {
    let rest = uri.strip_prefix("http://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if uri.contains('#') || uri.contains('\\') || authority.contains('@') || authority.contains('%') {
        return None;
    }
    let (host, port) = [("[::1]", "[::1]"), ("localhost", "localhost"), ("127.0.0.1", "127.0.0.1")].into_iter().find_map(|(name, h)| authority.strip_prefix(name).map(|p| (h, p)))?;
    match port.strip_prefix(':') {
        None if port.is_empty() => {}
        Some(p) if (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n > 0) => {}
        _ => return None,
    }
    Some((host, format!("http://{host}{tail}")))
}

/// Whether `asked` is the registered `uri`: exactly, character for character, or,
/// both loopback, the same but for the port (RFC 8252 §7.3).
pub fn same_redirect(uri: &str, asked: &str) -> bool {
    uri == asked || matches!((loopback(uri), loopback(asked)), (Some((_, a)), Some((_, b))) if a == b)
}

/// The host a redirect sends the person back to: who a registered agent is.
pub fn redirect_shown(uri: &str) -> String {
    loopback(uri).map(|(h, _)| h.to_string()).or_else(|| fetch::https_host(uri).ok().map(|h| h.0)).unwrap_or_else(|| uri.to_string())
}

/// A redirect URI an agent may be sent back to: a desktop app's loopback
/// (`loopback`), or https to a public name or address. That host is a DNS name of
/// letters, digits and hyphens with a dot and a last label a browser does not read
/// as a number (the WHATWG URL standard's "ends in a number": all digits, or `0x`
/// followed by hex digits or by nothing; `fetch::dns_name`), or a public IP address
/// written as it is canonically: every other spelling (a number, `127.1`, `0x7f.1`,
/// `127.0x`, a trailing dot, a percent escape, a backslash a browser ends the host
/// at) is refused, as a browser may read it as this machine.
pub fn redirect_ok(uri: &str) -> Result<(), String> {
    if loopback(uri).is_some() {
        return Ok(());
    }
    let (host, _) = fetch::https_host(uri)?;
    if uri.contains('#') {
        return Err(format!("{uri} carries a fragment"));
    }
    let refused = || Err(format!("{uri} redirects to what is not a public name or address, which a cloud agent never does"));
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return if ip.to_string() == host && fetch::public(ip) { Ok(()) } else { refused() };
    }
    if fetch::dns_name(&host) {
        Ok(())
    } else {
        refused()
    }
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

/// Who a client is, shown, whose redirect is this computer's loopback.
pub const LOCAL_APP: &str = "An app on your computer";

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
        let doc = fetch::metadata(client_id, &ctx.outside.resolve, ctx.outside.get.as_ref())?;
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
    // exactly, character for character, but a loopback's port: no prefix, no pattern
    if !client.redirect_uris.iter().any(|u| same_redirect(u, redirect_uri)) {
        return Err(format!("{redirect_uri} is not a redirect URI of this client"));
    }
    // an app on the person's own computer names itself, unchecked: any app can
    // claim any name (the specification's "Localhost Redirect URI Risks")
    let who = if client.metadata {
        fetch::https_host(client_id)?.0
    } else if loopback(redirect_uri).is_some() {
        LOCAL_APP.to_string()
    } else {
        redirect_shown(redirect_uri)
    };
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
    // the person is told what to do, in plain words; why is for whoever reads the
    // log, and the code for the agent's developer, in a header the page never shows
    crate::app::log(&format!("bagholder: a sign-in was refused ({error}): {why}"));
    let said = if status == StatusCode::TOO_MANY_REQUESTS { "Too many sign-ins are waiting on Bagholder. Try again once one is answered." } else { "This sign-in link doesn’t work. Start again from your agent." };
    let body = format!("{HEAD}<main><div class=dim style='font-size:13px'>Bagholder</div><h1>This sign-in can’t go ahead</h1><p>{said}</p></main>");
    let mut r = (status, Html(body)).into_response();
    if let Ok(v) = HeaderValue::from_str(error) {
        r.headers_mut().insert("x-refused", v);
    }
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
/// page asks for the code Bagholder shows the person, saying who wants to read
/// (brief 81 §2).
pub async fn authorize(State(ctx): State<Arc<Ctx>>, Extension(site): Extension<Site>, Query(q): Query<AuthorizeQuery>) -> Response {
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
        let resource = q.resource.unwrap_or_else(|| site.resource());
        if resource != site.resource() {
            return error_page(StatusCode::BAD_REQUEST, "invalid_target", &format!("This address signs in to {} alone.", site.resource()));
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
        let mut r = Html(waiting_page(&asking, &id)).into_response();
        page_headers(r.headers_mut());
        r
    })
    .await
}

/// The head both public pages share: Bagholder's colours, light or dark as the
/// person's device is, and its card.
const HEAD: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Bagholder</title><style>:root{color-scheme:light dark;--bg:#eef0f5;--card:#fafafc;--ink:#1b1d27;--dim:rgba(27,29,39,.72);--line:rgba(27,29,39,.12);--accent:#6b5cd6;--on:#eef0f5;--err:#c0505f}\
@media (prefers-color-scheme:dark){:root{--bg:#101220;--card:#161826;--ink:#e9e9ed;--dim:rgba(233,233,237,.68);--line:rgba(233,233,237,.14);--accent:#9184d9;--on:#161826;--err:#e0778a}}\
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.5 Inter,system-ui,-apple-system,Segoe UI,sans-serif}\
main{max-width:440px;margin:48px auto;padding:28px;background:var(--card);border:1px solid var(--line);border-radius:12px}\
.dim{color:var(--dim)}h1{font-size:20px;margin:6px 0 10px}form{margin-top:18px}\
input{box-sizing:border-box;width:100%;padding:12px 14px;border-radius:8px;border:1px solid var(--line);background:var(--bg);color:var(--ink);font:600 24px ui-monospace,SFMono-Regular,Menlo,monospace;letter-spacing:.25em}\
button{margin-top:14px;width:100%;padding:12px;border:0;border-radius:8px;background:var(--accent);color:var(--on);font:600 15px Inter,system-ui,sans-serif;cursor:pointer}\
button:disabled{opacity:.5;cursor:default}#s{min-height:24px;margin:12px 0 0;font-size:14px}.err{color:var(--err)}\
</style>";

fn waiting_page(a: &Asking, id: &str) -> String {
    // a name the agent gave itself is said to be so: any app can claim any name
    let name = if a.client.metadata {
        if a.client.name.is_empty() { String::new() } else { format!(" <span class=dim>({})</span>", escape(&a.client.name)) }
    } else {
        format!(" <span class=dim>(calls itself “{}”)</span>", escape(&a.client.name))
    };
    #[expect(clippy::disallowed_methods, reason = "a string always serializes")]
    let id_json = serde_json::to_string(id).unwrap_or_default();
    format!(
        "{head}<main><div class=dim style='font-size:13px'>Bagholder</div>\
<h1 id=t>Enter the code shown in Bagholder</h1>\
<p><b>{who}</b>{name} wants to read {read}.</p>\
<form id=f><input id=c aria-labelledby=t inputmode=numeric autocomplete=one-time-code maxlength=7 placeholder='000 000' autofocus>\
<button id=b>Allow</button></form><p id=s class=dim></p></main>\
<script>(function(){{var id={id},s=document.getElementById('s'),b=document.getElementById('b'),c=document.getElementById('c');\
function say(t,e){{s.textContent=t;s.className=e?'err':'dim'}}\
function over(t){{say(t,true);b.disabled=true;c.disabled=true}}\
function ask(){{fetch('/oauth/authorize/status?request='+id).then(function(r){{return r.json()}}).then(function(j){{if(j.redirect){{location.replace(j.redirect)}}else if(j.state==='waiting'){{setTimeout(ask,2000)}}else{{over('This sign-in has ended. Start again from your agent.')}}}}).catch(function(){{setTimeout(ask,4000)}})}}\
document.getElementById('f').addEventListener('submit',function(e){{e.preventDefault();var v=c.value.replace(/\\D/g,'');if(v.length!==6){{say('The code is six digits.',true);return}}b.disabled=true;say('Checking…');\
fetch('/oauth/authorize/code',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{request:id,code:v}})}}).then(function(r){{return r.json()}}).then(function(j){{\
if(j.state==='allowed'){{say('Allowed. Taking you back to your agent…');ask()}}\
else if(j.state==='wrong'){{b.disabled=false;c.value='';say(j.left===1?'That code doesn\\u2019t match. One try left.':'That code doesn\\u2019t match. '+j.left+' tries left.',true)}}\
else if(j.state==='paused'){{over('Sign-in is paused: the code was entered wrong too many times. Resume it in Bagholder, then start again from your agent.')}}\
else{{over('This sign-in has ended. Start again from your agent.')}}}}).catch(function(){{b.disabled=false;say('Bagholder didn\\u2019t answer. Try again.',true)}})}});\
ask()}})()</script>",
        head = HEAD,
        read = super::tools::READ,
        who = escape(&a.who),
        name = name,
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
pub async fn status(State(ctx): State<Arc<Ctx>>, Extension(site): Extension<Site>, Query(q): Query<StatusQuery>) -> Response {
    blocking(move || {
        let not = || super::listener::spend_before_sign_in(&ctx).err().unwrap_or_else(|| said(StatusCode::NOT_FOUND, "not_found", "No such sign-in."));
        // an id is 32 hex digits: anything else, the code among them, names nothing
        let Some(id) = q.request.filter(|i| i.len() == 32 && i.bytes().all(|b| b.is_ascii_hexdigit())) else { return not() };
        let now = ctx.now();
        if let Err(too_soon) = super::listener::too_soon(&ctx, &id, now) {
            return too_soon;
        }
        let store = match ctx.store() {
            Ok(s) => s,
            Err(e) => return unavailable(e),
        };
        let r = match store.request(&id) {
            Ok(Some(r)) if !ended(&r, now) => r,
            Ok(_) => return not(),
            Err(e) => return unavailable(e),
        };
        super::listener::polled(&ctx, &r.id, now);
        let iss = site.base();
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

#[derive(Deserialize)]
pub struct Entered {
    request: Option<String>,
    code: Option<String>,
}

/// `/oauth/authorize/code`: the code the person typed on the sign-in page (brief
/// 81 §2), counted against the allowance before sign-in as every call here is.
/// The code is shown only in the person's own panel, on the page's listener, which
/// no tunnel reaches. Typed right, the request is allowed, and the page's next poll
/// takes the agent back with its code. Its bounds: a request ends after
/// `CODE_TRIES` wrong entries; after `WRONG_CODES_PAUSE` wrong entries in all, on
/// every request, no code is taken until the person resumes sign-in in their
/// panel. A stranger who reaches the address therefore gets at most
/// `WRONG_CODES_PAUSE` guesses at six digits before the person sees it: 5 in a
/// million. A code also ends with its request, `REQUEST_LIFE` after it was shown.
pub async fn enter_code(State(ctx): State<Arc<Ctx>>, body: axum::body::Bytes) -> Response {
    blocking(move || {
        let state = |s: &str| axum::Json(json!({ "state": s })).into_response();
        let entered: Entered = serde_json::from_slice(&body).unwrap_or(Entered { request: None, code: None });
        let Some(id) = entered.request.filter(|i| i.len() == 32 && i.bytes().all(|b| b.is_ascii_hexdigit())) else { return state("ended") };
        let code = entered.code.unwrap_or_default();
        let now = ctx.now();
        let store = match ctx.store() {
            Ok(s) => s,
            Err(e) => return unavailable(e),
        };
        // the whole entry in one transaction, which waits for any other's: entries
        // sent together are taken one at a time, each seeing the counts the one
        // before it left, so no more are compared than the pause allows
        let refused = |e: String| bagholder_book::BookError::Refused(e);
        let entry = store.atomically(|| {
            if super::wrong_codes(&store).map_err(refused)? >= super::WRONG_CODES_PAUSE {
                return Ok(Entry::Paused);
            }
            let r = match store.request(&id)? {
                Some(r) if r.answer.is_none() && !ended(&r, now) => r,
                _ => return Ok(Entry::Ended),
            };
            if secret::same(code.as_bytes(), r.code.as_bytes()) {
                super::approve::answer(&store, &id, true, now).map_err(refused)?;
                return Ok(Entry::Allowed);
            }
            let wrong = super::wrong_codes(&store).map_err(refused)? + 1;
            store.set_setting(super::WRONG_CODES, Some(&wrong.to_string()), now)?;
            let tries = store.add_wrong_code(&id)?;
            let paused = wrong >= super::WRONG_CODES_PAUSE;
            if tries >= u32::from(super::CODE_TRIES) {
                super::approve::answer(&store, &id, false, now).map_err(refused)?;
                return Ok(if paused { Entry::Paused } else { Entry::Ended });
            }
            Ok(if paused { Entry::Paused } else { Entry::Wrong(u32::from(super::CODE_TRIES) - tries) })
        });
        match entry {
            Ok(Entry::Allowed) => state("allowed"),
            Ok(Entry::Ended) => state("ended"),
            Ok(Entry::Paused) => state("paused"),
            Ok(Entry::Wrong(left)) => axum::Json(json!({ "state": "wrong", "left": left })).into_response(),
            Err(e) => unavailable(e),
        }
    })
    .await
}

/// What a typed code came to.
enum Entry {
    Allowed,
    /// Wrong, with the entries its request has left.
    Wrong(u32),
    Ended,
    Paused,
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
pub async fn token(State(ctx): State<Arc<Ctx>>, Extension(site): Extension<Site>, body: axum::body::Bytes) -> Response {
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
                    && r.resource == site.resource()
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
