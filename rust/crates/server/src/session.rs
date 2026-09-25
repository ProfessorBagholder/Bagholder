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

pub fn load_session(app: &Arc<App>) -> Option<Session> {
    app.ws_home().load_session()
}

/// Writes the login; a failure is said in words the header can show.
pub fn save_session(app: &Arc<App>, sess: &Session) -> Result<(), String> {
    app.ws_home().save_session(sess).map_err(|e| format!("Could not save the Wealthsimple login: {}", e))
}

fn set_error(app: &Arc<App>, msg: &str) {
    app.state.lock().unwrap().error = msg.to_string();
}

/// The refresh grant. A failure says why on the header.
pub fn refresh_session(app: &Arc<App>, sess: &mut Session, adopt: bool) -> bool {
    match one_refresh(app, sess, adopt) {
        Ok(()) => true,
        Err(msg) => {
            set_error(app, &msg);
            false
        }
    }
}

/// Every refresh in the process goes through the adapter's session
/// (`bagholder_wealthsimple::session`): one lock, the file read again under it so
/// a token another caller rotated is adopted without a post, and a refresh token
/// Wealthsimple refused never posted again. The reads, the orders and the sign-in
/// share it, so two of them at once post a refresh token once
/// (`docs/plans/stage-3c-switch.md`, §3, "One session"). A login newer than the
/// file (`adopt` false, a sign-in just captured) is written to the file first.
fn one_refresh(app: &Arc<App>, sess: &mut Session, adopt: bool) -> Result<(), String> {
    let home = app.ws_home();
    let client_id = Client { home: &home }.client_id_for(sess);
    if client_id.is_empty() {
        return Err("session has no client id".into());
    }
    sess.client_id = client_id.clone();
    if !adopt {
        home.save_session(sess).map_err(|e| format!("Could not save the Wealthsimple login: {e}"))?;
    }
    if sess.identity().is_empty() {
        // the adapter asks for the accounts by it: a login without it cannot be read
        return Err("Wealthsimple did not say whose login this is; connect again".into());
    }
    let held = bagholder_wealthsimple::session::Tokens {
        access: sess.access_token.clone(),
        refresh: sess.refresh_token.clone(),
        client_id,
        identity: sess.identity(),
        expires_at: None,
    };
    if held.refresh.is_empty() {
        return Err("missing refresh token".into());
    }
    let file = bagholder_wealthsimple::session::SessionFile { path: home.session_path() };
    let fresh = bagholder_wealthsimple::session::refresh(&app.net, &file, &held).map_err(|f| match f {
        bagholder_broker::Failure::Lapsed(why) => why,
        other => other.to_string(),
    })?;
    sess.access_token = fresh.access;
    sess.refresh_token = fresh.refresh;
    sess.client_id = fresh.client_id;
    if let Some(at) = fresh.expires_at {
        sess.expires_at = Some(bagholder_ws::session::Expiry::Text(at.to_string()));
    }
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
    app.ws_home().save_client_id(&cid);
    cid
}

/// The production client id from Wealthsimple's
/// login script, cached.
pub fn scrape_client_id(app: &Arc<App>) -> String {
    let home = app.ws_home();
    let cached = home.cached_client_id();
    if !cached.is_empty() {
        return cached;
    }
    let ua = home.cached_user_agent();
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
            home.save_client_id(&c[1]);
            c[1].to_string()
        }
        None => String::new(),
    }
}

/// Refresh ahead of the expiry. Connected means
/// this grant produced a new token.
pub fn ensure_fresh_token(app: &Arc<App>, sess: Option<Session>) -> bool {
    let mut sess = match sess.or_else(|| load_session(app)) {
        Some(s) if !s.refresh_token.is_empty() => s,
        _ => {
            let mut st = app.state.lock().unwrap();
            st.connected = false;
            st.error = "missing refresh token".into();
            return false;
        }
    };
    let connected = app.state.lock().unwrap().connected;
    if connected && !bagholder_ws::sync::token_refresh_needed(&sess, now_unix()) {
        return true;
    }
    let ok = refresh_session(app, &mut sess, true);
    let mut st = app.state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    } else if st.error.trim().is_empty() {
        st.error = "Wealthsimple token refresh failed".into();
    }
    ok
}

/// The login only; stored rows stay.
pub fn delete_session(app: &Arc<App>) {
    app.ws_home().delete_session();
    let mut st = app.state.lock().unwrap();
    st.connected = false;
    st.email.clear();
    st.last_sync.clear();
    st.capturing = false;
    st.error.clear();
    st.portfolio_error.clear();
}

pub fn boot_session(app: &Arc<App>) {
    let conn = match app.open() {
        Ok(c) => c,
        Err(e) => {
            set_error(app, &format!("Could not open the database: {}", e));
            return;
        }
    };
    let mut sess = match load_session(app) {
        Some(s) => s,
        None => {
            let mut st = app.state.lock().unwrap();
            st.connected = false;
            st.last_sync = bagholder_store::tables::get_meta(&conn, "synced_at", "").unwrap_or_default();
            return;
        }
    };
    let mut info_ok = false;
    let mut saved = Ok(());
    if !sess.access_token.is_empty() {
        let info = token_info(app, &sess);
        info_ok = info.is_ok();
        if info_ok {
            apply_token_info_client_id(app, &mut sess, Some(&info));
            let identity = info.identity();
            if !identity.is_empty() && sess.identity().is_empty() {
                sess.ids.identity_canonical_id = identity;
            }
            if !info.email.is_empty() {
                sess.email = info.email.clone();
            }
            saved = save_session(app, &sess);
        }
    }
    let mut ok = info_ok;
    if !ok && !sess.refresh_token.is_empty() {
        ok = refresh_session(app, &mut sess, true);
        sess = load_session(app).unwrap_or(sess);
    }
    let mut st = app.state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    } else if st.error.trim().is_empty() {
        st.error = if sess.refresh_token.is_empty() { "missing refresh token".into() } else { "Wealthsimple token refresh failed".into() };
    }
    if let Err(e) = saved {
        st.error = e;
    }
    st.email = sess.email.clone();
    match bagholder_store::tables::get_meta(&conn, "synced_at", "") {
        Ok(v) => st.last_sync = v,
        Err(e) => st.error = format!("Could not read the database: {}", e),
    }
}

/// Told once per expiry.
pub fn note_session_expired(app: &Arc<App>) {
    let was = {
        let mut st = app.state.lock().unwrap();
        let was = st.connected;
        st.connected = false;
        st.error = "Session expired. Connect again.".into();
        was
    };
    if was {
        if let Ok(conn) = app.open() {
            crate::notify::emit(app, &conn, "connection", &format!("session:{}", now_iso()), "Sign in needed", "The Wealthsimple session expired. Connect again from the menu.", None);
        }
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
    if load_session(app).is_none() {
        return SyncAnswer { ok: false, error: Some("not connected".into()), syncing: None };
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
        Some(s) if !s.refresh_token.is_empty() => s,
        _ => {
            let mut st = app.state.lock().unwrap();
            st.connected = false;
            st.error = "not connected".into();
            return RefreshAnswer { ok: false, error: "not connected".into(), connected: false };
        }
    };
    let ok = refresh_session(app, &mut sess, true);
    let mut st = app.state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    }
    RefreshAnswer { ok, error: st.error.trim().to_string(), connected: ok }
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
    let has_login = |app: &Arc<App>| load_session(app).map_or(false, |s| !s.refresh_token.is_empty());
    let mut retry: Option<Duration> = None;
    loop {
        let now = now_unix();
        let connected = app.state.lock().unwrap().connected;
        let login = has_login(&app);
        let sleep = match retry {
            Some(d) => d,
            None if !login => Duration::MAX, // nothing to keep fresh until someone signs in
            None => Duration::from_secs_f64(load_session(&app).map_or(0.0, |s| bagholder_ws::sync::seconds_until_token_refresh(&s, now)).max(0.0)),
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
        if !has_login(&app) {
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

/// Keep the captured login and take it over.
pub fn capture_tokens(app: &Arc<App>, capture: &Capture) -> OkOr {
    let capture = capture.clone();
    if capture.access_token.is_empty() {
        return OkOr::err("missing access_token");
    }
    let mut sess = load_session(app).unwrap_or_default();
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
        return OkOr::err(err);
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
        let ua = app.ws_home().cached_user_agent();
        if !ua.is_empty() {
            sess.user_agent = ua;
        }
    }
    // take the login over: rotate its refresh token now, so the copy the
    // browser holds goes stale instead of ours
    if !refresh_session(app, &mut sess, false) {
        let mut st = app.state.lock().unwrap();
        st.connected = false;
        let err = if st.error.is_empty() { "Wealthsimple refused the captured login".to_string() } else { st.error.clone() };
        return OkOr::err(err);
    }
    if let Err(e) = save_session(app, &sess) {
        app.state.lock().unwrap().error = e.clone();
        return OkOr::err(e);
    }
    {
        let mut st = app.state.lock().unwrap();
        st.connected = true;
        st.capturing = false;
        st.error.clear();
    }
    // the first read with the new sign-in
    app.pull_asked.store(true, std::sync::atomic::Ordering::SeqCst);
    app.events.signal();
    OkOr::ok()
}
