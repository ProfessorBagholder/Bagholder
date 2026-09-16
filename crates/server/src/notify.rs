//! `notify.py`'s settings: which kinds of event the person is told about, and
//! how they can be delivered from here.
//!
//! Every kind is off until it is turned on from the menu.

use rusqlite::{Connection, Result};
use serde_json::{json, Map, Value};

/// `notify.KINDS`.
pub const KINDS: [&str; 6] = ["fills", "problems", "connection", "updates", "releases", "disclosures"];

/// The Releases and Disclosures kinds are each chosen by the tickers they
/// cover, a set per switch.
pub const RELEASE_SCOPES: [&str; 3] = ["releasesHeld", "releasesWatched", "releasesAll"];
pub const DISCLOSURE_SCOPES: [&str; 3] = ["disclosuresHeld", "disclosuresWatched", "disclosuresAll"];

/// `notify.SETTING_KEYS`, in order.
pub fn setting_keys() -> Vec<&'static str> {
    let mut out = vec!["fills", "problems", "connection", "updates"];
    out.extend(RELEASE_SCOPES);
    out.extend(DISCLOSURE_SCOPES);
    out
}

pub const SETTINGS_KEY: &str = "notify_settings";
pub const MODE_ENV: &str = "BAGHOLDER_NOTIFY";

/// `notify.settings`.
pub fn settings(conn: &Connection) -> Result<Map<String, Value>> {
    let raw = bagholder_store::tables::get_meta(conn, SETTINGS_KEY, "")?;
    let parsed: Value = serde_json::from_str(if raw.is_empty() { "{}" } else { &raw }).unwrap_or_else(|_| json!({}));
    let m = parsed.as_object().cloned().unwrap_or_default();
    let mut out = Map::new();
    for k in setting_keys() {
        out.insert(k.into(), json!(m.get(k).map(truthy).unwrap_or(false)));
    }
    Ok(out)
}

pub fn save_settings(conn: &Connection, patch: &Value) -> Result<Map<String, Value>> {
    let mut current = settings(conn)?;
    if let Some(p) = patch.as_object() {
        for k in setting_keys() {
            if let Some(v) = p.get(k) {
                current.insert(k.into(), json!(truthy(v)));
            }
        }
    }
    bagholder_store::tables::set_meta(
        conn,
        SETTINGS_KEY,
        &bagholder_store::tables::py_json(&Value::Object(current.clone())),
    )?;
    Ok(current)
}

/// `notify.native_channel`: the system's own notifications from this process,
/// when the computer has a desktop to show them on; empty where the page is
/// the only way.
pub fn native_channel() -> String {
    let mode = std::env::var(MODE_ENV).unwrap_or_default().trim().to_lowercase();
    if ["browser", "off", "0", "none"].contains(&mode.as_str()) {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        return if which("osascript") { "mac".into() } else { String::new() };
    }
    if cfg!(target_os = "windows") {
        return "windows".into();
    }
    if which("notify-send") {
        return "linux".into();
    }
    String::new()
}

fn which(name: &str) -> bool {
    let path = match std::env::var("PATH") { Ok(p) => p, Err(_) => return false };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

/// `notify.status`.
pub fn status(conn: &Connection) -> Result<Value> {
    let mut out = settings(conn)?;
    out.insert("native".into(), json!(native_channel()));
    out.insert("unread".into(), json!(bagholder_store::feeds::unread_notifications(conn)?));
    Ok(Value::Object(out))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
