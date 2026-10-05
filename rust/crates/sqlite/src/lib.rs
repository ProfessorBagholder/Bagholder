//! The database plumbing every store shares: how a file is opened, how a change
//! is made as one transaction, the connections kept between uses, and how a
//! store's schema is brought up to date without losing what it holds.

pub mod migrate;
pub mod pool;

/// The one way a connection to the database file is made, so every connection
/// agrees on how it is kept. Write-ahead logging: a reader never waits on a
/// writer and never sees a change half made, and a crash leaves the last
/// committed state, not a journal to replay over a torn file. `synchronous =
/// FULL`: every commit is flushed to the log before it returns, so a commit
/// survives the machine losing power, not only the app dying (`SPEC.md` §6, The
/// store: a thesis or a grade is the one thing in the file that cannot be
/// fetched again). WAL's `NORMAL` would lose the commits since the last
/// checkpoint to a power cut, for one fsync a commit saved. The mode is a property of the file, so a database an
/// earlier build made in rollback mode is converted the first time it is opened.
pub fn open_db(path: &std::path::Path) -> rusqlite::Result<rusqlite::Connection> {
    open_db_hooked(path, None)
}

/// As `open_db`, but with `hook` wired to the connection's commits, if given.
/// `hook` runs inside SQLite's commit, on the writer's thread: it must only
/// signal (wake a waiter) and return -- it must not touch the database. What
/// changed is read afterwards, from the generation counters.
pub fn open_db_hooked(path: &std::path::Path, hook: Option<std::sync::Arc<dyn Fn() + Send + Sync>>) -> rusqlite::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    // Asking the mode takes no lock; changing it needs the file to itself for a
    // moment. So it is changed only when it is not already WAL -- once in a file's
    // life -- and a connection that finds the file busy right then (another is
    // converting it, or a volume that cannot take WAL at all) opens it as it is
    // rather than refusing: the next open finds it converted.
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        match conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
            Ok(_) => {}
            // busy right then: opened as it is, as said above
            Err(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {}
            Err(e) => return Err(e),
        }
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    // every connection is made here, so every commit in the process is heard: no
    // writer has to remember to say it wrote
    if let Some(heard) = hook {
        on_commit(&conn, heard)?;
    }
    Ok(conn)
}

/// The log's size, in pages, past which a commit checkpoints it: SQLite's own
/// default for automatic checkpoints (`SQLITE_DEFAULT_WAL_AUTOCHECKPOINT`, 1000),
/// which a write-ahead-log hook replaces (sqlite.org/c3ref/wal_hook.html).
pub const WAL_CHECKPOINT_PAGES: i32 = 1000;

/// The signals connections' commits are heard by, each once, in the order first met.
static HEARD: std::sync::Mutex<Vec<std::sync::Arc<dyn Fn() + Send + Sync>>> = std::sync::Mutex::new(Vec::new());

/// Have `heard` run after each commit on `conn` has landed: from SQLite's
/// write-ahead-log hook, which runs once the commit is in the log and the write
/// lock is released, so a reader it wakes sees the change (brief 19, change 4: a
/// commit hook runs before the commit completes, and a reader woken there reads
/// the rows as they were). `heard` must only signal a waiter and return. The hook
/// takes over SQLite's automatic checkpoint, so it checkpoints the log itself, as
/// SQLite would, at `WAL_CHECKPOINT_PAGES`. A file left in rollback mode (one busy
/// when it was first opened, `open_db_hooked`) has no log: its commits are heard
/// from the commit hook until it is converted.
pub fn on_commit(conn: &rusqlite::Connection, heard: std::sync::Arc<dyn Fn() + Send + Sync>) -> rusqlite::Result<()> {
    use rusqlite::ffi;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        conn.commit_hook(Some(move || {
            heard();
            false // never veto the commit
        }));
        return Ok(());
    }
    unsafe extern "C" fn landed(arg: *mut std::ffi::c_void, db: *mut ffi::sqlite3, name: *const std::ffi::c_char, pages: std::ffi::c_int) -> std::ffi::c_int {
        // `arg` is the signal's place in the registry, never a pointer
        let heard = HEARD.lock().unwrap_or_else(|e| e.into_inner()).get(arg as usize).cloned();
        if let Some(h) = heard {
            h();
        }
        if pages >= WAL_CHECKPOINT_PAGES {
            // what SQLite's own automatic checkpoint does: a passive one, which
            // never waits on a reader or a writer; one that cannot finish now is
            // finished by a later commit's
            // SAFETY: `db` and `name` are the ones SQLite passed this callback
            unsafe { ffi::sqlite3_wal_checkpoint_v2(db, name, ffi::SQLITE_CHECKPOINT_PASSIVE, std::ptr::null_mut(), std::ptr::null_mut()) };
        }
        ffi::SQLITE_OK
    }
    // a connection names its signal by its place in one registry: a store's
    // connections share its one signal, so opening many adds nothing
    let place = {
        let mut all = HEARD.lock().unwrap_or_else(|e| e.into_inner());
        match all.iter().position(|h| std::sync::Arc::ptr_eq(h, &heard)) {
            Some(i) => i,
            None => {
                all.push(heard);
                all.len() - 1
            }
        }
    };
    // SAFETY: `conn.handle()` is this open connection; the argument is an index, not a pointer
    unsafe { ffi::sqlite3_wal_hook(conn.handle(), Some(landed), place as *mut std::ffi::c_void) };
    Ok(())
}

/// Run `work` as one transaction on `conn`: all of it is committed, or, when it
/// fails or panics, none of it. A reader on another connection sees the rows as
/// they were until the commit and as they are after it, never the table between
/// a delete and its inserts. Nested calls join the transaction already open.
pub fn atomically<T>(conn: &rusqlite::Connection, work: impl FnOnce() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    if !conn.is_autocommit() {
        return work(); // already inside one: the outer call commits
    }
    // IMMEDIATE: the write lock is taken at the start, where a busy database is
    // waited for. A transaction that begins by reading and only later writes cannot
    // be waited for -- if another connection committed in between, SQLite refuses
    // the upgrade outright (BUSY_SNAPSHOT) -- so a change never begins that way.
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let out = work()?; // an error or a panic drops `tx`, which rolls back
    tx.commit()?;
    Ok(out)
}

#[cfg(test)]
mod commit_tests {
    use std::sync::{Arc, Mutex};

    /// What the signal is for: a reader it wakes, on another connection, reads the
    /// rows the commit wrote. A commit hook runs before the commit completes, and a
    /// reader there reads the rows as they were (brief 19, change 4).
    #[test]
    fn a_reader_woken_by_the_signal_sees_what_the_commit_wrote() {
        let dir = std::env::temp_dir().join(format!("bh-sqlite-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.db");
        crate::open_db(&path).unwrap().execute_batch("CREATE TABLE t (x INTEGER)").unwrap();
        let seen: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(vec![]));
        let (s2, p2) = (seen.clone(), path.clone());
        let heard: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let reader = rusqlite::Connection::open(&p2).unwrap();
            s2.lock().unwrap().push(reader.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap());
        });
        let writer = crate::open_db_hooked(&path, Some(heard)).unwrap();
        writer.execute("INSERT INTO t VALUES (1)", []).unwrap();
        crate::atomically(&writer, || writer.execute_batch("INSERT INTO t VALUES (2); INSERT INTO t VALUES (3);")).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![1, 3], "each signal after its commit, each reader seeing it");
    }

    /// The hook takes over SQLite's automatic checkpoint: the log is still kept short.
    #[test]
    fn the_log_is_checkpointed_as_sqlite_would() {
        let dir = std::env::temp_dir().join(format!("bh-sqlite-checkpoint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.db");
        let c = crate::open_db_hooked(&path, Some(Arc::new(|| {}))).unwrap();
        c.execute_batch("CREATE TABLE t (x BLOB)").unwrap();
        let blob = vec![0u8; 8192];
        for _ in 0..3000 {
            c.execute("INSERT INTO t VALUES (?1)", [&blob]).unwrap();
        }
        let wal = std::fs::metadata(dir.join("s.db-wal")).map(|m| m.len()).unwrap_or(0);
        let page: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        // without checkpoints 3000 rows of 8 KiB would leave a log of some 24 MB;
        // checkpointed, writing restarts at its start once a checkpoint completes
        assert!(wal < 3 * crate::WAL_CHECKPOINT_PAGES as u64 * page as u64, "a log of {wal} bytes");
    }
}
