//! One pull from Wealthsimple.
//!
//! It only ever inserts rows the broker sent and the store has not seen, or
//! replaces one the broker itself revised. It never rebuilds the activity
//! table, and nothing Bagholder derives is written back into it.

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::fetch;
use crate::mapping;
use crate::session::{CallError, Client};
use bagholder_model::value::{field_s, get};

/// Refresh this far ahead of the stated
/// expiry rather than waiting for a call to be refused.
pub const TOKEN_REFRESH_MARGIN_SEC: f64 = 300.0;

/// A failure in words the page may see.
///
/// Anything that could carry a token is taken out before it can be shown; a
/// certificate failure is named plainly because it is the one the user can act
/// on.
pub fn public_sync_error(msg: &str) -> String {
    let msg = msg.replace('\n', " ");
    let msg = msg.trim();
    if msg.contains("CERTIFICATE_VERIFY_FAILED") || msg.contains("unable to get local issuer certificate") {
        return "could not verify HTTPS certificates".into();
    }
    let mut out = redact(msg);
    // collapse the whitespace
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 180 {
        out = out.chars().take(177).collect::<String>() + "...";
    }
    if out.is_empty() { "unknown error".into() } else { out }
}

/// The three substitutions a public sync error goes through, in order.
///
/// A failure reaches the page, so nothing that could be a token may survive
/// this. The last pass is the backstop: if either word is still there at all,
/// the word itself goes too.
fn redact(msg: &str) -> String {
    let mut out = redact_bearer(msg);
    out = redact_assignment(&out);
    let lower = out.to_lowercase();
    if lower.contains("bearer") || lower.contains("access_token") || lower.contains("refresh_token") {
        out = redact_words(&out);
    }
    out
}

/// `(?i)bearer\s+\S+` -> `[redacted]`, the value with it.
fn redact_bearer(msg: &str) -> String {
    let lower = msg.to_lowercase();
    let mut out = String::new();
    let b = msg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if lower[i..].starts_with("bearer") {
            let mut j = i + 6;
            let spaces = j;
            while j < b.len() && (b[j] as char).is_whitespace() {
                j += 1;
            }
            if j > spaces {
                let val = j;
                while j < b.len() && !(b[j] as char).is_whitespace() {
                    j += 1;
                }
                if j > val {
                    out.push_str("[redacted]");
                    i = j;
                    continue;
                }
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// `(?i)(access_token|refresh_token)\s*[:=]\s*\S+` -> `<name>=[redacted]`.
fn redact_assignment(msg: &str) -> String {
    let lower = msg.to_lowercase();
    let mut out = String::new();
    let b = msg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let name = ["access_token", "refresh_token"]
            .iter()
            .find(|n| lower[i..].starts_with(**n))
            .copied();
        if let Some(n) = name {
            let mut j = i + n.len();
            while j < b.len() && (b[j] as char).is_whitespace() {
                j += 1;
            }
            if j < b.len() && (b[j] == b':' || b[j] == b'=') {
                j += 1;
                while j < b.len() && (b[j] as char).is_whitespace() {
                    j += 1;
                }
                let val = j;
                while j < b.len() && !(b[j] as char).is_whitespace() {
                    j += 1;
                }
                if j > val {
                    // the name keeps the case it was written in, as \1 does
                    out.push_str(&msg[i..i + n.len()]);
                    out.push_str("=[redacted]");
                    i = j;
                    continue;
                }
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// `(?i)(bearer|access_token|refresh_token)` -> `[redacted]`: the backstop.
fn redact_words(msg: &str) -> String {
    let lower = msg.to_lowercase();
    let mut out = String::new();
    let b = msg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let hit = ["bearer", "access_token", "refresh_token"]
            .iter()
            .find(|n| lower[i..].starts_with(**n))
            .copied();
        if let Some(n) = hit {
            out.push_str("[redacted]");
            i += n.len();
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

pub fn expires_at_unix(sess: &Value) -> Option<f64> {
    let raw = get(sess, "expires_at")?;
    match raw {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => {
            let s = s.trim();
            if let Ok(f) = s.parse::<f64>() {
                return Some(f);
            }
            parse_instant(s)
        }
        _ => None,
    }
}

/// Seconds since the epoch for an ISO instant, with a `Z` or an offset.
fn parse_instant(s: &str) -> Option<f64> {
    let s = if s.ends_with('Z') { format!("{}+00:00", &s[..s.len() - 1]) } else { s.to_string() };
    let (d, t) = s.split_once('T')?;
    let (y, m, day) = bagholder_model::dates::parse_iso(d)?;
    let mut rest = t;
    let mut offset = 0.0_f64;
    if let Some(pos) = t.rfind(['+', '-']) {
        if pos > 0 {
            let sign = if t.as_bytes()[pos] == b'-' { -1.0 } else { 1.0 };
            let off = &t[pos + 1..];
            let (oh, om) = off.split_once(':').unwrap_or((off, "0"));
            offset = sign * (oh.parse::<f64>().unwrap_or(0.0) * 3600.0 + om.parse::<f64>().unwrap_or(0.0) * 60.0);
            rest = &t[..pos];
        }
    }
    let parts: Vec<&str> = rest.split(':').collect();
    let hh: f64 = parts.first()?.parse().ok()?;
    let mm: f64 = parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let ss: f64 = parts.get(2).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    Some(bagholder_model::dates::to_days(y, m, day) as f64 * 86400.0 + hh * 3600.0 + mm * 60.0 + ss - offset)
}

/// Seconds from `now` until the token should be refreshed; zero when it already should.
pub fn seconds_until_token_refresh(sess: &Value, now: f64) -> f64 {
    match expires_at_unix(sess) {
        None => 0.0,
        Some(exp) => (exp - TOKEN_REFRESH_MARGIN_SEC - now).max(0.0),
    }
}

pub fn token_refresh_needed(sess: &Value, now: f64) -> bool {
    match expires_at_unix(sess) {
        None => true,
        Some(exp) => now >= exp - TOKEN_REFRESH_MARGIN_SEC,
    }
}

/// The full history only when the activity
/// table has no rows yet.
pub fn activity_sync_bounds(conn: &Connection) -> rusqlite::Result<(Option<String>, bool)> {
    if bagholder_store::activities::activity_count(conn)? == 0 {
        return Ok((None, true));
    }
    let start = incremental_start_date(conn)?;
    Ok((if start.is_empty() { None } else { Some(start) }, false))
}

/// A daily pull asks for a fortnight, because a row
/// can be revised after it is first posted.
pub const PULL_OVERLAP_DAYS: i64 = 14;

pub fn incremental_start_date(conn: &Connection) -> rusqlite::Result<String> {
    // the newest row the broker sent, and only if it has none, the newest of
    // any source: an imported row must not shorten the window
    let pick = |sql: &str| -> Option<String> {
        conn.query_row(sql, [], |r| {
            let occurred: Option<String> = r.get(0)?;
            let day: Option<String> = r.get(1)?;
            Ok(occurred.filter(|s| !s.is_empty()).or(day))
        })
        .unwrap_or(None)
    };
    let newest = pick(
        "SELECT occurred_at, transaction_date FROM activities WHERE source = 'wealthsimple' ORDER BY COALESCE(occurred_at, transaction_date) DESC LIMIT 1",
    )
    .or_else(|| pick("SELECT occurred_at, transaction_date FROM activities ORDER BY COALESCE(occurred_at, transaction_date) DESC LIMIT 1"));
    let newest = match newest { Some(n) if !n.trim().is_empty() => n, _ => return Ok(String::new()) };
    let day: String = newest.trim().chars().take(10).collect();
    if bagholder_model::dates::parse_iso(&day).is_none() {
        return Ok(day);
    }
    Ok(bagholder_model::dates::shift_date(&day, -PULL_OVERLAP_DAYS))
}

pub struct SyncOutcome {
    pub inserted: usize,
    pub linked: usize,
    pub revised: usize,
    pub skipped: usize,
    pub accounts: usize,
    pub balances: usize,
    pub synced_at: String,
    pub email: String,
}

fn now_stamp(now_unix: i64) -> String {
    let days = now_unix.div_euclid(86400);
    let rem = now_unix.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// The custodian account id a Margin Boost
/// feature points at.
///
/// An account Wealthsimple lets back a margin account as collateral carries
/// the feature MARGIN_BOOST, and its metadata names the margin account's
/// custodian account.
pub fn margin_boost_target(acc: &Value) -> String {
    let features = match acc.get("accountFeatures").and_then(|v| v.as_array()) { Some(f) => f, None => return String::new() };
    for f in features {
        if !f.is_object() || field_s(f, "name").to_uppercase() != "MARGIN_BOOST" {
            continue;
        }
        let enabled = f.get("enabled").map(truthy).unwrap_or(false);
        if !enabled || f.get("functional") == Some(&Value::Bool(false)) {
            continue;
        }
        let md = f.get("metadata").filter(|m| m.is_object()).cloned().unwrap_or(Value::Null);
        return field_s(&md, "targetMarginAccountId");
    }
    String::new()
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

pub fn slim_account(acc: &Value) -> Value {
    let nlv = acc
        .get("financials")
        .and_then(|f| f.get("currentCombined"))
        .and_then(|c| c.get("netLiquidationValue"))
        .and_then(|m| m.get("amount"))
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "id": acc.get("id").cloned().unwrap_or(Value::Null),
        "nickname": field_s(acc, "nickname"),
        "unifiedAccountType": field_s(acc, "unifiedAccountType"),
        "currency": field_s(acc, "currency"),
        "status": field_s(acc, "status"),
        "type": field_s(acc, "type"),
        "netLiquidationValue": nlv,
    })
}

/// The stored shape of every account, each
/// collateral account naming the margin account it backs.
pub fn slim_accounts(accounts: &[Value]) -> Vec<Value> {
    // the custodian account id resolved back to the account that owns it
    let mut custodian: Vec<(String, String)> = Vec::new();
    for a in accounts {
        if !a.is_object() {
            continue;
        }
        if let Some(cs) = a.get("custodianAccounts").and_then(|v| v.as_array()) {
            for c in cs {
                let cid = field_s(c, "id");
                if c.is_object() && !cid.is_empty() {
                    custodian.push((cid, field_s(a, "id")));
                }
            }
        }
    }
    let mut out = Vec::new();
    for a in accounts {
        if !a.is_object() {
            continue;
        }
        let mut row = slim_account(a);
        let target = margin_boost_target(a);
        let backs = if target.is_empty() {
            String::new()
        } else {
            custodian.iter().find(|(k, _)| *k == target).map(|(_, v)| v.clone()).unwrap_or_default()
        };
        row.as_object_mut().unwrap().insert("marginAccountId".into(), json!(backs));
        out.push(row);
    }
    out
}

/// The pull, without the state flags the server keeps in
/// memory: one pull, and what it wrote.
///
/// The caller has already made sure the access token is fresh.
pub fn run_sync(
    client: &Client,
    conn: &Connection,
    sess: &Value,
    identity: &str,
    now_unix: i64,
    new_id: &dyn Fn() -> String,
) -> Result<SyncOutcome, CallError> {
    let accounts = fetch::fetch_all_accounts(client, sess, identity)?;
    let acc_by_id: Value = {
        let mut m = serde_json::Map::new();
        for a in &accounts {
            let id = field_s(a, "id");
            if !id.is_empty() {
                m.insert(id, a.clone());
            }
        }
        Value::Object(m)
    };

    let (start_date, _full) = activity_sync_bounds(conn).map_err(|e| CallError::Failed(e.to_string()))?;

    let mut mapped: Vec<Value> = Vec::new();
    for acc in &accounts {
        let aid = field_s(acc, "id");
        if aid.is_empty() {
            continue;
        }
        let items = fetch::fetch_activities_for_account(client, sess, &aid, start_date.as_deref(), now_unix)?;
        for it in &items {
            mapped.extend(mapping::map_activity_rows(it, Some(&acc_by_id)));
        }
    }
    // the CAD and USD sides of one account share a FIFO book
    let pools = mapping::fifo_pool_ids(Some(&Value::Array(accounts.clone())));
    for row in mapped.iter_mut() {
        let aid = field_s(row, "accountId");
        let pool = pools.get(&aid).cloned().unwrap_or(aid);
        if let Value::Object(m) = row {
            m.insert("fifoId".into(), json!(pool));
        }
    }

    let ids: Vec<String> = accounts.iter().map(|a| field_s(a, "id")).filter(|i| !i.is_empty()).collect();
    let balances = fetch::fetch_balances(client, sess, &ids)?;

    let applied = bagholder_store::merge::apply_wealthsimple_mapped(conn, &mapped, new_id)
        .map_err(|e| CallError::Failed(e.to_string()))?;

    let synced = now_stamp(now_unix);
    let slim = slim_accounts(&accounts);
    bagholder_store::tables::replace_accounts(conn, &slim).map_err(|e| CallError::Failed(e.to_string()))?;
    bagholder_store::tables::replace_balances(conn, &balances).map_err(|e| CallError::Failed(e.to_string()))?;
    bagholder_store::tables::set_meta(conn, "synced_at", &synced).map_err(|e| CallError::Failed(e.to_string()))?;

    Ok(SyncOutcome {
        inserted: applied.inserted,
        linked: applied.linked,
        revised: applied.revised,
        skipped: applied.skipped,
        accounts: slim.len(),
        balances: balances.len(),
        synced_at: synced,
        email: field_s(sess, "email"),
    })
}
