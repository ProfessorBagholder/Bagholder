//! How the database file is kept: write-ahead logging, and every change that
//! spans rows made as one transaction, so the file is never observed half
//! written (docs/architecture.md, rule 6).

use rusqlite::Connection;

use bagholder_store::broker::Account;
use bagholder_store::{atomically, open_db, relabel, tables};

fn journal_mode(conn: &Connection) -> String {
    conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)).unwrap().to_lowercase()
}

fn account(id: &str) -> Account {
    Account { id: id.into(), nickname: id.into(), unified_account_type: "SELF_DIRECTED_TFSA".into(), currency: "CAD".into(), status: "open".into(), kind: "tfsa".into(), ..Default::default() }
}

fn ids(conn: &Connection) -> Vec<String> {
    tables::accounts(conn).unwrap().into_iter().map(|a| a.id).collect()
}

#[test]
fn test_a_new_database_is_kept_in_wal() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    assert_eq!(journal_mode(&conn), "wal");
    let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
    assert_eq!(sync, 2, "synchronous = FULL: every commit is flushed before it returns");
}

/// The pool's connections and a hooked one flush each commit too.
#[test]
fn test_every_connection_flushes_each_commit() {
    let dir = tempfile::tempdir().unwrap();
    let sync = |c: &Connection| c.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0)).unwrap();
    let pool = bagholder_store::pool::Pool::new(&dir.path().join("bagholder.db"));
    assert_eq!(sync(&pool.get().unwrap()), 2);
    let hooked = bagholder_store::open_db_hooked(&dir.path().join("hooked.db"), Some(std::sync::Arc::new(|| {}))).unwrap();
    assert_eq!(sync(&hooked), 2);
}

#[test]
fn test_a_database_an_earlier_build_made_is_converted_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    {
        // as the builds before this one left it: the default rollback journal
        let old = Connection::open(&path).unwrap();
        relabel::ensure(&old).unwrap();
        tables::replace_accounts(&old, &[account("a1")]).unwrap();
        assert_eq!(journal_mode(&old), "delete");
    }
    let conn = open_db(&path).unwrap();
    assert_eq!(journal_mode(&conn), "wal");
    assert_eq!(ids(&conn), ["a1"], "the rows are as they were");
    // the mode is the file's: a plain connection opened later finds it in WAL too
    assert_eq!(journal_mode(&Connection::open(&path).unwrap()), "wal");
}

#[test]
fn test_a_failed_change_leaves_the_rows_as_they_were() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    tables::replace_accounts(&conn, &[account("a1"), account("a2")]).unwrap();
    let out: rusqlite::Result<()> = atomically(&conn, || {
        conn.execute("DELETE FROM accounts", [])?;
        conn.execute("INSERT INTO accounts(id) VALUES ('half')", [])?;
        conn.execute("INSERT INTO no_such_table VALUES (1)", [])?; // the change fails part way
        Ok(())
    });
    assert!(out.is_err());
    assert_eq!(ids(&conn), ["a1", "a2"], "nothing of the failed change remains");
    assert!(conn.is_autocommit(), "and no transaction is left open");
}

#[test]
fn test_a_reader_never_sees_a_table_between_its_delete_and_its_inserts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    let writer = open_db(&path).unwrap();
    relabel::ensure(&writer).unwrap();
    tables::replace_accounts(&writer, &[account("a1"), account("a2")]).unwrap();
    let reader = open_db(&path).unwrap();
    let mut seen_midway = Vec::new();
    atomically(&writer, || {
        writer.execute("DELETE FROM accounts", [])?;
        seen_midway = ids(&reader); // another connection, while the table is empty in the writer's hands
        writer.execute("INSERT INTO accounts(id) VALUES ('b1')", [])?;
        Ok(())
    }).unwrap();
    assert_eq!(seen_midway, ["a1", "a2"], "the reader saw the rows as they were, not an empty table");
    assert_eq!(ids(&reader), ["b1"], "and sees the new rows once committed");
}

#[test]
fn test_a_change_inside_a_change_is_one_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    let out: rusqlite::Result<()> = atomically(&conn, || {
        tables::replace_accounts(&conn, &[account("inner")])?; // itself atomic: joins this one
        conn.execute("INSERT INTO no_such_table VALUES (1)", [])?;
        Ok(())
    });
    assert!(out.is_err());
    assert!(ids(&conn).is_empty(), "the inner change went with the outer one");
}

#[test]
fn test_every_commit_is_heard_whichever_connection_made_it() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let heard = Arc::new(AtomicUsize::new(0));
    let hook: Arc<dyn Fn() + Send + Sync> = {
        let heard = heard.clone();
        Arc::new(move || {
            heard.fetch_add(1, Ordering::SeqCst);
        })
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    let a = bagholder_store::open_db_hooked(&path, Some(hook.clone())).unwrap();
    relabel::ensure(&a).unwrap();
    let b = bagholder_store::open_db_hooked(&path, Some(hook)).unwrap();
    let before = heard.load(Ordering::SeqCst);
    tables::replace_accounts(&a, &[account("a1")]).unwrap(); // one transaction, several rows
    tables::set_meta(&b, "market_tiles", "[]").unwrap(); // a single statement, on another connection
    assert_eq!(heard.load(Ordering::SeqCst), before + 2, "both commits were heard");
}
