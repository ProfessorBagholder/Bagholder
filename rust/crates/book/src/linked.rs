//! One book account per set of broker ids the broker links as one (the
//! accounts read's `linkedAccount`, stored as `account_links`): an account the
//! broker gave a new id, its old id closed, is one account under both ids.
//!
//! Migration 37 split every account holding several ids to one account per id
//! (`pooled::split`), and so split what the broker links as one: a TFSA whose
//! holdings and cash moved to its new id with no row of the move, its sales on
//! the new id of units bought on the old (owner, 2026-10-23). The merge is that
//! split's inverse for a linked set, and nothing more: the set's oldest account
//! keeps its row and id and takes every other's broker ids, transactions and
//! holds, in place, so a trade's anchor and its note never change; the set's
//! history rows (days, values, statements, buying power, backing, rates,
//! activity reads) are deleted, so the next sync reads the merged account whole,
//! as the split did; and the emptied accounts go. Migration 39 runs it on every
//! book, and every accounts read runs it again, so a link the broker states
//! after both ids were added merges them then.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection};

/// What a merge did, for the start log and the pull's report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// Linked sets that sat on more than one account.
    pub sets: usize,
    /// Accounts merged into their set's oldest and removed.
    pub removed: usize,
    /// Transactions and holds moved onto the set's oldest account.
    pub moved: usize,
}

fn fail(why: String) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR), Some(why))
}

/// Merge every linked set of accounts, within one connection, to its oldest
/// account. Run inside the caller's transaction: a merge that leaves anything
/// pointing at a removed account fails, and the caller's rollback leaves the
/// book as it was.
pub fn merge(conn: &Connection) -> rusqlite::Result<Merged> {
    let mut done = Merged::default();
    // the links, both ways, between two accounts of one connection
    let links: Vec<(String, String)> = conn
        .prepare(
            "SELECT l.account_id, l.linked_to FROM account_links l
               JOIN accounts a ON a.id = l.account_id JOIN accounts b ON b.id = l.linked_to
              WHERE a.connection_id = b.connection_id AND l.account_id <> l.linked_to",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // the sets: each account's set by union-find
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    fn root(parent: &mut BTreeMap<String, String>, a: &str) -> String {
        let p = parent.get(a).cloned().unwrap_or_else(|| a.to_string());
        if p == a {
            return p;
        }
        let r = root(parent, &p);
        parent.insert(a.to_string(), r.clone());
        r
    }
    for (a, b) in &links {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent.insert(ra, rb);
        }
    }
    let mut sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (a, b) in &links {
        for x in [a, b] {
            let r = root(&mut parent, x);
            sets.entry(r).or_default().insert(x.clone());
        }
    }
    for members in sets.values() {
        // the oldest account keeps its row and id
        // two made in one read share a moment: the one the book added first
        let mut by_age: Vec<((String, i64), String, String)> = Vec::new();
        for id in members {
            let (created, rowid, status): (String, i64, String) = conn.query_row("SELECT created_at, rowid, status FROM accounts WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            by_age.push(((created, rowid), id.clone(), status));
        }
        by_age.sort();
        let keep = by_age[0].1.clone();
        let others: Vec<String> = by_age[1..].iter().map(|(_, id, _)| id.clone()).collect();
        done.sets += 1;
        // open if any id is open; type and nickname the open one's, else the newest's
        let open = by_age.iter().any(|(_, _, s)| s == "open");
        let from = by_age.iter().rev().find(|(_, _, s)| !open || s == "open").map(|(_, id, _)| id.clone()).expect("a set has members");
        if from != keep {
            conn.execute(
                "UPDATE accounts SET (kind, registration, managed, joint, unrecognised_type, nickname) =
                   (SELECT kind, registration, managed, joint, unrecognised_type, nickname FROM accounts WHERE id = ?1) WHERE id = ?2",
                params![from, keep],
            )?;
        }
        conn.execute("UPDATE accounts SET status = ?1 WHERE id = ?2", params![if open { "open" } else { "closed" }, keep])?;
        let all: Vec<&String> = by_age.iter().map(|(_, id, _)| id).collect();
        for id in &all {
            // the set's history, read whole again by the next sync
            conn.execute("DELETE FROM statement_cash WHERE statement_id IN (SELECT id FROM statements WHERE account_id = ?1)", [id])?;
            conn.execute("DELETE FROM statement_units WHERE statement_id IN (SELECT id FROM statements WHERE account_id = ?1)", [id])?;
            conn.execute("DELETE FROM statement_holds WHERE statement_id IN (SELECT id FROM statements WHERE account_id = ?1)", [id])?;
            for sql in [
                "DELETE FROM statements WHERE account_id = ?1",
                "DELETE FROM account_days WHERE account_id = ?1",
                "DELETE FROM account_values WHERE account_id = ?1",
                "DELETE FROM buying_power WHERE account_id = ?1",
                "DELETE FROM margin_backing WHERE account_id = ?1 OR margin_account_id = ?1",
                "DELETE FROM margin_rates WHERE account_id = ?1",
                "DELETE FROM activity_reads WHERE account_id = ?1",
                "DELETE FROM account_links WHERE account_id = ?1 OR linked_to = ?1",
            ] {
                conn.execute(sql, [id])?;
            }
        }
        for other in &others {
            conn.execute("UPDATE account_refs SET account_id = ?1 WHERE account_id = ?2", params![keep, other])?;
            done.moved += conn.execute("UPDATE transactions SET account_id = ?1 WHERE account_id = ?2", params![keep, other])?;
            done.moved += conn.execute("UPDATE record_holds SET account_id = ?1 WHERE account_id = ?2", params![keep, other])?;
            // anything else still naming it is something this merge cannot place
            let left: i64 = conn.query_row(
                "SELECT (SELECT COUNT(*) FROM account_refs WHERE account_id = ?1)
                      + (SELECT COUNT(*) FROM transactions WHERE account_id = ?1)
                      + (SELECT COUNT(*) FROM record_holds WHERE account_id = ?1)",
                [other],
                |r| r.get(0),
            )?;
            if left > 0 {
                return Err(fail(format!("account {other} still has {left} rows after its merge into {keep}")));
            }
            conn.execute("DELETE FROM accounts WHERE id = ?1", [other])?;
            done.removed += 1;
        }
    }
    // nothing in the book may name an account that is gone, checked where one went
    if done.removed > 0 {
        let dangling: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0))?;
        if dangling > 0 {
            return Err(fail(format!("{dangling} rows name an account a merge removed")));
        }
    }
    Ok(done)
}

/// Migration 39's step: the merge, and what it did said in the start log.
pub fn step(conn: &Connection) -> rusqlite::Result<()> {
    let merged = merge(conn)?;
    if merged.sets > 0 {
        bagholder_core::log::line(&format!(
            "bagholder: migration 39: {} linked sets of accounts made one, {} accounts merged, {} transactions and holds moved",
            merged.sets, merged.removed, merged.moved
        ));
    }
    Ok(())
}
