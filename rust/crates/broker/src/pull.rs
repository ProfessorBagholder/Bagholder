//! The pull (`docs/plans/stage-3b-wealthsimple.md`, "The pull, into the book"):
//! the accounts and their links, each account's activity as records (each
//! replacing the imported record of the same broker id), the two sides of a move
//! of holdings linked, and the broker's statements. The first pull reads each
//! account whole; after that only what changed (brief 07 §1).
//!
//! Each part's failure is its own: named in the report, the other parts carry
//! on, and nothing already stored is removed or overwritten by a failed part.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::records::Incoming;
use bagholder_book::statements::{AccountDay, UnitsLine};
use bagholder_book::Book;
use bagholder_core::account::{AccountRef, AccountStatus};
use bagholder_core::instrument::RefScheme;
use bagholder_core::json;
use bagholder_core::record::RecordState;
use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, ConnectionId, Dec, RecordId};

use crate::{BookMoves, BrokerAdapter, Failure, Moved, MovedWhat, Row};

/// What one pull did and what failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub accounts_added: usize,
    pub accounts_linked: usize,
    pub rows_read: usize,
    pub records_new: usize,
    pub records_revised: usize,
    pub records_unchanged: usize,
    /// Imported records a broker record replaced.
    pub superseded: usize,
    pub transfers_linked: usize,
    pub days_stored: usize,
    /// Days the broker stated again differently: kept beside the first.
    pub days_restated: usize,
    /// Each part that failed, and why.
    pub failures: Vec<(String, Failure)>,
}

impl Report {
    fn failed(&mut self, part: impl Into<String>, f: Failure) {
        self.failures.push((part.into(), f));
    }
}

/// An error of the book itself: the pull stops, as nothing can be stored.
pub type Result<T> = std::result::Result<T, bagholder_book::BookError>;

/// The book's own moves, from its transactions, keyed by the broker's ids.
struct Index {
    /// (account key, day) → what moved
    by: BTreeMap<(String, jiff::civil::Date), Vec<(MovedWhat, Dec)>>,
}

impl BookMoves for Index {
    fn moves(&mut self, accounts: &[String], days: &[jiff::civil::Date]) -> Vec<Moved> {
        let mut out = Vec::new();
        for a in accounts {
            for d in days {
                if let Some(list) = self.by.get(&(a.clone(), *d)) {
                    for (what, q) in list {
                        out.push(Moved { account: a.clone(), day: *d, what: what.clone(), quantity: *q });
                    }
                }
            }
        }
        out
    }
}

/// Pull everything the broker has for one connection. `today` is the day the
/// broker files rows under now; `now` the instant.
pub fn pull(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, today: jiff::civil::Date, now: jiff::Timestamp) -> Result<Report> {
    let mut report = Report::default();
    let broker = adapter.broker();

    // accounts, and the links between them
    let stated = match adapter.accounts() {
        Ok(a) => a,
        Err(f) => {
            report.failed("accounts", f);
            return Ok(report);
        }
    };
    let read = book.broker_read(connection, "accounts", now)?;
    let mut ids: BTreeMap<String, AccountId> = BTreeMap::new();
    for a in &stated {
        let r = AccountRef::new(broker.clone(), a.key.clone());
        let id = match book.account_by_ref(&r)? {
            Some(id) => id,
            None => {
                report.accounts_added += 1;
                let status = if a.open { AccountStatus::Open } else { AccountStatus::Closed };
                book.add_account(connection, std::slice::from_ref(&r), &a.account_type, status, a.nickname.as_deref(), now)?
            }
        };
        ids.insert(a.key.clone(), id);
    }
    for a in &stated {
        if let (Some(to), Some(from)) = (a.linked_to.as_ref().and_then(|k| ids.get(k)), ids.get(&a.key)) {
            book.link_accounts(*from, *to, &read)?;
            report.accounts_linked += 1;
        }
    }

    // each account's activity: whole the first time, then from its last full read
    // a row stored before it was final is read again, from its day
    let mut unsettled: BTreeMap<String, jiff::civil::Date> = BTreeMap::new();
    for r in book.live_records(&adapter.mapping().source())? {
        if let Some((_, _, payload)) = book.revisions(r)?.pop() {
            if let Some((account, day)) = json::parse(&payload).ok().and_then(|v| adapter.unsettled(&v)) {
                let e = unsettled.entry(account).or_insert(day);
                *e = (*e).min(day);
            }
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut read_accounts: Vec<(AccountId, jiff::Timestamp)> = Vec::new();
    for a in &stated {
        let id = ids[&a.key];
        let last = book.activity_read_at(id)?;
        if !a.open && last.is_some() {
            // a closed account already read in full has nothing new
            continue;
        }
        let from = last.map(|t| adapter.day(t)).map(|d| unsettled.get(&a.key).map_or(d, |u| d.min(*u)));
        match adapter.activity(&a.key, from) {
            Ok(r) => {
                report.rows_read += r.len();
                rows.extend(r);
                read_accounts.push((id, now));
            }
            Err(f) => {
                book.note_activity_read(id, now, false)?;
                report.failed(format!("activity:{}", a.key), f);
            }
        }
    }

    // the rows that move by themselves first; those read against positions
    // after, net of the book's own moves
    let source = adapter.mapping().source();
    let scheme = adapter.record_scheme();
    let (first, second): (Vec<&Row>, Vec<&Row>) = rows.iter().partition(|r| !r.reads_positions);
    let mut empty = Index { by: BTreeMap::new() };
    for row in &first {
        store_row(book, adapter, &mut empty, connection, row, &scheme, &mut report, now)?;
    }
    let mut stored_second: Vec<(RecordId, &Row)> = Vec::new();
    if !second.is_empty() {
        let mut index = index(book, adapter, &source, &broker)?;
        for row in &second {
            if let Some(id) = store_row(book, adapter, &mut index, connection, row, &scheme, &mut report, now)? {
                stored_second.push((id, row));
            }
        }
    }
    for (id, at) in &read_accounts {
        book.note_activity_read(*id, *at, true)?;
    }

    // the two sides of each move of holdings: the rows' own accounts and
    // instant join them, and a leg joins the other side's leg of the same
    // instrument and the opposite quantity
    report.transfers_linked += link_moves(book, &stored_second)?;

    // statements: cash now, units as of the last day read in full, and each
    // day's value after the last one stored. One book account can be several of
    // the broker's (the import joined them): its statement is theirs added up.
    let mut keys_of: BTreeMap<AccountId, Vec<String>> = BTreeMap::new();
    for a in stated.iter().filter(|a| a.open) {
        keys_of.entry(ids[&a.key]).or_default().push(a.key.clone());
    }
    let keys: Vec<String> = keys_of.values().flatten().cloned().collect();
    let add = |a: Dec, b: Dec| a.checked_add(b).map_err(|e| bagholder_book::BookError::Refused(format!("a statement too large to add: {e}")));
    match adapter.cash(&keys) {
        Ok(cash) => {
            let read = book.broker_read(connection, "cash", now)?;
            for (id, ks) in &keys_of {
                let mut sum: BTreeMap<bagholder_core::Currency, Dec> = BTreeMap::new();
                for k in ks {
                    for (c, v) in cash.get(k).cloned().unwrap_or_default() {
                        let e = sum.entry(c).or_insert(Dec::ZERO);
                        *e = add(*e, v)?;
                    }
                }
                book.store_cash(*id, now, &sum, &read)?;
            }
        }
        Err(f) => report.failed("cash", f),
    }
    let as_of = today.yesterday().map_err(|e| bagholder_book::BookError::Refused(e.to_string()))?;
    for (id, ks) in &keys_of {
        let mut sum: BTreeMap<bagholder_core::InstrumentId, Dec> = BTreeMap::new();
        let mut complete = true;
        for k in ks {
            match adapter.units(k, as_of) {
                Ok(units) => {
                    let mut unknown = Vec::new();
                    for u in units {
                        match book.instrument_by_ref(&u.instrument)? {
                            Some(i) => {
                                let e = sum.entry(i).or_insert(Dec::ZERO);
                                *e = add(*e, u.quantity)?;
                            }
                            None => unknown.push(u.instrument.value.clone()),
                        }
                    }
                    if !unknown.is_empty() {
                        report.failed(format!("units:{k}"), Failure::Mismatch(format!("positions in instruments no row names: {}", unknown.join(", "))));
                    }
                }
                Err(f) => {
                    complete = false;
                    report.failed(format!("units:{k}"), f);
                }
            }
        }
        // a statement of units is stored only whole: every account behind it read
        if complete {
            let read = book.broker_read(connection, "units", now)?;
            let lines: Vec<UnitsLine> = sum.into_iter().map(|(instrument, quantity)| UnitsLine { instrument, quantity, book_value: None }).collect();
            book.store_units(*id, as_of, &lines, &read, now)?;
        }
    }
    let mut days_of: BTreeMap<AccountId, BTreeMap<jiff::civil::Date, (bagholder_core::Money, bagholder_core::Money)>> = BTreeMap::new();
    let mut history_failed: BTreeSet<AccountId> = BTreeSet::new();
    for a in &stated {
        let id = ids[&a.key];
        let from = book.last_account_day(id)?.and_then(|d| d.tomorrow().ok());
        match adapter.history(&a.key, from) {
            Ok(days) => {
                let e = days_of.entry(id).or_default();
                for d in days {
                    match e.get_mut(&d.day) {
                        Some((v, n)) => {
                            if v.currency != d.net_value.currency || n.currency != d.net_deposits.currency {
                                return Err(bagholder_book::BookError::Refused(format!("account {id}'s accounts state {} in different currencies", d.day)));
                            }
                            *v = bagholder_core::Money::new(add(v.amount, d.net_value.amount)?, v.currency);
                            *n = bagholder_core::Money::new(add(n.amount, d.net_deposits.amount)?, n.currency);
                        }
                        None => {
                            e.insert(d.day, (d.net_value, d.net_deposits));
                        }
                    }
                }
            }
            Err(f) => {
                history_failed.insert(id);
                report.failed(format!("history:{}", a.key), f);
            }
        }
    }
    for (id, days) in days_of {
        // an account's days are stored only when every account behind it answered
        if history_failed.contains(&id) {
            continue;
        }
        let read = book.broker_read(connection, "history", now)?;
        let days: Vec<AccountDay> = days.into_iter().map(|(day, (net_value, net_deposits))| AccountDay { day, net_value, net_deposits }).collect();
        report.days_stored += days.len();
        report.days_restated += book.store_account_days(id, &days, &read)?.len();
    }
    Ok(report)
}

/// Store one row's record, replacing the imported record of the same broker id.
#[allow(clippy::too_many_arguments)]
fn store_row(book: &Book, adapter: &mut dyn BrokerAdapter, moves: &mut dyn BookMoves, connection: ConnectionId, row: &Row, scheme: &str, report: &mut Report, now: jiff::Timestamp) -> Result<Option<RecordId>> {
    let payload = match adapter.record(row, moves) {
        Ok(p) => p,
        Err(f) => {
            report.failed(format!("record:{}", row.key), f);
            return Ok(None);
        }
    };
    let text = payload.canonical();
    let incoming = Incoming { connection: Some(connection), source_key: &row.key, payload: &text, refs: vec![(scheme.to_string(), row.key.clone())] };
    // the imported record of this row, still counted
    let mut replaces = Vec::new();
    for r in book.records_by_ref(scheme, &row.key)? {
        let rec = book.record(r)?;
        if rec.state == RecordState::Live && rec.source != adapter.mapping().source() {
            replaces.push(r);
        }
    }
    let stored = if replaces.is_empty() {
        book.store(adapter.mapping(), &incoming, now)?
    } else {
        report.superseded += replaces.len();
        book.store_superseding(adapter.mapping(), &incoming, &replaces, "the broker's own row", now)?
    };
    match stored.outcome {
        bagholder_book::records::Outcome::New => report.records_new += 1,
        bagholder_book::records::Outcome::Revised(_) => report.records_revised += 1,
        bagholder_book::records::Outcome::Unchanged => report.records_unchanged += 1,
    }
    Ok(Some(stored.record))
}

/// The book's own moves, by the broker's account and instrument ids, leaving
/// out the records that are themselves read against positions.
fn index(book: &Book, adapter: &dyn BrokerAdapter, source: &bagholder_core::SourceName, broker: &bagholder_core::Broker) -> Result<Index> {
    let mut skip: BTreeSet<RecordId> = BTreeSet::new();
    for r in book.live_records(source)? {
        if let Some((_, _, payload)) = book.revisions(r)?.pop() {
            if let Ok(v) = json::parse(&payload) {
                if adapter.reads_positions(&v) {
                    skip.insert(r);
                }
            }
        }
    }
    let mut account_key: BTreeMap<AccountId, String> = BTreeMap::new();
    for a in book.accounts()? {
        if let Some(r) = book.account_refs(a.id)?.into_iter().find(|r| &r.broker == broker) {
            account_key.insert(a.id, r.value);
        }
    }
    let mut instrument_ref: BTreeMap<bagholder_core::InstrumentId, Option<bagholder_core::instrument::Reference>> = BTreeMap::new();
    let mut by: BTreeMap<(String, jiff::civil::Date), Vec<(MovedWhat, Dec)>> = BTreeMap::new();
    for t in book.transactions()? {
        if skip.contains(&t.id.record) || t.kind == Kind::Unclassified {
            continue;
        }
        let Some(a) = account_key.get(&t.account) else { continue };
        let key = (a.clone(), t.trade_date);
        if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
            let r = match instrument_ref.get(&i) {
                Some(r) => r.clone(),
                None => {
                    let r = book.instrument_refs(i)?.into_iter().find(|r| matches!(&r.scheme, RefScheme::BrokerSecurity(b) if b == broker));
                    instrument_ref.insert(i, r.clone());
                    r
                }
            };
            if let Some(r) = r {
                by.entry(key.clone()).or_default().push((MovedWhat::Instrument(r), q));
            }
        }
        if let Some(c) = t.cash {
            by.entry(key).or_default().push((MovedWhat::Cash(c.currency), c.amount));
        }
    }
    Ok(Index { by })
}

/// Link the out and in legs of each move of holdings stored in this pull.
fn link_moves(book: &Book, stored: &[(RecordId, &Row)]) -> Result<usize> {
    let mut linked = 0;
    let txs: Vec<(RecordId, Vec<Transaction>)> = stored.iter().map(|(id, _)| Ok((*id, book.transactions_of(*id)?))).collect::<Result<_>>()?;
    let outs: Vec<&Transaction> = txs.iter().flat_map(|(_, t)| t.iter()).filter(|t| t.kind == Kind::TransferOut && t.instrument.is_some()).collect();
    let ins: Vec<&Transaction> = txs.iter().flat_map(|(_, t)| t.iter()).filter(|t| t.kind == Kind::TransferIn && t.instrument.is_some()).collect();
    for o in &outs {
        let found: Vec<&&Transaction> = ins
            .iter()
            .filter(|i| i.account != o.account && i.instrument == o.instrument && i.occurred_at == o.occurred_at && i.quantity.zip(o.quantity).is_some_and(|(a, b)| a.checked_add(b).ok() == Some(Dec::ZERO)))
            .collect();
        if let [one] = found.as_slice() {
            book.link_transfer(&o.id, &one.id)?;
            linked += 1;
        }
    }
    Ok(linked)
}
