//! The SQLite store: the database file every desktop copy reads and writes.

pub mod schema;
pub mod relabel;
pub mod activities;
pub mod tables;
pub mod merge;
pub mod csvimport;
pub mod snapshot;
pub mod market;
pub mod orders;
pub mod feeds;
pub mod admin;
pub mod gens;

/// Whether the program running is a test run: a test harness, which cargo
/// builds into a `deps` folder, never the app it ships.
pub fn running_tests() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|d| d.file_name()).map(|n| n == "deps"))
        .unwrap_or(false)
}

/// The person's own data folder, refused to a test run. A test that reaches
/// ~/.bagholder without a temporary home writes its fixtures into the live
/// database: a suite did exactly that, leaving a made-up QIMC headline in the
/// person's news. The real folder is an error there, not a default.
pub fn guard_home(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let base = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let resolve = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let here = resolve(path);
    let same = !base.is_empty()
        && [".bagholder", ".bagholder-rust"].iter().any(|n| here == resolve(&std::path::Path::new(&base).join(n)));
    if same && running_tests() {
        return Err("a test reached the real ~/.bagholder: give it a temporary home (BAGHOLDER_HOME)".into());
    }
    Ok(path.to_path_buf())
}

/// A connection to the store in `home`, ready; refused for the person's own
/// folder in a test run, so a test that forgets its home fails instead of
/// opening the live database.
pub fn connect(home: &std::path::Path) -> rusqlite::Result<rusqlite::Connection> {
    guard_home(home).map_err(|_| rusqlite::Error::InvalidPath(home.to_path_buf()))?;
    open_db(&home.join("bagholder.db"))
}

/// The one way a connection to the database file is made, so every connection
/// agrees on how it is kept. Write-ahead logging: a reader never waits on a
/// writer and never sees a change half made, and a crash leaves the last
/// committed state, not a journal to replay over a torn file. `synchronous =
/// NORMAL` is the setting made for WAL: a commit is durable against the app
/// dying, and against the machine dying up to the last checkpoint, without an
/// fsync per statement. The mode is a property of the file, so a database an
/// earlier build made in rollback mode is converted the first time it is opened.
pub fn open_db(path: &std::path::Path) -> rusqlite::Result<rusqlite::Connection> {
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
    conn.commit_hook(Some(|| {
        if let Some(heard) = ON_COMMIT.get() {
            heard();
        }
        false // never veto the commit
    }));
    Ok(conn)
}

static ON_COMMIT: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// Be told whenever any connection commits. `heard` runs inside SQLite's commit,
/// on the writer's thread: it must only signal (wake a waiter) and return -- it
/// must not touch the database. What changed is read afterwards, from the
/// generation counters. Set once, at start.
pub fn on_commit(heard: impl Fn() + Send + Sync + 'static) {
    let _ = ON_COMMIT.set(Box::new(heard));
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
