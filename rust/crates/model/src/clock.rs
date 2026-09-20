//! The app's clock. Activities are pulled and read in America/Edmonton, so an
//! instant becomes a date and a time there, not in UTC.
//!
//! The zone comes from the system's own tzdata, rather than from a snapshot compiled into this binary.
//! The rules move: Alberta's switch to permanent Central Standard Time during
//! 2026 is in the system database already, and a bundled copy a version behind
//! puts every winter fill an hour out.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use std::sync::OnceLock;

/// `ACTIVITY_PULL_TZ`.
pub const TZ_NAME: &str = "America/Edmonton";

fn zone() -> &'static Option<tz::TimeZone> {
    static ZONE: OnceLock<Option<tz::TimeZone>> = OnceLock::new();
    ZONE.get_or_init(|| tz::TimeZone::from_posix_tz(TZ_NAME).ok())
}

/// The local civil date and time for an instant, as seconds since the epoch.
fn local_parts(unix: i64) -> Option<(i64, u32, u32, u32, u32)> {
    let tz = zone().as_ref()?;
    let t = tz.find_local_time_type(unix).ok()?;
    let local = unix + t.ut_offset() as i64;
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    let (y, m, d) = crate::dates::from_days(days);
    Some((y, m, d, (secs / 3600) as u32, ((secs % 3600) / 60) as u32))
}

/// The civil day and minute an instant falls on in a named zone: `(days since
/// 1970-01-01, minute of the day)`. The zone's rules come from the system's tzdata.
pub fn civil_in(zone_name: &str, unix: i64) -> Option<(i64, u32)> {
    static ZONES: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<tz::TimeZone>>>> = OnceLock::new();
    let zones = ZONES.get_or_init(Default::default);
    let mut z = zones.lock().unwrap_or_else(|e| e.into_inner());
    let tz = z.entry(zone_name.to_string()).or_insert_with(|| tz::TimeZone::from_posix_tz(zone_name).ok()).as_ref()?;
    let local = unix + tz.find_local_time_type(unix).ok()?.ut_offset() as i64;
    Some((local.div_euclid(86400), (local.rem_euclid(86400) / 60) as u32))
}

/// Seconds from now until the local day turns.
pub fn seconds_until_local_midnight() -> u64 {
    let now = Utc::now().timestamp();
    let offset = zone().as_ref().and_then(|tz| tz.find_local_time_type(now).ok()).map(|t| t.ut_offset() as i64).unwrap_or(0);
    (86400 - (now + offset).rem_euclid(86400)) as u64
}

/// `when_parts`: an ISO instant becomes `(YYYY-MM-DD, HH:MM)` locally.
/// A bare date has no time, and an unreadable instant keeps its first ten
/// characters.
pub fn when_parts(occurred: &str) -> (String, String) {
    let s = occurred.trim();
    if s.is_empty() {
        return (String::new(), String::new());
    }
    if !s.contains('T') {
        return (s.chars().take(10).collect(), String::new());
    }
    let normalized = if s.ends_with('Z') { format!("{}+00:00", &s[..s.len() - 1]) } else { s.to_string() };
    let parsed: Option<DateTime<Utc>> = DateTime::<FixedOffset>::parse_from_rfc3339(&normalized)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            // No offset at all: read as naive and pinned to UTC.
            NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%.f"))
                .or_else(|_| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M"))
                .ok()
                .map(|n| Utc.from_utc_datetime(&n))
        });
    let fallback = || (s.chars().take(10).collect::<String>(), String::new());
    match parsed.and_then(|dt| local_parts(dt.timestamp())) {
        Some((y, m, d, hh, mm)) => (crate::dates::fmt(y, m, d), format!("{:02}:{:02}", hh, mm)),
        None => fallback(),
    }
}

/// `today_local`.
pub fn today_local() -> String {
    match local_parts(Utc::now().timestamp()) {
        Some((y, m, d, _, _)) => crate::dates::fmt(y, m, d),
        None => Utc::now().format("%Y-%m-%d").to_string(),
    }
}

/// The stamp `build_view` puts on its output: UTC, to the second.
pub fn now_utc_stamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// The same stamp, `days` back: what a prune keeps above.
pub fn stamp_days_ago(days: i64) -> String {
    (Utc::now() - chrono::Duration::days(days)).format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
