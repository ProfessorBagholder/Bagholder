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

/// The tokens a refresh posts. The client id and whose login it is come from
/// the one reader (`read_tokens`) and the one discovery (`discover_client_id`).
fn held_tokens(app: &Arc<App>, sess: &mut Session) -> Result<bagholder_wealthsimple::session::Tokens, RefreshFailed> {
    let client_id = discover_client_id(app, sess, None, None)?;
    sess.client_id = client_id.clone();
    let identity = resolve_identity(sess);
    if identity.is_empty() {
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
        identity,
        expires_at: None,
    })
}

/// The identity the saved login names, under whichever accepted key, written
/// back as `identity_canonical_id` so every path stores it the same way.
fn resolve_identity(sess: &mut Session) -> String {
    let id = sess.identity();
    if !id.is_empty() {
        sess.ids.identity_canonical_id = id.clone();
    }
    id
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

fn nonempty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// One list, used by capture, boot and refresh. Stored first (the login, then
/// the cache file), then the sign-in cookie, then token info, then the login
/// script. The cache file is written only from the script, never from another source.
fn discover_client_id(app: &Arc<App>, sess: &Session, cookie: Option<&str>, info: Option<&bagholder_ws::session::TokenInfo>) -> Result<String, RefreshFailed> {
    // the id already on the login, before the cookie, token info, or the script
    if let Some(id) = nonempty(&sess.client_id) {
        return Ok(id);
    }
    match app.ws_home().cached_client_id() {
        Ok(c) => {
            if let Some(id) = nonempty(&c) {
                return Ok(id);
            }
        }
        Err(e) => return Err(RefreshFailed::NotAsked(format!("Could not read the cached Wealthsimple client id: {e}"))),
    }
    if let Some(id) = cookie.and_then(nonempty) {
        return Ok(id);
    }
    let fetched;
    let info = if sess.access_token.is_empty() {
        None
    } else if let Some(i) = info {
        Some(i)
    } else {
        fetched = token_info(app, sess);
        Some(&fetched)
    };
    if let Some(info) = info {
        let cid = bagholder_ws::session::client_id_from_token_info(info);
        if info.is_ok() && !cid.is_empty() {
            return Ok(cid);
        }
    }
    client_id_from_login_script(app)
}

/// What the login script said about the production client id.
pub(crate) enum IdInScript {
    Found(String),
    Missing,
    Ambiguous,
}

/// The production client id, from the one flat object that has `env` `production`
/// and `clientId` (or `client_id`) together. Two such objects, or the two fields
/// only inside an object that contains another, is ambiguous: nothing is guessed.
pub(crate) fn production_client_id(js: &str) -> IdInScript {
    let mut ids = Vec::new();
    for body in flat_object_bodies(js) {
        let fields = string_fields(&body);
        let production = fields.iter().any(|(k, v)| k == "env" && v == "production");
        let mut found = Vec::new();
        for (k, v) in &fields {
            if (k == "clientId" || k == "client_id") && is_client_id(v) {
                found.push(v.clone());
            }
        }
        if !production {
            continue;
        }
        if found.len() > 1 {
            return IdInScript::Ambiguous;
        }
        if let Some(id) = found.pop() {
            ids.push(id);
        }
    }
    match ids.len() {
        1 => IdInScript::Found(ids.remove(0)),
        0 => {
            // the words are in the script but not as one flat object: do not stitch them
            let mentions_id = ident_present(js, "clientId") || ident_present(js, "client_id");
            let mentions_prod = js.contains("\"production\"") || js.contains("'production'");
            if mentions_id && mentions_prod { IdInScript::Ambiguous } else { IdInScript::Missing }
        }
        _ => IdInScript::Ambiguous,
    }
}

fn is_client_id(s: &str) -> bool {
    s.len() >= 8 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

fn ident_present(js: &str, ident: &str) -> bool {
    let b = js.as_bytes();
    let n = ident.len();
    let mut i = 0;
    while i + n <= b.len() {
        if &b[i..i + n] == ident.as_bytes() {
            let before = i == 0 || !is_ident(b[i - 1]);
            let after = i + n == b.len() || !is_ident(b[i + n]);
            if before && after {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Interiors of `{...}` that contain no further `{...}` outside a string.
fn flat_object_bodies(js: &str) -> Vec<String> {
    let b = js.as_bytes();
    let mut out = Vec::new();
    let mut stack: Vec<(usize, bool)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => i = end_of_string(b, i),
            b'{' => {
                stack.push((i, false));
                i += 1;
            }
            b'}' => {
                if let Some((start, nested)) = stack.pop() {
                    if let Some(parent) = stack.last_mut() {
                        parent.1 = true;
                    }
                    if !nested && i > start + 1 {
                        out.push(js[start + 1..i].to_string());
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

fn end_of_string(b: &[u8], start: usize) -> usize {
    let q = b[start];
    let mut i = start + 1;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 2;
            continue;
        }
        if b[i] == q {
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// `key: "value"` fields of one flat object. A key is a whole identifier.
fn string_fields(body: &str) -> Vec<(String, String)> {
    let b = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b',' {
            i += 1;
            continue;
        }
        if !is_ident(b[i]) || b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < b.len() && is_ident(b[i]) {
            i += 1;
        }
        let key = body[start..i].to_string();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b':' {
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || (b[i] != b'"' && b[i] != b'\'') {
            continue;
        }
        let q = b[i];
        i += 1;
        let mut val = String::new();
        while i < b.len() && b[i] != q {
            if b[i] == b'\\' && i + 1 < b.len() {
                i += 1;
            }
            val.push(b[i] as char);
            i += 1;
        }
        if i < b.len() {
            i += 1;
        }
        out.push((key, val));
    }
    out
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

/// The production client id from the login script. A page that is not the app
/// is tried again; the cache is written only when the script names one id.
fn client_id_from_login_script(app: &Arc<App>) -> Result<String, RefreshFailed> {
    let home = app.ws_home();
    let ua = match home.cached_user_agent() {
        Ok(ua) => ua,
        Err(e) => return Err(RefreshFailed::NotAsked(format!("Could not read the cached browser user agent: {e}"))),
    };
    let hdrs: Vec<(&str, &str)> = if ua.is_empty() { vec![] } else { vec![("User-Agent", ua.as_str())] };
    let get = |url: &str| bagholder_net::client::request("GET", url, &hdrs, None, Duration::from_secs(20)).ok().map(|r| r.text());
    bagholder_net::client::named_call("wealthsimple", "web app/login");
    let html = match get(&bagholder_wealthsimple::hosts::login_url()) {
        Some(h) => h,
        None => return Err(RefreshFailed::NotAsked("Wealthsimple's login page could not be fetched. Try connecting again.".into())),
    };
    let Some(src) = app_script_from_login_html(&html) else {
        return Err(RefreshFailed::NotAsked("Wealthsimple's login page did not include the app script. Try connecting again.".into()));
    };
    bagholder_net::client::named_call("wealthsimple", "web app script");
    let js = match get(&absolute_script_url(&src)) {
        Some(j) => j,
        None => return Err(RefreshFailed::NotAsked("Wealthsimple's login script could not be fetched. Try connecting again.".into())),
    };
    match production_client_id(&js) {
        IdInScript::Found(id) => {
            if let Err(e) = home.save_client_id(&id) {
                set_error(app, &format!("Could not save the Wealthsimple client id: {e}"));
            }
            Ok(id)
        }
        IdInScript::Missing => Err(RefreshFailed::Lapsed("Wealthsimple's login script has no production client id.".into())),
        IdInScript::Ambiguous => Err(RefreshFailed::Lapsed("Wealthsimple's login script does not name one production client id.".into())),
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
    let info = if sess.access_token.is_empty() { None } else { Some(token_info(app, &sess)) };
    let info_ok = info.as_ref().is_some_and(|i| i.is_ok());
    if let Some(info) = info.as_ref().filter(|i| i.is_ok()) {
        if sess.identity().is_empty() {
            let id = info.identity();
            if !id.is_empty() {
                sess.ids.identity_canonical_id = id;
            }
        }
        if !info.email.is_empty() {
            sess.email = info.email.clone();
        }
        // Wealthsimple answered a request signed with the token held
        heard(app, Heard::Issued);
        app.state.lock().unwrap().error.clear();
    }
    if let Err(f) = settle_client_id(app, &mut sess, None, info.as_ref()) {
        // a failed discovery writes nothing: the saved login stays as it was
        if !info_ok {
            match f {
                RefreshFailed::Lapsed(why) => heard(app, Heard::Lapsed(why)),
                RefreshFailed::NotAsked(why) => heard(app, Heard::NotAsked(why)),
            }
        } else {
            set_error(app, f.why());
        }
        app.state.lock().unwrap().email = sess.email.clone();
        return;
    }
    resolve_identity(&mut sess);
    if info_ok {
        saved = save_session(app, &sess);
    } else if sess.refresh_token.is_empty() {
        heard(app, Heard::Lapsed("missing refresh token".into()));
    } else if refresh_session(app, &mut sess) {
        app.state.lock().unwrap().error.clear();
        match load_session(app) {
            Ok(Some(s)) => sess = s,
            Ok(None) => {}
            Err(e) => saved = Err(e),
        }
    }
    let mut st = app.state.lock().unwrap();
    if let Err(e) = saved {
        st.error = e;
    }
    st.email = sess.email.clone();
}

fn settle_client_id(app: &Arc<App>, sess: &mut Session, cookie: Option<&str>, info: Option<&bagholder_ws::session::TokenInfo>) -> Result<(), RefreshFailed> {
    sess.client_id = discover_client_id(app, sess, cookie, info)?;
    Ok(())
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
    if let Err(f) = settle_client_id(app, &mut sess, Some(capture.client_id.as_str()), Some(&info)) {
        return refused_capture(app, f);
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
