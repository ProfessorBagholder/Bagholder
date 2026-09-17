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
    let real = std::path::Path::new(&base).join(".bagholder");
    let resolve = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let same = !base.is_empty() && resolve(path) == resolve(&real);
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
    let conn = rusqlite::Connection::open(home.join("bagholder.db"))?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(conn)
}
