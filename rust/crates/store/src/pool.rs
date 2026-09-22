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

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// How many idle connections are kept. More than this many at once is a burst;
/// the extra ones are closed when they come back.
const KEPT: usize = 8;

pub struct Pool {
    path: PathBuf,
    hook: Option<Arc<dyn Fn() + Send + Sync>>,
    idle: Mutex<Vec<Connection>>,
}

impl Pool {
    pub fn new(path: &Path) -> Pool {
        Pool { path: path.to_path_buf(), hook: None, idle: Mutex::new(Vec::new()) }
    }

    /// As `new`, but every connection this pool opens carries `hook` on its commits.
    pub fn with_hook(path: &Path, hook: Arc<dyn Fn() + Send + Sync>) -> Pool {
        Pool { path: path.to_path_buf(), hook: Some(hook), idle: Mutex::new(Vec::new()) }
    }

    pub fn get(&self) -> rusqlite::Result<Pooled<'_>> {
        let kept = self.idle.lock().unwrap_or_else(|e| e.into_inner()).pop();
        let conn = match kept {
            Some(c) => c,
            None => crate::open_db_hooked(&self.path, self.hook.clone())?,
        };
        Ok(Pooled { pool: self, conn: Some(conn) })
    }

    /// Idle connections now (for tests).
    pub fn idle(&self) -> usize {
        self.idle.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// A connection on loan: use it as a `&Connection`.
pub struct Pooled<'a> {
    pool: &'a Pool,
    conn: Option<Connection>,
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
            idle.push(conn);
        }
    }
}
