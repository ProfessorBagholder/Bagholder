//! How the database file is kept: write-ahead logging, and every change that
//! spans rows made as one transaction, so the file is never observed half
//! written (docs/architecture.md, rule 6).

use rusqlite::Connection;
use serde_json::json;

use bagholder_store::{atomically, open_db, relabel, tables};

fn journal_mode(conn: &Connection) -> String {
    conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)).unwrap().to_lowercase()
}

fn account(id: &str) -> serde_json::Value {
    json!({"id": id, "nickname": id, "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa"})
}

fn ids(conn: &Connection) -> Vec<String> {
    tables::accounts(conn).unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn test_a_new_database_is_kept_in_wal() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    assert_eq!(journal_mode(&conn), "wal");
    let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
    assert_eq!(sync, 1, "synchronous = NORMAL, the setting made for WAL");
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
    static HEARD: AtomicUsize = AtomicUsize::new(0);
    bagholder_store::on_commit(|| {
        HEARD.fetch_add(1, Ordering::SeqCst);
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bagholder.db");
    let a = open_db(&path).unwrap();
    relabel::ensure(&a).unwrap();
    let b = open_db(&path).unwrap();
    let before = HEARD.load(Ordering::SeqCst);
    tables::replace_accounts(&a, &[account("a1")]).unwrap(); // one transaction, several rows
    tables::set_meta(&b, "market_tiles", "[]").unwrap(); // a single statement, on another connection
    // at least: the hook is the process's, and the tests beside this one commit too
    assert!(HEARD.load(Ordering::SeqCst) >= before + 2, "both commits were heard");
}
