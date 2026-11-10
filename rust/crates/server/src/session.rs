//! The Wealthsimple session: the saved login, the refresh grant, the sign-in's
//! capture, and Sync now. The reads themselves are the broker's (`broker_reads`).

use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use ts_rs::TS;

use bagholder_ws::session::{Client, Session};

use crate::app::{now_iso, now_unix, App};
use crate::http::OkOr;


/// The saved login: `None` where none is saved. A file that cannot be read, or
/// that holds no login, is the error, for the caller to say: never read as no
/// login, nor as an empty one.
pub fn load_session(app: &Arc<App>) -> Result<Option<Session>, String> {
    use serde::Deserialize;
    let unread = |e: String| format!("The saved Wealthsimple login could not be read: {e}");
    let text = match std::fs::read_to_string(app.ws_home().session_path()) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(unread(e.to_string())),
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| unread(e.to_string()))?;
    if !v.is_object() {
        return Err(unread("it holds no login".into()));
    }
    Session::deserialize(v).map(Some).map_err(|e| unread(e.to_string()))
}

/// Writes the login; a failure is said in words the header can show.
pub fn save_session(app: &Arc<App>, sess: &Session) -> Result<(), String> {
    app.ws_home().save_session(sess).map_err(|e| format!("Could not save the Wealthsimple login: {}", e))
}

fn set_error(app: &Arc<App>, msg: &str) {
    app.state.lock().unwrap().error = msg.to_string();
}

/// What the session code is told of the sign-in. It alone sets "connected", which
/// means it holds a sign-in Wealthsimple issued and has not refused
/// (`docs/plans/stage-money.md`, part C: one writer). Only Wealthsimple ends one:
/// an access token past its expiry is renewed by the refresh, and no answer says
/// nothing of the sign-in.
pub enum Heard {
    /// A sign-in saved by an earlier run, found at start: it stands until
    /// Wealthsimple is asked.
    Saved,
    /// Wealthsimple issued a token, or answered a request signed with one.
    Issued,
    /// Wealthsimple refused the sign-in, or there is none: connect again.
    Lapsed(String),
    /// Wealthsimple could not be asked (no answer, a refusal of the request, not of
    /// the sign-in): the sign-in stands as it was.
    NotAsked(String),
}

/// The one writer of "connected", and of the error the sign-in leaves. A sign-in
/// that ends is said in the log, with why.
pub fn heard(app: &Arc<App>, h: Heard) {
    let mut st = app.state.lock().unwrap();
    match h {
        Heard::Saved | Heard::Issued => st.connected = true,
        Heard::Lapsed(why) => {
            if st.connected {
                crate::app::log(&format!("bagholder sign-in: ended: {}", if why.is_empty() { "no saved sign-in" } else { &why }));
            }
            st.connected = false;
            st.error = why;
        }
        Heard::NotAsked(why) => st.error = why,
    }
}

/// A refresh that did not give a new token: refused (connect again), or not asked.
#[derive(Debug, PartialEq)]
pub enum RefreshFailed {
    Lapsed(String),
    NotAsked(String),
}

impl RefreshFailed {
    pub fn why(&self) -> &str {
        match self {
            RefreshFailed::Lapsed(w) | RefreshFailed::NotAsked(w) => w,
        }
    }
}

impl From<bagholder_broker::Failure> for RefreshFailed {
    fn from(f: bagholder_broker::Failure) -> Self {
        match f {
            bagholder_broker::Failure::Lapsed(why) => RefreshFailed::Lapsed(why),
            other => RefreshFailed::NotAsked(other.to_string()),
        }
    }
}

/// The refresh grant on the saved sign-in, told to `heard`.
pub fn refresh_session(app: &Arc<App>, sess: &mut Session) -> bool {
    match one_refresh(app, sess) {
        Ok(()) => {
            heard(app, Heard::Issued);
            true
        }
        Err(RefreshFailed::Lapsed(why)) => {
            heard(app, Heard::Lapsed(why));
            false
        }
        Err(RefreshFailed::NotAsked(why)) => {
            heard(app, Heard::NotAsked(why));
            false
        }
    }
}

/// What a refresh tells the person when the saved login names no client id.
/// A refresh reads the session and the cached id only: it does not call token
/// info (that needs an access token Wealthsimple still accepts) and it does not
/// read the login page. Capture and boot do those, and store what they find.
const REFRESH_NEEDS_A_STORED_CLIENT_ID: &str = "The saved Wealthsimple login has no client id. A refresh does not read token info or the login page. Connect again to read the client id.";

/// The tokens a refresh posts, from a login: its client id (the cached one where it
/// names none) and whose login it is.
fn held_tokens(app: &Arc<App>, sess: &mut Session) -> Result<bagholder_wealthsimple::session::Tokens, RefreshFailed> {
    let home = app.ws_home();
    let client_id = Client { home: &home }.client_id_for(sess).map_err(|e| RefreshFailed::NotAsked(format!("Could not read the cached Wealthsimple client id: {e}")))?;
    if client_id.is_empty() {
        return Err(RefreshFailed::Lapsed(REFRESH_NEEDS_A_STORED_CLIENT_ID.into()));
    }
    sess.client_id = client_id.clone();
    if sess.identity().is_empty() {
        // the adapter asks for the accounts by it: a login without it cannot be read
        return Err(RefreshFailed::Lapsed("Wealthsimple did not say whose login this is; connect again".into()));
    }
    if sess.refresh_token.is_empty() {
        return Err(RefreshFailed::Lapsed("missing refresh token".into()));
    }
    Ok(bagholder_wealthsimple::session::Tokens {
        access: sess.access_token.clone(),
        refresh: sess.refresh_token.clone(),
        client_id,
        identity: sess.identity(),
        expires_at: None,
    })
}

fn adopt(sess: &mut Session, fresh: bagholder_wealthsimple::session::Tokens) {
    sess.access_token = fresh.access;
    sess.refresh_token = fresh.refresh;
    sess.client_id = fresh.client_id;
    if let Some(at) = fresh.expires_at {
        sess.expires_at = Some(bagholder_ws::session::Expiry::Text(at.to_string()));
    }
}

/// Every refresh in the process goes through the adapter's session
/// (`bagholder_wealthsimple::session`): one lock, the file read again under it so
/// a token another caller rotated is adopted without a post, and a refresh token
/// Wealthsimple refused never posted again. The reads, the orders and the sign-in
/// share it, so two of them at once post a refresh token once
/// (`docs/plans/stage-3c-switch.md`, §3, "One session").
fn one_refresh(app: &Arc<App>, sess: &mut Session) -> Result<(), RefreshFailed> {
    let held = held_tokens(app, sess)?;
    let file = bagholder_wealthsimple::session::SessionFile { path: app.ws_home().session_path() };
    let fresh = bagholder_wealthsimple::session::refresh(&app.net, &file, &held)?;
    adopt(sess, fresh);
    Ok(())
}

pub fn token_info(app: &Arc<App>, sess: &Session) -> bagholder_ws::session::TokenInfo {
    let home = app.ws_home();
    Client { home: &home }.token_info(sess)
}

pub fn apply_token_info_client_id(app: &Arc<App>, sess: &mut Session, info: Option<&bagholder_ws::session::TokenInfo>) -> String {
    if sess.access_token.is_empty() {
        return String::new();
    }
    let fetched;
    let info = match info {
        Some(i) => i,
        None => {
            fetched = token_info(app, sess);
            &fetched
        }
    };
    let cid = bagholder_ws::session::client_id_from_token_info(info);
    if cid.is_empty() {
        return String::new();
    }
    sess.client_id = cid.clone();
    if let Err(e) = app.ws_home().save_client_id(&cid) {
        set_error(app, &format!("Could not save the Wealthsimple client id: {e}"));
    }
    cid
}

/// Why a client id was not found. The words are the diagnostic: they name the
/// step, and they never include a token, a cookie or the id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientIdGap {
    CookieAbsent,
    TokenInfoFailed,
    TokenInfoNoId,
    LoginPageUnfetched,
    ScriptUnfetched,
    ParserNoScript,
    ParserNoClientId,
}

impl ClientIdGap {
    fn phrase(self) -> &'static str {
        match self {
            ClientIdGap::CookieAbsent => "the sign-in cookie has no client id",
            ClientIdGap::TokenInfoFailed => "token info could not be read",
            ClientIdGap::TokenInfoNoId => "token info gave no client id",
            ClientIdGap::LoginPageUnfetched => "the login page could not be fetched",
            ClientIdGap::ScriptUnfetched => "the login script could not be fetched",
            ClientIdGap::ParserNoScript => "the login page matched no script",
            ClientIdGap::ParserNoClientId => "the login script matched no client id",
        }
    }

    /// The page was fetched and did not name an id. The next sign-in reads that
    /// same page, so signing in again does not supply one.
    fn page_will_not_change(self) -> bool {
        matches!(self, ClientIdGap::ParserNoScript | ClientIdGap::ParserNoClientId)
    }
}

fn client_id_message(gaps: &[ClientIdGap]) -> String {
    let why = gaps.iter().map(|g| g.phrase()).collect::<Vec<_>>().join("; ");
    let next = if gaps.iter().any(|g| g.page_will_not_change()) {
        "Connecting again reads the same login page and will not supply a client id."
    } else {
        "Try connecting again."
    };
    format!("Wealthsimple's client id could not be read: {why}. {next}")
}

fn client_id_failed(gaps: &[ClientIdGap]) -> RefreshFailed {
    let msg = client_id_message(gaps);
    crate::app::log(&format!("bagholder login: {msg}"));
    if gaps.iter().any(|g| g.page_will_not_change()) { RefreshFailed::Lapsed(msg) } else { RefreshFailed::NotAsked(msg) }
}

/// `env` is `production`, then the id, before the object ends. The id is group 1.
/// The `regex` crate has no backreferences, so either quote is accepted on its own.
const PRODUCTION_ENV_THEN_ID: &str = r#"(?i:env)\s*:\s*["']production["'][^}]*?(?i:client_?id)\s*:\s*["']([0-9a-fA-F]{8,})["']"#;
/// The id, then `env` is `production`, before the object ends. The id is group 1.
const PRODUCTION_ID_THEN_ENV: &str = r#"(?i:client_?id)\s*:\s*["']([0-9a-fA-F]{8,})["'][^}]*?(?i:env)\s*:\s*["']production["']"#;
/// An older build: a `production` key whose object holds the id. The id is group 1.
const PRODUCTION_KEY_THEN_ID: &str = r#"(?i:production)\s*:\s*\{[^}]*?(?i:client_?id)\s*:\s*["']([0-9a-fA-F]{8,})["']"#;

fn first_hex(pattern: &str, text: &str, group: usize) -> Option<String> {
    regex::Regex::new(pattern).unwrap().captures(text).and_then(|c| c.get(group)).map(|m| m.as_str().to_string())
}

/// The production OAuth client id in Wealthsimple's login script.
///
/// The current build names it on the flat object whose `env` is `production`
/// (`{env:"production",…,clientId:"<hex>"}`), beside a sandbox object that a
/// sign-in does not use. An older build named it under a `production` key
/// (`{production:{clientId:"<hex>"}}`). Either quote, either order of the keys,
/// either spelling `clientId` or `client_id`, and the whitespace a minifier
/// keeps or drops are the same id. The value is hex of at least eight digits.
pub(crate) fn client_id_from_app_script(js: &str) -> Option<String> {
    first_hex(PRODUCTION_ENV_THEN_ID, js, 1).or_else(|| first_hex(PRODUCTION_ID_THEN_ENV, js, 1)).or_else(|| first_hex(PRODUCTION_KEY_THEN_ID, js, 1))
}

/// The login page's app bundle (`app-<hex>.js`), in either quote, with the
/// whitespace and attribute order the page uses. The first matching script.
pub(crate) fn app_script_from_login_html(html: &str) -> Option<String> {
    let script = regex::Regex::new(r#"(?i)<script\b[^>]*?\bsrc\s*=\s*(?:"([^"]+)"|'([^']+)')"#).unwrap();
    let bundle = regex::Regex::new(r#"(?i)(?:^|/)app-[0-9a-f]+\.js(?:$|[?#])"#).unwrap();
    for found in script.captures_iter(html) {
        let Some(src) = found.get(1).or_else(|| found.get(2)).map(|m| m.as_str()) else { continue };
        if bundle.is_match(src) {
            return Some(src.to_string());
        }
    }
    None
}

/// `src` as an address: absolute as it is, protocol-relative under https, and
/// a path (or a bare name) on the web app's origin.
pub(crate) fn absolute_script_url(src: &str) -> String {
    if src.starts_with("https://") || src.starts_with("http://") {
        src.to_string()
    } else if src.starts_with("//") {
        format!("https:{src}")
    } else if src.starts_with('/') {
        format!("{}{src}", bagholder_wealthsimple::hosts::web())
    } else {
        format!("{}/{src}", bagholder_wealthsimple::hosts::web())
    }
}

enum ScrapeFail {
    /// The cached id, or the cached user agent, could not be read. The words
    /// are the error.
    Unread(String),
    Gap(ClientIdGap),
}

/// The production client id from Wealthsimple's login script, cached.
fn scrape_client_id(app: &Arc<App>) -> Result<String, ScrapeFail> {
    let home = app.ws_home();
    let cached = match home.cached_client_id() {
        Ok(c) => c,
        Err(e) => return Err(ScrapeFail::Unread(format!("Could not read the cached Wealthsimple client id: {e}"))),
    };
    if !cached.is_empty() {
        return Ok(cached);
    }
    let ua = match home.cached_user_agent() {
        Ok(ua) => ua,
        Err(e) => return Err(ScrapeFail::Unread(format!("Could not read the cached browser user agent: {e}"))),
    };
    let hdrs: Vec<(&str, &str)> = if ua.is_empty() { vec![] } else { vec![("User-Agent", ua.as_str())] };
    let get = |url: &str| bagholder_net::client::request("GET", url, &hdrs, None, Duration::from_secs(20)).ok().map(|r| r.text());
    bagholder_net::client::named_call("wealthsimple", "web app/login");
    let html = match get(&bagholder_wealthsimple::hosts::login_url()) {
        Some(h) => h,
        None => return Err(ScrapeFail::Gap(ClientIdGap::LoginPageUnfetched)),
    };
    let src = match app_script_from_login_html(&html) {
        Some(s) => s,
        None => return Err(ScrapeFail::Gap(ClientIdGap::ParserNoScript)),
    };
    bagholder_net::client::named_call("wealthsimple", "web app script");
    let js = match get(&absolute_script_url(&src)) {
        Some(j) => j,
        None => return Err(ScrapeFail::Gap(ClientIdGap::ScriptUnfetched)),
    };
    match client_id_from_app_script(&js) {
        Some(id) => {
            if let Err(e) = home.save_client_id(&id) {
                set_error(app, &format!("Could not save the Wealthsimple client id: {e}"));
            }
            Ok(id)
        }
        None => Err(ScrapeFail::Gap(ClientIdGap::ParserNoClientId)),
    }
}

/// Refresh ahead of the expiry; what came of it is told to `heard`.
pub fn ensure_fresh_token(app: &Arc<App>, sess: Option<Session>) -> bool {
    let sess = match sess {
        Some(s) => Some(s),
        None => match load_session(app) {
            Ok(s) => s,
            Err(e) => {
                heard(app, Heard::NotAsked(e));
                return false;
            }
        },
    };
    let mut sess = match sess {
        Some(s) if !s.refresh_token.is_empty() => s,
        _ => {
            heard(app, Heard::Lapsed("missing refresh token".into()));
            return false;
        }
    };
    let connected = app.state.lock().unwrap().connected;
    if connected && !bagholder_ws::session::token_refresh_needed(&sess, now_unix()) {
        return true;
    }
    if refresh_session(app, &mut sess) {
        app.state.lock().unwrap().error.clear();
        return true;
    }
    false
}

/// The login only; stored rows stay. A login file that cannot be removed is
/// still there: nothing is marked disconnected, and the caller answers why.
pub fn delete_session(app: &Arc<App>) -> Result<(), String> {
    app.ws_home().delete_session().map_err(|e| format!("Could not remove the Wealthsimple login: {e}"))?;
    heard(app, Heard::Lapsed(String::new()));
    let mut st = app.state.lock().unwrap();
    st.email.clear();
    st.last_sync.clear();
    st.capturing = false;
    st.portfolio_error.clear();
    Ok(())
}

/// The saved session read and its tokens checked. When Wealthsimple was last
/// pulled is the book's (`broker_reads::run`), not the session's.
pub fn boot_session(app: &Arc<App>) {
    let mut sess = match load_session(app) {
        Ok(Some(s)) => {
            if !s.refresh_token.is_empty() {
                heard(app, Heard::Saved);
            }
            s
        }
        Ok(None) => {
            heard(app, Heard::Lapsed(String::new()));
            return;
        }
        Err(e) => {
            heard(app, Heard::NotAsked(e));
            return;
        }
    };
    let mut saved = Ok(());
    let mut answered = false;
    let mut token_gap = None;
    if !sess.access_token.is_empty() {
        let info = token_info(app, &sess);
        if info.is_ok() {
            answered = true;
            apply_token_info_client_id(app, &mut sess, Some(&info));
            if sess.client_id.is_empty() {
                token_gap = Some(ClientIdGap::TokenInfoNoId);
            }
            let identity = info.identity();
            if !identity.is_empty() && sess.identity().is_empty() {
                sess.ids.identity_canonical_id = identity;
            }
            if !info.email.is_empty() {
                sess.email = info.email.clone();
            }
            // Wealthsimple answered a request signed with the token held
            heard(app, Heard::Issued);
            app.state.lock().unwrap().error.clear();
        } else {
            token_gap = Some(ClientIdGap::TokenInfoFailed);
        }
    }
    // an older saved login with no client id: the login script, which a refresh does not read
    let had_id = !sess.client_id.is_empty();
    let mut unread = None;
    if !had_id {
        match scrape_client_id(app) {
            Ok(cid) => sess.client_id = cid,
            Err(ScrapeFail::Unread(msg)) => unread = Some(RefreshFailed::NotAsked(msg)),
            Err(ScrapeFail::Gap(gap)) => {
                let mut gaps = Vec::new();
                if let Some(g) = token_gap {
                    gaps.push(g);
                }
                gaps.push(gap);
                unread = Some(client_id_failed(&gaps));
            }
        }
    }
    if answered {
        saved = save_session(app, &sess);
    }
    if !answered {
        if sess.refresh_token.is_empty() {
            heard(app, Heard::Lapsed("missing refresh token".into()));
        } else if sess.client_id.is_empty() {
            match unread.as_ref() {
                Some(RefreshFailed::NotAsked(why)) => heard(app, Heard::NotAsked(why.clone())),
                Some(RefreshFailed::Lapsed(why)) => heard(app, Heard::Lapsed(why.clone())),
                None => heard(app, Heard::Lapsed(REFRESH_NEEDS_A_STORED_CLIENT_ID.into())),
            }
        } else {
            // the adapter reads the saved login before it posts, and that read requires the id
            if !had_id {
                saved = save_session(app, &sess);
            }
            if saved.is_ok() && refresh_session(app, &mut sess) {
                app.state.lock().unwrap().error.clear();
                match load_session(app) {
                    Ok(Some(s)) => sess = s,
                    Ok(None) => {}
                    Err(e) => saved = Err(e),
                }
            }
        }
    }
    let mut st = app.state.lock().unwrap();
    if let Err(e) = saved {
        st.error = e;
    } else if answered {
        if let Some(failed) = unread.as_ref() {
            st.error = failed.why().to_string();
        }
    }
    st.email = sess.email.clone();
}

/// Told once per expiry.
pub fn note_session_expired(app: &Arc<App>) {
    let was = app.state.lock().unwrap().connected;
    heard(app, Heard::Lapsed("Session expired. Connect again.".into()));
    if was {
        // a notice that could not be recorded is said in the header until one is
        crate::notify::tell(app, "connection", &format!("session:{}", now_iso()), "Sign in needed", "The Wealthsimple session expired. Connect again from the menu.", None);
    }
}

/// `POST /api/refresh`.
#[derive(Serialize, TS)]
pub struct RefreshAnswer {
    pub ok: bool,
    pub error: String,
    pub connected: bool,
}

/// `POST /api/sync`.
#[derive(Serialize, TS)]
pub struct SyncAnswer {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub syncing: Option<bool>,
}

/// Start a pull if a login is saved; its progress reaches the page as status
/// changes.
pub fn sync_now(app: &Arc<App>) -> SyncAnswer {
    match load_session(app) {
        Ok(Some(_)) => {}
        Ok(None) => return SyncAnswer { ok: false, error: Some("not connected".into()), syncing: None },
        Err(e) => return SyncAnswer { ok: false, error: Some(e), syncing: None },
    }
    app.state.lock().unwrap().error.clear();
    // the broker's reads pull at once
    app.pull_asked.store(true, std::sync::atomic::Ordering::SeqCst);
    app.events.signal();
    SyncAnswer { ok: true, error: None, syncing: Some(true) }
}

/// The grant, always.
pub fn refresh_now(app: &Arc<App>) -> RefreshAnswer {
    let mut sess = match load_session(app) {
        Ok(Some(s)) if !s.refresh_token.is_empty() => s,
        Ok(_) => {
            heard(app, Heard::Lapsed("not connected".into()));
            return RefreshAnswer { ok: false, error: "not connected".into(), connected: false };
        }
        Err(e) => {
            heard(app, Heard::NotAsked(e.clone()));
            return RefreshAnswer { ok: false, error: e, connected: app.state.lock().unwrap().connected };
        }
    };
    let ok = refresh_session(app, &mut sess);
    let mut st = app.state.lock().unwrap();
    if ok {
        st.error.clear();
    }
    RefreshAnswer { ok, error: st.error.trim().to_string(), connected: st.connected }
}

/// The token kept fresh, backing off on failure. The pull and the balances are
/// the broker's reads' (`broker_reads`); this keeps the sign-in the order code
/// sends with from lapsing between them.
pub fn token_loop(app: Arc<App>) {
    // One thing is waited for, known ahead: the token coming up for refresh. The
    // loop sleeps until then and wakes early only when the session changes under
    // it (a sign-in, a disconnect). Only a failure is retried on a period,
    // doubling from `RETRY_FIRST` to half an hour.
    const RETRY_FIRST: Duration = Duration::from_secs(30);
    const RETRY_MOST: Duration = Duration::from_secs(1800);
    // a login that cannot be read is its own value: said in the header, and waited on until it changes
    let has_login = |app: &Arc<App>| load_session(app).map(|s| s.is_some_and(|s| !s.refresh_token.is_empty()));
    let mut retry: Option<Duration> = None;
    loop {
        let now = now_unix();
        let connected = app.state.lock().unwrap().connected;
        let login = has_login(&app);
        if let Err(e) = &login {
            app.state.lock().unwrap().error = e.clone();
        }
        let sleep = match retry {
            Some(d) => d,
            None if login != Ok(true) => Duration::MAX, // nothing to keep fresh until someone signs in
            None => Duration::from_secs_f64(match load_session(&app) {
                Ok(Some(s)) => bagholder_ws::session::seconds_until_token_refresh(&s, now).max(0.0),
                // read again at once: the refresh that follows says why it cannot be
                Ok(None) | Err(_) => 0.0,
            }),
        };
        let was = (connected, login);
        let changed = || (app.state.lock().unwrap().connected, has_login(&app)) != was;
        if sleep == Duration::MAX {
            if !app.events.park_until(&app, changed) {
                return;
            }
        } else if !sleep.is_zero() {
            app.events.park_until_or(&app, sleep, changed);
        }
        if app.stopping() {
            return;
        }
        if has_login(&app) != Ok(true) {
            retry = None;
            continue;
        }
        if !ensure_fresh_token(&app, None) {
            retry = Some(retry.map_or(RETRY_FIRST, |d| (d * 2).min(RETRY_MOST)));
            continue;
        }
        retry = None;
    }
}

/// The captured login's fields; any other key the body carries is kept as it
/// was, exactly as a saved `Session` keeps what it does not read.
#[derive(Clone, Debug, Default, serde::Deserialize, TS)]
#[serde(default)]
pub struct Capture {
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub access_token: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub refresh_token: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub client_id: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub wssdi: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub session_id: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub user_agent: String,
    pub expires_at: Option<bagholder_ws::session::Expiry>,
    #[serde(flatten)]
    pub ids: bagholder_ws::session::IdentityKeys,
}

/// What became of a captured login.
#[derive(Debug, PartialEq)]
pub enum Taken {
    /// Wealthsimple answered with new tokens: the login is the app's, and saved.
    Taken,
    /// Wealthsimple refused it, or it says nothing of whose it is: not tried again.
    Refused(String),
    /// It could not be asked, or not read or written: tried again while the window is up.
    NotAsked(String),
}

/// `POST /api/capture`: a captured login handed over, taken over or refused in words.
pub fn capture_tokens(app: &Arc<App>, capture: &Capture) -> OkOr {
    match take_capture(app, capture) {
        Taken::Taken => OkOr::ok(),
        Taken::Refused(why) | Taken::NotAsked(why) => OkOr::err(why),
    }
}

/// Keep the captured login in memory and take it over.
pub fn take_capture(app: &Arc<App>, capture: &Capture) -> Taken {
    let capture = capture.clone();
    if capture.access_token.is_empty() {
        return Taken::Refused("missing access_token".into());
    }
    // a new login is taken over the saved one; a saved one that cannot be read is said, not overwritten unseen
    let mut sess = match load_session(app) {
        Ok(s) => s.unwrap_or_default(),
        Err(e) => {
            app.state.lock().unwrap().error = e.clone();
            return Taken::NotAsked(e);
        }
    };
    sess.access_token = capture.access_token;
    if !capture.refresh_token.is_empty() {
        sess.refresh_token = capture.refresh_token;
    }
    // a captured `expires_at` of 0 or "" is not truthy and does not overwrite
    let expiry_truthy = match &capture.expires_at {
        Some(bagholder_ws::session::Expiry::Unix(n)) => *n != 0.0,
        Some(bagholder_ws::session::Expiry::Text(t)) => !t.is_empty(),
        None => false,
    };
    if expiry_truthy {
        sess.expires_at = capture.expires_at;
    }
    if !capture.wssdi.is_empty() {
        sess.wssdi = capture.wssdi;
    }
    if !capture.client_id.is_empty() {
        sess.client_id = capture.client_id;
    }
    if !capture.session_id.is_empty() {
        sess.session_id = capture.session_id;
    }
    if !capture.user_agent.is_empty() {
        sess.user_agent = capture.user_agent;
    }

    let mut ident = capture.ids.identity();
    if ident.is_empty() {
        ident = sess.identity();
    }
    let info = if sess.access_token.is_empty() { bagholder_ws::session::TokenInfo::default() } else { token_info(app, &sess) };
    if ident.is_empty() {
        ident = info.identity();
    }
    if ident.is_empty() {
        // the accounts are asked for by it: a login that does not say whose it is
        // cannot be taken over
        let why = info.error.as_ref().map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())).unwrap_or_else(|| "no answer".into());
        let err = format!("Wealthsimple did not say whose login this is: {why}");
        app.state.lock().unwrap().error = err.clone();
        return Taken::Refused(err);
    }
    sess.ids.identity_canonical_id = ident;
    if sess.session_id.is_empty() {
        sess.session_id = crate::app::uuid4();
    }
    apply_token_info_client_id(app, &mut sess, Some(&info));
    if sess.client_id.is_empty() {
        match scrape_client_id(app) {
            Ok(cid) => sess.client_id = cid,
            Err(ScrapeFail::Unread(msg)) => return refused_capture(app, RefreshFailed::NotAsked(msg)),
            Err(ScrapeFail::Gap(gap)) => {
                // the captured client id is the sign-in cookie's; token info was just asked
                let mut gaps = vec![ClientIdGap::CookieAbsent];
                gaps.push(if info.is_ok() { ClientIdGap::TokenInfoNoId } else { ClientIdGap::TokenInfoFailed });
                gaps.push(gap);
                return refused_capture(app, client_id_failed(&gaps));
            }
        }
    }
    if sess.user_agent.is_empty() {
        let ua = match app.ws_home().cached_user_agent() {
            Ok(ua) => ua,
            Err(e) => {
                let err = format!("Could not read the cached browser user agent: {e}");
                set_error(app, &err);
                return Taken::NotAsked(err);
            }
        };
        if !ua.is_empty() {
            sess.user_agent = ua;
        }
    }
    // take the login over: rotate its refresh token now, so the copy the browser
    // holds goes stale instead of ours. Nothing is written until Wealthsimple has
    // answered with new tokens: a refused or unanswered capture leaves the saved
    // login as it was (`docs/plans/stage-money.md`, part C)
    let held = match held_tokens(app, &mut sess) {
        Ok(h) => h,
        Err(f) => return refused_capture(app, f),
    };
    let keep = |t: &bagholder_wealthsimple::session::Tokens| {
        let mut taken = sess.clone();
        adopt(&mut taken, t.clone());
        save_session(app, &taken).map_err(bagholder_broker::Failure::Refused)
    };
    match bagholder_wealthsimple::session::take_over(&app.net, &crate::broker_reads::session_file(app), &held, keep) {
        Ok(fresh) => adopt(&mut sess, fresh),
        Err(f) => return refused_capture(app, f.into()),
    }
    heard(app, Heard::Issued);
    {
        let mut st = app.state.lock().unwrap();
        st.capturing = false;
        st.error.clear();
    }
    // the first read with the new sign-in
    app.pull_asked.store(true, std::sync::atomic::Ordering::SeqCst);
    app.events.signal();
    Taken::Taken
}

/// A capture not taken over. Refused (the sign-in is not good): that token is not
/// tried again. Not asked (no answer, or the login could not be read or written):
/// tried again while the window is up. Either way the saved login stands as it was,
/// and `connected` is left to what the saved login is.
fn refused_capture(app: &Arc<App>, f: RefreshFailed) -> Taken {
    set_error(app, f.why());
    match f {
        RefreshFailed::Lapsed(why) => Taken::Refused(why),
        RefreshFailed::NotAsked(why) => Taken::NotAsked(why),
    }
}
