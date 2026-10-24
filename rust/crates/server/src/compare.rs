//! `bagholder compare <a> <b>`: two books' figures side by side, every difference
//! named, for anyone checking what an update did to a book before trusting it
//! (README, Keeping up to date).
//!
//! Each side is a data folder (its `book.db`, and `market.db` where there is one)
//! or a book file alone (a snapshot under `snapshots/`). Neither is touched: each
//! is copied to a folder of its own, opened there by this version (migrated and
//! derived again as an update would), and the copy deleted. Nothing is asked of
//! the network. What is printed: each account's units of each holding and its
//! cash in each currency where the two books differ, then what each book
//! disagrees with its broker on, as the header says it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bagholder_core::Dec;

/// One book's figures: by (account, what) its amount as text.
#[derive(Debug, Default, PartialEq)]
pub struct Read {
    pub held: BTreeMap<(String, String), String>,
    /// What the book disagrees with its broker on, as the header says it.
    pub said: Vec<String>,
}

/// The command: 0 when it ran, 1 when a side could not be read, 2 when asked wrong.
pub fn cli(args: &[String]) -> i32 {
    let [a, b] = args else {
        eprintln!("usage: bagholder compare <data folder or book file> <data folder or book file>");
        return 2;
    };
    match (read(Path::new(a)), read(Path::new(b))) {
        (Ok(x), Ok(y)) => {
            print!("{}", report(a, b, &x, &y));
            0
        }
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("bagholder compare: {e}");
            1
        }
    }
}

/// A folder of the command's own for one copy, deleted when done with.
struct Apart(PathBuf);

impl Apart {
    fn new() -> Result<Apart, String> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bagholder-compare-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("no folder for a copy at {}: {e}", dir.display()))?;
        Ok(Apart(dir))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Apart {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A side's book (and market cache where it has one), copied apart and opened.
pub fn read(path: &Path) -> Result<Read, String> {
    let copy = Apart::new()?;
    let (book, market): (PathBuf, Option<PathBuf>) = if path.is_dir() { (path.join("book.db"), Some(path.join(crate::figures::CACHE_FILE))) } else { (path.to_path_buf(), None) };
    if !book.is_file() {
        return Err(format!("{} holds no book", path.display()));
    }
    let take = |from: &Path, to: &str| -> Result<(), String> {
        std::fs::copy(from, copy.path().join(to)).map_err(|e| format!("{} could not be copied: {e}", from.display()))?;
        // the writes a running app has not folded into the file yet
        let wal = PathBuf::from(format!("{}-wal", from.display()));
        if wal.is_file() {
            std::fs::copy(&wal, copy.path().join(format!("{to}-wal"))).map_err(|e| format!("{} could not be copied: {e}", wal.display()))?;
        }
        Ok(())
    };
    take(&book, "book.db")?;
    if let Some(m) = market.filter(|m| m.is_file()) {
        take(&m, crate::figures::CACHE_FILE)?;
    }
    let f = crate::figures::Figures::open(copy.path(), bagholder_core::jiff::Timestamp::now()).map_err(|e| format!("{}: {e}", path.display()))?;
    f.read(of).ok_or_else(|| format!("{}: the book states no time zone, so nothing is worked out from it", path.display()))
}

/// The figures of a built engine.
pub fn of(e: &bagholder_engine::Engine) -> Read {
    let inputs = e.inputs();
    let figures = e.figures();
    let name = |a| crate::wire::build::account_name(inputs, a);
    let text = |v: &bagholder_engine::gap::Fig<Dec>| match v {
        Ok(d) => d.to_text(),
        Err(g) => format!("cannot be worked out ({})", g.words().join(", ")),
    };
    let mut held: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut units: BTreeMap<(String, String), bagholder_engine::gap::Fig<Dec>> = BTreeMap::new();
    for p in figures.positions {
        let key = (name(p.account), crate::wire::build::shown(inputs, p.instrument).symbol);
        let sum = match (units.remove(&key).unwrap_or(Ok(Dec::ZERO)), &p.qty) {
            (Ok(a), Ok(q)) => a.checked_add(*q).map_err(bagholder_engine::gap::Gaps::from),
            (Err(g), _) => Err(g),
            (_, Err(g)) => Err(g.clone()),
        };
        units.insert(key, sum);
    }
    for (k, v) in units {
        if v.as_ref().is_ok_and(|q| q.is_zero()) {
            continue;
        }
        held.insert(k, text(&v));
    }
    for (account, by) in &figures.matched.cash {
        for (c, v) in by {
            if v.as_ref().is_ok_and(|q| q.is_zero()) {
                continue;
            }
            held.insert((name(*account), format!("{c} cash")), text(v));
        }
    }
    Read { held, said: crate::status::broker_failures(e) }
}

/// What differs, then what each book disagrees with its broker on.
pub fn report(a: &str, b: &str, x: &Read, y: &Read) -> String {
    let mut out = format!("A: {a}\nB: {b}\n\n");
    let keys: BTreeSet<&(String, String)> = x.held.keys().chain(y.held.keys()).collect();
    let differ: Vec<String> = keys
        .into_iter()
        .filter_map(|k| {
            let (l, r) = (x.held.get(k).map_or("none", String::as_str), y.held.get(k).map_or("none", String::as_str));
            (l != r).then(|| format!("{} · {}: A {l}, B {r}", k.0, k.1))
        })
        .collect();
    if differ.is_empty() {
        out.push_str("Every account's holdings and cash are the same in both.\n");
    } else {
        out.push_str(&format!("{} {} in holdings and cash:\n", differ.len(), if differ.len() == 1 { "difference" } else { "differences" }));
        for d in differ {
            out.push_str(&format!("  {d}\n"));
        }
    }
    for (side, r) in [("A", x), ("B", y)] {
        out.push('\n');
        if r.said.is_empty() {
            out.push_str(&format!("{side} agrees with its broker on everything it was last told.\n"));
        } else {
            out.push_str(&format!("{side} disagrees with its broker on:\n"));
            for s in &r.said {
                out.push_str(&format!("  {s}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_of(held: &[(&str, &str, &str)], said: &[&str]) -> Read {
        Read { held: held.iter().map(|(a, w, v)| ((a.to_string(), w.to_string()), v.to_string())).collect(), said: said.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn a_report_names_each_difference_with_both_sides_and_each_book_s_disagreements() {
        let x = read_of(&[("Trading", "ZQA", "10"), ("Trading", "CAD cash", "100.00"), ("Saving", "ZQB", "5")], &["ZQA in Trading: Wealthsimple states none, the book holds 10."]);
        let y = read_of(&[("Trading", "CAD cash", "100.00"), ("Saving", "ZQB", "7")], &[]);
        let r = report("before", "after", &x, &y);
        assert!(r.contains("2 differences in holdings and cash:\n  Saving · ZQB: A 5, B 7\n  Trading · ZQA: A 10, B none\n"), "{r}");
        assert!(r.contains("A disagrees with its broker on:\n  ZQA in Trading: Wealthsimple states none, the book holds 10.\n"), "{r}");
        assert!(r.contains("B agrees with its broker on everything it was last told.\n"), "{r}");
        assert!(report("a", "b", &y, &y).contains("Every account's holdings and cash are the same in both.\n"));
    }

    #[test]
    fn a_side_that_holds_no_book_is_refused_naming_it() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(read(empty.path()).unwrap_err(), format!("{} holds no book", empty.path().display()));
    }
}
