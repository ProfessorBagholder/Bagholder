//! Zone rules for the mappings: the day a broker files a row under is a day in
//! the broker's zone (`docs/architecture.md` §7).
//!
//! The convention of `docs/plans/time-zone-rules.md`: the system's IANA database,
//! kept current by the operating system, and the built-in copy only where the
//! system has none; tests read the built-in copy alone (`pinned-tzdb`), so no
//! machine changes an answer.

use jiff::tz::{TimeZone, TimeZoneDatabase};

pub struct Zones {
    db: &'static TimeZoneDatabase,
}

#[cfg(feature = "pinned-tzdb")]
fn db() -> &'static TimeZoneDatabase {
    static DB: std::sync::OnceLock<TimeZoneDatabase> = std::sync::OnceLock::new();
    DB.get_or_init(TimeZoneDatabase::bundled)
}

#[cfg(not(feature = "pinned-tzdb"))]
fn db() -> &'static TimeZoneDatabase {
    static DB: std::sync::OnceLock<TimeZoneDatabase> = std::sync::OnceLock::new();
    DB.get_or_init(TimeZoneDatabase::from_env)
}

impl Zones {
    pub fn new() -> Zones {
        Zones { db: db() }
    }

    /// The zone of that IANA name, or an error naming it.
    pub fn get(&self, name: &str) -> Result<TimeZone, String> {
        self.db.get(name).map_err(|e| format!("no rules for the time zone {name}: {e}"))
    }

    /// The day `at` falls on in `zone`.
    pub fn day(&self, at: jiff::Timestamp, zone: &str) -> Result<jiff::civil::Date, String> {
        Ok(at.to_zoned(self.get(zone)?).date())
    }
}

impl Default for Zones {
    fn default() -> Self {
        Zones::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_evening_in_alberta_is_still_that_day() {
        let z = Zones::new();
        // 02:01 UTC on 20 January is 19:01 on 19 January in Edmonton
        let at: jiff::Timestamp = "2026-01-20T02:01:15.227185Z".parse().unwrap();
        assert_eq!(z.day(at, "America/Edmonton").unwrap(), jiff::civil::date(2026, 1, 19));
        assert!(z.get("Mars/Olympus").is_err());
    }
}
