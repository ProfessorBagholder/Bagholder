//! What Bagholder told the person (`docs/architecture.md` §6: the notification
//! history is the book's): the notifications, what each stream has met (`told`),
//! the notification settings and each stream's mark (the settings
//! `notify.settings` and `notify.seen.<stream>`). The tables are the earlier
//! store's, moved whole (migration 15): the server reads and writes them with
//! that store's SQL on this book's connection (`notices`), in one transaction
//! with the marks where it tells something.
//!
//! Carried once from the earlier store (`carry_notices`), in one transaction,
//! marked by the setting `carried.notices`.

use std::path::Path;

use rusqlite::{params, Connection};

use crate::text::at as at_text;
use crate::{Book, BookError, Result};

/// The settings of the notification kinds, as JSON.
pub const SETTINGS: &str = "notify.settings";
/// Before a stream's name: the newest moment it has shown and what it showed then.
pub const SEEN: &str = "notify.seen.";
/// Whether the earlier store's notices were carried into the book.
const CARRIED: &str = "carried.notices";

/// The earlier store's keys for the settings and the marks.
const OLD_SETTINGS: &str = "notify_settings";
const OLD_SEEN: &str = "notify_seen:";

/// What a carry brought over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarriedNotices {
    pub notifications: i64,
    pub told: i64,
    /// Settings and marks.
    pub settings: i64,
}

impl Book {
    /// The connection the notices are kept on, for the earlier store's SQL that
    /// reads and writes them (`bagholder_store::feeds`), and for a transaction
    /// (`bagholder_sqlite::atomically`) that joins a notice to the marks it
    /// sets: the settings written through `set_setting` on this book join it.
    pub fn notices(&self) -> &Connection {
        self.conn()
    }

    /// Have `heard` run after each commit on this connection has landed
    /// (`bagholder_sqlite::on_commit`): it must only signal a waiter and return.
    pub fn on_commit(&self, heard: std::sync::Arc<dyn Fn() + Send + Sync>) -> rusqlite::Result<()> {
        bagholder_sqlite::on_commit(self.conn(), heard)
    }

    /// Whether the earlier store's notices were carried.
    pub fn notices_carried(&self) -> Result<bool> {
        Ok(self.setting(CARRIED)?.is_some())
    }

    /// Carry the earlier store at `old` -- its notifications, what its streams
    /// met, its notification settings and its marks -- into the book, once, in
    /// one transaction: every row with its values, or nothing. The earlier store
    /// is only read. `None` when it was carried before.
    pub fn carry_notices(&self, old: &Path, at: jiff::Timestamp) -> Result<Option<CarriedNotices>> {
        if self.notices_carried()? {
            return Ok(None);
        }
        let c = self.conn();
        c.execute("ATTACH DATABASE ?1 AS old", [old.to_string_lossy()])?;
        let carried = self.atomically(|| {
            let has = |t: &str| -> Result<bool> {
                Ok(c.query_row("SELECT COUNT(*) FROM old.sqlite_master WHERE type = 'table' AND name = ?", [t], |r| r.get::<_, i64>(0))? > 0)
            };
            let mut out = CarriedNotices::default();
            if has("notifications")? {
                out.notifications = c.execute(
                    "INSERT INTO notifications (id, at, kind, key, title, body, extra, seen_at, read_at)
                     SELECT id, at, kind, key, title, body, extra, seen_at, read_at FROM old.notifications",
                    [],
                )? as i64;
                // the next id follows the earlier store's, as it would have there: a
                // page's stream reads on from the last id it was sent
                if has("sqlite_sequence")? {
                    let seq: i64 = c.query_row("SELECT COALESCE(MAX(seq), 0) FROM old.sqlite_sequence WHERE name = 'notifications'", [], |r| r.get(0))?;
                    let held: i64 = c.query_row("SELECT COUNT(*) FROM main.sqlite_sequence WHERE name = 'notifications'", [], |r| r.get(0))?;
                    if held > 0 {
                        c.execute("UPDATE main.sqlite_sequence SET seq = MAX(seq, ?1) WHERE name = 'notifications'", [seq])?;
                    } else if seq > 0 {
                        c.execute("INSERT INTO main.sqlite_sequence (name, seq) VALUES ('notifications', ?1)", [seq])?;
                    }
                }
            }
            if has("told")? {
                out.told = c.execute("INSERT INTO told (scope, event, at) SELECT scope, event, at FROM old.told", [])? as i64;
            }
            if has("meta")? {
                let keys: Vec<(String, Option<String>)> = c
                    .prepare("SELECT key, value FROM old.meta WHERE key = ?1 OR substr(key, 1, ?3) = ?2 ORDER BY key")?
                    .query_map(params![OLD_SETTINGS, OLD_SEEN, OLD_SEEN.len() as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                for (key, value) in keys {
                    // an empty value is a key the earlier store cleared: nothing to carry
                    let Some(value) = value.filter(|v| !v.is_empty()) else { continue };
                    let new = match key.strip_prefix(OLD_SEEN) {
                        Some(stream) => format!("{SEEN}{stream}"),
                        None => SETTINGS.to_string(),
                    };
                    self.set_setting(&new, Some(&value), at)?;
                    out.settings += 1;
                }
            }
            c.execute(
                "INSERT INTO settings(key, value, source, set_at) VALUES (?1, ?2, 'bagholder', ?3)",
                params![CARRIED, serde_json::json!({ "notifications": out.notifications, "told": out.told, "settings": out.settings }).to_string(), at_text(at)],
            )?;
            Ok(out)
        });
        let detached = c.execute("DETACH DATABASE old", []).map_err(BookError::from);
        let carried = carried?;
        detached?;
        Ok(Some(carried))
    }
}
