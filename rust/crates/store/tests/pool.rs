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
