//! The code step of migration 37 (brief 75 §4): every account that holds more
//! than one broker id is split to one account per id. The import was the only
//! thing that ever put several ids on one account; the broker states each of
//! them as its own account, and a migrated book is held to be indistinguishable
//! from one synced fresh.
//!
//! The first id stays on the account. Each other id gets an account of its own,
//! its type, status and nickname copied, and every live record with a
//! transaction or a hold on the pooled account is mapped again with its source's
//! mapping: a leg or hold whose broker id is now another account's is moved
//! there in place, so a trade's anchor never changes. A leg that cannot be
//! resolved stays where it is, said as the record's problem. The pooled
//! account's history (its days, values, statements, buying power, backing, links)
//! is deleted, so the first sync reads every part of every split account whole.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};

use bagholder_core::account::AccountRef;
use bagholder_core::{ConnectionId, RecordId};

use crate::mapping::{MapContext, Mapping};

/// What the split did, for the start log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// Accounts that held more than one broker id.
    pub pooled: usize,
    /// Accounts added, one for each id beyond a pooled account's first.
    pub added: usize,
    /// Transactions and holds moved to the account their id now names.
    pub moved: usize,
    /// Records a leg of which could not be resolved, left where they were.
    pub unresolved: usize,
}

fn fail(why: String) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR), Some(why))
}

/// Split every pooled account, with each record's source's mapping among
/// `mappings`. A book with a pooled account and no mapping for a record on it
/// fails the migration, leaving the book as it was.
pub fn split(conn: &Connection, mappings: &[&dyn Mapping], at: jiff::Timestamp) -> rusqlite::Result<Split> {
    let mut done = Split::default();
    let pooled: Vec<String> = conn
        .prepare("SELECT account_id FROM account_refs GROUP BY account_id HAVING COUNT(*) > 1 ORDER BY account_id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let zones = crate::zones::Zones::new();
    for account in pooled {
        done.pooled += 1;
        let refs: Vec<(String, String)> =
            conn.prepare("SELECT scheme, value FROM account_refs WHERE account_id = ?1 ORDER BY rowid")?.query_map([&account], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        // each id beyond the first: an account of its own, as the pooled one was
        for (scheme, value) in refs.iter().skip(1) {
            let id = crate::new_uuid(at).to_string();
            conn.execute(
                "INSERT INTO accounts (id, connection_id, kind, registration, managed, joint, unrecognised_type, status, nickname, created_at)
                 SELECT ?1, connection_id, kind, registration, managed, joint, unrecognised_type, status, nickname, ?2 FROM accounts WHERE id = ?3",
                params![id, crate::text::at(at), account],
            )?;
            conn.execute("UPDATE account_refs SET account_id = ?1 WHERE scheme = ?2 AND value = ?3", params![id, scheme, value])?;
            done.added += 1;
        }
        // the account each broker id names now
        let owner = |r: &AccountRef| -> rusqlite::Result<Option<String>> {
            conn.query_row("SELECT account_id FROM account_refs WHERE scheme = ?1 AND value = ?2", params![r.scheme_text(), r.value], |x| x.get(0)).optional()
        };
        let records: Vec<(String, Option<String>, String)> = conn
            .prepare(
                "SELECT r.id, r.connection_id, r.source FROM source_records r WHERE r.state = 'live' AND (
                    r.id IN (SELECT record_id FROM transactions WHERE account_id = ?1) OR r.id IN (SELECT record_id FROM record_holds WHERE account_id = ?1))
                 ORDER BY r.first_received_at, r.rowid",
            )?
            .query_map([&account], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (record, connection, source) in records {
            let Some(mapping) = mappings.iter().find(|m| m.source().as_str() == source) else {
                return Err(fail(format!("account {account} holds several broker ids, and no mapping reads its {source} record {record}")));
            };
            let payload: String = conn.query_row("SELECT payload FROM record_revisions WHERE record_id = ?1 ORDER BY revision DESC LIMIT 1", [&record], |r| r.get(0))?;
            let ctx = MapContext {
                connection: connection.as_deref().map(ConnectionId::parse).transpose().map_err(|e| fail(format!("record {record}'s connection: {e}")))?,
                record: RecordId::parse(&record).map_err(|e| fail(format!("record {record}: {e}")))?,
                zones: &zones,
            };
            let mapped = mapping.map(&ctx, &payload);
            let mut lost = false;
            let mut legs: BTreeMap<String, Option<String>> = BTreeMap::new();
            for d in &mapped.legs {
                legs.insert(d.leg.as_str().to_string(), owner(&d.account)?);
            }
            let txs: Vec<String> = conn.prepare("SELECT leg FROM transactions WHERE record_id = ?1 AND account_id = ?2")?.query_map(params![record, account], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            for leg in txs {
                match legs.get(&leg) {
                    Some(Some(to)) if *to != account => {
                        conn.execute("UPDATE transactions SET account_id = ?1 WHERE record_id = ?2 AND leg = ?3", params![to, record, leg])?;
                        done.moved += 1;
                    }
                    Some(Some(_)) => {}
                    _ => lost = true,
                }
            }
            let held: Option<String> = conn.query_row("SELECT account_id FROM record_holds WHERE record_id = ?1", [&record], |r| r.get(0)).optional()?;
            if held.as_deref() == Some(account.as_str()) {
                match mapped.hold.as_ref().map(|h| owner(&h.account)).transpose()?.flatten() {
                    Some(to) if to != account => {
                        conn.execute("UPDATE record_holds SET account_id = ?1 WHERE record_id = ?2", params![to, record])?;
                        done.moved += 1;
                    }
                    Some(_) => {}
                    None => lost = true,
                }
            }
            if lost {
                conn.execute(
                    "INSERT INTO record_problems (record_id, code, detail) VALUES (?1, 'account-unknown', ?2)",
                    params![record, format!("a leg of it names no broker id of the accounts {account} was split to; it stays on {account}")],
                )?;
                done.unresolved += 1;
            }
        }
        // its history, read whole again by the first sync
        conn.execute("DELETE FROM statement_cash WHERE statement_id IN (SELECT id FROM statements WHERE account_id = ?1)", [&account])?;
        conn.execute("DELETE FROM statement_units WHERE statement_id IN (SELECT id FROM statements WHERE account_id = ?1)", [&account])?;
        conn.execute("DELETE FROM statement_holds WHERE statement_id IN (SELECT id FROM statements WHERE account_id = ?1)", [&account])?;
        for sql in [
            "DELETE FROM statements WHERE account_id = ?1",
            "DELETE FROM account_days WHERE account_id = ?1",
            "DELETE FROM account_values WHERE account_id = ?1",
            "DELETE FROM buying_power WHERE account_id = ?1",
            "DELETE FROM margin_backing WHERE account_id = ?1 OR margin_account_id = ?1",
            "DELETE FROM account_links WHERE account_id = ?1 OR linked_to = ?1",
            "DELETE FROM activity_reads WHERE account_id = ?1",
        ] {
            conn.execute(sql, [&account])?;
        }
    }
    Ok(done)
}

/// Migration 37's step: the split, and what the migration did said in the start
/// log, counted by its SQL before it deleted anything (`migration_37`).
pub fn step(conn: &Connection, mappings: &[&dyn Mapping], at: jiff::Timestamp) -> rusqlite::Result<()> {
    let counted: Vec<(String, i64)> = conn.prepare("SELECT what, n FROM temp.migration_37 ORDER BY what")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    conn.execute_batch("DROP TABLE temp.migration_37")?;
    let split = split(conn, mappings, at)?;
    let mut said: Vec<String> = counted.into_iter().filter(|(_, n)| *n > 0).map(|(w, n)| format!("{n} {w}")).collect();
    if split.pooled > 0 {
        said.push(format!("{} accounts holding several broker ids split into {} more, {} transactions and holds moved, {} records left with a leg unresolved", split.pooled, split.added, split.moved, split.unresolved));
    }
    if !said.is_empty() {
        bagholder_core::log::line(&format!("bagholder: migration 37: {}", said.join("; ")));
    }
    Ok(())
}
