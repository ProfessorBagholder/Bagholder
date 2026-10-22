//! What Bagholder told the person (`docs/architecture.md` §6: the notification
//! history is the book's): the notifications, what each stream has met (`told`),
//! the notification settings and each stream's mark (the settings
//! `notify.settings` and `notify.seen.<stream>`). The server reads and writes
//! them with the market readers' SQL (`bagholder_market::stored::feeds`) on this
//! book's connection (`notices`), in one transaction with the marks where it
//! tells something.

use rusqlite::{params, Connection};

use crate::text::at as at_text;
use crate::{Book, Result};

/// The settings of the notification kinds, as JSON.
pub const SETTINGS: &str = "notify.settings";
/// Before a stream's name: the newest moment it has shown and what it showed then.
pub const SEEN: &str = "notify.seen.";
impl Book {
    /// The connection the notices are kept on, for the SQL that reads and writes
    /// them (`bagholder_market::stored::feeds`), and for a transaction
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

    /// Every browser subscribed to Web Push: (endpoint, p256dh, auth), each as the
    /// browser gave it (base64url keys).
    pub fn push_subscriptions(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn().prepare_cached("SELECT endpoint, p256dh, auth FROM push_subscriptions ORDER BY made_at, endpoint")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// A browser's subscription kept, or its keys replaced where it is kept.
    pub fn keep_push_subscription(&self, endpoint: &str, p256dh: &str, auth: &str, at: jiff::Timestamp) -> Result<()> {
        self.conn().execute(
            "INSERT INTO push_subscriptions (endpoint, p256dh, auth, made_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth",
            params![endpoint, p256dh, auth, at_text(at)],
        )?;
        Ok(())
    }

    /// A subscription gone: the browser left it, or its push service says it is
    /// gone (RFC 8030 §7.3). Whether one was kept.
    pub fn drop_push_subscription(&self, endpoint: &str) -> Result<bool> {
        Ok(self.conn().execute("DELETE FROM push_subscriptions WHERE endpoint = ?1", params![endpoint])? > 0)
    }
}
