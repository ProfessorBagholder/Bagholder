//! Generations: for each thing the derived model reads, a counter that moves
//! when, and only when, its rows actually change.
//!
//! The model used to be fingerprinted by counting rows and taking the newest
//! stamp of fourteen tables -- some thirty-six statements a look, several looks
//! a request -- and the stamp was often `fetched_at`, so a margin figure read
//! again and found the same still "changed" the book and sent every open page
//! to fetch all of it. It also missed a change that kept the count and the
//! newest date, which an edit in place does.
//!
//! The counters are kept by the database itself, in triggers, so no writer can
//! forget one and none can move one for nothing: an insert and a delete move
//! the table's counter; an update moves it only when a column that is data
//! differs (`IS NOT`, so a null is compared too), never for a bookkeeping stamp
//! alone. The triggers are generated from each table's columns as they stand
//! and re-made at every start, so a column a later version adds is compared
//! from then on. (docs/architecture.md, rules 3 and 4.)

use rusqlite::{Connection, Result};
use std::collections::BTreeMap;

/// A table the model reads: its counter's name, and the columns that say when
/// a row was read rather than what it holds.
struct Tracked {
    table: &'static str,
    gen: &'static str,
    bookkeeping: &'static [&'static str],
}

const TABLES: &[Tracked] = &[
    Tracked { table: "activities", gen: "activities", bookkeeping: &[] },
    Tracked { table: "securities", gen: "securities", bookkeeping: &["fetched_at"] },
    Tracked { table: "nav_history", gen: "nav", bookkeeping: &[] },
    Tracked { table: "fx_rates", gen: "fx", bookkeeping: &[] },
    Tracked { table: "benchmark_prices", gen: "benchmark", bookkeeping: &[] },
    Tracked { table: "distributions", gen: "distributions", bookkeeping: &[] },
    Tracked { table: "balances", gen: "balances", bookkeeping: &[] },
    Tracked { table: "accounts", gen: "accounts", bookkeeping: &[] },
    Tracked { table: "margin", gen: "margin", bookkeeping: &["fetched_at"] },
    Tracked { table: "exposures", gen: "exposures", bookkeeping: &["fetched_at"] },
    Tracked { table: "watchlist", gen: "watchlist", bookkeeping: &[] },
    Tracked { table: "news", gen: "news", bookkeeping: &["fetched_at"] },
    Tracked { table: "universes", gen: "universes", bookkeeping: &["fetched_at"] },
    // a quote read again at the same price is the same quote
    Tracked { table: "quotes", gen: "quotes", bookkeeping: &["fetched_at"] },
];

/// The `meta` keys the model reads, and the counter each moves.
const META: &[(&str, &str)] = &[
    ("synced_at", "synced"),
    ("trade_groups", "groups"),
    ("trade_notes", "journal"),
    ("journal_v2", "journal"),
    ("market_tiles", "tiles"),
];

/// Every counter there is.
pub fn names() -> Vec<&'static str> {
    let mut out: Vec<&str> = TABLES.iter().map(|t| t.gen).chain(META.iter().map(|m| m.1)).collect();
    out.sort();
    out.dedup();
    out
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let cols = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<Vec<_>>>()?;
    Ok(cols)
}

/// Make the counters and their triggers. Run at every start, after the schema
/// and its migrations, inside the same transaction.
pub fn install(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS gen (name TEXT PRIMARY KEY, n INTEGER NOT NULL DEFAULT 0)")?;
    for name in names() {
        conn.execute("INSERT OR IGNORE INTO gen(name, n) VALUES (?, 0)", [name])?;
    }
    for t in TABLES {
        let bump = format!("UPDATE gen SET n = n + 1 WHERE name = '{}'", t.gen);
        let differs: Vec<String> = columns(conn, t.table)?
            .into_iter()
            .filter(|c| !t.bookkeeping.contains(&c.as_str()))
            .map(|c| format!("OLD.\"{0}\" IS NOT NEW.\"{0}\"", c))
            .collect();
        if differs.is_empty() {
            continue; // the table is not there: nothing to watch
        }
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS gen_{t}_ins; DROP TRIGGER IF EXISTS gen_{t}_del; DROP TRIGGER IF EXISTS gen_{t}_upd;
             CREATE TRIGGER gen_{t}_ins AFTER INSERT ON {t} BEGIN {bump}; END;
             CREATE TRIGGER gen_{t}_del AFTER DELETE ON {t} BEGIN {bump}; END;
             CREATE TRIGGER gen_{t}_upd AFTER UPDATE ON {t} WHEN {when} BEGIN {bump}; END;",
            t = t.table,
            bump = bump,
            when = differs.join(" OR "),
        ))?;
    }
    let keys = META.iter().map(|m| format!("'{}'", m.0)).collect::<Vec<_>>().join(", ");
    let which = |row: &str| {
        let arms = META.iter().map(|m| format!("WHEN '{}' THEN '{}'", m.0, m.1)).collect::<Vec<_>>().join(" ");
        format!("UPDATE gen SET n = n + 1 WHERE name = CASE {}.key {} END", row, arms)
    };
    conn.execute_batch(&format!(
        "DROP TRIGGER IF EXISTS gen_meta_ins; DROP TRIGGER IF EXISTS gen_meta_del; DROP TRIGGER IF EXISTS gen_meta_upd;
         CREATE TRIGGER gen_meta_ins AFTER INSERT ON meta WHEN NEW.key IN ({keys}) BEGIN {new}; END;
         CREATE TRIGGER gen_meta_del AFTER DELETE ON meta WHEN OLD.key IN ({keys}) BEGIN {old}; END;
         CREATE TRIGGER gen_meta_upd AFTER UPDATE ON meta WHEN NEW.key IN ({keys}) AND OLD.value IS NOT NEW.value BEGIN {new}; END;",
        keys = keys,
        new = which("NEW"),
        old = which("OLD"),
    ))?;
    Ok(())
}

/// Every counter, by name, in one statement.
pub fn all(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare_cached("SELECT name, n FROM gen")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<BTreeMap<_, _>>>()?;
    Ok(rows)
}

/// The counters named, spelled as one key: equal keys mean nothing among them
/// changed.
pub fn key(gens: &BTreeMap<String, i64>, of: &[&str]) -> String {
    of.iter().map(|n| format!("{}:{}", n, gens.get(*n).copied().unwrap_or(0))).collect::<Vec<_>>().join("|")
}

fn rows_of(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<Vec<String>>> {
    let mut stmt = conn.prepare(sql)?;
    let n = stmt.column_count();
    let mut out = Vec::new();
    let mut q = stmt.query(params)?;
    while let Some(r) = q.next()? {
        out.push((0..n).map(|i| r.get_ref(i).map(|v| format!("{:?}", v))).collect::<Result<Vec<_>>>()?);
    }
    out.sort();
    Ok(out)
}

/// Run a writer that replaces a set of rows (delete them, insert the new ones),
/// and keep what it wrote only if the set is different afterwards. `watch`
/// selects the data columns of the rows in question -- never a bookkeeping stamp
/// -- and the rows are compared as the database holds them, so "the same" is
/// exact and needs no second copy of the writer's own conversions. When nothing
/// differs the writes are undone, the counters with them, and `false` comes
/// back: the caller re-stamps whatever says when the rows were last read.
/// Must run inside `atomically`.
pub fn replace_if_changed(conn: &Connection, watch: &str, params: &[&dyn rusqlite::ToSql], write: impl FnOnce() -> Result<()>) -> Result<bool> {
    debug_assert!(!conn.is_autocommit(), "replace_if_changed runs inside atomically");
    let before = rows_of(conn, watch, params)?;
    conn.execute_batch("SAVEPOINT replace_if_changed")?;
    let done = write().and_then(|_| rows_of(conn, watch, params));
    match done {
        Ok(after) if after != before => {
            conn.execute_batch("RELEASE replace_if_changed")?;
            Ok(true)
        }
        Ok(_) => {
            conn.execute_batch("ROLLBACK TO replace_if_changed; RELEASE replace_if_changed")?;
            Ok(false)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO replace_if_changed; RELEASE replace_if_changed");
            Err(e)
        }
    }
}
