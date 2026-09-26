//! Connections kept between uses.
use bagholder_store::pool::Pool;

fn pool() -> (tempfile::TempDir, Pool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = Pool::new(&dir.path().join("bagholder.db"));
    bagholder_store::schema::init_schema(&pool.get().unwrap()).unwrap();
    (dir, pool)
}

#[test]
fn test_a_connection_given_back_is_the_one_handed_out_next() {
    let (_d, pool) = pool();
    assert_eq!(pool.idle(), 1);
    {
        let c = pool.get().unwrap();
        assert_eq!(pool.idle(), 0, "on loan");
        c.execute_batch("CREATE TEMP TABLE mark(x)").unwrap(); // lives as long as the connection does
    }
    assert_eq!(pool.idle(), 1);
    pool.get().unwrap().execute("INSERT INTO mark VALUES (1)", []).expect("the same connection: its temp table is there");
}

#[test]
fn test_two_at_once_are_two_connections_and_neither_waits() {
    let (_d, pool) = pool();
    let a = pool.get().unwrap();
    let b = pool.get().unwrap();
    a.execute("INSERT INTO meta(key, value) VALUES ('k', 'v')", []).unwrap();
    let seen: String = b.query_row("SELECT value FROM meta WHERE key = 'k'", [], |r| r.get(0)).unwrap();
    assert_eq!(seen, "v");
    drop((a, b));
    assert_eq!(pool.idle(), 2);
}

#[test]
fn test_a_connection_left_inside_a_transaction_is_not_kept() {
    let (_d, pool) = pool();
    {
        let c = pool.get().unwrap();
        c.execute_batch("BEGIN IMMEDIATE; INSERT INTO meta(key, value) VALUES ('half', 'done')").unwrap();
    } // as if the work had panicked part-way
    assert_eq!(pool.idle(), 0, "closed, which rolls it back");
    let n: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM meta WHERE key = 'half'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

/// A pool held to the store's schema, counting how often it prepares the file.
fn schema_pool(path: &std::path::Path) -> (Pool, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let runs = std::sync::Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let prepare: bagholder_store::pool::Prepare = std::sync::Arc::new(move |c| {
        counted.fetch_add(1, Ordering::SeqCst);
        bagholder_store::schema::init_schema(c)
    });
    (Pool::new(path).with_schema(bagholder_store::schema::SCHEMA_VERSION as i32, prepare), runs)
}

fn has_meta(c: &rusqlite::Connection) -> bool {
    c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'", [], |r| r.get::<_, i64>(0)).unwrap() == 1
}

#[test]
fn test_a_file_is_prepared_once_and_not_on_every_borrow() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let (pool, runs) = schema_pool(&dir.path().join("bagholder.db"));
    for _ in 0..20 {
        let a = pool.get().unwrap();
        let b = pool.get().unwrap(); // a second connection opened while one is on loan
        assert!(has_meta(&a) && has_meta(&b));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the stamped version holds it: one preparation");
}

#[test]
fn test_a_file_replaced_under_the_pool_is_migrated_on_the_next_borrow() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    let (pool, runs) = schema_pool(&path);
    pool.get().unwrap().execute("INSERT INTO meta(key, value) VALUES ('before', '1')", []).unwrap();
    assert_eq!(pool.idle(), 1);
    // another file moved over the path while the app runs: an empty one, as a
    // database restored from before the store existed would be
    let other = dir.path().join("restored.db");
    rusqlite::Connection::open(&other).unwrap().execute_batch("CREATE TABLE unrelated(x)").unwrap();
    std::fs::rename(&other, &path).unwrap();
    let c = pool.get().unwrap();
    assert!(has_meta(&c), "migrated on this borrow, not at the next start");
    let before: i64 = c.query_row("SELECT COUNT(*) FROM meta WHERE key = 'before'", [], |r| r.get(0)).unwrap();
    assert_eq!(before, 0, "the connection reads the file now at the path, not the one moved away");
    let unrelated: i64 = c.query_row("SELECT COUNT(*) FROM unrelated", [], |r| r.get(0)).unwrap();
    assert_eq!(unrelated, 0);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn test_a_file_rolled_back_in_place_is_migrated_on_the_next_borrow() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    let (pool, runs) = schema_pool(&path);
    drop(pool.get().unwrap());
    // the same file, taken back to a state with no stamp and a table missing
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("DROP TABLE meta; PRAGMA user_version = 0").unwrap();
    drop(raw);
    let c = pool.get().unwrap();
    assert!(has_meta(&c), "the kept connection finds the older stamp and brings the file up");
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}
