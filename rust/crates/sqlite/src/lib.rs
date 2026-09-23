//! The database plumbing every store shares: how a file is opened, how a change
//! is made as one transaction, the connections kept between uses, and how a
//! store's schema is brought up to date without losing what it holds.

pub mod migrate;
pub mod pool;

/// The one way a connection to the database file is made, so every connection
/// agrees on how it is kept. Write-ahead logging: a reader never waits on a
/// writer and never sees a change half made, and a crash leaves the last
/// committed state, not a journal to replay over a torn file. `synchronous =
/// NORMAL` is the setting made for WAL: a commit is durable against the app
/// dying, and against the machine dying up to the last checkpoint, without an
/// fsync per statement. The mode is a property of the file, so a database an
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
        let _ = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0));
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // every connection is made here, so every commit in the process is heard: no
    // writer has to remember to say it wrote
    if let Some(heard) = hook {
        conn.commit_hook(Some(move || {
            heard();
            false // never veto the commit
        }));
    }
    Ok(conn)
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
