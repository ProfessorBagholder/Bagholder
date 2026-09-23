//! The import of a database kept by an earlier version of Bagholder (schema 13:
//! Python's `~/.bagholder/bagholder.db`, or the Rust build's
//! `~/.bagholder-rust/bagholder.db`), into a book (`docs/plans/stage-1-foundation.md`).
//!
//! The old file is never written: it is copied with SQLite's backup API and only
//! the copy is read (`old.rs`). Its activity rows are already mapped, not what
//! Wealthsimple sent, so each becomes a record of a source of its own,
//! `bagholder-import`, mapped by `mapping.rs`; when Wealthsimple's own rows are
//! stored (stage 3) each supersedes the imported record with its id.
//!
//! The journal and the groups are keyed by the old engine's own trade and slice
//! ids (a round trip's first fill, a lane's hash, a saved group's id), which only
//! that engine can read. The server crate, which still has it, translates every
//! key into the row that opened its trade, or the group it names, or why neither
//! could be found (`Translated`), and the book places them here.

pub mod mapping;
pub mod old;

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
use bagholder_core::journal::{JournalEntry, JournalSubject, Opening};
use bagholder_core::{Broker, ConnectionId, GroupId, Leg, TradeId, TransactionId};

use crate::records::{Incoming, Outcome};
use crate::{Book, Result};

pub use mapping::{import_source, ImportMapping, ImportedRow, IMPORT_SOURCE, WEALTHSIMPLE_RECORD};
pub use old::{copy_database, OldAccount, OldActivity, OldDatabase, OldSecurity};

/// The one leg an imported row becomes: each old row was one transaction.
pub fn row_leg() -> Leg {
    Leg::named("row")
}

/// The earlier app's journal and groups, with every key translated by the
/// engine that made it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Translated {
    pub journal: Vec<ImportedNote>,
    pub groups: Vec<TranslatedGroup>,
}

/// A note the earlier app kept, and what it was written on.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedNote {
    /// The key the earlier app kept it under.
    pub key: String,
    pub on: NoteOn,
    pub entry: JournalEntry,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NoteOn {
    /// A trade: the id of the row that opened it, or why none was found.
    Trade(std::result::Result<String, String>),
    /// A group the person saved, by its key in the earlier app.
    Group(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranslatedGroup {
    pub key: String,
    pub locked: bool,
    /// Each member's old key, and the row that opened its trade or why none was found.
    pub members: Vec<(String, std::result::Result<String, String>)>,
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub rows_read: usize,
    pub records_new: usize,
    pub records_revised: usize,
    pub records_unchanged: usize,
    pub accounts_made: usize,
    pub transactions_by_kind: BTreeMap<String, usize>,
    pub problems_by_code: BTreeMap<String, usize>,
    /// Account groupings the old book made that were not kept, and why.
    pub account_problems: Vec<String>,
    pub journal_attached: usize,
    /// Journal keys kept on orphaned trades: the key and why.
    pub journal_orphaned: Vec<(String, String)>,
    pub groups: usize,
    pub group_members_attached: usize,
    pub group_members_orphaned: Vec<(String, String)>,
}

impl Book {
    /// Import `old` (read from a copy by `old::read`) into this book. Importing
    /// the same database again changes nothing.
    pub fn import(&self, old: &OldDatabase, translated: &Translated, at: jiff::Timestamp) -> Result<Report> {
        self.atomically(|| {
            let mut report = Report { rows_read: old.activities.len(), ..Report::default() };
            // a database with no accounts and no rows has no connection, and its
            // notes, if any, are all kept orphaned
            let connection = self.import_connection(old, at)?;
            if let Some(connection) = connection {
                self.import_accounts(old, connection, &mut report, at)?;
            }
            let mapping = ImportMapping;
            for row in old.activities.iter().filter(|_| connection.is_some()) {
                let payload = ImportedRow::of(row, old);
                let text = serde_json::to_string(&payload).map_err(|e| crate::BookError::Refused(format!("an imported row could not be written: {e}")))?;
                let mut refs = Vec::new();
                if let Some(cid) = row.canonical_id.as_deref().filter(|c| !c.is_empty()) {
                    refs.push((WEALTHSIMPLE_RECORD.to_string(), cid.to_string()));
                }
                let incoming = Incoming { connection, source_key: &row.id, payload: &text, refs };
                let stored = self.store(&mapping, &incoming, at)?;
                match stored.outcome {
                    Outcome::New => report.records_new += 1,
                    Outcome::Revised(_) => report.records_revised += 1,
                    Outcome::Unchanged => report.records_unchanged += 1,
                }
            }
            for r in self.live_records(&import_source())? {
                for t in self.transactions_of(r)? {
                    *report.transactions_by_kind.entry(t.kind.as_str().to_string()).or_default() += 1;
                }
            }
            for (r, p) in self.problems()? {
                if self.record(r)?.source == import_source() {
                    *report.problems_by_code.entry(p.code).or_default() += 1;
                }
            }
            let groups = self.import_groups(translated, connection, &mut report, at)?;
            self.import_journal(translated, &groups, connection, &mut report, at)?;
            Ok(report)
        })
    }

    /// The Wealthsimple connection the old database's rows came through: the one
    /// an earlier import made (found by any account id the database names), or a
    /// new one. A database with no accounts and no rows has nothing to import.
    fn import_connection(&self, old: &OldDatabase, at: jiff::Timestamp) -> Result<Option<ConnectionId>> {
        let ws = Broker::named("wealthsimple");
        let ids = old.accounts.iter().map(|a| a.id.as_str()).chain(old.activities.iter().filter_map(|r| r.account_id.as_deref()));
        let mut any = false;
        for id in ids.filter(|s| !s.is_empty()) {
            any = true;
            if let Some(account) = self.account_by_ref(&AccountRef::new(ws.clone(), id))? {
                return Ok(Some(self.account(account)?.connection));
            }
        }
        if !any {
            return Ok(None);
        }
        Ok(Some(self.add_connection(&ws, "Wealthsimple", at)?))
    }

    /// One account per group of Wealthsimple ids the old book pooled (the CAD
    /// and USD sides of one account), each id a reference. A group whose ids the
    /// broker typed differently is not merged.
    fn import_accounts(&self, old: &OldDatabase, connection: ConnectionId, report: &mut Report, at: jiff::Timestamp) -> Result<()> {
        let ws = Broker::named("wealthsimple");
        let by_id: BTreeMap<&str, &OldAccount> = old.accounts.iter().map(|a| (a.id.as_str(), a)).collect();
        // the old book's pools: every account id a row names, under its pool's id
        let mut pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for row in &old.activities {
            let (Some(aid), Some(pool)) = (row.account_id.as_deref().filter(|s| !s.is_empty()), row.fifo_id.as_deref().filter(|s| !s.is_empty())) else { continue };
            // the pool's id is itself an account id: the one the old book filed the pool under
            let members = pools.entry(pool.to_string()).or_default();
            members.insert(aid.to_string());
            members.insert(pool.to_string());
        }
        let mut groups: Vec<BTreeSet<String>> = Vec::new();
        let mut placed: BTreeSet<String> = BTreeSet::new();
        // pools that share an id are one group
        for members in pools.into_values() {
            let mut merged = members;
            groups.retain(|g| {
                if g.iter().any(|x| merged.contains(x)) {
                    merged.extend(g.iter().cloned());
                    false
                } else {
                    true
                }
            });
            groups.push(merged);
        }
        for g in &groups {
            placed.extend(g.iter().cloned());
        }
        for a in &old.accounts {
            if !placed.contains(&a.id) {
                groups.push(BTreeSet::from([a.id.clone()]));
            }
        }
        // a row's account id that the accounts table does not have is still an account
        for row in &old.activities {
            if let Some(aid) = row.account_id.as_deref().filter(|s| !s.is_empty()) {
                if !groups.iter().any(|g| g.contains(aid)) {
                    groups.push(BTreeSet::from([aid.to_string()]));
                }
            }
        }
        for g in groups {
            let typed: Vec<(String, AccountType)> = g.iter().map(|id| (id.clone(), by_id.get(id.as_str()).map_or_else(|| AccountType::Unrecognised("no account row in the imported database".into()), |a| account_type(a)))).collect();
            for (id, t) in typed.iter().filter(|(id, _)| g.len() == 1 || by_id.contains_key(id.as_str())) {
                if let AccountType::Unrecognised(words) = t {
                    report.account_problems.push(format!("account {id}: a type the import does not know ({words}); kept in those words"));
                }
            }
            // an id the old book pooled but the accounts table lacks takes the
            // pool's type; the types that are stated must agree
            let stated: Vec<&AccountType> = typed.iter().filter(|(id, _)| by_id.contains_key(id.as_str())).map(|(_, t)| t).collect();
            let first = stated.first().map(|t| (*t).clone()).unwrap_or_else(|| typed[0].1.clone());
            let sets: Vec<Vec<String>> = if stated.iter().all(|t| **t == first) {
                vec![g.iter().cloned().collect()]
            } else {
                report.account_problems.push(format!(
                    "the earlier app counted accounts {} as one, but the broker types them differently; they are kept apart",
                    g.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
                g.iter().map(|id| vec![id.clone()]).collect()
            };
            for ids in sets {
                let refs: Vec<AccountRef> = ids.iter().map(|id| AccountRef::new(ws.clone(), id.clone())).collect();
                // imported before: a pool that has grown since gains the new ids
                let mut existing = BTreeSet::new();
                for r in &refs {
                    if let Some(a) = self.account_by_ref(r)? {
                        existing.insert(a);
                    }
                }
                if existing.len() > 1 {
                    report.account_problems.push(format!(
                        "the earlier app now counts accounts {} as one, but an earlier import made them separate accounts; they are kept apart",
                        ids.join(", ")
                    ));
                    continue;
                }
                if let Some(account) = existing.into_iter().next() {
                    for r in &refs {
                        if self.account_by_ref(r)?.is_none() {
                            self.add_account_ref(account, r)?;
                        }
                    }
                    continue;
                }
                let members: Vec<&OldAccount> = ids.iter().filter_map(|id| by_id.get(id.as_str()).copied()).collect();
                let status = if members.is_empty() || members.iter().any(|a| a.status.as_deref() != Some("closed")) {
                    AccountStatus::Open
                } else {
                    AccountStatus::Closed
                };
                let nickname = members.iter().filter_map(|a| a.nickname.as_deref()).find(|n| !n.trim().is_empty());
                let account_type = typed
                    .iter()
                    .find(|(id, _)| ids.contains(id) && by_id.contains_key(id.as_str()))
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| if ids.len() > 1 { first.clone() } else { typed.iter().find(|(id, _)| ids.contains(id)).map(|(_, t)| t.clone()).unwrap_or(first.clone()) });
                self.add_account(connection, &refs, &account_type, status, nickname, at)?;
                report.accounts_made += 1;
            }
        }
        Ok(())
    }

    fn import_groups(&self, translated: &Translated, connection: Option<ConnectionId>, report: &mut Report, at: jiff::Timestamp) -> Result<BTreeMap<String, GroupId>> {
        let mut made = BTreeMap::new();
        for g in &translated.groups {
            let mut members: Vec<TradeId> = Vec::new();
            for (key, row) in &g.members {
                let (t, orphaned) = self.trade_for_row(key, row.clone(), connection, at)?;
                match orphaned {
                    None => report.group_members_attached += 1,
                    Some(why) => report.group_members_orphaned.push((key.clone(), why)),
                }
                // two slices of one round trip are one member
                if !members.contains(&t) {
                    members.push(t);
                }
            }
            made.insert(g.key.clone(), self.add_group(&members, g.locked, Some(&g.key), at)?);
            report.groups += 1;
        }
        Ok(made)
    }

    fn import_journal(&self, translated: &Translated, groups: &BTreeMap<String, GroupId>, connection: Option<ConnectionId>, report: &mut Report, at: jiff::Timestamp) -> Result<()> {
        for note in &translated.journal {
            let (subject, orphaned) = match &note.on {
                NoteOn::Group(key) => match groups.get(key) {
                    Some(g) => (JournalSubject::Group(*g), None),
                    None => {
                        let why = format!("the earlier app wrote it on a group ({key}) that is not among its saved groups");
                        (JournalSubject::Trade(self.orphan_trade(&note.key, &why, at)?), Some(why))
                    }
                },
                NoteOn::Trade(row) => {
                    let (t, orphaned) = self.trade_for_row(&note.key, row.clone(), connection, at)?;
                    (JournalSubject::Trade(t), orphaned)
                }
            };
            // two of the earlier app's keys can name one trade: the second note is
            // kept on its own, orphaned, rather than written over the first
            let taken = match self.journal(subject)? {
                Some(existing) if existing != note.entry => match subject {
                    JournalSubject::Trade(t) => self.trade(t)?.legacy_key.as_deref() != Some(note.key.as_str()),
                    JournalSubject::Group(_) => true,
                },
                _ => false,
            };
            let (subject, orphaned) = if taken {
                let why = "another of the earlier app's notes is on the same trade".to_string();
                (JournalSubject::Trade(self.orphan_trade(&note.key, &why, at)?), Some(why))
            } else {
                (subject, orphaned)
            };
            self.write_journal_if_changed(subject, &note.entry, at)?;
            match orphaned {
                None => report.journal_attached += 1,
                Some(why) => report.journal_orphaned.push((note.key.clone(), why)),
            }
        }
        Ok(())
    }

    /// The trade a row opened, known in the earlier app by `key`; or, where the
    /// row or its transaction cannot be found, a trade kept orphaned under `key`
    /// with the reason (returned as well).
    fn trade_for_row(&self, key: &str, row: std::result::Result<String, String>, connection: Option<ConnectionId>, at: jiff::Timestamp) -> Result<(TradeId, Option<String>)> {
        let row = match row {
            Ok(row) => row,
            Err(why) => return Ok((self.orphan_trade(key, &why, at)?, Some(why))),
        };
        let Some(record) = self.record_by_key(connection, &import_source(), &row)? else {
            let why = format!("the row {row} that opened it is not in the imported database (the earlier app made some rows up, such as expiries it inferred)");
            return Ok((self.orphan_trade(key, &why, at)?, Some(why)));
        };
        let opening = TransactionId::new(record, row_leg());
        let found = self.transaction(&opening)?;
        let moves = found.as_ref().map(|t| t.instrument.is_some() && t.quantity.is_some_and(|q| !q.is_zero()));
        if moves != Some(true) {
            if let Some(t) = self.trade_by_legacy_key(key)? {
                return Ok((t, self.trade(t)?.orphaned_reason()));
            }
            let why = match (moves, self.problems_of(record)?.first()) {
                (_, Some(p)) => format!("the row that opened it could not be booked whole: {}", p.detail),
                (Some(false), None) => "the row that opened it moves no position".to_string(),
                _ => "the row that opened it has no transaction".to_string(),
            };
            return Ok((self.orphan_trade(key, &why, at)?, Some(why)));
        }
        // a key orphaned by an earlier import stays that trade, and is reported as orphaned
        let instrument = found.and_then(|t| t.instrument).expect("a transaction that moves a position names its instrument");
        let t = self.open_trade(&Opening { transaction: opening, instrument }, Some(key), at)?;
        Ok((t, self.trade(t)?.orphaned_reason()))
    }

    fn write_journal_if_changed(&self, subject: JournalSubject, entry: &JournalEntry, at: jiff::Timestamp) -> Result<()> {
        if self.journal(subject)?.as_ref() != Some(entry) {
            self.set_journal(subject, entry, at)?;
        }
        Ok(())
    }
}

/// What Wealthsimple's `unifiedAccountType` says an account is, in Bagholder's
/// vocabulary; a type not listed here is kept in Wealthsimple's words.
pub fn account_type(a: &OldAccount) -> AccountType {
    use AccountKind::*;
    use Registration::*;
    let words = a.unified_account_type.clone().unwrap_or_default();
    let known = |kind, registration, managed, joint| AccountType::Known { kind, registration, managed, joint };
    match words.as_str() {
        "SELF_DIRECTED_NON_REGISTERED" => known(Cash, Unregistered, false, false),
        "SELF_DIRECTED_NON_REGISTERED_MARGIN" => known(Margin, Unregistered, false, false),
        "SELF_DIRECTED_JOINT_NON_REGISTERED" => known(Cash, Unregistered, false, true),
        "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN" => known(Margin, Unregistered, false, true),
        "SELF_DIRECTED_TFSA" => known(Cash, Tfsa, false, false),
        "SELF_DIRECTED_FHSA" => known(Cash, Fhsa, false, false),
        "SELF_DIRECTED_RRSP" => known(Cash, Rrsp, false, false),
        "SELF_DIRECTED_RRIF" => known(Cash, Rrif, false, false),
        "SELF_DIRECTED_LIRA" => known(Cash, Lira, false, false),
        "SELF_DIRECTED_RESP_FAMILY" => known(Cash, Resp, false, false),
        "SELF_DIRECTED_JOINT_RESP_FAMILY" => known(Cash, Resp, false, true),
        "SELF_DIRECTED_CRYPTO" => known(Crypto, Unregistered, false, false),
        "SELF_DIRECTED_NON_REGISTERED_PREDICTIONS" => known(EventContracts, Unregistered, false, false),
        "MANAGED_NON_REGISTERED" | "MANAGED_PORTFOLIO_NON_REGISTERED" => known(Cash, Unregistered, true, false),
        "MANAGED_JOINT" => known(Cash, Unregistered, true, true),
        "MANAGED_TFSA" => known(Cash, Tfsa, true, false),
        "MANAGED_FHSA" => known(Cash, Fhsa, true, false),
        "MANAGED_RRSP" => known(Cash, Rrsp, true, false),
        "MANAGED_LIRA" => known(Cash, Lira, true, false),
        "MANAGED_RESP_FAMILY" => known(Cash, Resp, true, false),
        "MANAGED_GROUP_RRSP" => known(Cash, GroupRrsp, true, false),
        "CASH" | "CASH_USD" | "YOUTH_CASH" => known(Spending, Unregistered, false, false),
        "CREDIT_CARD" => known(CreditCard, Unregistered, false, false),
        "PORTFOLIO_LINE_OF_CREDIT" => known(LineOfCredit, Unregistered, false, false),
        _ => AccountType::Unrecognised(if words.is_empty() { "no type given".into() } else { words }),
    }
}
