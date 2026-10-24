//! The pull (`docs/plans/stage-3b-wealthsimple.md`, "The pull, into the book"):
//! the accounts and their links, each account's activity as records, the two
//! sides of a move of holdings linked, and the broker's statements. An open
//! account is read whole the first time, after that from its last full read
//! (brief 07 §1); a closed account, whose activity no longer changes, is read
//! whole once and not again (owner, 2026-10-24: "why would closed accounts be
//! re-read at all, ever?"). A move out of it the broker lists by its arrival
//! alone is booked from the arrival (rule 15).
//!
//! Each part's failure is its own: named in the report, the other parts carry
//! on, and nothing already stored is removed or overwritten by a failed part.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::records::Incoming;
use bagholder_book::statements::{AccountDay, UnitsLine};
use bagholder_book::Book;
use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType};
use bagholder_core::instrument::RefScheme;
use bagholder_core::json;
use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, ConnectionId, Dec, RecordId};

use crate::{AccountStated, BookMoves, BrokerAdapter, Failure, Moved, MovedWhat, Row, Step};

/// What one pull did and what failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub accounts_added: usize,
    pub accounts_linked: usize,
    /// Accounts merged into another the broker links them to.
    pub accounts_merged: usize,
    pub rows_read: usize,
    pub records_new: usize,
    pub records_revised: usize,
    pub records_unchanged: usize,
    /// Records of rows the broker no longer lists over a span it was read in,
    /// each removed: the record and the broker's id for its row.
    pub removed: Vec<(RecordId, String)>,
    /// Accounts whose read would have removed what a sound read does not (a
    /// final row older than the rows read again, or more than a few final
    /// rows): nothing was removed for them, and why.
    pub suspect: Vec<(String, String)>,
    pub transfers_linked: usize,
    pub days_stored: usize,
    /// Days the broker stated again differently: kept beside the first.
    pub days_restated: usize,
    /// What the monthly statements read and booked, and each month that does
    /// not reconcile (`statements`).
    pub statements: crate::statements::Done,
    /// Each part that failed, and why.
    pub failures: Vec<(String, Failure)>,
    /// The book's accounts this pull read the broker's statements of (cash,
    /// holdings, days, value, activity): what the figures read again with its rows.
    pub stated_for: std::collections::BTreeSet<AccountId>,
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
/// broker files rows under now; `now` the instant. `step` is told where the
/// pull is as it goes.
pub fn pull(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, today: jiff::civil::Date, now: jiff::Timestamp, step: &mut dyn FnMut(Step)) -> Result<Report> {
    pull_in(book, adapter, connection, today, now, step, true)
}

/// A pull of what moved since the last: the accounts, each account's activity
/// from its last full read, the balances, and the statements where the cash
/// disagrees. Not the holdings or the daily history, which are as of the last
/// full day and are read by the day's pull: for a movement seen during the day
/// (the cash moving at the broker), the rows that moved, quickly.
pub fn pull_activity(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, today: jiff::civil::Date, now: jiff::Timestamp, step: &mut dyn FnMut(Step)) -> Result<Report> {
    pull_in(book, adapter, connection, today, now, step, false)
}

/// `daily`: the holdings and the daily history as of the last full day are read too.
fn pull_in(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, today: jiff::civil::Date, now: jiff::Timestamp, step: &mut dyn FnMut(Step), daily: bool) -> Result<Report> {
    let mut report = Report::default();
    let broker = adapter.broker();
    step(Step::Accounts);

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
    // the links the broker states; ids it links that sit on two accounts are made one
    let links: Vec<(AccountId, AccountId)> = stated.iter().filter_map(|a| Some((*ids.get(&a.key)?, *a.linked_to.as_ref().and_then(|k| ids.get(k))?))).collect();
    report.accounts_linked += links.len();
    report.accounts_merged += book.store_account_links(connection, &links, &read)?.removed;
    // each broker id's account, after any merge
    for a in &stated {
        if let Some(id) = book.account_by_ref(&AccountRef::new(broker.clone(), a.key.clone()))? {
            ids.insert(a.key.clone(), id);
        }
    }
    // each account's status is the broker's: open where any of its ids is
    let mut open: BTreeMap<AccountId, bool> = BTreeMap::new();
    for a in &stated {
        *open.entry(ids[&a.key]).or_default() |= a.open;
    }
    for (id, is_open) in &open {
        book.set_account_status(*id, if *is_open { AccountStatus::Open } else { AccountStatus::Closed })?;
    }
    report.stated_for = ids.values().copied().collect();
    let backing: Vec<(AccountId, AccountId)> = stated.iter().filter_map(|a| Some((*ids.get(&a.key)?, *a.backs.as_ref().and_then(|k| ids.get(k))?))).collect();
    book.store_margin_backing(connection, &backing, &read)?;

    // each account's activity: whole the first time, then from its last full read
    // a row stored before it was final is read again, from its day
    // (from its first day where one's day is not known)
    let mut unsettled: BTreeMap<String, Option<jiff::civil::Date>> = BTreeMap::new();
    // a record put together before the adapter read a part it reads now:
    // added to once, below
    let mut stale: Vec<(RecordId, json::Value)> = Vec::new();
    // the stored records holding another account's rows over some days, by
    // that account: a row of it recorded now is added to them (`amend`)
    let mut watching: Watching = BTreeMap::new();
    for r in book.live_records(&adapter.mapping().source())? {
        if let Some((_, _, payload)) = book.revisions(r)?.pop() {
            let parsed = json::parse(&payload).ok();
            if let Some(v) = parsed.as_ref().filter(|v| adapter.refreshes(v)) {
                stale.push((r, v.clone()));
            }
            if let Some(a) = parsed.as_ref().and_then(|v| adapter.watches(v)) {
                watching.entry(a).or_default().push(r);
            }
            if let Some((account, day)) = parsed.and_then(|v| adapter.unsettled(&v)) {
                let e = unsettled.entry(account).or_insert(day);
                *e = match (*e, day) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    _ => None,
                };
            }
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut read_accounts: Vec<(AccountId, jiff::Timestamp)> = Vec::new();
    // the day each account's activity was read from (the whole of it when `None`)
    let mut spans: BTreeMap<String, Option<jiff::civil::Date>> = BTreeMap::new();
    // the day of each account's last full read before this one
    let mut read_before: BTreeMap<String, jiff::civil::Date> = BTreeMap::new();
    // a closed account read whole once is not read again
    let mut to_read: Vec<(&AccountStated, Option<jiff::Timestamp>)> = Vec::new();
    for a in &stated {
        let last = book.activity_read_at(ids[&a.key])?;
        if a.open || last.is_none() {
            to_read.push((a, last));
        }
    }
    let of = to_read.len();
    for (n, (a, last)) in to_read.into_iter().enumerate() {
        let id = ids[&a.key];
        step(Step::Activity { account: a.name(), n: n + 1, of });
        if let Some(t) = last {
            read_before.insert(a.key.clone(), adapter.day(t));
        }
        let from = last.map(|t| adapter.day(t)).and_then(|d| match unsettled.get(&a.key) {
            None => Some(d),
            Some(Some(u)) => Some(d.min(*u)),
            Some(None) => None,
        });
        match adapter.activity(&a.key, from) {
            Ok(read) => {
                report.rows_read += read.rows.len();
                rows.extend(read.rows);
                if read.unkeyed.is_empty() {
                    read_accounts.push((id, now));
                    spans.insert(a.key.clone(), from);
                } else {
                    // rows it cannot keep: the read is not complete, and
                    // nothing is removed on it
                    book.note_activity_read(id, now, false)?;
                    report.failed(format!("activity:{}", a.key), Failure::Mismatch(format!("rows that state no id to keep them by: {}", read.unkeyed.join("; "))));
                }
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
    // a row its stored record already holds as it is changed nothing, and is
    // not put together again
    let mut changed: Vec<&Row> = Vec::new();
    for row in &rows {
        let same = match book.record_by_key(Some(connection), &source, &row.key)? {
            Some(r) => book.revisions(r)?.pop().is_some_and(|(_, _, p)| json::parse(&p).is_ok_and(|v| adapter.holds(&v, row))),
            None => false,
        };
        if same {
            report.records_unchanged += 1;
        } else {
            changed.push(row);
        }
    }
    adapter.prepare(&changed);
    let (first, second): (Vec<&Row>, Vec<&Row>) = changed.into_iter().partition(|r| !r.reads_positions);
    let recording = first.len() + second.len();
    let mut done = 0;
    // the broker's accounts with a row whose record could not be put together:
    // their read is not complete, so the row is asked for again
    let mut unrecorded: BTreeSet<String> = BTreeSet::new();
    let mut empty = Index { by: BTreeMap::new() };
    for row in &first {
        step(Step::Recording { done, of: recording });
        if store_row(book, adapter, &mut empty, connection, row, &watching, &mut report, now)?.is_none() {
            unrecorded.insert(row.account.clone());
        }
        done += 1;
    }
    let mut stored_second: Vec<(RecordId, &Row)> = Vec::new();
    if !second.is_empty() {
        let mut index = index(book, adapter, &source, &broker)?;
        for row in &second {
            step(Step::Recording { done, of: recording });
            match store_row(book, adapter, &mut index, connection, row, &watching, &mut report, now)? {
                Some(id) => stored_second.push((id, row)),
                None => {
                    unrecorded.insert(row.account.clone());
                }
            }
            done += 1;
        }
    }
    // the records put together before the adapter read a part it reads now,
    // that part added and derived again: once, as a record holding it is not
    // stale again
    let mut refreshed: Vec<RecordId> = Vec::new();
    if !stale.is_empty() {
        let mut index = index(book, adapter, &source, &broker)?;
        for (r, v) in &stale {
            let key = book.record(*r)?.source_key;
            match adapter.refresh(v, &mut index) {
                Some(Ok(payload)) => {
                    let text = payload.canonical();
                    let incoming = Incoming { connection: Some(connection), source_key: &key, payload: &text, refs: vec![] };
                    let stored = book.store(adapter.mapping(), &incoming, now)?;
                    match stored.outcome {
                        bagholder_book::records::Outcome::New => report.records_new += 1,
                        bagholder_book::records::Outcome::Revised(_) => report.records_revised += 1,
                        bagholder_book::records::Outcome::Unchanged => report.records_unchanged += 1,
                    }
                    if adapter.reads_positions(&payload) {
                        refreshed.push(stored.record);
                    }
                }
                Some(Err(f)) => report.failed(format!("record:{key}"), f),
                None => {}
            }
        }
    }
    // the rows of the person's accounts read for another row's record beside
    // their accounts' own reads (a move's other side over its days), the book
    // not holding them as they are: stored as every row read is
    let mut beside: Vec<Row> = Vec::new();
    let mut beside_linked: Vec<RecordId> = Vec::new();
    loop {
        let mut fresh = Vec::new();
        for row in adapter.read_beside() {
            if rows.iter().chain(&beside).chain(&fresh).any(|r| r.key == row.key) {
                continue;
            }
            let held = match book.record_by_key(Some(connection), &source, &row.key)? {
                Some(r) => book.revisions(r)?.pop().is_some_and(|(_, _, p)| json::parse(&p).is_ok_and(|v| adapter.holds(&v, &row))),
                None => false,
            };
            if !held {
                fresh.push(row);
            }
        }
        if fresh.is_empty() {
            break;
        }
        adapter.prepare(&fresh.iter().collect::<Vec<_>>());
        let (first, second): (Vec<&Row>, Vec<&Row>) = fresh.iter().partition(|r| !r.reads_positions);
        let mut empty = Index { by: BTreeMap::new() };
        for row in &first {
            store_row(book, adapter, &mut empty, connection, row, &watching, &mut report, now)?;
        }
        if !second.is_empty() {
            let mut index = index(book, adapter, &source, &broker)?;
            for row in &second {
                if let Some(id) = store_row(book, adapter, &mut index, connection, row, &watching, &mut report, now)? {
                    beside_linked.push(id);
                }
            }
        }
        beside.extend(fresh);
    }
    let listed: BTreeSet<&str> = rows.iter().chain(&beside).map(|r| r.key.as_str()).collect();
    for (r, key) in unlisted(book, adapter, &spans, &listed, &read_before, &mut report)? {
        book.mark_removed(r, now)?;
        report.removed.push((r, key));
    }
    let incomplete: BTreeSet<AccountId> = unrecorded.iter().filter_map(|k| ids.get(k)).copied().collect();
    for (id, at) in &read_accounts {
        book.note_activity_read(*id, *at, !incomplete.contains(id))?;
    }

    // the two sides of each move of holdings: the rows' own accounts and
    // instant join them, and a leg joins the other side's leg of the same
    // instrument and the opposite quantity
    let linked: Vec<RecordId> = stored_second.iter().map(|(id, _)| *id).chain(refreshed).chain(beside_linked).collect();
    report.transfers_linked += link_moves(book, &linked)?;

    // statements: cash now, units as of the last day read in full, and each
    // day's value after the last one stored. One book account can be several of
    // the broker's (ids it links as one, `linked`): its statement is theirs added
    // up, every one of them read, as 2.9.0 read them.
    let mut keys_of: BTreeMap<AccountId, Vec<String>> = BTreeMap::new();
    for a in stated.iter().filter(|a| a.open) {
        keys_of.entry(ids[&a.key]).or_default().push(a.key.clone());
    }
    let margin: BTreeSet<AccountId> = stated.iter().filter(|a| a.open && is_margin(&a.account_type)).map(|a| ids[&a.key]).collect();
    store_net_values(book, connection, &stated, &keys_of, now)?;
    step(Step::Balances);
    store_balances(book, adapter, connection, &keys_of, &margin, now, &mut report.failures)?;
    // each margin account's interest rates, with the day's pull alone: a rate
    // changes seldom, and the five-minute balances do not ask it again
    if daily {
        store_margin_rates(book, adapter, connection, &keys_of, &margin, now, &mut report.failures)?;
    }
    // the movements the activity feed left out, for an account whose cash now
    // disagrees with the broker's: read from its monthly statements, every
    // account's, closed ones too
    let mut statement_keys: BTreeMap<AccountId, Vec<String>> = BTreeMap::new();
    let mut closed = BTreeSet::new();
    for a in &stated {
        statement_keys.entry(ids[&a.key]).or_default().push(a.key.clone());
        if !a.open {
            closed.insert(a.key.clone());
        }
    }
    report.statements = crate::statements::run(book, adapter, connection, &statement_keys, &closed, today, now, step, &mut report.failures)?;
    report.statements.fills = crate::statements::fills(book, adapter, connection, &statement_keys, now, &mut report.failures)?;
    let add = |a: Dec, b: Dec| a.checked_add(b).map_err(|e| bagholder_book::BookError::Refused(format!("a statement too large to add: {e}")));
    let as_of = today.yesterday().map_err(|e| bagholder_book::BookError::Refused(e.to_string()))?;
    // units already stated as of that day are not asked again (brief 07 §1),
    // unless one held goes by no name yet (its description is read with them),
    // the statement kept predates the broker's stated worth of each, or it states
    // nothing held: a statement of nothing is no read of the broker's to stand
    // on, and is asked again (2.10.2 wrote one onto accounts it merged)
    let mut holdings = Vec::new();
    for (id, ks) in keys_of.iter().filter(|_| daily) {
        let current = match book.stated(*id)?.units {
            Some((d, lines)) if d == as_of && !lines.is_empty() && lines.keys().all(|i| book.stated(*id).map(|s| s.unit_values.contains_key(i)).unwrap_or(false)) => {
                let mut named = true;
                for i in lines.keys() {
                    named &= book.current_name(*i)?.is_some() || book.option_terms(*i)?.is_some();
                }
                named
            }
            _ => false,
        };
        if !current {
            holdings.push((id, ks));
        }
    }
    let of = holdings.len();
    for (n, (id, ks)) in holdings.into_iter().enumerate() {
        let name = stated.iter().find(|a| ids[&a.key] == *id).map(|a| a.name()).unwrap_or_default();
        step(Step::Holdings { account: name, n: n + 1, of });
        let mut sum: BTreeMap<bagholder_core::InstrumentId, Dec> = BTreeMap::new();
        let mut valued: BTreeMap<bagholder_core::InstrumentId, Option<bagholder_core::Money>> = BTreeMap::new();
        let mut costed: BTreeMap<bagholder_core::InstrumentId, Option<bagholder_core::Money>> = BTreeMap::new();
        let mut complete = true;
        for k in ks {
            match adapter.units(k, as_of) {
                Ok(units) => {
                    // a holding no row names (a token migrated, a gift): its
                    // instrument, and the name it goes by, from the broker's own
                    // description, read together, so the check shows it beside
                    // what the book holds and it is priced like any other
                    let mut unnamed = Vec::new();
                    for u in &units {
                        let named = match book.instrument_by_ref(&u.instrument)? {
                            Some(i) => book.current_name(i)?.is_some() || book.option_terms(i)?.is_some(),
                            None => false,
                        };
                        if !named {
                            unnamed.push(u.instrument.clone());
                        }
                    }
                    if !unnamed.is_empty() {
                        for (r, draft) in adapter.instruments(&unnamed, as_of) {
                            let why = match draft {
                                Ok(d) => book.instrument_stated(&d, &adapter.mapping().source(), now)?.err().map(|p| Failure::Mismatch(format!("{}: {}", r.value, p.detail))),
                                Err(f) => Some(f),
                            };
                            if let Some(f) = why {
                                report.failed(format!("units:{k}"), f);
                            }
                        }
                    }
                    // each line kept on the instrument the broker's id names
                    // itself, so a succession parted later leaves it there
                    for u in units {
                        match book.own_instrument(&u.instrument)? {
                            Some(i) => {
                                let e = sum.entry(i).or_insert(Dec::ZERO);
                                *e = add(*e, u.quantity)?;
                                // its value, none once one account's states none or another currency
                                let v = valued.entry(i).or_insert(u.value.map(|v| bagholder_core::Money::zero(v.currency)));
                                *v = match (*v, u.value) {
                                    (Some(a), Some(b)) if a.currency == b.currency => Some(bagholder_core::Money::new(add(a.amount, b.amount)?, a.currency)),
                                    _ => None,
                                };
                                // its book value the same way
                                let c = costed.entry(i).or_insert(u.book_value.map(|v| bagholder_core::Money::zero(v.currency)));
                                *c = match (*c, u.book_value) {
                                    (Some(a), Some(b)) if a.currency == b.currency => Some(bagholder_core::Money::new(add(a.amount, b.amount)?, a.currency)),
                                    _ => None,
                                };
                            }
                            // a statement is kept only whole
                            None => complete = false,
                        }
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
            let lines: Vec<UnitsLine> = sum.into_iter().map(|(instrument, quantity)| UnitsLine { instrument, quantity, book_value: costed.get(&instrument).copied().flatten(), value: valued.get(&instrument).copied().flatten() }).collect();
            book.store_units(*id, as_of, &lines, &read, now)?;
        }
    }
    // an id the broker now states retired by a corporate action, and the id its
    // listing trades under now, over every row and holding read so far
    book.settle_successions()?;
    let mut days_of: BTreeMap<AccountId, BTreeMap<jiff::civil::Date, (bagholder_core::Money, bagholder_core::Money)>> = BTreeMap::new();
    let mut history_failed: BTreeSet<AccountId> = BTreeSet::new();
    // an account whose days are stated up to the last full day has none new
    let mut dated = Vec::new();
    for a in stated.iter().filter(|_| daily) {
        let last = book.last_account_day(ids[&a.key])?;
        if !(last.is_some_and(|d| d >= as_of) || (!a.open && last.is_some())) {
            dated.push((a, last));
        }
    }
    let of = dated.len();
    for (n, (a, last)) in dated.into_iter().enumerate() {
        let id = ids[&a.key];
        step(Step::History { account: a.name(), n: n + 1, of });
        let from = last.and_then(|d| d.tomorrow().ok());
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
    // the day's pull is done: what the window asks of it (`DAILY_READ`) is read
    if daily {
        book.broker_read(connection, DAILY_READ, now)?;
    }
    Ok(report)
}

/// The read a whole pull marks itself with: the day's window is met by it, never
/// by a pull of the activity alone.
pub const DAILY_READ: &str = "daily";

/// More final rows than this gone from one account in one read makes the read
/// suspect.
const FEW: usize = 3;

/// The records of rows the broker no longer lists over a span it was read in:
/// a row posted again under another id once final (a card purchase's pending
/// row), a row withdrawn, by the account and day they name.
///
/// A row not yet final that is gone is the pending row's life; a final row
/// gone is not, so an account whose read would remove a final row older than
/// the rows read again (before the last full read's day, where only rows not
/// yet final are read again), or more than a few final rows, removes nothing
/// and is reported suspect (brief 08).
fn unlisted(
    book: &Book,
    adapter: &dyn BrokerAdapter,
    spans: &BTreeMap<String, Option<jiff::civil::Date>>,
    listed: &BTreeSet<&str>,
    read_before: &BTreeMap<String, jiff::civil::Date>,
    report: &mut Report,
) -> Result<Vec<(RecordId, String)>> {
    let covers = |from: &Option<jiff::civil::Date>, day: jiff::civil::Date| from.is_none_or(|f| day >= f);
    let mut out = Vec::new();
    // the broker's own records, by account: a row not final that is gone was
    // posted again or withdrawn; a final row gone is a guard's matter
    let mut own: BTreeMap<String, Vec<(RecordId, String, jiff::civil::Date, bool)>> = BTreeMap::new();
    for r in book.live_records(&adapter.mapping().source())? {
        let rec = book.record(r)?;
        if listed.contains(rec.source_key.as_str()) {
            continue;
        }
        let Some((_, _, payload)) = book.revisions(r)?.pop() else { continue };
        let Ok(v) = json::parse(&payload) else { continue };
        let Some((account, day)) = adapter.placed(&v) else { continue };
        if spans.get(&account).is_some_and(|from| covers(from, day)) {
            let settled = adapter.unsettled(&v).is_none();
            own.entry(account).or_default().push((r, rec.source_key, day, settled));
        }
    }
    for (account, gone) in own {
        let final_rows: Vec<&(RecordId, String, jiff::civil::Date, bool)> = gone.iter().filter(|g| g.3).collect();
        // before the last full read's day the account is read again only for
        // its rows not yet final: a final row there was read in full before
        let older = final_rows.iter().find(|g| read_before.get(&account).is_some_and(|d| g.2 < *d));
        let why = match older {
            Some(g) => Some(format!("{} is final, older than the rows read again, and no longer listed", g.1)),
            None if final_rows.len() > FEW => Some(format!("{} final rows are no longer listed", final_rows.len())),
            None => None,
        };
        match why {
            Some(w) => report.suspect.push((account, w)),
            None => out.extend(gone.into_iter().map(|(r, k, _, _)| (r, k))),
        }
    }
    Ok(out)
}

/// The stored records holding another account's rows over some days, by that
/// account's key.
type Watching = BTreeMap<String, Vec<RecordId>>;

/// Store one row's record, adding the row to each stored record holding its
/// account's rows over its day: one write, so no reader sees the book between
/// the two. None where it cannot be put together.
#[allow(clippy::too_many_arguments)]
fn store_row(book: &Book, adapter: &mut dyn BrokerAdapter, moves: &mut dyn BookMoves, connection: ConnectionId, row: &Row, watching: &Watching, report: &mut Report, now: jiff::Timestamp) -> Result<Option<RecordId>> {
    let payload = match adapter.record(row, moves) {
        Ok(p) => p,
        Err(f) => {
            report.failed(format!("record:{}", row.key), f);
            return Ok(None);
        }
    };
    let text = payload.canonical();
    let incoming = Incoming { connection: Some(connection), source_key: &row.key, payload: &text, refs: vec![] };
    book.atomically(|| {
        let stored = book.store(adapter.mapping(), &incoming, now)?;
        match stored.outcome {
            bagholder_book::records::Outcome::New => report.records_new += 1,
            bagholder_book::records::Outcome::Revised(_) => report.records_revised += 1,
            bagholder_book::records::Outcome::Unchanged => report.records_unchanged += 1,
        }
        // the stored records holding this account's rows over the row's day:
        // the row added, and each derived again (a move's arrival that booked
        // it leaving here, now that its own row is read, books it no more)
        for r in watching.get(&row.account).into_iter().flatten() {
            if *r == stored.record {
                continue;
            }
            let rec = book.record(*r)?;
            if rec.state != bagholder_core::record::RecordState::Live {
                continue;
            }
            let Some((_, _, held)) = book.revisions(*r)?.pop() else { continue };
            let Ok(held) = json::parse(&held) else { continue };
            if let Some(amended) = adapter.amend(&held, row) {
                let text = amended.canonical();
                let incoming = Incoming { connection: Some(connection), source_key: &rec.source_key, payload: &text, refs: vec![] };
                if let bagholder_book::records::Outcome::Revised(_) = book.store(adapter.mapping(), &incoming, now)?.outcome {
                    report.records_revised += 1;
                }
            }
        }
        Ok(Some(stored.record))
    })
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
    // each move under the broker's own id for what it moved
    for t in book.transactions_as_named()? {
        if skip.contains(&t.id.record) || t.kind == Kind::Unclassified {
            continue;
        }
        let Some(a) = account_key.get(&t.account) else { continue };
        let key = (a.clone(), t.trade_date);
        if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
            let r = match instrument_ref.get(&i) {
                Some(r) => r.clone(),
                None => {
                    let r = book.own_refs(i)?.into_iter().find(|r| matches!(&r.scheme, RefScheme::BrokerSecurity(b) if b == broker));
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
fn link_moves(book: &Book, stored: &[RecordId]) -> Result<usize> {
    let mut linked = 0;
    let txs: Vec<(RecordId, Vec<Transaction>)> = stored.iter().map(|id| Ok((*id, book.transactions_of(*id)?))).collect::<Result<_>>()?;
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


/// Whether an account borrows: only a margin account's buying power is margin
/// it can draw (a registered account's is its cash).
fn is_margin(t: &AccountType) -> bool {
    matches!(t, AccountType::Known { kind: AccountKind::Margin, .. })
}

/// What a read of the balances stored, and what failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BalancesRead {
    /// The book's accounts a statement was stored for.
    pub accounts: Vec<AccountId>,
    pub failures: Vec<(String, Failure)>,
    /// The accounts whose cash, as the broker states it now, is not what it stated
    /// when the activity was last read: a movement the book has not read yet (a
    /// distribution paid, interest, a deposit), so the activity is due.
    pub cash_moved: Vec<AccountId>,
}

/// The balances now, apart from a pull (`docs/plans/stage-3c-switch.md`, §3: every
/// five minutes while a page is open): each open account's worth and cash, and
/// what each margin account can borrow, for the connection's accounts the book holds
/// (`SPEC.md`, the sources: net liquidation value at every sync and every five
/// minutes while a page is open).
pub fn balances(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, now: jiff::Timestamp) -> Result<BalancesRead> {
    let broker = adapter.broker();
    let mut failures = Vec::new();
    let mut accounts: BTreeSet<AccountId> = BTreeSet::new();
    // the broker's open accounts, each under the book account its id names: every
    // open id behind a book account read and added up, as the pull reads them
    let stated = match adapter.accounts() {
        Ok(s) => s,
        Err(f) => {
            failures.push(("accounts".into(), f));
            return Ok(BalancesRead { accounts: vec![], failures, cash_moved: vec![] });
        }
    };
    let mut keys_of: BTreeMap<AccountId, Vec<String>> = BTreeMap::new();
    let mut margin = BTreeSet::new();
    for a in stated.iter().filter(|a| a.open) {
        let Some(id) = book.account_by_ref(&AccountRef::new(broker.clone(), a.key.clone()))? else { continue };
        if book.account(id)?.connection != connection {
            continue;
        }
        if is_margin(&a.account_type) {
            margin.insert(id);
        }
        keys_of.entry(id).or_default().push(a.key.clone());
    }
    accounts.extend(store_net_values(book, connection, &stated, &keys_of, now)?);
    accounts.extend(store_balances(book, adapter, connection, &keys_of, &margin, now, &mut failures)?);
    let mut cash_moved = Vec::new();
    for a in &accounts {
        let stated = book.stated_latest(*a)?;
        if let (Some((_, now_cash)), Some(at_read)) = (&stated.cash, &stated.cash_read) {
            // a currency stated at nothing is the same as one not stated
            let held = |m: &BTreeMap<bagholder_core::Currency, bagholder_core::Dec>| m.iter().filter(|(_, v)| !v.is_zero()).map(|(c, v)| (*c, *v)).collect::<BTreeMap<_, _>>();
            if held(now_cash) != held(at_read) {
                cash_moved.push(*a);
            }
        }
    }
    Ok(BalancesRead { accounts: accounts.into_iter().collect(), failures, cash_moved })
}

/// Each margin account's yearly interest rates; a failed read is its own failed part.
fn store_margin_rates(
    book: &Book,
    adapter: &mut dyn BrokerAdapter,
    connection: ConnectionId,
    keys_of: &BTreeMap<AccountId, Vec<String>>,
    margin: &BTreeSet<AccountId>,
    now: jiff::Timestamp,
    failures: &mut Vec<(String, Failure)>,
) -> Result<()> {
    for (id, ks) in keys_of.iter().filter(|(id, _)| margin.contains(id)) {
        for k in ks {
            match adapter.margin_rates(k) {
                Ok(Some(rates)) => {
                    let read = book.broker_read(connection, "margin-rates", now)?;
                    book.store_margin_rates(*id, now, &rates, &read)?;
                }
                Ok(None) => {}
                Err(f) => failures.push((format!("margin rate:{k}"), f)),
            }
        }
    }
    Ok(())
}

/// What each account is worth now, as the broker states it, where every account of
/// the broker's behind it states it (one book account can be several of the
/// broker's: its worth is theirs added up). The accounts a value was stored for.
fn store_net_values(book: &Book, connection: ConnectionId, stated: &[AccountStated], keys_of: &BTreeMap<AccountId, Vec<String>>, now: jiff::Timestamp) -> Result<Vec<AccountId>> {
    let read = book.broker_read(connection, "net-value", now)?;
    let mut stored = Vec::new();
    for (id, keys) in keys_of {
        let mut total: Option<bagholder_core::Money> = None;
        let mut whole = true;
        for k in keys {
            match (stated.iter().find(|a| &a.key == k).and_then(|a| a.net_value), total) {
                (Some(v), None) => total = Some(v),
                (Some(v), Some(t)) if t.currency == v.currency => total = Some(bagholder_core::Money::new(t.amount.checked_add(v.amount).map_err(|e| bagholder_book::BookError::Refused(format!("an account's worth too large to add: {e}")))?, t.currency)),
                _ => whole = false,
            }
        }
        if let (true, Some(v)) = (whole, total) {
            book.store_net_value(*id, now, v, &read)?;
            stored.push(*id);
        }
    }
    Ok(stored)
}

/// Each account's cash and, for a margin account, what it can borrow, stored as
/// statements; one book account that is several of the broker's is theirs added
/// up. What each stored statement was for.
/// The first day the net worth page's "All" range asks for.
pub const ALL_FROM: jiff::civil::Date = jiff::civil::date(2014, 1, 1);

/// The ranges the net worth page asks besides "All", shortest first: one week,
/// one month, three months, six months and a year back from today. A short range
/// is answered a value a day; "All" is answered a value a week in its old years.
pub fn page_ranges() -> [jiff::Span; 5] {
    [jiff::Span::new().weeks(1), jiff::Span::new().months(1), jiff::Span::new().months(3), jiff::Span::new().months(6), jiff::Span::new().years(1)]
}

/// The range a read of an outside account's days asks, ending today: "All" for
/// one with none held yet; else the shortest range of the page's that reaches
/// back to its last day held, so every day since is stated daily.
pub fn outside_ranges(last_held: Option<jiff::civil::Date>, today: jiff::civil::Date) -> Vec<(jiff::civil::Date, jiff::civil::Date)> {
    let back = |span: jiff::Span| today.checked_sub(span).unwrap_or(ALL_FROM).max(ALL_FROM);
    match last_held {
        // the whole history once, then the last six months a value a day
        None => vec![(ALL_FROM, today), (back(page_ranges()[3]), today)],
        Some(last) => {
            let start = page_ranges().into_iter().map(back).find(|s| *s <= last).unwrap_or_else(|| back(page_ranges()[4]));
            vec![(start, today)]
        }
    }
}

/// Each outside account's balance by day, read once a day: each account's
/// ranges (`outside_ranges`) asked of that account alone and kept span by span.
/// Whether every one was read.
fn store_outside_days(book: &Book, adapter: &mut dyn BrokerAdapter, outside: &[bagholder_book::debts::OutsideDebt], today: jiff::civil::Date, now: jiff::Timestamp, failures: &mut Vec<(String, Failure)>) -> Result<bool> {
    let held = book.outside_days()?;
    let mut whole = true;
    let mut days = 0;
    for e in outside {
        let last = held.get(&e.entity_id).and_then(|d| d.last()).map(|d| d.day);
        for (from, to) in outside_ranges(last, today) {
            match adapter.outside_days(e, from, to) {
                Ok(Some(stated)) => {
                    days += stated.len();
                    book.keep_outside_days(&e.entity_id, from, to, &stated, now)?;
                }
                // a broker that states no such history
                Ok(None) => return Ok(false),
                Err(f) => {
                    failures.push((format!("the balance by day of {}", e.name), f));
                    whole = false;
                    break;
                }
            }
        }
    }
    bagholder_core::log::line(&format!("bagholder: outside balances by day read: {} accounts, {days} days", outside.len()));
    Ok(whole)
}

/// The part of a connection's reads that is the outside accounts' balances by day.
const OUTSIDE_DAYS: &str = "outside-days";

fn store_balances(
    book: &Book,
    adapter: &mut dyn BrokerAdapter,
    connection: ConnectionId,
    keys_of: &BTreeMap<AccountId, Vec<String>>,
    margin: &BTreeSet<AccountId>,
    now: jiff::Timestamp,
    failures: &mut Vec<(String, Failure)>,
) -> Result<Vec<AccountId>> {
    let add = |a: Dec, b: Dec| a.checked_add(b).map_err(|e| bagholder_book::BookError::Refused(format!("a statement too large to add: {e}")));
    let mut stored = BTreeSet::new();
    let keys: Vec<String> = keys_of.values().flatten().cloned().collect();
    match adapter.cash(&keys) {
        Ok(cash) => {
            let read = book.broker_read(connection, "cash", now)?;
            for (id, ks) in keys_of {
                // an account the answer does not state has its cash unstated,
                // not nothing: a statement is stored only whole
                if !ks.iter().all(|k| cash.contains_key(k)) {
                    continue;
                }
                let mut sum: BTreeMap<bagholder_core::Currency, Dec> = BTreeMap::new();
                for k in ks {
                    for (c, v) in cash.get(k).cloned().unwrap_or_default() {
                        let e = sum.entry(c).or_insert(Dec::ZERO);
                        *e = add(*e, v)?;
                    }
                }
                book.store_cash(*id, now, &sum, &read)?;
                stored.insert(*id);
            }
        }
        Err(f) => failures.push(("cash".into(), f)),
    }
    let borrowing: Vec<String> = keys_of.iter().filter(|(id, _)| margin.contains(id)).flat_map(|(_, ks)| ks.iter().cloned()).collect();
    if !borrowing.is_empty() {
        match adapter.buying_power(&borrowing) {
            Ok(bp) => {
                let read = book.broker_read(connection, "buying-power", now)?;
                for (id, ks) in keys_of.iter().filter(|(id, _)| margin.contains(id)) {
                    if !ks.iter().all(|k| bp.contains_key(k)) {
                        continue;
                    }
                    // several of the broker's accounts: their amounts added, or every reason one cannot say
                    let why: Vec<&str> = ks.iter().filter_map(|k| bp[k].as_ref().err().map(|w| w.as_str())).collect();
                    let stated = if why.is_empty() {
                        let mut sum = Dec::ZERO;
                        for k in ks {
                            sum = add(sum, *bp[k].as_ref().expect("stated"))?;
                        }
                        Ok(bagholder_core::Money::new(sum, bagholder_core::Currency::CAD))
                    } else {
                        Err(why.join("; "))
                    };
                    book.store_buying_power(*id, now, &stated, &read)?;
                    stored.insert(*id);
                }
            }
            Err(f) => failures.push(("buying power".into(), f)),
        }
    }
    // what the broker's net worth feature states is held outside it: a debt's
    // balance, read with the statements it sits beside
    match adapter.outside_accounts() {
        Ok(Some(outside)) => {
            book.keep_outside_debts(connection, now, &outside)?;
            // each outside account's balance by day, read once a day
            let read_today = book.last_read(connection, OUTSIDE_DAYS)?.is_some_and(|t| now.duration_since(t) < jiff::SignedDuration::from_hours(20));
            if !read_today && !outside.is_empty() {
                let today = match book.zone()? {
                    Some(z) => now.to_zoned(z.zone).date(),
                    None => now.to_zoned(jiff::tz::TimeZone::UTC).date(),
                };
                if store_outside_days(book, adapter, &outside, today, now, failures)? {
                    book.broker_read(connection, OUTSIDE_DAYS, now)?;
                }
            }
        }
        Ok(None) => {}
        Err(f) => failures.push(("outside accounts".into(), f)),
    }
    Ok(stored.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> jiff::civil::Date {
        s.parse().unwrap()
    }

    /// An outside account with no day held is read whole, then its last six
    /// months a value a day; after, the shortest of the page's ranges that
    /// reaches back to its last day held, a year at the most.
    #[test]
    fn an_outside_accounts_days_are_asked_as_the_page_asks_its_ranges() {
        let today = day("2026-10-20");
        assert_eq!(outside_ranges(None, today), [(day("2014-01-01"), today), (day("2026-04-20"), today)]);
        assert_eq!(outside_ranges(Some(day("2026-10-19")), today), [(day("2026-10-13"), today)], "a week");
        assert_eq!(outside_ranges(Some(day("2026-10-01")), today), [(day("2026-09-20"), today)], "a month");
        assert_eq!(outside_ranges(Some(day("2026-06-01")), today), [(day("2026-04-20"), today)], "six months");
        assert_eq!(outside_ranges(Some(day("2023-01-01")), today), [(day("2025-10-20"), today)], "a year at the most");
    }
}
