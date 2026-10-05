//! The Wealthsimple session: the saved login, the refresh grant, the sign-in's
//! capture, and Sync now. The reads themselves are the broker's (`broker_reads`).

use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use ts_rs::TS;

use bagholder_ws::session::{Client, Session};

use crate::app::{now_iso, now_unix, App};
use crate::http::OkOr;

pub const LOGIN_URL: &str = "https://my.wealthsimple.com/app/login";

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
/// means it holds a token Wealthsimple issued that has not lapsed
/// (`docs/plans/stage-money.md`, part C: one writer).
pub enum Heard {
    /// Wealthsimple issued a token, or answered a request signed with one.
    Issued,
    /// Wealthsimple refused the sign-in, or there is none: connect again.
    Lapsed(String),
    /// Wealthsimple could not be asked (no answer, a refusal of the request, not of
    /// the sign-in): connected stays while the token held has not expired.
    NotAsked(String),
}

/// The one writer of "connected", and of the error the sign-in leaves.
pub fn heard(app: &Arc<App>, h: Heard) {
    let held_live = || load_session(app).ok().flatten().is_some_and(|s| !s.access_token.is_empty() && !bagholder_ws::session::token_expired(&s, now_unix()));
    let live = matches!(h, Heard::NotAsked(_)) && held_live();
    let mut st = app.state.lock().unwrap();
    match h {
        Heard::Issued => st.connected = true,
        Heard::Lapsed(why) => {
            st.connected = false;
            st.error = why;
        }
        Heard::NotAsked(why) => {
            st.connected = st.connected && live;
            st.error = why;
        }
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

/// The tokens a refresh posts, from a login: its client id (the cached one where it
/// names none) and whose login it is.
fn held_tokens(app: &Arc<App>, sess: &mut Session) -> Result<bagholder_wealthsimple::session::Tokens, RefreshFailed> {
    let home = app.ws_home();
    let client_id = Client { home: &home }.client_id_for(sess).map_err(|e| RefreshFailed::NotAsked(format!("Could not read the cached Wealthsimple client id: {e}")))?;
    if client_id.is_empty() {
        return Err(RefreshFailed::Lapsed("The Wealthsimple login has no client id; connect again".into()));
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
        device: (!sess.wssdi.is_empty()).then(|| sess.wssdi.clone()),
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

/// The production client id from Wealthsimple's
/// login script, cached.
pub fn scrape_client_id(app: &Arc<App>) -> String {
    let home = app.ws_home();
    let cached = match home.cached_client_id() {
        Ok(c) => c,
        Err(e) => {
            set_error(app, &format!("Could not read the cached Wealthsimple client id: {e}"));
            return String::new();
        }
    };
    if !cached.is_empty() {
        return cached;
    }
    let ua = match home.cached_user_agent() {
        Ok(ua) => ua,
        Err(e) => {
            set_error(app, &format!("Could not read the cached browser user agent: {e}"));
            return String::new();
        }
    };
    let hdrs: Vec<(&str, &str)> = if ua.is_empty() { vec![] } else { vec![("User-Agent", ua.as_str())] };
    let get = |url: &str| bagholder_net::client::request("GET", url, &hdrs, None, Duration::from_secs(20)).ok().map(|r| r.text());
    let html = match get(LOGIN_URL) { Some(h) => h, None => return String::new() };
    let script = regex::Regex::new(r#"(?i)<script[^>]+src="([^"]*app-[a-f0-9]+\.js[^"]*)""#).unwrap();
    let mut js_url = match script.captures(&html) { Some(c) => c[1].to_string(), None => return String::new() };
    if js_url.starts_with("//") {
        js_url = format!("https:{}", js_url);
    } else if js_url.starts_with('/') {
        js_url = format!("https://my.wealthsimple.com{}", js_url);
    }
    let js = match get(&js_url) { Some(j) => j, None => return String::new() };
    match regex::Regex::new(r#"(?s)production:.*?clientId:"([a-f0-9]+)""#).unwrap().captures(&js) {
        Some(c) => {
            if let Err(e) = home.save_client_id(&c[1]) {
                set_error(app, &format!("Could not save the Wealthsimple client id: {e}"));
            }
            c[1].to_string()
        }
        None => String::new(),
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
        Ok(Some(s)) => s,
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
    if !sess.access_token.is_empty() {
        let info = token_info(app, &sess);
        if info.is_ok() {
            answered = true;
            apply_token_info_client_id(app, &mut sess, Some(&info));
            let identity = info.identity();
            if !identity.is_empty() && sess.identity().is_empty() {
                sess.ids.identity_canonical_id = identity;
            }
            if !info.email.is_empty() {
                sess.email = info.email.clone();
            }
            saved = save_session(app, &sess);
            // Wealthsimple answered a request signed with the token held
            heard(app, Heard::Issued);
            app.state.lock().unwrap().error.clear();
        }
    }
    if !answered {
        if sess.refresh_token.is_empty() {
            heard(app, Heard::Lapsed("missing refresh token".into()));
        } else if refresh_session(app, &mut sess) {
            app.state.lock().unwrap().error.clear();
            match load_session(app) {
                Ok(Some(s)) => sess = s,
                Ok(None) => {}
                Err(e) => saved = Err(e),
            }
        }
    }
    let mut st = app.state.lock().unwrap();
    if let Err(e) = saved {
        st.error = e;
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
        let cid = scrape_client_id(app);
        if !cid.is_empty() {
            sess.client_id = cid;
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
    match bagholder_wealthsimple::session::take_over(&app.net, &held, keep) {
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
