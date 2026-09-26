//! Connections kept between uses.
//!
//! A request, a loop's pass and a stream's look each need a connection for a few
//! milliseconds. Opening one each time means opening the file, setting it up and
//! preparing every statement from text again; a kept connection has all of that
//! done, and its prepared statements (`prepare_cached`) stay prepared. WAL lets
//! any number read while one writes, and a writer waits for its turn in SQLite
//! itself (`atomically` begins IMMEDIATE, under the busy timeout), so connections
//! are not divided into readers and a writer: any of them may do either.
//!
//! `get` hands out an idle connection, or opens one when none is idle: it never
//! waits. A connection comes back when its `Pooled` is dropped -- unless it was
//! left inside a transaction (the work panicked part-way), in which case it is
//! closed, which rolls the transaction back.
//!
//! A pool given a schema (`with_schema`) holds the file to it on every borrow,
//! cheaply: the file at the path is the one the kept connections opened (one
//! `stat`; a file replaced under the app closes them all, since they would go on
//! reading and writing the file that was moved away), and its stamped version
//! (`PRAGMA user_version`, read from the file's header) is this build's. A file
//! replaced or rolled back is brought up to the schema on the borrow that finds
//! it, not at the next start.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// How many idle connections are kept. More than this many at once is a burst;
/// the extra ones are closed when they come back.
const KEPT: usize = 8;

/// Brings a file up to the store's schema. Run on a connection outside any
/// transaction; it must be safe to run again on a file already up to date.
pub type Prepare = Arc<dyn Fn(&Connection) -> rusqlite::Result<()> + Send + Sync>;

/// Which file a path names now: two paths naming the same file agree, and a file
/// renamed over the path is another. `None` where the platform gives no identity,
/// or no file is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileId(u64, u64);

fn file_id(path: &Path) -> Option<FileId> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FileId(meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Some(FileId(meta.creation_time(), 0))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        None
    }
}

struct Kept {
    conn: Connection,
    file: Option<FileId>,
}

pub struct Pool {
    path: PathBuf,
    hook: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The version stamped in the file's header once it is prepared, and what
    /// prepares it.
    schema: Option<(i32, Prepare)>,
    idle: Mutex<Vec<Kept>>,
    /// One preparation at a time: a second borrow finding the same stale file
    /// waits for the first and then finds it done.
    preparing: Mutex<()>,
}

impl Pool {
    pub fn new(path: &Path) -> Pool {
        Pool { path: path.to_path_buf(), hook: None, schema: None, idle: Mutex::new(Vec::new()), preparing: Mutex::new(()) }
    }

    /// As `new`, but every connection this pool opens carries `hook` on its commits.
    pub fn with_hook(path: &Path, hook: Arc<dyn Fn() + Send + Sync>) -> Pool {
        Pool { hook: Some(hook), ..Pool::new(path) }
    }

    /// This pool, holding its file to a schema: a borrow that finds the file's
    /// stamped version is not `version` runs `prepare` on it and stamps it.
    pub fn with_schema(self, version: i32, prepare: Prepare) -> Pool {
        Pool { schema: Some((version, prepare)), ..self }
    }

    pub fn get(&self) -> rusqlite::Result<Pooled<'_>> {
        let now = file_id(&self.path);
        let kept = {
            let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
            // a file replaced under the path: a connection kept from before holds the
            // one moved away, and is closed. Its log is emptied into that file first:
            // SQLite leaves the log of a moved file on close, and the next connection
            // would read those commits into the file now at the path
            if let Some(stale) = idle.iter().find(|k| k.file != now) {
                let _ = stale.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
            }
            idle.retain(|k| k.file == now);
            idle.pop()
        };
        let (conn, file) = match kept {
            Some(k) => (k.conn, k.file),
            None => {
                let conn = crate::open_db_hooked(&self.path, self.hook.clone())?;
                (conn, file_id(&self.path))
            }
        };
        if let Some((version, prepare)) = &self.schema {
            if stamped(&conn)? != *version {
                let _one = self.preparing.lock().unwrap_or_else(|e| e.into_inner());
                if stamped(&conn)? != *version {
                    prepare(&conn)?;
                    conn.pragma_update(None, "user_version", version)?;
                }
            }
        }
        Ok(Pooled { pool: self, conn: Some(conn), file })
    }

    /// Idle connections now (for tests).
    pub fn idle(&self) -> usize {
        self.idle.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

fn stamped(conn: &Connection) -> rusqlite::Result<i32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// A connection on loan: use it as a `&Connection`.
pub struct Pooled<'a> {
    pool: &'a Pool,
    conn: Option<Connection>,
    file: Option<FileId>,
}

impl Deref for Pooled<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("held until dropped")
    }
}

impl Drop for Pooled<'_> {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else { return };
        if !conn.is_autocommit() {
            return; // left mid-transaction: closing it rolls that back
        }
        let mut idle = self.pool.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < KEPT {
            idle.push(Kept { conn, file: self.file });
        }
    }
}
