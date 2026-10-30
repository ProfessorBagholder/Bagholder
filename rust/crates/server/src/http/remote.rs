//! The page from anywhere, as the page sets it up and as its address is let in
//! (`crate::remote`): the person's Access token kept, the tunnel, its name and the
//! Access application made, the tunnel started and stopped, the address checked
//! from outside, and all of it removed. A request at the address's name is let in
//! by `gate` alone, on Access's signed token for the email the person named.

use std::sync::Arc;

use axum::extract::{ConnectInfo, Request};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use bagholder_core::jiff::Timestamp;
use bagholder_diff_derive::Diff;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{api_routes, blocking, Api, AppState, OkOr, Required, Routed};
use crate::agents::{self, tunnel, tunnel::Provider};
use crate::app::App;
use crate::remote::{self, Refused};
use axum::extract::State;

pub fn routes() -> Routed {
    api_routes! {
        post "/api/remote/token" => token;
        post "/api/remote/team" => team;
        post "/api/remote/github" => github;
        post "/api/remote/github/login" => github_login;
        post "/api/remote/set-up" => set_up;
        post "/api/remote/switch" => switch;
        post "/api/remote/check" => check;
        post "/api/remote/remove" => remove;
    }
}

/// The page from anywhere, as the page's status carries it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    /// The address, once set up: `https://{name}`.
    pub url: String,
    /// The one email let in.
    pub email: String,
    /// Whether the person gave Bagholder a token for Access that Cloudflare took.
    pub token: bool,
    /// Cloudflare's page for that token, filled in.
    pub token_page: String,
    /// The token found the account has no Zero Trust team yet, which only
    /// Cloudflare's dashboard makes (`team_page`).
    pub no_team: bool,
    pub team_page: String,
    /// The person's API tokens at Cloudflare, where the token made for Bagholder
    /// is deleted once the address is removed.
    pub tokens_page: String,
    /// Bagholder's own GitHub sign-in: its name once Cloudflare holds it as a login
    /// method, else empty.
    pub github: String,
    /// Its manifest waits on the person in GitHub's tab.
    pub making_github: bool,
    /// Why Cloudflare didn't take the GitHub app GitHub made, until it does.
    pub github_said: String,
    /// The GitHub app's own page, where only the person can delete it.
    pub github_page: String,
    /// The tunnel runs with Bagholder.
    pub on: bool,
    pub running: bool,
    /// What the check from outside found: none while it is still trying, since the
    /// tunnel started.
    pub ready: Option<bool>,
    /// What the check's last try found, while it keeps trying.
    pub trying: String,
    /// Why it isn't reachable, or how the tunnel ended, in words.
    pub said: String,
    /// The address an earlier Bagholder set-up's tunnel, connected nowhere, still
    /// holds: offered to take over.
    pub held: Option<RemoteHeld>,
}

/// An address an earlier set-up's tunnel holds, and when that tunnel was last
/// connected (empty where it never was).
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHeld {
    pub host: String,
    pub last_connected: String,
}

pub fn status(app: &Arc<App>) -> RemoteStatus {
    let kept = app.remote.kept(&app.home);
    let st = {
        let mut t = app.remote.tunnel.lock().unwrap_or_else(|e| e.into_inner());
        t.reap();
        t.state()
    };
    let ready = app.remote.ready();
    let running = st.running.is_some();
    RemoteStatus {
        url: if kept.set_up() { format!("https://{}", kept.host) } else { String::new() },
        email: kept.email.clone(),
        token: remote::token(&app.home).is_some(),
        token_page: remote::TOKEN_PAGE.to_string(),
        no_team: app.remote.team().is_some_and(|t| t.is_empty()),
        team_page: remote::TEAM_PAGE.to_string(),
        tokens_page: remote::TOKENS_PAGE.to_string(),
        github: kept.github.as_ref().filter(|g| !g.login.is_empty()).map(|g| g.name.clone()).unwrap_or_default(),
        making_github: app.remote.making_github(Timestamp::now()),
        github_said: app.remote.github_said(),
        github_page: kept.github.as_ref().map(|g| format!("https://github.com/settings/apps/{}/advanced", g.slug)).unwrap_or_default(),
        on: kept.on,
        running,
        ready: if running { ready.as_ref().map(Result::is_ok) } else { None },
        trying: if running && ready.is_none() { app.remote.last_try() } else { String::new() },
        said: match (&ready, running) {
            (Some(Err(e)), true) => e.clone(),
            (_, false) if kept.on => st.exited.unwrap_or_default(),
            _ => String::new(),
        },
        held: app.remote.held().map(|(host, last_connected)| RemoteHeld { host, last_connected }),
    }
}

// ---- the gate at the address's name ----

/// Whether a request names the address's host: it is let in by `gate` here alone,
/// never by the page's own gate, whatever else is listed.
pub fn is_remote(app: &App, host: &str) -> bool {
    let kept = app.remote.kept(&app.home);
    !kept.host.is_empty() && host.eq_ignore_ascii_case(&kept.host)
}

/// A request at the address's name: from the tunnel on this computer, a `GET`,
/// `HEAD` or `POST`, carrying Access's token for this application and the email
/// the person named; a write, from the page itself. Anything else gets one plain
/// sentence, and nothing of the page.
pub async fn gate(app: Arc<App>, req: Request, next: Next) -> Response {
    // the request as the log names it: its method, path, peer and the program that
    // sent it (its User-Agent, cut short), never its other headers
    let peer = req.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|c| c.0.ip());
    let agent: String = req.headers().get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("no user agent").chars().take(80).collect();
    let asked = format!("{} {} from {}, {agent:?}", req.method(), req.uri().path(), peer.map_or("an unknown peer".to_string(), |p| p.to_string()));
    let refused = |why: &str| refused(&asked, why);
    let write = match *req.method() {
        Method::GET | Method::HEAD => false,
        Method::POST => true,
        _ => return refused("not a method the page takes"),
    };
    if !super::peer_ok(&app.bind_host, peer) {
        return refused("not from this computer");
    }
    let kept = app.remote.kept(&app.home);
    if !kept.set_up() {
        return refused("the address is not set up");
    }
    // read before any wait: the request is not shared across one
    let (jwt, from_the_page) = {
        let head = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_string();
        (head(remote::JWT_HEADER), head("sec-fetch-site").eq_ignore_ascii_case("same-origin") || !head("x-bagholder").is_empty())
    };
    let now = Timestamp::now();
    let mut kept = kept;
    let mut verdict = app.remote.check(&jwt, &kept, now);
    if verdict == Err(Refused::OtherTeam) {
        // the team renamed at Cloudflare: its address asked again, at most once a minute
        let a = app.clone();
        if let Ok(true) = tokio::task::spawn_blocking(move || ask_team_again(&a, now)).await {
            kept = app.remote.kept(&app.home);
            verdict = app.remote.check(&jwt, &kept, now);
        }
    }
    if verdict == Err(Refused::UnknownKey) {
        // a key not seen yet: the team's keys asked again, at most once a minute
        let (a, team) = (app.clone(), kept.team.clone());
        let fetched = tokio::task::spawn_blocking(move || a.remote.fetch_keys(&team, now)).await;
        if let Ok(Err(e)) = &fetched {
            crate::app::log(&format!("bagholder: Access’s keys could not be read: {e}"));
        }
        verdict = app.remote.check(&jwt, &kept, now);
    }
    if let Err(why) = verdict {
        return refused(&format!("{why:?}"));
    }
    if write && !from_the_page {
        return refused("a write not from the page");
    }
    next.run(req).await
}

/// The refusal at the address's name: said in the log with the request it was,
/// one sentence to the person.
fn refused(asked: &str, why: &str) -> Response {
    crate::app::log(&format!("bagholder: a request at the page’s address was refused ({asked}): {why}"));
    let body = format!("{}<main><div class=dim style='font-size:13px'>Bagholder</div><h1>This page can’t open</h1><p>Only the person who set up this Bagholder can open it, once Cloudflare has signed them in. Open the address again.</p></main>", agents::oauth::HEAD);
    let mut r = (StatusCode::FORBIDDEN, Html(body)).into_response();
    r.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    r
}

// ---- setting it up ----

#[derive(Deserialize, TS)]
pub struct TokenAsked {
    pub token: String,
}

/// The token the person made on Cloudflare's page, kept once Cloudflare takes it
/// and it can read Access; then whether the account has a Zero Trust team to put
/// the application in (`look_for_team`).
async fn token(State(state): State<AppState>, Required(asked): Required<TokenAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let t = asked.token.trim().to_string();
        if t.is_empty() || t.len() > 200 || !t.bytes().all(|c| c.is_ascii_graphic()) {
            return Err("That isn’t a Cloudflare token: copy the whole token Cloudflare shows after Create Token.".into());
        }
        remote::token_works(&t)?;
        let access = agents::cloudflare::Files::new(&app.home).access().ok_or("Sign in to Cloudflare first.")?;
        let team = remote::team(&t, &access.account).map_err(|e| format!("{e} Make the token again from Bagholder’s link, so it can read Access."))?;
        remote::keep_token(&app.home, &t)?;
        app.remote.set_team(Some(team.unwrap_or_default()));
        find_github(&app, &t, &access);
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// The account asked again for its Zero Trust team, with the token kept: after the
/// person made one on Cloudflare's dashboard.
async fn team(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let t = remote::token(&app.home).ok_or("Give Bagholder its Cloudflare token first.")?;
        let access = agents::cloudflare::Files::new(&app.home).access().ok_or("Sign in to Cloudflare first.")?;
        app.remote.set_team(Some(remote::team(&t, &access.account)?.unwrap_or_default()));
        find_github(&app, &t, &access);
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// The Cloudflare account Bagholder's sign-in is for: the page's own copy, else the agents'.
fn account(app: &App) -> Option<String> {
    remote::cloudflare(&app.home).access().or_else(|| agents::cloudflare::Files::new(&app.home).access()).map(|a| a.account)
}

/// The team's address asked of Cloudflare again (at most once a minute), and kept
/// when it changed: true then. A team renamed at Cloudflare signs its passes, and
/// sends its sign-in, from its new address.
fn ask_team_again(app: &Arc<App>, now: Timestamp) -> bool {
    if !app.remote.may_ask_team(now) {
        return false;
    }
    let (Some(token), Some(account)) = (remote::token(&app.home), account(app)) else { return false };
    let mut kept = app.remote.kept(&app.home);
    match remote::team(&token, &account) {
        Ok(Some(team)) if team != kept.team => {
            kept.team = team;
            if let Err(e) = app.remote.keep(&app.home, kept) {
                crate::app::log(&format!("bagholder: the team’s new address could not be kept: {e}"));
                return false;
            }
            true
        }
        Ok(_) => false,
        Err(e) => {
            crate::app::log(&format!("bagholder: the team’s address could not be asked: {e}"));
            false
        }
    }
}

#[derive(Deserialize, TS)]
pub struct GitHubAsked {
    /// The page's own address (`location.origin`): where GitHub sends the person
    /// back once the app is made.
    pub origin: String,
}

/// What the page posts to GitHub, in a tab of its own: the manifest of Bagholder's
/// GitHub app, to `action`.
#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitHubManifest {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    pub action: String,
    pub manifest: String,
}

/// Whether `origin` is the page's own address on this Bagholder: 127.0.0.1 and its
/// port, or a name listed for a proxy in front, with no path.
fn page_origin(app: &App, origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")) else { return false };
    let port = *app.port.lock().unwrap_or_else(|e| e.into_inner());
    let host = rest.to_ascii_lowercase();
    !host.contains('/') && (host == format!("127.0.0.1:{port}") || app.hosts.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|h| *h == host))
}

/// The manifest of Bagholder's GitHub app, for the page to post to GitHub: named for
/// the domain, signing in through the team's own callback, GitHub sending the
/// person back to the page they made it from.
async fn github(State(state): State<AppState>, Required(asked): Required<GitHubAsked>) -> Api<GitHubManifest> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<GitHubManifest, String> {
        let origin = asked.origin.trim().trim_end_matches('/').to_string();
        if !page_origin(&app, &origin) {
            return Err("This page isn’t one Bagholder answers at.".into());
        }
        let token = remote::token(&app.home).ok_or("Give Bagholder its Cloudflare token first.")?;
        let access = agents::cloudflare::Files::new(&app.home).access().ok_or("Sign in to Cloudflare first.")?;
        let team = remote::team(&token, &access.account)?.ok_or_else(|| format!("Your Cloudflare account has no Zero Trust team yet: make one at {}.", remote::TEAM_PAGE))?;
        let domain = super::agents::zone_name(&access)?;
        let mut kept = app.remote.kept(&app.home);
        kept.team = team.clone();
        app.remote.keep(&app.home, kept)?;
        app.remote.set_github_said(String::new());
        let state = app.remote.manifest_out(Timestamp::now());
        app.events.signal();
        Ok(GitHubManifest { ok: true, error: None, action: format!("https://github.com/settings/apps/new?state={state}"), manifest: remote::manifest(&team, &domain, &origin).to_string() })
    })
    .await?;
    Ok(Json(done.unwrap_or_else(|e| GitHubManifest { ok: false, error: Some(e), ..GitHubManifest::default() })))
}

/// Bagholder's GitHub login made by an earlier set-up for this domain, taken up when
/// Cloudflare still has it: the GitHub step is done already. Not found, or not to be
/// asked, it is made as new.
fn find_github(app: &Arc<App>, token: &str, access: &agents::cloudflare::Access) {
    let mut kept = app.remote.kept(&app.home);
    if kept.github.as_ref().is_some_and(|g| !g.login.is_empty()) {
        return;
    }
    let name = match super::agents::zone_name(access) {
        Ok(domain) => remote::github_name(&domain),
        Err(_) => return,
    };
    match remote::find_login(token, &access.account, &name) {
        Ok(Some(login)) => {
            kept.github = Some(remote::GitHubLogin { slug: remote::slug_of(&name), name, client_id: String::new(), client_secret: None, login });
            if let Err(e) = app.remote.keep(&app.home, kept) {
                crate::app::log(&format!("bagholder: Bagholder’s GitHub sign-in could not be kept: {e}"));
            }
        }
        Ok(None) => {}
        Err(e) => crate::app::log(&format!("bagholder: Cloudflare’s login methods could not be read: {e}")),
    }
}

/// Bagholder's GitHub app added to Cloudflare as its login method, with the secret
/// kept until Cloudflare holds it; the secret dropped once it does.
fn add_login(app: &Arc<App>) -> Result<(), String> {
    let mut kept = app.remote.kept(&app.home);
    let mut g = kept.github.clone().ok_or("Make Bagholder’s sign-in on GitHub first.")?;
    if !g.login.is_empty() {
        return Ok(());
    }
    let secret = g.client_secret.clone().ok_or("Bagholder no longer has its GitHub app’s secret: delete the app on GitHub and make it again.")?;
    let token = remote::token(&app.home).ok_or("Give Bagholder its Cloudflare token first.")?;
    let account = account(app).ok_or("Sign in to Cloudflare first.")?;
    let added = remote::make_login(&token, &account, &g.name, &g.client_id, &secret).map_err(|e| format!("Cloudflare didn’t add Bagholder’s GitHub sign-in: {e} Make a new token from Bagholder’s link, so it can add login methods, then try again."));
    let login = match added {
        Ok(l) => l,
        Err(e) => {
            app.remote.set_github_said(e.clone());
            return Err(e);
        }
    };
    g.login = login;
    g.client_secret = None;
    kept.github = Some(g);
    app.remote.keep(&app.home, kept)?;
    app.remote.set_github_said(String::new());
    Ok(())
}

/// Bagholder's GitHub app added to Cloudflare again, after Cloudflare refused it.
async fn github_login(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || {
        let r = add_login(&app);
        app.events.signal();
        r
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

#[derive(Deserialize)]
pub struct MadeQuery {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
}

/// Where GitHub sends the person once Bagholder's GitHub app is made, in GitHub's
/// tab: the app taken from GitHub (only for a manifest this Bagholder handed out)
/// and added to Cloudflare as Bagholder's login method; the tab says so.
pub async fn made(State(state): State<AppState>, axum::extract::Query(q): axum::extract::Query<MadeQuery>) -> Response {
    let app = state.app.clone();
    let done = tokio::task::spawn_blocking(move || -> Result<String, String> {
        if !app.remote.manifest_back(&q.state, Timestamp::now()) {
            return Err("This link doesn’t make Bagholder’s sign-in. Start again from Bagholder.".into());
        }
        let answer = remote::github_made(&q.code)?;
        let (name, slug, client_id, client_secret) = remote::made_of(&answer).ok_or("GitHub’s answer for the app could not be read.")?;
        let mut kept = app.remote.kept(&app.home);
        kept.github = Some(remote::GitHubLogin { name: name.clone(), slug, client_id, client_secret: Some(client_secret), login: String::new() });
        app.remote.keep(&app.home, kept)?;
        let added = add_login(&app);
        app.events.signal();
        added.map(|()| name)
    })
    .await;
    let (title, said) = match done {
        Ok(Ok(name)) => ("Bagholder’s sign-in is made".to_string(), format!("GitHub will show {name} when you sign in. You can close this tab and go back to Bagholder.", name = agents::oauth::escape(&name))),
        Ok(Err(e)) => ("Bagholder’s sign-in isn’t made yet".to_string(), format!("{} Go back to Bagholder to try again.", agents::oauth::escape(&e))),
        Err(e) => ("Bagholder’s sign-in isn’t made yet".to_string(), agents::oauth::escape(&e.to_string())),
    };
    let body = format!("{}<main><div class=dim style='font-size:13px'>Bagholder</div><h1>{title}</h1><p>{said}</p></main>", agents::oauth::HEAD);
    let mut r = Html(body).into_response();
    r.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    r
}

#[derive(Deserialize, TS)]
pub struct SetUpAsked {
    /// The name before the domain ("app"), or the whole address.
    pub name: String,
    /// The one email Access lets in.
    pub email: String,
    /// The address the person chose to take over from an earlier set-up's tunnel
    /// that runs nowhere (`RemoteStatus::held`).
    #[serde(default, rename = "takeOver")]
    pub take_over: String,
}

/// Everything at Cloudflare made, then the tunnel started: the page's own tunnel
/// with the same sign-in as the agents' (copied), its DNS record, and the Access
/// application letting in the email alone. Made again, the name before is deleted
/// at Cloudflare first.
async fn set_up(State(state): State<AppState>, Required(asked): Required<SetUpAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let email = asked.email.trim().to_ascii_lowercase();
        if !(email.contains('@') && !email.contains(char::is_whitespace) && email.len() <= 254 && email.split_once('@').is_some_and(|(u, d)| !u.is_empty() && d.contains('.'))) {
            return Err("Type the email you sign in to GitHub with.".into());
        }
        let token = remote::token(&app.home).ok_or("Give Bagholder its Cloudflare token first.")?;
        let program = tunnel::find_here(&app.home, Provider::Cloudflare).ok_or("Cloudflare’s connector isn’t installed: install it first.")?;
        let program = program.display().to_string();
        let ours = agents::cloudflare::Files::new(&app.home);
        let access = ours.access().ok_or("Sign in to Cloudflare first.")?;
        let team = remote::team(&token, &access.account)?.ok_or_else(|| format!("Your Cloudflare account has no Zero Trust team yet: make one at {}.", remote::TEAM_PAGE))?;
        let before = app.remote.kept(&app.home);
        let github = before.github.clone().filter(|g| !g.login.is_empty()).ok_or("Make Bagholder’s sign-in on GitHub first.")?;
        let files = remote::cloudflare(&app.home);
        files.take_sign_in(&ours)?;
        stop(&app);
        // an address an earlier set-up of Bagholder's still holds, its tunnel connected
        // nowhere, is offered to the person, and taken over once they choose to: a
        // computer asleep is connected nowhere too, so it is never taken unasked
        let made = match files.set_up(&program, asked.name.trim()) {
            Err(e) if e == agents::cloudflare::NAME_TAKEN => match super::agents::stale_holder(&access, asked.name.trim(), files.tunnel_id().as_deref())? {
                Some(h) if asked.take_over.eq_ignore_ascii_case(&h.host) => {
                    super::agents::take_over(&access, &h)?;
                    files.set_up(&program, asked.name.trim())
                }
                Some(h) => {
                    let said = format!("{} is held by an earlier Bagholder set-up whose tunnel isn’t connected.", h.host);
                    app.remote.set_held(Some((h.host, h.last_connected)));
                    app.events.signal();
                    Err(said)
                }
                None => Err(e),
            },
            other => other,
        };
        let host = tunnel::clean_name(&made?)?;
        app.remote.set_held(None);
        if !before.host.is_empty() && before.host != host {
            if let (Some(id), Some(a)) = (files.tunnel_id(), files.access()) {
                super::agents::delete_record(&a, &before.host, &format!("{id}.cfargotunnel.com"))?;
            }
        }
        let (app_id, aud) = remote::make_application(&token, &access.account, &host, &email, &github.login)?;
        if !before.app_id.is_empty() && before.app_id != app_id {
            remote::delete_application(&token, &access.account, &before.app_id)?;
        }
        app.remote.keep(&app.home, remote::Kept { host, on: true, email, team, app_id, aud, github: Some(github) })?;
        up(&app)
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

#[derive(Deserialize, TS)]
pub struct RemoteSwitch {
    pub on: bool,
}

/// The tunnel started or stopped; it starts with Bagholder again only while on.
async fn switch(State(state): State<AppState>, Required(asked): Required<RemoteSwitch>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let mut kept = app.remote.kept(&app.home);
        if !kept.set_up() {
            return Err("Set the address up first.".into());
        }
        if asked.on {
            up(&app)?;
        } else {
            stop(&app);
        }
        kept.on = asked.on;
        app.remote.keep(&app.home, kept)?;
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// The address checked from outside again, after the check gave up: tried again
/// as while it comes up.
async fn check(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    app.remote.set_ready(None);
    check_while_coming_up(&app);
    app.events.signal();
    Ok(Json(OkOr::ok()))
}

/// Everything Bagholder made for the address gone: the tunnel stopped, its DNS
/// record and the tunnel deleted at Cloudflare, the Access application deleted,
/// and its files here with the token the person made; and the Cloudflare sign-in
/// too when no agents' tunnel uses it. One that fails at Cloudflare keeps the rest,
/// so it can be removed again.
async fn remove(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        // off first: nothing says it is starting while it goes
        let mut kept = app.remote.kept(&app.home);
        kept.on = false;
        app.remote.keep(&app.home, kept.clone())?;
        stop(&app);
        app.events.signal();
        let files = remote::cloudflare(&app.home);
        if !kept.app_id.is_empty() {
            let token = remote::token(&app.home).ok_or("Bagholder’s Cloudflare token is gone, so the Access application can’t be deleted.")?;
            let account = files.access().or_else(|| agents::cloudflare::Files::new(&app.home).access()).ok_or("Cloudflare’s sign-in is gone, so the Access application can’t be deleted.")?.account;
            remote::delete_application(&token, &account, &kept.app_id)?;
        }
        let address = (!kept.host.is_empty()).then_some(kept.host.as_str());
        super::agents::remove_made(&files, &app.home, address, &super::agents::delete_record)?;
        remote::forget(&app.home)?;
        app.remote.keep(&app.home, remote::Kept::default())?;
        app.remote.set_team(None);
        // the sign-in this set-up used, where no agents' tunnel runs on it
        let theirs = agents::cloudflare::Files::new(&app.home);
        if theirs.tunnel().is_none() {
            theirs.forget()?;
        }
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

// ---- the tunnel ----

fn stop(app: &App) {
    app.remote.tunnel.lock().unwrap_or_else(|e| e.into_inner()).stop();
    app.remote.next_check();
    app.remote.set_ready(None);
}

/// The tunnel started to the page's port, from its own folder, and checked from
/// outside until it answers (`check_while_coming_up`).
fn up(app: &Arc<App>) -> Result<(), String> {
    let kept = app.remote.kept(&app.home);
    if !kept.set_up() {
        return Err("Set the address up first.".into());
    }
    if app.remote.tunnel.lock().unwrap_or_else(|e| e.into_inner()).state().running.is_some() {
        return Ok(());
    }
    let files = remote::cloudflare(&app.home);
    let program = tunnel::find_here(&app.home, Provider::Cloudflare).ok_or("Cloudflare’s connector isn’t installed: install it first.")?;
    let port = *app.port.lock().unwrap_or_else(|e| e.into_inner());
    let command = files.run(&program.display().to_string(), &format!("127.0.0.1:{port}")).ok_or("The tunnel isn’t made yet: set the address up again.")?;
    let (argv, mut env) = tunnel::guarded(&command)?;
    env.extend(files.env());
    let signal = app.events.clone();
    let on_line = move || signal.signal();
    let weak = Arc::downgrade(app);
    let ended = move |run: u64| {
        let Some(app) = weak.upgrade() else { return };
        app.remote.tunnel.lock().unwrap_or_else(|e| e.into_inner()).ended(run);
        app.events.signal();
    };
    app.remote.tunnel.lock().unwrap_or_else(|e| e.into_inner()).start(Provider::Cloudflare, (argv, env), Arc::new(on_line), Arc::new(ended))?;
    app.remote.set_ready(None);
    check_while_coming_up(app);
    app.events.signal();
    Ok(())
}

/// How often the address is asked from outside while it comes up: often for the
/// first minute, when it is most likely to begin answering, then less.
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
const CHECK_FAST_FOR: std::time::Duration = std::time::Duration::from_secs(60);
const CHECK_THEN_EVERY: std::time::Duration = std::time::Duration::from_secs(15);
/// How long it is tried before the check gives up and says why.
const CHECK_FOR: std::time::Duration = std::time::Duration::from_secs(600);

/// The address asked from outside on a thread of its own until it asks for the
/// sign-in: each try's result told to the panel as it comes, and given up only after
/// `CHECK_FOR`, with the last try's reason. A new run, or the tunnel stopping, ends it.
fn check_while_coming_up(app: &Arc<App>) {
    let run = app.remote.next_check();
    let weak = Arc::downgrade(app);
    let started = std::thread::Builder::new().name("remote-check".into()).spawn(move || {
        let began = std::time::Instant::now();
        loop {
            std::thread::sleep(if began.elapsed() < CHECK_FAST_FOR { CHECK_EVERY } else { CHECK_THEN_EVERY });
            let Some(app) = weak.upgrade() else { return };
            if !app.remote.checking(run) || app.remote.tunnel.lock().unwrap_or_else(|e| e.into_inner()).state().running.is_none() {
                return;
            }
            let kept = app.remote.kept(&app.home);
            match remote::check_through(&kept.host, &kept.team) {
                Ok(()) => {
                    app.remote.set_last_try(String::new());
                    app.remote.set_ready(Some(Ok(())));
                    app.events.signal();
                    return;
                }
                Err(e) if began.elapsed() >= CHECK_FOR => {
                    app.remote.set_ready(Some(Err(e)));
                    app.events.signal();
                    return;
                }
                Err(e) => {
                    // a team renamed at Cloudflare sends its sign-in from its new address
                    ask_team_again(&app, Timestamp::now());
                    app.remote.set_last_try(e);
                    app.events.signal();
                }
            }
        }
    });
    if let Err(e) = started {
        crate::app::log(&format!("bagholder: the page’s address could not be checked: {e}"));
    }
}

/// At start: the tunnel again when the person left it on. A copy of the book
/// started for some work (`BAGHOLDER_EXIT_WITH`) never opens it: it would answer
/// the person's own address with a scratch book.
pub fn open(app: &Arc<App>) {
    if std::env::var_os(crate::exit_with::VAR).is_some() {
        return;
    }
    if app.remote.kept(&app.home).on {
        if let Err(e) = up(app) {
            crate::app::log(&format!("bagholder: the page’s address could not open again: {e}"));
        }
    }
}
