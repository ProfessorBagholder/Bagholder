//! The database work the process does, counted for the capacity test
//! (`docs/architecture.md` §16, the growth rule): every virtual-machine step
//! SQLite takes on every connection the app opens, which is the same on every
//! machine; and, for a test that holds a path off a table or lists what a read
//! touches, the statements run that name a table and the tables read. Off unless `start_counting` is called before the connections are
//! opened; the app itself never calls it, so no connection of a running app
//! carries the counter.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

static COUNTING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The steps taken on this thread: a statement's steps are counted on the
    /// thread that runs it, so a measurement sees its own work and none of another
    /// thread's running beside it (the test suite runs tests in parallel).
    static STEPS: Cell<u64> = const { Cell::new(0) };
    /// The table a test asked about and the statements run on this thread that
    /// named it since.
    static NAMING: RefCell<Option<(&'static str, u64)>> = const { RefCell::new(None) };
    /// The tables statements prepared on this thread read, while a test asks.
    static READING: RefCell<Option<std::collections::BTreeSet<String>>> = const { RefCell::new(None) };
}

/// Count from here on: each connection opened after this counts its steps.
pub fn start_counting() {
    COUNTING.store(true, Ordering::SeqCst);
}

thread_local! {
    /// A capacity probe's measured window is open on this thread.
    static MEASURING: Cell<bool> = const { Cell::new(false) };
}

/// Open or close a probe's measured window on this thread.
pub fn set_measuring(on: bool) {
    MEASURING.with(|m| m.set(on));
}

/// Whether a probe's measured window is open on this thread: a test build's
/// checks that read the whole book stay out of what is measured.
pub fn measuring() -> bool {
    MEASURING.with(Cell::get)
}

/// The steps counted so far on this thread, on every counted connection.
pub fn steps() -> u64 {
    STEPS.with(Cell::get)
}

/// What `f` returns, and the statements it ran on this thread on counted
/// connections that name `table` (a word of their text, as SQLite runs it).
pub fn statements_naming<T>(table: &'static str, f: impl FnOnce() -> T) -> (T, u64) {
    NAMING.with(|n| *n.borrow_mut() = Some((table, 0)));
    let out = f();
    let (_, count) = NAMING.with(|n| n.borrow_mut().take()).expect("set above");
    (out, count)
}

/// What `f` returns, and every table the statements it prepared on this thread
/// read, on counted connections, as SQLite's authorizer names them while it
/// prepares each one. A statement already prepared and cached is not prepared
/// again, so a caller that must see every table reads through connections
/// opened inside `f`.
pub fn tables_read<T>(f: impl FnOnce() -> T) -> (T, std::collections::BTreeSet<String>) {
    READING.with(|r| *r.borrow_mut() = Some(Default::default()));
    let out = f();
    let tables = READING.with(|r| r.borrow_mut().take()).expect("set above");
    (out, tables)
}

/// A statement starting, as SQLite's trace hook gives it.
fn traced(sql: &str) {
    NAMING.with(|n| {
        if let Some((table, count)) = n.borrow_mut().as_mut() {
            let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if sql.match_indices(*table).any(|(i, _)| !word(sql[..i].chars().next_back()) && !word(sql[i + table.len()..].chars().next())) {
                *count += 1;
            }
        }
    });
}

/// A connection just opened, counted where counting is on.
pub(crate) fn watch(conn: &mut rusqlite::Connection) {
    if COUNTING.load(Ordering::Relaxed) {
        conn.trace(Some(traced));
        conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
            if let rusqlite::hooks::AuthAction::Read { table_name, .. } = ctx.action {
                READING.with(|r| {
                    if let Some(tables) = r.borrow_mut().as_mut() {
                        tables.insert(table_name.to_string());
                    }
                });
            }
            rusqlite::hooks::Authorization::Allow
        }));
        // the handler is called once per virtual-machine instruction, and never interrupts
        conn.progress_handler(
            1,
            Some(|| {
                STEPS.with(|s| s.set(s.get() + 1));
                false
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

    #[test]
    fn a_counted_connection_counts_its_steps_and_more_rows_take_more() {
        start_counting();
        let dir = tempfile::tempdir().unwrap();
        let c = crate::open_db(&dir.path().join("w.db")).unwrap();
        c.execute_batch("CREATE TABLE t (x INTEGER)").unwrap();
        let scan = |n: i64| {
            c.execute("DELETE FROM t", []).unwrap();
            for i in 0..n {
                c.execute("INSERT INTO t VALUES (?1)", [i]).unwrap();
            }
            let before = steps();
            let _: i64 = c.query_row("SELECT sum(x) FROM t", [], |r| r.get(0)).unwrap();
            steps() - before
        };
        let (small, large) = (scan(100), scan(400));
        assert!(small > 0, "a scan is counted");
        // a scan of four times the rows takes about four times the steps
        assert!(large > small * 3, "{small} then {large}");
    }

    #[test]
    fn the_statements_naming_a_table_are_counted_and_no_others() {
        start_counting();
        let dir = tempfile::tempdir().unwrap();
        let c = crate::open_db(&dir.path().join("n.db")).unwrap();
        c.execute_batch("CREATE TABLE days (x INTEGER); CREATE TABLE days_kept (x INTEGER)").unwrap();
        let ((), n) = statements_naming("days", || {
            c.execute("INSERT INTO days VALUES (1)", []).unwrap();
            let _: i64 = c.query_row("SELECT count(*) FROM days", [], |r| r.get(0)).unwrap();
            let _: i64 = c.query_row("SELECT count(*) FROM days_kept", [], |r| r.get(0)).unwrap();
        });
        assert_eq!(n, 2, "the two on `days`, not the one on `days_kept`");
        // a statement prepared once and run again is counted each run
        let ((), again) = statements_naming("days", || {
            for _ in 0..3 {
                let _: i64 = c.prepare_cached("SELECT count(*) FROM days").unwrap().query_row([], |r| r.get(0)).unwrap();
            }
        });
        assert_eq!(again, 3);
    }

    #[test]
    fn the_tables_a_statement_reads_are_listed_through_its_joins_and_no_others() {
        start_counting();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.db");
        crate::open_db(&path).unwrap().execute_batch("CREATE TABLE a (x INTEGER); CREATE TABLE b (x INTEGER); CREATE TABLE c (x INTEGER)").unwrap();
        let ((), tables) = tables_read(|| {
            let c = crate::open_db(&path).unwrap();
            let _: Option<i64> = c.query_row("SELECT a.x FROM a LEFT JOIN b ON b.x = a.x", [], |r| r.get(0)).optional().unwrap();
        });
        assert_eq!(tables.into_iter().collect::<Vec<_>>(), ["a", "b"]);
    }
}
