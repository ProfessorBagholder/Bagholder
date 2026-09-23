//! What the session needs of the clock, and the account rows the fetch layer
//! hands back reduced to what the store keeps.
//!
//! The pull itself lives in `crates/server/src/session.rs`'s `sync_body`; this
//! crate only reads and maps what Wealthsimple sends.

use rusqlite::Connection;

use bagholder_store::broker::Account;

use crate::session::Session;
use crate::wire::AccountNode;

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

pub fn expires_at_unix(sess: &Session) -> Option<f64> {
    match sess.expires_at.as_ref()? {
        crate::session::Expiry::Unix(f) => Some(*f),
        crate::session::Expiry::Text(s) => {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            if let Ok(f) = s.parse::<f64>() {
                return Some(f);
            }
            parse_instant(s)
        }
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
pub fn seconds_until_token_refresh(sess: &Session, now: f64) -> f64 {
    match expires_at_unix(sess) {
        None => 0.0,
        Some(exp) => (exp - TOKEN_REFRESH_MARGIN_SEC - now).max(0.0),
    }
}

pub fn token_refresh_needed(sess: &Session, now: f64) -> bool {
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

/// The custodian account id a Margin Boost
/// feature points at.
///
/// An account Wealthsimple lets back a margin account as collateral carries
/// the feature MARGIN_BOOST, and its metadata names the margin account's
/// custodian account.
pub fn margin_boost_target(acc: &AccountNode) -> String {
    for f in &acc.account_features {
        if f.name.to_uppercase() != "MARGIN_BOOST" {
            continue;
        }
        if !f.enabled || f.functional == Some(false) {
            continue;
        }
        return f.metadata.as_ref().map(|m| m.target_margin_account_id.clone()).unwrap_or_default();
    }
    String::new()
}

fn slim_account(acc: &AccountNode) -> Account {
    Account {
        id: acc.id.clone(),
        nickname: acc.nickname.clone(),
        unified_account_type: acc.unified_account_type.clone(),
        currency: acc.currency.clone(),
        status: acc.status.clone(),
        kind: acc.kind.clone(),
        net_liquidation_value: acc.financials.as_ref().and_then(|f| f.current_combined.as_ref()).and_then(|c| c.net_liquidation_value.as_ref()).and_then(|m| m.amount),
        margin_account_id: String::new(),
    }
}

/// The stored shape of every account, each
/// collateral account naming the margin account it backs.
pub fn slim_accounts(accounts: &[AccountNode]) -> Vec<Account> {
    // the custodian account id resolved back to the account that owns it
    let mut custodian: Vec<(String, String)> = Vec::new();
    for a in accounts {
        for c in &a.custodian_accounts {
            if !c.id.is_empty() {
                custodian.push((c.id.clone(), a.id.clone()));
            }
        }
    }
    let mut out = Vec::new();
    for a in accounts {
        let mut row = slim_account(a);
        let target = margin_boost_target(a);
        let backs = if target.is_empty() {
            String::new()
        } else {
            custodian.iter().find(|(k, _)| *k == target).map(|(_, v)| v.clone()).unwrap_or_default()
        };
        row.margin_account_id = backs;
        out.push(row);
    }
    out
}
