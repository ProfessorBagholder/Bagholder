//! Wealthsimple's session (`docs/plans/stage-3b-wealthsimple.md`, "The session"):
//! the tokens a sign-in left in the data folder's `session.json`, refreshed one at
//! a time. Wealthsimple accepts each refresh token once (OAuth 2.0 Security Best
//! Current Practice, RFC 9700 §4.14, rotation), so:
//! - a refresh runs under one lock, and reads the file again first: a token
//!   another caller already rotated is adopted, and nothing is posted;
//! - a refresh token Wealthsimple refused is never posted again;
//! - a refused refresh is a lapse: the session is no longer valid until the
//!   person signs in again.
//! Every other key the file holds is kept as it was.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use bagholder_broker::Failure;
use bagholder_core::json::{self, Value};
use bagholder_net::{Ask, Net};
use bagholder_sources::reply::{Node, Read};

pub const TOKEN_URL: &str = "https://api.production.wealthsimple.com/v1/oauth/v2/token";

/// What a request is signed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub client_id: String,
    /// The person's identity id, which the accounts are asked by.
    pub identity: String,
    pub expires_at: Option<jiff::Timestamp>,
}

/// The tokens a session file holds, read strictly: an access token, a refresh
/// token, a client id and an identity id (under either spelling the sign-in
/// writes it), and when the access token expires where it says.
pub fn read_tokens(v: &Value) -> Read<Tokens> {
    let n = Node::root(v);
    let identity = match n.field("identity_canonical_id") {
        Ok(f) => f.as_text()?,
        Err(_) => n.text("identityCanonicalId")?,
    };
    let expires_at = match n.field("expires_at") {
        Err(_) => None,
        Ok(f) => match f.value() {
            Value::Null => None,
            Value::Number(s) => Some(jiff::Timestamp::from_second(s.split('.').next().unwrap_or("").parse::<i64>().map_err(|e| f.mismatch(e.to_string()))?).map_err(|e| f.mismatch(e.to_string()))?),
            Value::String(s) => Some(s.parse().map_err(|e: jiff::Error| f.mismatch(e.to_string()))?),
            other => return Err(f.mismatch(format!("expected an instant, found {}", other.kind()))),
        },
    };
    Ok(Tokens { access: n.text("access_token")?.to_string(), refresh: n.text("refresh_token")?.to_string(), client_id: n.text("client_id")?.to_string(), identity: identity.to_string(), expires_at })
}

/// Refreshes run one at a time, in the process.
static REFRESH: Mutex<()> = Mutex::new(());
/// The refresh token Wealthsimple last refused: never posted again.
static REFUSED: Mutex<Option<String>> = Mutex::new(None);

/// The session file in the data folder.
pub struct SessionFile {
    pub path: PathBuf,
}

impl SessionFile {
    pub fn load(&self) -> Result<Option<(Value, Tokens)>, Failure> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Failure::Lapsed(format!("the saved sign-in could not be read: {e}"))),
        };
        let v = json::parse(&text).map_err(|e| Failure::Lapsed(format!("the saved sign-in is not JSON: {e}")))?;
        let t = read_tokens(&v).map_err(|m| Failure::Lapsed(format!("the saved sign-in: {m}")))?;
        Ok(Some((v, t)))
    }

    /// Write the tokens into the file, every other key kept, atomically and
    /// readable by the person only.
    pub fn save(&self, previous: &Value, t: &Tokens) -> Result<(), Failure> {
        let mut m: BTreeMap<String, Value> = match previous {
            Value::Object(m) => m.clone(),
            _ => BTreeMap::new(),
        };
        m.insert("access_token".into(), Value::String(t.access.clone()));
        m.insert("refresh_token".into(), Value::String(t.refresh.clone()));
        m.insert("client_id".into(), Value::String(t.client_id.clone()));
        if let Some(at) = t.expires_at {
            m.insert("expires_at".into(), Value::String(at.to_string()));
        }
        let text = Value::Object(m).canonical();
        let tmp = self.path.with_extension("json.new");
        let write = || -> std::io::Result<()> {
            std::fs::write(&tmp, text.as_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            }
            std::fs::rename(&tmp, &self.path)
        };
        write().map_err(|e| Failure::Lapsed(format!("the refreshed sign-in could not be saved: {e}")))
    }
}

/// A fresh access token for `held`: the file's where another caller already
/// rotated it; otherwise Wealthsimple's answer to the refresh token, saved.
pub fn refresh(net: &Net, file: &SessionFile, held: &Tokens) -> Result<Tokens, Failure> {
    let _one_at_a_time = REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    let (previous, on_disk) = file.load()?.ok_or_else(|| Failure::Lapsed("no saved sign-in".into()))?;
    if on_disk.refresh != held.refresh {
        // rotated by another caller while this one waited: adopt it, post nothing
        return Ok(on_disk);
    }
    if REFUSED.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(held.refresh.as_str()) {
        return Err(Failure::Lapsed("Wealthsimple refused the saved sign-in; sign in again".into()));
    }
    let mut body = BTreeMap::new();
    body.insert("grant_type".to_string(), Value::String("refresh_token".into()));
    body.insert("refresh_token".to_string(), Value::String(held.refresh.clone()));
    body.insert("client_id".to_string(), Value::String(held.client_id.clone()));
    let body = Value::Object(body).canonical();
    let headers = [("Content-Type", "application/json"), ("Accept", "application/json")];
    let reply = net.send(&Ask::post(TOKEN_URL, &headers, body.as_bytes())).map_err(|e| Failure::Unreachable(e.to_string()))?;
    let v = json::parse(&String::from_utf8_lossy(&reply.body)).map_err(|e| Failure::Mismatch(format!("the token answer is not JSON: {e}")))?;
    let n = Node::root(&v);
    if reply.status != 200 {
        let code = n.opt_text("error").ok().flatten().unwrap_or("");
        if code == "invalid_grant" || reply.status == 401 {
            *REFUSED.lock().unwrap_or_else(|e| e.into_inner()) = Some(held.refresh.clone());
            return Err(Failure::Lapsed(format!("Wealthsimple refused the saved sign-in ({code}); sign in again")));
        }
        return Err(Failure::Refused(format!("the token refresh answered {}: {code}", reply.status)));
    }
    let access = n.text("access_token").map_err(|m| Failure::Mismatch(m.to_string()))?.to_string();
    let refresh = n.text("refresh_token").map_err(|m| Failure::Mismatch(m.to_string()))?.to_string();
    let expires_in = n.int("expires_in").map_err(|m| Failure::Mismatch(m.to_string()))?;
    let expires_at = reply.received_at.checked_add(jiff::Span::new().seconds(expires_in)).ok();
    let fresh = Tokens { access, refresh, client_id: held.client_id.clone(), identity: held.identity.clone(), expires_at };
    file.save(&previous, &fresh)?;
    Ok(fresh)
}
