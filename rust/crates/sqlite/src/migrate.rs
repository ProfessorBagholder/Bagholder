//! Bringing a store's file up to the schema this build expects, without ever
//! losing what it holds (`docs/architecture.md` §6, "Schema changes never lose
//! data").
//!
//! A store declares its schema as numbered migrations, 1 to N with no gaps. A
//! file records the last one applied in its header (`PRAGMA user_version`) and
//! every one applied, with when and by which version of the app, in
//! `schema_migrations`. Opening a file:
//!
//! - a file written by a newer build (its version above N) is refused, naming both;
//! - a file that is not this store (another store's, or a database of tables this
//!   runner never made) is refused;
//! - before any pending migration runs on a file that already holds a schema, the
//!   whole file is copied to `snapshots/` beside it, and the two newest snapshots
//!   of the store are kept;
//! - each pending migration runs in its own transaction with its version bump, so
//!   one that fails leaves the file at the version before it, and says which failed.
//!
//! A migration once released is never edited: each store commits the schema each
//! version produces, and its tests compare (`schema_text`).

use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// One step of a store's schema.
pub struct Migration {
    pub number: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// A store: which file it is, and its migrations in order.
pub struct Schema {
    /// What the store is called in messages and snapshot names: `book`, `cache`.
    pub name: &'static str,
    /// Written to the file's header (`PRAGMA application_id`), so a file of one
    /// store is never opened, and migrated, as another.
    pub application_id: i32,
    pub migrations: &'static [Migration],
}

impl Schema {
    /// The version the latest migration brings a file to.
    pub fn latest(&self) -> u32 {
        self.migrations.last().map_or(0, |m| m.number)
    }
}

/// How many schema versions of one store keep a snapshot: the newest of each of
/// the two latest versions. The newest is of the file as the last update found
/// it, the one a failed update restores; an update that fails and is retried
/// replaces only its own version's snapshot, never an older version's.
pub const SNAPSHOTS_KEPT: usize = 2;

#[derive(Debug)]
pub enum MigrateError {
    /// The migrations are not numbered 1 to N in order: a fault in the build.
    Numbering { expected: u32, found: u32 },
    /// The file was written by a newer build.
    Newer { store: &'static str, file: u32, known: u32 },
    /// The file is not this store.
    Foreign { store: &'static str, path: PathBuf, why: String },
    /// A migration failed; the file is left at the version before it.
    Failed { store: &'static str, number: u32, name: &'static str, error: rusqlite::Error },
    /// The copy taken before migrating could not be made; nothing was migrated.
    Snapshot { store: &'static str, path: PathBuf, error: String },
    Sqlite(rusqlite::Error),
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::Numbering { expected, found } => {
                write!(f, "migrations are out of order: expected {expected}, found {found}")
            }
            MigrateError::Newer { store, file, known } => write!(
                f,
                "the {store} was written by a newer version of Bagholder (schema {file}); this version knows schema {known} at most"
            ),
            MigrateError::Foreign { store, path, why } => {
                write!(f, "{} is not a Bagholder {store}: {why}", path.display())
            }
            MigrateError::Failed { store, number, name, error } => write!(
                f,
                "the {store}'s migration {number} ({name}) failed and was undone; the {store} is left at schema {}: {error}",
                number - 1
            ),
            MigrateError::Snapshot { store, path, error } => write!(
                f,
                "the {store} could not be copied to {} before migrating, so it was not migrated: {error}",
                path.display()
            ),
            MigrateError::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MigrateError {}

impl From<rusqlite::Error> for MigrateError {
    fn from(e: rusqlite::Error) -> Self {
        MigrateError::Sqlite(e)
    }
}

/// What opening did, for the caller to report.
#[derive(Debug, Default, PartialEq)]
pub struct Migrated {
    /// The file's version before, and after.
    pub from: u32,
    pub to: u32,
    /// The copy taken before migrating, when there was something to protect.
    pub snapshot: Option<PathBuf>,
}

/// Open the store at `path` (made when it does not exist) and bring it to the
/// latest version. `at` is now, and `app_version` this build's version: both are
/// recorded against each migration applied, and `at` names the snapshot.
pub fn open(schema: &Schema, path: &Path, app_version: &str, at: jiff::Timestamp) -> Result<(Connection, Migrated), MigrateError> {
    check_numbering(schema)?;
    // the file is looked at before anything is done to it: one that is not this
    // store, or is newer, is refused exactly as it was found
    if path.exists() {
        let look = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        identify(schema, &look, path)?;
    }
    let conn = crate::open_db(path)?;
    let done = migrate(schema, &conn, path, app_version, at)?;
    Ok((conn, done))
}

fn check_numbering(schema: &Schema) -> Result<(), MigrateError> {
    for (i, m) in schema.migrations.iter().enumerate() {
        let expected = i as u32 + 1;
        if m.number != expected {
            return Err(MigrateError::Numbering { expected, found: m.number });
        }
    }
    Ok(())
}

/// The version the file's header records.
pub fn version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// The file is this store (or empty), and not newer than this build knows.
fn identify(schema: &Schema, conn: &Connection, path: &Path) -> Result<(), MigrateError> {
    let from = version(conn)?;
    let known = schema.latest();
    let app_id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let tables: i64 =
        conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'", [], |r| r.get(0))?;
    let foreign = |why: String| MigrateError::Foreign { store: schema.name, path: path.to_path_buf(), why };
    if from == 0 {
        if tables > 0 {
            return Err(foreign(format!("it holds {tables} tables this store did not make")));
        }
    } else if app_id != schema.application_id {
        return Err(foreign(format!("its header names store {app_id:#x}, not {:#x}", schema.application_id)));
    }
    if from > known {
        return Err(MigrateError::Newer { store: schema.name, file: from, known });
    }
    Ok(())
}

fn migrate(schema: &Schema, conn: &Connection, path: &Path, app_version: &str, at: jiff::Timestamp) -> Result<Migrated, MigrateError> {
    identify(schema, conn, path)?;
    let from = version(conn)?;
    let known = schema.latest();
    let mut done = Migrated { from, to: from, snapshot: None };
    if from == known {
        return Ok(done);
    }
    if from > 0 {
        done.snapshot = Some(snapshot(schema, conn, path, from, at)?);
    }
    let applied_at = at.to_string();
    for m in &schema.migrations[from as usize..] {
        crate::atomically(conn, || {
            if m.number == 1 {
                conn.execute_batch(&format!(
                    "PRAGMA application_id = {};
                     CREATE TABLE schema_migrations (
                        number INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL,
                        app_version TEXT NOT NULL
                     ) STRICT;",
                    schema.application_id
                ))?;
            }
            conn.execute_batch(m.sql)?;
            conn.execute(
                "INSERT INTO schema_migrations(number, name, applied_at, app_version) VALUES (?, ?, ?, ?)",
                rusqlite::params![m.number, m.name, applied_at, app_version],
            )?;
            conn.execute_batch(&format!("PRAGMA user_version = {}", m.number))
        })
        .map_err(|error| MigrateError::Failed { store: schema.name, number: m.number, name: m.name, error })?;
        done.to = m.number;
    }
    Ok(done)
}

/// The folder a store's snapshots are kept in: `snapshots/` beside its file.
pub fn snapshot_dir(path: &Path) -> PathBuf {
    path.parent().unwrap_or(Path::new(".")).join("snapshots")
}

/// Copy the whole file, consistently, before it is migrated from `from`; then
/// keep only the newest `SNAPSHOTS_KEPT` of this store.
fn snapshot(schema: &Schema, conn: &Connection, path: &Path, from: u32, at: jiff::Timestamp) -> Result<PathBuf, MigrateError> {
    let dir = snapshot_dir(path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(schema.name).to_string();
    // a name that sorts in time order: the instant to the second, in UTC
    let stamp = at.strftime("%Y%m%dT%H%M%SZ").to_string();
    let target = dir.join(format!("{stem}-v{from}-{stamp}.db"));
    let fail = |error: String| MigrateError::Snapshot { store: schema.name, path: target.clone(), error };
    std::fs::create_dir_all(&dir).map_err(|e| fail(e.to_string()))?;
    if target.exists() {
        return Err(fail("a snapshot of that name already exists".into()));
    }
    // VACUUM INTO writes a consistent copy from a read transaction: a writer on
    // another connection is neither blocked for long nor copied half done
    conn.execute("VACUUM INTO ?", [target.to_string_lossy()]).map_err(|e| fail(e.to_string()))?;
    prune(&dir, &stem).map_err(|e| fail(e.to_string()))?;
    Ok(target)
}

/// The snapshots of the store whose file is called `stem`, oldest first, each
/// with the version it was taken from.
pub fn snapshots(dir: &Path, stem: &str) -> std::io::Result<Vec<PathBuf>> {
    Ok(listed(dir, stem)?.into_iter().map(|(_, _, p)| p).collect())
}

fn listed(dir: &Path, stem: &str) -> std::io::Result<Vec<(String, u32, PathBuf)>> {
    let prefix = format!("{stem}-v");
    let mut found: Vec<(String, u32, PathBuf)> = Vec::new();
    if !dir.exists() {
        return Ok(vec![]);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(rest) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".db")) else { continue };
        // `<version>-<stamp>`: ordered by the stamp, which is when it was taken
        let Some((version, stamp)) = rest.split_once('-') else { continue };
        let Ok(version) = version.parse() else { continue };
        found.push((stamp.to_string(), version, path));
    }
    found.sort();
    Ok(found)
}

/// Keep the newest snapshot of each of the `SNAPSHOTS_KEPT` latest versions.
fn prune(dir: &Path, stem: &str) -> std::io::Result<()> {
    let all = listed(dir, stem)?;
    let mut versions: Vec<u32> = all.iter().map(|(_, v, _)| *v).collect();
    versions.sort_unstable();
    versions.dedup();
    let kept_versions: Vec<u32> = versions.iter().rev().take(SNAPSHOTS_KEPT).copied().collect();
    for (i, (_, v, path)) in all.iter().enumerate() {
        let newest_of_its_version = !all[i + 1..].iter().any(|(_, w, _)| w == v);
        if !(kept_versions.contains(v) && newest_of_its_version) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// The schema a file holds, as text: every table, index, trigger and view in
/// name order, with the version. A store's tests compare this against the text
/// committed for each version, so a released migration cannot change unnoticed.
pub fn schema_text(conn: &Connection) -> rusqlite::Result<String> {
    let mut out = format!("-- user_version {}\n", version(conn)?);
    let mut stmt = conn.prepare(
        "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for sql in rows {
        out.push_str(&sql?);
        out.push_str(";\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: Migration = Migration { number: 1, name: "notes", sql: "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL) STRICT;" };
    const V2: Migration = Migration { number: 2, name: "note tags", sql: "ALTER TABLE notes ADD COLUMN tag TEXT;" };
    const V3_BAD: Migration = Migration { number: 3, name: "broken", sql: "CREATE TABLE later (x INTEGER); INSERT INTO nowhere VALUES (1);" };

    static ONE: [Migration; 1] = [V1];
    static TWO: [Migration; 2] = [V1, V2];
    static THREE_BAD: [Migration; 3] = [V1, V2, V3_BAD];
    static GAP: [Migration; 2] = [V1, Migration { number: 3, name: "skipped", sql: "" }];

    fn schema(migrations: &'static [Migration]) -> Schema {
        Schema { name: "test store", application_id: 0x4248_5431, migrations }
    }

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn a_new_file_reaches_the_latest_version_and_records_each_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        let (conn, done) = open(&schema(&TWO), &path, "9.9.9", at("2026-09-23T12:00:00Z")).unwrap();
        assert_eq!(done, Migrated { from: 0, to: 2, snapshot: None });
        assert_eq!(version(&conn).unwrap(), 2);
        let steps: Vec<(u32, String, String, String)> = conn
            .prepare("SELECT number, name, applied_at, app_version FROM schema_migrations ORDER BY number")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(steps, vec![
            (1, "notes".into(), "2026-09-23T12:00:00Z".into(), "9.9.9".into()),
            (2, "note tags".into(), "2026-09-23T12:00:00Z".into(), "9.9.9".into()),
        ]);
        // nothing to protect in a new file, so no snapshot folder either
        assert!(!snapshot_dir(&path).exists());
    }

    #[test]
    fn an_older_file_keeps_its_rows_and_is_copied_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        {
            let (conn, _) = open(&schema(&ONE), &path, "1.0.0", at("2026-01-01T00:00:00Z")).unwrap();
            conn.execute("INSERT INTO notes(body) VALUES ('kept')", []).unwrap();
        }
        let (conn, done) = open(&schema(&TWO), &path, "2.0.0", at("2026-02-01T08:30:00Z")).unwrap();
        assert_eq!((done.from, done.to), (1, 2));
        let body: String = conn.query_row("SELECT body FROM notes", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "kept");
        let snap = done.snapshot.unwrap();
        assert_eq!(snap.file_name().unwrap(), "s-v1-20260201T083000Z.db");
        let copy = crate::open_db(&snap).unwrap();
        assert_eq!(version(&copy).unwrap(), 1);
        let body: String = copy.query_row("SELECT body FROM notes", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "kept");
    }

    #[test]
    fn a_file_at_the_latest_version_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        open(&schema(&TWO), &path, "1", at("2026-01-01T00:00:00Z")).unwrap();
        let (_, done) = open(&schema(&TWO), &path, "1", at("2026-01-02T00:00:00Z")).unwrap();
        assert_eq!(done, Migrated { from: 2, to: 2, snapshot: None });
    }

    #[test]
    fn a_newer_file_is_refused_naming_both_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        open(&schema(&TWO), &path, "2", at("2026-01-01T00:00:00Z")).unwrap();
        let before = std::fs::read(&path).unwrap();
        let err = open(&schema(&ONE), &path, "1", at("2026-01-02T00:00:00Z")).unwrap_err();
        assert!(matches!(err, MigrateError::Newer { file: 2, known: 1, .. }), "{err:?}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(err.to_string().contains("schema 2") && err.to_string().contains("schema 1"), "{err}");
    }

    #[test]
    fn a_failing_migration_is_undone_and_names_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        open(&schema(&TWO), &path, "2", at("2026-01-01T00:00:00Z")).unwrap();
        let err = open(&schema(&THREE_BAD), &path, "3", at("2026-01-02T00:00:00Z")).unwrap_err();
        assert!(matches!(err, MigrateError::Failed { number: 3, name: "broken", .. }), "{err:?}");
        assert!(err.to_string().contains("left at schema 2"), "{err}");
        let conn = crate::open_db(&path).unwrap();
        assert_eq!(version(&conn).unwrap(), 2);
        // the half of it that did run is gone with the rest
        let later: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'later'", [], |r| r.get(0)).unwrap();
        assert_eq!(later, 0);
        let recorded: i64 = conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0)).unwrap();
        assert_eq!(recorded, 2);
    }

    #[test]
    fn migrations_out_of_order_are_a_fault_in_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let err = open(&schema(&GAP), &dir.path().join("s.db"), "1", at("2026-01-01T00:00:00Z")).unwrap_err();
        assert!(matches!(err, MigrateError::Numbering { expected: 2, found: 3 }), "{err:?}");
    }

    #[test]
    fn a_database_this_store_did_not_make_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE activities (id TEXT); INSERT INTO activities VALUES ('a')").unwrap();
        let before = std::fs::read(&path).unwrap();
        let err = open(&schema(&ONE), &path, "1", at("2026-01-01T00:00:00Z")).unwrap_err();
        assert!(matches!(err, MigrateError::Foreign { .. }), "{err:?}");
        // and it is left exactly as it was, byte for byte
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn another_stores_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        open(&schema(&ONE), &path, "1", at("2026-01-01T00:00:00Z")).unwrap();
        let other = Schema { name: "other store", application_id: 0x4248_5432, migrations: &TWO };
        let err = open(&other, &path, "1", at("2026-01-02T00:00:00Z")).unwrap_err();
        assert!(matches!(err, MigrateError::Foreign { .. }), "{err:?}");
    }

    #[test]
    fn only_the_two_newest_snapshots_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        static STEPS: [Migration; 4] = [
            V1,
            V2,
            Migration { number: 3, name: "c", sql: "CREATE TABLE c (x INTEGER);" },
            Migration { number: 4, name: "d", sql: "CREATE TABLE d (x INTEGER);" },
        ];
        for (n, when) in [(1, "2026-01-01T00:00:00Z"), (2, "2026-02-01T00:00:00Z"), (3, "2026-03-01T00:00:00Z"), (4, "2026-04-01T00:00:00Z")] {
            open(&schema(&STEPS[..n]), &path, "x", at(when)).unwrap();
        }
        let kept: Vec<String> = snapshots(&snapshot_dir(&path), "s")
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept, vec!["s-v2-20260301T000000Z.db", "s-v3-20260401T000000Z.db"]);
    }

    #[test]
    fn a_retried_update_keeps_the_snapshot_of_the_version_before() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        open(&schema(&ONE), &path, "1", at("2026-01-01T00:00:00Z")).unwrap();
        open(&schema(&TWO), &path, "2", at("2026-02-01T00:00:00Z")).unwrap(); // snapshot of v1
        // an update to 3 that fails, three times
        for when in ["2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z", "2026-03-03T00:00:00Z"] {
            assert!(open(&schema(&THREE_BAD), &path, "3", at(when)).is_err());
        }
        let kept: Vec<String> = snapshots(&snapshot_dir(&path), "s")
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept, vec!["s-v1-20260201T000000Z.db", "s-v2-20260303T000000Z.db"]);
    }

    #[test]
    fn the_schema_text_names_every_object_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (conn, _) = open(&schema(&TWO), &dir.path().join("s.db"), "1", at("2026-01-01T00:00:00Z")).unwrap();
        let text = schema_text(&conn).unwrap();
        assert!(text.starts_with("-- user_version 2\n"), "{text}");
        assert!(text.contains("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL, tag TEXT) STRICT;"), "{text}");
        assert!(text.contains("CREATE TABLE schema_migrations"), "{text}");
    }
}

