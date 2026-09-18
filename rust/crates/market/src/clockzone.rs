//! A named time zone, read from the system's own tzdata.
//!
//! Yahoo names the exchange's zone on every chart, and each bar needs its own
//! standard or daylight offset -- the `gmtoffset` the feed states is today's,
//! not the bar's.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn cache() -> &'static Mutex<HashMap<String, Option<tz::TimeZone>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<tz::TimeZone>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `(day, minute of day, offset in seconds)` for an instant in a named zone,
/// or nothing when the zone is not one the system knows.
pub fn local_at(zone: &str, unix: i64) -> Option<(String, i64, i64)> {
    if zone.is_empty() {
        return None;
    }
    let mut c = cache().lock().unwrap();
    let entry = c.entry(zone.to_string()).or_insert_with(|| tz::TimeZone::from_posix_tz(zone).ok());
    let tz = entry.as_ref()?;
    let t = tz.find_local_time_type(unix).ok()?;
    let off = t.ut_offset() as i64;
    let local = unix + off;
    let days = local.div_euclid(86400);
    let rem = local.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    Some((bagholder_model::dates::fmt(y, m, d), rem / 60, off))
}
