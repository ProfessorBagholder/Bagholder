//! What the person's own page states, kept in the book for when no page is open
//! (`docs/plans/stage-3c-switch.md`, §2, "The zone"; `docs/decisions.md`,
//! 2026-09-25, time).
//!
//! The zone is the time zone of the browser in use: each page states it when it
//! opens, and the latest one stated is the person's. Days, months, years and
//! "today" are in it; the server's own zone is never used.

use jiff::tz::TimeZone;
use rusqlite::{params, OptionalExtension};

use crate::text::{self, at as at_text};
use crate::{Book, BookError, Result};

const ZONE: &str = "zone";
/// Who states the zone: the page, from its browser.
const BROWSER: &str = "browser";

/// The zone as the book keeps it: its name, its rules, and when it was stated.
#[derive(Clone, Debug)]
pub struct StatedZone {
    pub name: String,
    pub zone: TimeZone,
    pub stated_at: jiff::Timestamp,
}

impl Book {
    /// The zone a page stated last, if any has.
    pub fn zone(&self) -> Result<Option<StatedZone>> {
        let row: Option<(String, String)> = self.conn().query_row("SELECT value, set_at FROM settings WHERE key = ?", [ZONE], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let Some((name, at)) = row else { return Ok(None) };
        let zone = self.zones.get(&name).map_err(|e| text::corrupt("settings", "value", &name, &e))?;
        let stated_at = at.parse().map_err(|_| text::corrupt("settings", "set_at", &at, "not an instant"))?;
        Ok(Some(StatedZone { name, zone, stated_at }))
    }

    /// A page states its browser's zone: kept when it differs from the one held.
    /// True when the zone changed; a name with no rules is refused.
    pub fn state_zone(&self, name: &str, at: jiff::Timestamp) -> Result<bool> {
        self.zones.get(name).map_err(BookError::Refused)?;
        self.atomically(|| {
            let held: Option<String> = self.conn().query_row("SELECT value FROM settings WHERE key = ?", [ZONE], |r| r.get(0)).optional()?;
            if held.as_deref() == Some(name) {
                return Ok(false);
            }
            self.conn().execute(
                "INSERT INTO settings(key, value, source, set_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, source = excluded.source, set_at = excluded.set_at",
                params![ZONE, name, BROWSER, at_text(at)],
            )?;
            Ok(true)
        })
    }

    /// A setting the person made, by its name: its value, if one is kept.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT value FROM settings WHERE key = ?", [key], |r| r.get(0)).optional()?)
    }

    /// Keep a setting the person made (`None` forgets it).
    pub fn set_setting(&self, key: &str, value: Option<&str>, at: jiff::Timestamp) -> Result<()> {
        if key == ZONE {
            return Err(BookError::Refused("the zone is stated by a page".into()));
        }
        match value {
            None => self.conn().execute("DELETE FROM settings WHERE key = ?", [key])?,
            Some(v) => self.conn().execute(
                "INSERT INTO settings(key, value, source, set_at) VALUES (?1, ?2, 'person', ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, source = excluded.source, set_at = excluded.set_at",
                params![key, v, at_text(at)],
            )?,
        };
        Ok(())
    }
}
