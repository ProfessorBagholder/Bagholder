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
use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType};
use bagholder_core::instrument::RefScheme;
use bagholder_core::json;
use bagholder_core::record::RecordState;
use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, ConnectionId, Dec, RecordId};

use crate::{AccountStated, BookMoves, BrokerAdapter, Failure, Moved, MovedWhat, Row, Step};

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
    report.stated_for = ids.values().copied().collect();
    for a in &stated {
        if let (Some(to), Some(from)) = (a.linked_to.as_ref().and_then(|k| ids.get(k)), ids.get(&a.key)) {
            book.link_accounts(*from, *to, &read)?;
            report.accounts_linked += 1;
        }
    }
    let backing: Vec<(AccountId, AccountId)> = stated.iter().filter_map(|a| Some((*ids.get(&a.key)?, *a.backs.as_ref().and_then(|k| ids.get(k))?))).collect();
    book.store_margin_backing(connection, &backing, &read)?;

    // each account's activity: whole the first time, then from its last full read
    // a row stored before it was final is read again, from its day
    // (from its first day where one's day is not known)
    let mut unsettled: BTreeMap<String, Option<jiff::civil::Date>> = BTreeMap::new();
    for r in book.live_records(&adapter.mapping().source())? {
        if let Some((_, _, payload)) = book.revisions(r)?.pop() {
            if let Some((account, day)) = json::parse(&payload).ok().and_then(|v| adapter.unsettled(&v)) {
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
    // a closed account already read in full has nothing new
    let mut to_read = Vec::new();
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
    let scheme = adapter.record_scheme();
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
    let mut empty = Index { by: BTreeMap::new() };
    for row in &first {
        step(Step::Recording { done, of: recording });
        store_row(book, adapter, &mut empty, connection, row, &scheme, &mut report, now)?;
        done += 1;
    }
    let mut stored_second: Vec<(RecordId, &Row)> = Vec::new();
    if !second.is_empty() {
        let mut index = index(book, adapter, &source, &broker)?;
        for row in &second {
            step(Step::Recording { done, of: recording });
            if let Some(id) = store_row(book, adapter, &mut index, connection, row, &scheme, &mut report, now)? {
                stored_second.push((id, row));
            }
            done += 1;
        }
    }
    let listed: BTreeSet<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    let mut keys_all: BTreeMap<AccountId, Vec<&str>> = BTreeMap::new();
    for a in &stated {
        keys_all.entry(ids[&a.key]).or_default().push(&a.key);
    }
    for (r, key) in unlisted(book, adapter, &scheme, &spans, &keys_all, &listed, &read_before, &mut report)? {
        book.mark_removed(r, now)?;
        report.removed.push((r, key));
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
    let margin: BTreeSet<AccountId> = stated.iter().filter(|a| a.open && is_margin(&a.account_type)).map(|a| ids[&a.key]).collect();
    store_net_values(book, connection, &stated, &keys_of, now)?;
    step(Step::Balances);
    store_balances(book, adapter, connection, &keys_of, &margin, now, &mut report.failures)?;
    // the movements the activity feed left out, for an account whose cash now
    // disagrees with the broker's: read from its monthly statements
    // every account of the broker's behind an open one of the book's, closed ones
    // too: an account merged into another holds the history the book carries
    // over from it, and holds no cash now
    let mut statement_keys: BTreeMap<AccountId, Vec<String>> = BTreeMap::new();
    let mut closed = BTreeSet::new();
    for a in stated.iter().filter(|a| keys_of.contains_key(&ids[&a.key])) {
        statement_keys.entry(ids[&a.key]).or_default().push(a.key.clone());
        if !a.open {
            closed.insert(a.key.clone());
        }
    }
    report.statements = crate::statements::run(book, adapter, connection, &statement_keys, &closed, today, now, step, &mut report.failures)?;
    report.statements.fills = crate::statements::fills(book, adapter, connection, &statement_keys, now, &mut report.failures)?;
    let add = |a: Dec, b: Dec| a.checked_add(b).map_err(|e| bagholder_book::BookError::Refused(format!("a statement too large to add: {e}")));
    let as_of = today.yesterday().map_err(|e| bagholder_book::BookError::Refused(e.to_string()))?;
    // units already stated as of that day are not asked again, unless one
    // held goes by no name yet (its description is read with them), or the
    // statement kept predates the broker's stated worth of each
    let mut holdings = Vec::new();
    for (id, ks) in keys_of.iter().filter(|_| daily) {
        let current = match book.stated(*id)?.units {
            Some((d, lines)) if d == as_of && lines.keys().all(|i| book.stated(*id).map(|s| s.unit_values.contains_key(i)).unwrap_or(false)) => {
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
/// row), a row withdrawn. Its own records by the account and day they name; an
/// imported record of its rows by its transactions' account, once every
/// account of the broker's behind it was read, and their day.
///
/// A row not yet final that is gone is the pending row's life; a final row
/// gone is not, so an account whose read would remove a final row older than
/// the rows read again (before the last full read's day, where only rows not
/// yet final are read again), or more than a few final rows, removes nothing
/// and is reported suspect (brief 08). The imported records are not held to that
/// guard: they are replaced on the first full read, and those the broker no
/// longer lists are the import's copies of rows it has since posted again
/// (the owner's card: 35 at once), which a count would keep counted twice.
fn unlisted(
    book: &Book,
    adapter: &dyn BrokerAdapter,
    scheme: &str,
    spans: &BTreeMap<String, Option<jiff::civil::Date>>,
    keys_all: &BTreeMap<AccountId, Vec<&str>>,
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
    // the span every account behind a book account was read over: the latest start
    let span_of = |id: &AccountId| -> Option<Option<jiff::civil::Date>> {
        let mut span = None;
        for k in keys_all.get(id)? {
            span = Some(match (span.flatten(), *spans.get(*k)?) {
                (Some(a), Some(b)) => Some(std::cmp::max(a, b)),
                (a, b) => a.or(b),
            });
        }
        span
    };
    for (r, key) in book.live_records_by_scheme(scheme, &adapter.mapping().source())? {
        if listed.contains(key.as_str()) {
            continue;
        }
        let txs = book.transactions_of(r)?;
        let Some(first) = txs.iter().map(|t| t.occurred_at.map_or(t.trade_date, |at| adapter.day(at))).min() else { continue };
        let accounts: BTreeSet<AccountId> = txs.iter().map(|t| t.account).collect();
        if accounts.iter().all(|a| span_of(a).is_some_and(|from| covers(&from, first))) {
            out.push((r, key));
        }
    }
    Ok(out)
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
/// five minutes while a page is open): each open account's worth and cash, and what
/// each margin account can borrow, for the connection's accounts the book holds
/// (`SPEC.md`, the sources: net liquidation value at every sync and every five
/// minutes while a page is open).
pub fn balances(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, now: jiff::Timestamp) -> Result<BalancesRead> {
    let broker = adapter.broker();
    let mut keys_of: BTreeMap<AccountId, Vec<String>> = BTreeMap::new();
    let mut margin = BTreeSet::new();
    for a in book.accounts()? {
        if a.connection != connection || a.status != AccountStatus::Open {
            continue;
        }
        let keys: Vec<String> = book.account_refs(a.id)?.into_iter().filter(|r| r.broker == broker).map(|r| r.value).collect();
        if keys.is_empty() {
            continue;
        }
        if is_margin(&a.account_type) {
            margin.insert(a.id);
        }
        keys_of.insert(a.id, keys);
    }
    let mut failures = Vec::new();
    let mut accounts: BTreeSet<AccountId> = BTreeSet::new();
    match adapter.accounts() {
        Ok(stated) => accounts.extend(store_net_values(book, connection, &stated, &keys_of, now)?),
        Err(f) => failures.push(("accounts".into(), f)),
    }
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
/// Net worth by day over every account and outside account, and what the outside
/// debts alone came to each day (a second answer, of those debts alone). The
/// debts' last day must be what is stated of them now, or what they came to is
/// left unread and said. Err(None): the broker states no such history.
fn net_worth_by_day(adapter: &mut dyn BrokerAdapter, accounts: &[String], entities: &[String], outside: &[bagholder_book::debts::OutsideDebt]) -> std::result::Result<(Vec<bagholder_book::debts::NetWorthDay>, Option<Failure>), Option<Failure>> {
    use bagholder_book::debts::{NetWorthDay, LIABILITY_TYPES};
    let days = adapter.net_worth_days(accounts, entities).map_err(Some)?.ok_or(None)?;
    // an answer of no days is no read: said, and asked again at the next
    if days.is_empty() {
        return Err(Some(Failure::Mismatch("Wealthsimple's answer held no days".into())));
    }
    let debts: Vec<&bagholder_book::debts::OutsideDebt> = outside.iter().filter(|d| LIABILITY_TYPES.contains(&d.kind.as_str())).collect();
    let mut owed: BTreeMap<jiff::civil::Date, bagholder_core::Dec> = BTreeMap::new();
    let mut trouble = None;
    if debts.is_empty() {
        owed = days.iter().map(|d| (d.0, bagholder_core::Dec::ZERO)).collect();
    } else {
        let ids: Vec<String> = debts.iter().map(|d| d.entity_id.clone()).collect();
        match adapter.net_worth_days(&[], &ids) {
            Ok(Some(by_day)) if !by_day.is_empty() => match owed_by_day(by_day, &debts) {
                Ok(o) => owed = o,
                Err(f) => trouble = Some(f),
            },
            Ok(_) => trouble = Some(Failure::Mismatch("Wealthsimple's answer for the outside debts held no days".into())),
            Err(f) => trouble = Some(f),
        }
    }
    let out = days.into_iter().map(|(day, amount, currency)| NetWorthDay { day, amount, owed: owed.get(&day).copied(), currency }).collect();
    // the net worth is kept; what the debts came to is said, and read again tomorrow
    Ok((out, trouble))
}

/// What the outside debts came to each day, owed positive, from their share of
/// net worth by day (below zero for what is owed): its last day must be what is
/// stated of them now, within a dollar or a fiftieth, as their balances move
/// during the day.
fn owed_by_day(by_day: Vec<(jiff::civil::Date, bagholder_core::Dec, bagholder_core::Currency)>, debts: &[&bagholder_book::debts::OutsideDebt]) -> std::result::Result<BTreeMap<jiff::civil::Date, bagholder_core::Dec>, Failure> {
    use bagholder_core::Dec;
    let Some(&(_, last, _)) = by_day.last() else { return Err(Failure::Mismatch("Wealthsimple's answer for the outside debts held no days".into())) };
    let flip = last.is_negative();
    let ends = if flip { last.neg() } else { last };
    let stated = debts.iter().try_fold(Dec::ZERO, |t, d| t.checked_add(d.balance).ok());
    let close = stated.is_some_and(|s| {
        let off = ends.checked_sub(s).map(|x| x.abs()).unwrap_or(s);
        off <= Dec::ONE || off.checked_mul(Dec::from_int(50)).is_ok_and(|o| o <= s.abs())
    });
    if !close {
        return Err(Failure::Mismatch("the outside debts' history does not end at what is stated of them today".into()));
    }
    Ok(by_day.into_iter().map(|(d, v, _)| (d, if flip { v.neg() } else { v })).collect())
}

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
            // net worth by day, read once a day: a day's value is the broker's as of its close
            let read_today = book.net_worth_read_at()?.is_some_and(|t| now.duration_since(t) < jiff::SignedDuration::from_hours(20));
            if !read_today {
                let accounts: Vec<String> = keys_of.values().flatten().cloned().collect();
                let entities: Vec<String> = outside.iter().map(|d| d.entity_id.clone()).collect();
                match net_worth_by_day(adapter, &accounts, &entities, &outside) {
                    Ok((days, trouble)) => {
                        book.keep_net_worth_days(&days, now)?;
                        if let Some(f) = trouble {
                            failures.push(("the outside debts by day".into(), f));
                        }
                        let owed = days.iter().filter(|d| d.owed.is_some()).count();
                        bagholder_core::log::line(&format!("bagholder: net worth by day read: {} days, {} to {}, the outside debts on {owed}", days.len(), days[0].day, days[days.len() - 1].day));
                    }
                    Err(Some(f)) => failures.push(("net worth by day".into(), f)),
                    Err(None) => {}
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
    use bagholder_core::{Currency, Dec};

    fn debt(balance: &str) -> bagholder_book::debts::OutsideDebt {
        bagholder_book::debts::OutsideDebt { entity_id: "loc".into(), kind: "LineOfCredit".into(), name: "Line of credit".into(), institution: "Bank".into(), last4: None, balance: Dec::parse(balance).unwrap(), currency: Currency::CAD, as_of: None }
    }

    fn days(values: &[(&str, &str)]) -> Vec<(jiff::civil::Date, Dec, Currency)> {
        values.iter().map(|(d, v)| (d.parse().unwrap(), Dec::parse(v).unwrap(), Currency::CAD)).collect()
    }

    /// The debts' share of net worth is below zero; what they came to is owed
    /// positive, kept when the last day is what is stated of them now.
    #[test]
    fn the_outside_debts_by_day_are_owed_positive_and_end_at_what_is_stated() {
        let (a, b) = (debt("7000"), debt("600"));
        let got = owed_by_day(days(&[("2026-10-01", "-7400"), ("2026-10-02", "-7590")]), &[&a, &b]).unwrap();
        assert_eq!(got.values().map(|v| v.to_string()).collect::<Vec<_>>(), ["7400", "7590"]);
    }

    /// An answer that does not end at what is stated (the whole net worth, say,
    /// were the debts not asked alone) is no read of them.
    #[test]
    fn an_answer_that_ends_elsewhere_is_not_the_debts() {
        let a = debt("7600");
        assert!(owed_by_day(days(&[("2026-10-02", "250000")]), &[&a]).is_err());
        assert!(owed_by_day(days(&[("2026-10-02", "-9000")]), &[&a]).is_err());
        assert!(owed_by_day(Vec::new(), &[&a]).is_err());
    }
}
