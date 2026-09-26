//! The app's clock. Activities are pulled and read in America/Edmonton, so an
//! instant becomes a date and a time there, not in UTC.
//!
//! Every zone is read here and nowhere else. The rules are the IANA database the
//! system keeps current, with jiff's built-in copy where the system has none
//! (Windows, a slim container): the convention Python's zoneinfo and Go's time
//! follow. Tests pin
//! the built-in copy (`pinned-tzdb`), so a machine's database never changes an
//! expected figure.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};

/// `ACTIVITY_PULL_TZ`.
pub const TZ_NAME: &str = "America/Edmonton";

#[cfg(feature = "pinned-tzdb")]
fn db() -> &'static jiff::tz::TimeZoneDatabase {
    static DB: std::sync::OnceLock<jiff::tz::TimeZoneDatabase> = std::sync::OnceLock::new();
    DB.get_or_init(jiff::tz::TimeZoneDatabase::bundled)
}

#[cfg(not(feature = "pinned-tzdb"))]
fn db() -> &'static jiff::tz::TimeZoneDatabase {
    jiff::tz::db()
}

/// The UTC offset, in seconds, a named zone has at an instant, or nothing when
/// the zone is not one the database knows.
pub fn offset_at(zone: &str, unix: i64) -> Option<i64> {
    if zone.is_empty() {
        return None;
    }
    let tz = db().get(zone).ok()?;
    let at = jiff::Timestamp::from_second(unix).ok()?;
    Some(tz.to_offset(at).seconds() as i64)
}

/// The UTC offset, in seconds, of a wall-clock time in a named zone (seconds
/// since the epoch as if the wall clock were UTC). A time a change skips reads
/// with the offset after it, a time a change repeats with the one before it,
/// as `mktime` settles them.
pub fn offset_for_wall(zone: &str, wall: i64) -> Option<i64> {
    let first = offset_at(zone, wall)?;
    Some(offset_at(zone, wall - first).unwrap_or(first))
}

/// The local civil date and time for an instant, as seconds since the epoch.
fn local_parts(unix: i64) -> Option<(i64, u32, u32, u32, u32)> {
    let local = unix + offset_at(TZ_NAME, unix)?;
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    let (y, m, d) = crate::dates::from_days(days);
    Some((y, m, d, (secs / 3600) as u32, ((secs % 3600) / 60) as u32))
}

/// The civil day and minute an instant falls on in a named zone: `(days since
/// 1970-01-01, minute of the day)`.
pub fn civil_in(zone_name: &str, unix: i64) -> Option<(i64, u32)> {
    let local = unix + offset_at(zone_name, unix)?;
    Some((local.div_euclid(86400), (local.rem_euclid(86400) / 60) as u32))
}

/// `(day, minute of day, offset in seconds)` for an instant in a named zone.
/// Yahoo names the exchange's zone on every chart, and each bar needs its own
/// standard or daylight offset -- the `gmtoffset` the feed states is today's,
/// not the bar's.
pub fn local_at(zone: &str, unix: i64) -> Option<(String, i64, i64)> {
    let off = offset_at(zone, unix)?;
    let local = unix + off;
    let (y, m, d) = crate::dates::from_days(local.div_euclid(86400));
    Some((crate::dates::fmt(y, m, d), local.rem_euclid(86400) / 60, off))
}

/// Seconds from now until the local day turns.
pub fn seconds_until_local_midnight() -> u64 {
    let now = Utc::now().timestamp();
    let offset = offset_at(TZ_NAME, now).unwrap_or(0);
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
