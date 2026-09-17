//! The Wealthsimple session: the saved tokens, the refresh grant, and the
//! GraphQL call that every fetch goes through.
//!
//! Nothing here ever puts a token, a client id or a raw response body into a
//! message the page can see. Only the short OAuth `error` field is reported,
//! and only when it looks like one.

use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use bagholder_market::client::{request, Error as HttpError};
use bagholder_model::value::{field_s, get};

pub const GRAPHQL: &str = "https://my.wealthsimple.com/graphql";
pub const GRAPHQL_VERSION: &str = "12";
pub const WS_CLIENT: &str = "@wealthsimple/wealthsimple";
pub const OAUTH: &str = "https://api.production.wealthsimple.com/v1/oauth/v2";

/// Where GraphQL and OAuth calls go. `BAGHOLDER_WS_BASE` points both at a
/// stand-in server, for testing the client against recorded answers.
pub fn graphql_url() -> String {
    match std::env::var("BAGHOLDER_WS_BASE") {
        Ok(b) if !b.is_empty() => format!("{}/graphql", b.trim_end_matches('/')),
        _ => GRAPHQL.to_string(),
    }
}

pub fn oauth_url() -> String {
    match std::env::var("BAGHOLDER_WS_BASE") {
        Ok(b) if !b.is_empty() => format!("{}/oauth", b.trim_end_matches('/')),
        _ => OAUTH.to_string(),
    }
}

pub const REFUSED_LOGIN_MESSAGE: &str = "Saved login refused. Connect Wealthsimple again.";

/// `bagholder.IDENTITY_KEYS`.
const IDENTITY_KEYS: [&str; 6] = [
    "identity_canonical_id",
    "identityCanonicalId",
    "canonical_id",
    "identity_id",
    "resource_owner_id",
    "sub",
];

/// Refreshes run one at a time. Wealthsimple rotates the refresh token on
/// every grant, so two callers posting the same one would leave the loser with
/// `invalid_grant` and the login dead.
static REFRESH_LOCK: Mutex<()> = Mutex::new(());
static REFUSED: Mutex<Option<String>> = Mutex::new(None);

pub struct Home {
    pub dir: PathBuf,
}

impl Home {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Home { dir: dir.as_ref().to_path_buf() }
    }
    pub fn session_path(&self) -> PathBuf { self.dir.join("session.json") }
    pub fn client_id_path(&self) -> PathBuf { self.dir.join("client_id") }
    pub fn user_agent_path(&self) -> PathBuf { self.dir.join("user_agent") }

    /// `bagholder.load_session`.
    pub fn load_session(&self) -> Option<Value> {
        let text = std::fs::read_to_string(self.session_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// `bagholder.save_session`, written the way `_atomic_write` does: a
    /// private temporary file renamed over the old one, so a crash cannot
    /// leave half a session behind.
    pub fn save_session(&self, sess: &Value) -> std::io::Result<()> {
        let body = serde_json::to_string_pretty(sess).unwrap_or_default();
        atomic_write(&self.session_path(), body.as_bytes(), 0o600)
    }

    /// `bagholder.delete_session_and_book`: the login only. The stored
    /// activity rows stay.
    pub fn delete_session(&self) {
        let _ = std::fs::remove_file(self.session_path());
        *REFUSED.lock().unwrap() = None;
    }

    pub fn cached_client_id(&self) -> String {
        std::fs::read_to_string(self.client_id_path()).unwrap_or_default().trim().to_string()
    }

    pub fn save_client_id(&self, cid: &str) {
        let _ = atomic_write(&self.client_id_path(), cid.as_bytes(), 0o600);
    }

    /// `bagholder.cached_user_agent`: the session's own, else the file.
    pub fn cached_user_agent(&self) -> String {
        if let Some(s) = self.load_session() {
            let v = field_s(&s, "user_agent").trim().to_string();
            if !v.is_empty() {
                return v;
            }
        }
        std::fs::read_to_string(self.user_agent_path()).unwrap_or_default().trim().to_string()
    }

    /// `bagholder.save_user_agent`.
    pub fn save_user_agent(&self, ua: &str) {
        if !ua.is_empty() {
            let _ = atomic_write(&self.user_agent_path(), ua.as_bytes(), 0o600);
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// `bagholder._atomic_write`.
fn atomic_write(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        set_mode(dir, 0o700);
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        set_mode(&tmp, mode);
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    set_mode(path, mode);
    Ok(())
}

/// `bagholder._oauth_error_code`: the short OAuth `error` field and nothing
/// else -- never a token, a client id, or a raw body.
pub fn oauth_error_code(data: &Value) -> String {
    let err = match get(data, "error") { Some(Value::String(s)) => s.trim().to_string(), _ => return String::new() };
    if err.is_empty() {
        return String::new();
    }
    // a long hex string is an identifier, not an error name
    if err.len() >= 32 && err.bytes().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    if err.len() > 64 || !err.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'-') {
        return String::new();
    }
    err
}

/// `bagholder._refresh_failure_message`.
pub fn refresh_failure_message(data: &Value) -> String {
    let status = get(data, "_http_status").and_then(|v| v.as_i64());
    let oauth_err = oauth_error_code(data);
    if oauth_err == "invalid_grant" {
        return REFUSED_LOGIN_MESSAGE.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = status {
        parts.push(format!("Wealthsimple token refresh HTTP {}", s));
    }
    if !oauth_err.is_empty() {
        parts.push(oauth_err);
    }
    if parts.is_empty() { "Wealthsimple token refresh failed".into() } else { parts.join(" ") }
}

/// `bagholder._expires_at_as_timestamp`: the expiry kept in the same shape the
/// cookie uses.
pub fn expires_at_as_timestamp(data: &Value, now_unix: f64) -> Option<String> {
    match get(data, "expires_at") {
        Some(Value::String(s)) if s.contains('T') => return Some(s.trim().to_string()),
        Some(Value::Number(n)) => {
            let unix = n.as_f64()?;
            return Some(stamp(unix));
        }
        _ => {}
    }
    // Python reads this with `int(...)`, which takes the string form too and
    // truncates a float toward zero.
    let raw = get(data, "expires_in")?;
    let expires_in = bagholder_model::value::num(Some(raw), f64::NAN);
    if expires_in.is_nan() {
        return None;
    }
    Some(stamp(now_unix + expires_in.trunc()))
}

fn stamp(unix: f64) -> String {
    let secs = unix as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// `bagholder._identity_from`.
pub fn identity_from(obj: &Value) -> String {
    for k in IDENTITY_KEYS {
        if let Some(v) = get(obj, k) {
            let s = bagholder_model::value::s(Some(v));
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

/// `bagholder.client_id_from_token_info`: the OAuth application uid that
/// issued these tokens.
pub fn client_id_from_token_info(info: &Value) -> String {
    let uid = field_s(info, "application_uid");
    if !uid.trim().is_empty() {
        return uid.trim().to_string();
    }
    info.get("application")
        .filter(|a| a.is_object())
        .map(|a| field_s(a, "uid").trim().to_string())
        .unwrap_or_default()
}

fn headers_for(sess: &Value, extra: &[(&str, String)], ua: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![("Accept".into(), "application/json".into())];
    if !ua.is_empty() {
        out.push(("User-Agent".into(), ua.to_string()));
    }
    // `bagholder._ws_session_headers`
    let wssdi = field_s(sess, "wssdi");
    if !wssdi.is_empty() {
        out.push(("x-ws-device-id".into(), wssdi));
    }
    let sid = field_s(sess, "session_id");
    if !sid.is_empty() {
        out.push(("x-ws-session-id".into(), sid));
    }
    for (k, v) in extra {
        out.push(((*k).to_string(), v.clone()));
    }
    out
}

/// `bagholder._http_json`: the parsed body, with the HTTP status folded in
/// under `_http_status` when the call failed, exactly as Python returns it.
pub fn http_json(method: &str, url: &str, body: Option<&Value>, headers: &[(String, String)]) -> Value {
    http_json_timeout(method, url, body, headers, 60)
}

/// The same, with the call's own timeout. An HTTP error answers with its
/// body and `_http_status`, as Python's `HTTPError` branch does; a failure to
/// reach the host at all is `transport`.
pub fn http_json_timeout(method: &str, url: &str, body: Option<&Value>, headers: &[(String, String)], timeout_sec: u64) -> Value {
    let mut hdrs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let payload = body.map(|b| serde_json::to_vec(b).unwrap_or_default());
    if payload.is_some() {
        hdrs.retain(|(k, _)| !k.eq_ignore_ascii_case("Content-Type"));
        hdrs.push(("Content-Type", "application/json"));
    }
    match bagholder_market::client::request_any(method, url, &hdrs, payload.as_deref(), Duration::from_secs(timeout_sec)) {
        Ok(resp) => {
            let text = resp.text();
            if resp.status >= 400 {
                let mut parsed = if text.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({"error": format!("http_{}", resp.status)}))
                };
                if !parsed.is_object() || parsed.as_object().map(|m| m.is_empty()).unwrap_or(false) {
                    if !parsed.is_object() {
                        parsed = json!({});
                    }
                }
                parsed["_http_status"] = json!(resp.status);
                return parsed;
            }
            if text.is_empty() {
                return json!({});
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => v,
                Err(_) => json!({"error": "invalid_json", "_http_status": resp.status}),
            }
        }
        Err(e) => json!({"error": "transport", "_message": e.to_string()}),
    }
}

pub struct Client<'a> {
    pub home: &'a Home,
}

#[derive(Debug)]
pub enum CallError {
    NotAuthorized,
    Graphql(String),
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::NotAuthorized => write!(f, "not authorized"),
            CallError::Graphql(m) => write!(f, "{}", m),
            CallError::Failed(m) => write!(f, "{}", m),
        }
    }
}

impl<'a> Client<'a> {
    /// `bagholder.client_id_for`: the session's own, else the cached one.
    pub fn client_id_for(&self, sess: &Value) -> String {
        let cid = field_s(sess, "client_id").trim().to_string();
        if !cid.is_empty() {
            self.home.save_client_id(&cid);
            return cid;
        }
        self.home.cached_client_id()
    }

    /// `bagholder.refresh_session`: the refresh-token grant, with no
    /// Authorization header.
    ///
    /// Under the lock the session on disk is read again, and when another
    /// caller has rotated it meanwhile that session is adopted and nothing is
    /// posted. A caller holding a login newer than the file passes `adopt`
    /// false.
    pub fn refresh_session(&self, sess: &mut Value, adopt: bool) -> Result<(), String> {
        let rt = field_s(sess, "refresh_token");
        if rt.is_empty() {
            return Err("missing refresh token".into());
        }
        let _guard = REFRESH_LOCK.lock().unwrap();
        if adopt {
            if let Some(current) = self.home.load_session() {
                let has = !field_s(&current, "access_token").is_empty() && !field_s(&current, "refresh_token").is_empty();
                if has && field_s(&current, "refresh_token") != rt {
                    if let (Value::Object(dst), Value::Object(src)) = (&mut *sess, &current) {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                    return Ok(());
                }
            }
        }
        if REFUSED.lock().unwrap().as_deref() == Some(rt.as_str()) {
            return Err(REFUSED_LOGIN_MESSAGE.into());
        }

        let cid = self.client_id_for(sess);
        if cid.is_empty() {
            return Err("session has no client id".into());
        }
        let body = json!({"grant_type": "refresh_token", "refresh_token": rt, "client_id": cid});
        let headers = headers_for(
            sess,
            &[
                ("x-wealthsimple-client", WS_CLIENT.to_string()),
                ("x-ws-profile", "invest".to_string()),
            ],
            &self.home.cached_user_agent(),
        );
        let data = http_json("POST", &format!("{}/token", oauth_url()), Some(&body), &headers);
        let access = field_s(&data, "access_token");
        if access.is_empty() {
            if oauth_error_code(&data) == "invalid_grant" {
                *REFUSED.lock().unwrap() = Some(rt);
            }
            return Err(refresh_failure_message(&data));
        }
        let m = sess.as_object_mut().ok_or("session is not an object")?;
        m.insert("access_token".into(), json!(access));
        let new_rt = field_s(&data, "refresh_token");
        if !new_rt.is_empty() {
            m.insert("refresh_token".into(), json!(new_rt));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if let Some(stamped) = expires_at_as_timestamp(&data, now) {
            m.insert("expires_at".into(), json!(stamped));
        }
        m.insert("client_id".into(), json!(cid));
        let _ = self.home.save_session(sess);
        Ok(())
    }

    /// `bagholder.token_info`.
    pub fn token_info(&self, sess: &Value) -> Value {
        let token = field_s(sess, "access_token");
        if token.is_empty() {
            return json!({});
        }
        let headers = headers_for(
            sess,
            &[
                ("Authorization", format!("Bearer {}", token)),
                ("x-wealthsimple-client", WS_CLIENT.to_string()),
            ],
            &self.home.cached_user_agent(),
        );
        let data = http_json("GET", &format!("{}/token/info", oauth_url()), None, &headers);
        match get(&data, "_http_status").and_then(|v| v.as_i64()) {
            Some(401) | Some(403) => json!({}),
            _ => data,
        }
    }

    /// `bagholder.graphql`.
    pub fn graphql(&self, sess: &Value, operation: &str, variables: &Value, query: Option<&str>) -> Result<Value, CallError> {
        let token = field_s(sess, "access_token");
        let mut extra: Vec<(&str, String)> = vec![
            ("Authorization", format!("Bearer {}", token)),
            ("x-wealthsimple-client", WS_CLIENT.to_string()),
            ("x-ws-profile", "trade".to_string()),
            ("x-ws-api-version", GRAPHQL_VERSION.to_string()),
            ("x-ws-locale", "en-CA".to_string()),
            ("x-platform-os", "web".to_string()),
            ("Content-Type", "application/json".to_string()),
            ("Origin", "https://my.wealthsimple.com".to_string()),
            ("Referer", "https://my.wealthsimple.com/app/trade".to_string()),
        ];
        extra.retain(|(k, v)| !(*k == "Authorization" && v == "Bearer "));
        let headers = headers_for(sess, &extra, &self.home.cached_user_agent());

        let q = match query {
            Some(q) => q.to_string(),
            None => crate::queries::query(operation)
                .ok_or_else(|| CallError::Failed(format!("unknown operation {}", operation)))?
                .to_string(),
        };
        // a variable that is absent is not sent at all, as Python's filter does
        let vars: Map<String, Value> = variables
            .as_object()
            .map(|m| m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let body = json!({"operationName": operation, "query": q, "variables": Value::Object(vars)});

        let data = http_json_timeout("POST", &graphql_url(), Some(&body), &headers, 90);
        match get(&data, "_http_status").and_then(|v| v.as_i64()) {
            Some(401) | Some(403) => return Err(CallError::NotAuthorized),
            _ => {}
        }
        if let Some(errs) = data.get("errors") {
            let truthy = match errs { Value::Null => false, Value::Array(a) => !a.is_empty(), Value::Object(m) => !m.is_empty(), Value::String(s) => !s.is_empty(), Value::Bool(b) => *b, Value::Number(n) => n.as_f64() != Some(0.0) };
            if truthy {
                let first = errs.as_array().and_then(|a| a.first()).cloned().unwrap_or_else(|| errs.clone());
                let msg = if first.is_object() {
                    let m = field_s(&first, "message");
                    if !m.is_empty() { m } else {
                        let e = field_s(&first, "error");
                        if e.is_empty() { first.to_string() } else { e }
                    }
                } else {
                    bagholder_model::value::s(Some(&first))
                };
                return Err(CallError::Graphql(format!("{}: {}", operation, msg)));
            }
        }
        match data.get("data") {
            Some(d) if !d.is_null() => Ok(d.clone()),
            _ => Err(CallError::Failed(format!("graphql failed: {}", operation))),
        }
    }
}
