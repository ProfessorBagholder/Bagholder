//! The database work the process does, counted for the capacity test
//! (`docs/architecture.md` §16, the growth rule): every virtual-machine step
//! SQLite takes on every connection the app opens, which is the same on every
//! machine. Off unless `start_counting` is called before the connections are
//! opened; the app itself never calls it, so no connection of a running app
//! carries the counter.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static COUNTING: AtomicBool = AtomicBool::new(false);
static STEPS: AtomicU64 = AtomicU64::new(0);

/// Count from here on: each connection opened after this counts its steps.
pub fn start_counting() {
    COUNTING.store(true, Ordering::SeqCst);
}

/// The steps counted so far, on every counted connection.
pub fn steps() -> u64 {
    STEPS.load(Ordering::SeqCst)
}

/// A connection just opened, counted where counting is on.
pub(crate) fn watch(conn: &rusqlite::Connection) {
    if COUNTING.load(Ordering::Relaxed) {
        // the handler is called once per virtual-machine instruction, and never interrupts
        conn.progress_handler(
            1,
            Some(|| {
                STEPS.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
