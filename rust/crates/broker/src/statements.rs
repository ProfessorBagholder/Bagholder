//! The movements a broker's activity feed leaves out, read from its monthly
//! statements (`docs/plans/statement-gaps.md`), as bank reconciliation does it.
//!
//! Statements are read only for an account whose cash disagrees with the
//! broker's, and only its currencies that disagree. Each statement row is
//! matched one-to-one to a book transaction of the same account and currency,
//! exact signed amount, on the row's day or the day it states it was executed;
//! the kind only breaks a tie, since the feed and the statement code one move
//! differently. A book transaction is counted on its matched row's day (the
//! statement dates a trade at settlement, the feed at execution), so a sale
//! executed on a month's last session and settled in the next is an outstanding
//! item of the next month, not a difference.
//!
//! The walk goes back from the newest issued month to the base month, the
//! newest one whose closing balance agrees with nothing unmatched, then forward:
//! a month's unmatched rows are booked only if the month then reconciles, and a
//! month that does not stops the booking there, named for the person. A
//! completed month is read once and kept; a later walk reads only months not
//! kept.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::records::Incoming;
use bagholder_book::Book;
use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, ConnectionId, Currency, Dec, RecordId, SourceName};

use crate::pull::Result;
use crate::{BrokerAdapter, Failure, StatementRead, StatementRow};

/// A month whose statement and book do not reconcile: nothing of it, nor of
/// any later month, is booked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreconciled {
    pub account: AccountId,
    /// The month's first day.
    pub month: jiff::civil::Date,
    pub currency: Currency,
    /// The statement's closing balance, where every account behind the book's
    /// states one.
    pub statement: Option<Dec>,
    /// The book's, with the rows that would be booked.
    pub book: Dec,
    /// What stopped it, where it is not the balance alone.
    pub why: Option<String>,
}

/// What the statements did in one pull.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    /// Statement requests sent.
    pub read: usize,
    /// Rows booked: movements the feed left out.
    pub booked: usize,
    /// The two sides of a move between the person's own accounts joined.
    pub joined: usize,
    pub unreconciled: Vec<Unreconciled>,
}

/// A row's place: the broker's account, the month, and its place among the
/// month's rows.
type Place = (String, jiff::civil::Date, usize);

fn month_of(d: jiff::civil::Date) -> jiff::civil::Date {
    d.first_of_month()
}

fn month_end(m: jiff::civil::Date) -> jiff::civil::Date {
    m.last_of_month()
}

fn previous(m: jiff::civil::Date) -> Option<jiff::civil::Date> {
    m.checked_sub(jiff::Span::new().months(1)).ok().map(month_of)
}

/// Read what is needed and book what the feed left out, for each account
/// whose cash disagrees with what the broker states. `keys_of` names the
/// broker's open accounts behind each of the book's; `cash_now`, each one's
/// cash as the broker states it now.
#[allow(clippy::too_many_arguments)]
pub fn run(book: &Book, adapter: &mut dyn BrokerAdapter, connection: ConnectionId, keys_of: &BTreeMap<AccountId, Vec<String>>, cash_now: &BTreeMap<String, BTreeMap<Currency, Dec>>, today: jiff::civil::Date, now: jiff::Timestamp, step: &mut dyn FnMut(crate::Step), failures: &mut Vec<(String, Failure)>) -> Result<Done> {
    let mut done = Done::default();
    let Some(mapping_source) = adapter.statement_mapping().map(|m| m.source()) else { return Ok(done) };
    let txs = book.transactions()?;
    let Some(newest) = previous(month_of(today)) else { return Ok(done) };
    let mut booked_now: Vec<RecordId> = Vec::new();
    for (account, keys) in keys_of {
        let differing = differing(book, &txs, *account)?;
        if differing.is_empty() {
            continue;
        }
        let own: Vec<&Transaction> = txs.iter().filter(|t| t.account == *account).collect();
        let Some(first) = own.iter().map(|t| month_of(t.trade_date)).min() else { continue };
        // the months kept, and those read now, per broker account: a month
        // the broker answered it has not issued is kept as none
        let mut kept: BTreeMap<String, BTreeMap<jiff::civil::Date, Option<Vec<StatementRow>>>> = BTreeMap::new();
        for k in keys {
            let mut months = BTreeMap::new();
            for (m, payload) in book.monthly_statements(connection, k)? {
                let v = bagholder_core::json::parse(&payload).map_err(|e| bagholder_book::BookError::Refused(format!("a kept statement of {k} for {m} is not JSON: {e}")))?;
                if matches!(v, bagholder_core::json::Value::Null) {
                    months.insert(m, None);
                    continue;
                }
                match adapter.statement_rows(k, &v) {
                    Ok(rows) => {
                        months.insert(m, Some(rows));
                    }
                    Err(f) => failures.push((format!("statement:{k}:{m}"), f)),
                }
            }
            kept.insert(k.clone(), months);
        }
        // the newest month issued: the one before it until the broker issues it
        let mut top = newest;
        let mut base: BTreeMap<Currency, Option<jiff::civil::Date>> = differing.iter().map(|c| (*c, None)).collect();
        let mut m = newest;
        let mut stopped = false;
        // the oldest month statements were read back to
        let mut oldest = newest;
        'walk: loop {
            // a month is issued where any of the broker's accounts behind the
            // book's has a statement for it: one merged into another, or opened
            // later, has none, and its rows are none
            let mut issued = false;
            for k in keys {
                let known = kept[k].get(&m).cloned();
                let answer = match known {
                    Some(Some(_)) => {
                        issued = true;
                        continue;
                    }
                    Some(None) => StatementRead::NotIssued,
                    None => {
                        if done.read == 0 {
                            step(crate::Step::Statements);
                        }
                        done.read += 1;
                        match adapter.statement(k, m) {
                            Ok(a) => a,
                            Err(f) => {
                                failures.push((format!("statement:{k}:{m}"), f));
                                stopped = true;
                                break 'walk;
                            }
                        }
                    }
                };
                match answer {
                    StatementRead::Issued { payload, rows } => {
                        let read = book.broker_read(connection, &format!("statement:{k}"), now)?;
                        book.keep_monthly_statement(connection, k, m, &payload.canonical(), &read)?;
                        kept.get_mut(k).expect("each key has its months").insert(m, Some(rows));
                        issued = true;
                    }
                    StatementRead::NotIssued => {
                        // a month before the newest completed one that is not
                        // issued stays so: it is kept, and not asked again
                        if m < newest && kept[k].get(&m).is_none() {
                            let read = book.broker_read(connection, &format!("statement:{k}"), now)?;
                            book.keep_monthly_statement(connection, k, m, "null", &read)?;
                            kept.get_mut(k).expect("each key has its months").insert(m, None);
                        }
                    }
                }
            }
            if !issued {
                // the newest completed month not issued yet: the one before is
                // the newest there is; any earlier month not issued is where the
                // account's statements begin
                match previous(m) {
                    Some(p) if m == newest && p >= first => {
                        top = p;
                        m = p;
                        continue 'walk;
                    }
                    _ => break 'walk,
                }
            }
            oldest = m;
            // every currency that disagrees needs its base month
            let months: Vec<jiff::civil::Date> = months_between(m, top);
            for (c, b) in base.iter_mut() {
                if b.is_none() && reconcile(*c, &months, keys, &kept, cash_now, &own).is_base(m) {
                    *b = Some(m);
                }
            }
            if base.values().all(Option::is_some) {
                break;
            }
            match previous(m) {
                Some(p) if p >= first => m = p,
                _ => break,
            }
        }
        if stopped {
            continue;
        }
        // no statement issued for any month read: nothing to reconcile against
        let any_issued = keys.iter().any(|k| kept[k].range(oldest..=top).any(|(_, rows)| rows.is_some()));
        if !any_issued {
            continue;
        }
        for (currency, b) in base {
            let Some(b) = b else {
                // no month read back to the account's first reconciles: nothing is booked
                let r = reconcile(currency, &months_between(oldest, top), keys, &kept, cash_now, &own);
                let (statement, book_side) = r.closings(top);
                done.unreconciled.push(Unreconciled { account: *account, month: top, currency, statement, book: book_side, why: Some("no month read back to the account's first reconciles".into()) });
                continue;
            };
            let r = reconcile(currency, &months_between(b, top), keys, &kept, cash_now, &own);
            let forward = r.forward(b, top);
            // the months that reconcile: from the base to the month before any that does not
            let proven = |m: jiff::civil::Date| m >= b && forward.stopped.as_ref().is_none_or(|s| m < s.month);
            let mut places = forward.book.clone();
            // a row an imported file's row holds, in a month proven, takes its place
            places.extend(r.file_rows.keys().filter(|p| proven(p.1)).cloned());
            if let Some(s) = forward.stopped {
                done.unreconciled.push(Unreconciled { account: *account, month: s.month, currency, statement: s.statement, book: s.book, why: s.why });
            }
            for place in places {
                let row = &kept[&place.0][&place.1].as_ref().expect("a booked row's month was issued")[place.2];
                let Some((key, payload)) = adapter.statement_record(&place.0, place.1, place.2, row) else { continue };
                let payload = payload.canonical();
                let incoming = Incoming { connection: Some(connection), source_key: &key, payload: &payload, refs: vec![] };
                let mapping = adapter.statement_mapping().expect("checked above");
                let stored = match r.file_rows.get(&place) {
                    // the file's row for the same movement gives way to the statement's
                    Some(file) => book.store_superseding(mapping, &incoming, &[*file], "the broker's statement row for the same movement", now)?,
                    None => book.store(mapping, &incoming, now)?,
                };
                if stored.outcome == bagholder_book::records::Outcome::New {
                    done.booked += 1;
                    booked_now.push(stored.record);
                }
            }
        }
    }
    done.joined = join_moves(book, &mapping_source, &booked_now)?;
    Ok(done)
}

/// Every month from `from` to `to`, oldest first.
fn months_between(from: jiff::civil::Date, to: jiff::civil::Date) -> Vec<jiff::civil::Date> {
    let mut out = vec![];
    let mut m = month_of(from);
    while m <= to {
        out.push(m);
        match m.checked_add(jiff::Span::new().months(1)) {
            Ok(n) => m = n,
            Err(_) => break,
        }
    }
    out
}

/// The currencies whose cash the book and the broker's newest statement of it
/// disagree on.
fn differing(book: &Book, txs: &[Transaction], account: AccountId) -> Result<BTreeSet<Currency>> {
    let Some((_, stated)) = book.stated(account)?.cash else { return Ok(BTreeSet::new()) };
    let mut own: BTreeMap<Currency, Dec> = BTreeMap::new();
    for t in txs.iter().filter(|t| t.account == account) {
        if let Some(c) = t.cash {
            let e = own.entry(c.currency).or_insert(Dec::ZERO);
            match e.checked_add(c.amount) {
                Ok(v) => *e = v,
                // a sum that does not fit is the broker check's to say
                Err(_) => return Ok(BTreeSet::new()),
            }
        }
    }
    let currencies: BTreeSet<Currency> = own.keys().chain(stated.keys()).copied().collect();
    Ok(currencies.into_iter().filter(|c| own.get(c).copied().unwrap_or(Dec::ZERO) != stated.get(c).copied().unwrap_or(Dec::ZERO)).collect())
}

/// One currency of one account over a run of months.
struct Reconciled {
    months: Vec<jiff::civil::Date>,
    /// Each month's statement closing, where every broker account states one.
    closing: BTreeMap<jiff::civil::Date, Option<Dec>>,
    /// Each month's book closing, each matched transaction on its row's day;
    /// none where the sum is too large to hold.
    book: BTreeMap<jiff::civil::Date, Option<Dec>>,
    /// The rows no book transaction matched, by month.
    unmatched: BTreeMap<jiff::civil::Date, Vec<(Place, Dec, Option<Kind>)>>,
    /// Rows matched to a transaction of an imported statement file: booked in
    /// its place, since the broker's statement comes before a file.
    file_rows: BTreeMap<Place, RecordId>,
}

/// Where the forward walk stopped: the month, both closings, and why.
struct Stop {
    month: jiff::civil::Date,
    statement: Option<Dec>,
    book: Dec,
    why: Option<String>,
}

struct Forward {
    book: Vec<Place>,
    stopped: Option<Stop>,
}

impl Reconciled {
    fn is_base(&self, m: jiff::civil::Date) -> bool {
        self.unmatched.get(&m).is_none_or(Vec::is_empty) && matches!((self.closing.get(&m), self.book.get(&m)), (Some(Some(s)), Some(Some(b))) if s == b)
    }

    fn closings(&self, m: jiff::civil::Date) -> (Option<Dec>, Dec) {
        (self.closing.get(&m).copied().flatten(), self.book.get(&m).copied().flatten().unwrap_or(Dec::ZERO))
    }

    /// From the base month forward: each month's unmatched rows are booked
    /// when the month then reconciles; the first that does not stops it.
    fn forward(&self, base: jiff::civil::Date, top: jiff::civil::Date) -> Forward {
        let mut out = Forward { book: vec![], stopped: None };
        let mut pending = Dec::ZERO;
        for m in self.months.iter().copied().filter(|m| *m > base && *m <= top) {
            let rows = self.unmatched.get(&m).cloned().unwrap_or_default();
            let stop = |why: Option<String>, book: Dec| Stop { month: m, statement: self.closing.get(&m).copied().flatten(), book, why };
            // a trade only the statement states is the feed's to state, with its units and price
            if let Some((_, _, k)) = rows.iter().find(|(_, _, k)| !matches!(k, Some(k) if !matches!(k, Kind::Buy | Kind::Sell))) {
                let what = match k {
                    Some(k) => format!("a {k} only the statement states, which is not booked from it"),
                    None => "a row of a code not placed".to_string(),
                };
                out.stopped = Some(stop(Some(what), self.book.get(&m).copied().flatten().unwrap_or(Dec::ZERO)));
                return out;
            }
            let mut with = pending;
            for (_, cash, _) in &rows {
                match with.checked_add(*cash) {
                    Ok(v) => with = v,
                    Err(_) => {
                        out.stopped = Some(stop(Some("a sum too large to add".into()), self.book.get(&m).copied().flatten().unwrap_or(Dec::ZERO)));
                        return out;
                    }
                }
            }
            let book_with = match self.book.get(&m).copied().flatten().ok_or(()).and_then(|b| b.checked_add(with).map_err(|_| ())) {
                Ok(v) => v,
                Err(()) => {
                    out.stopped = Some(stop(Some("a sum too large to add".into()), Dec::ZERO));
                    return out;
                }
            };
            if self.closing.get(&m).copied().flatten() != Some(book_with) {
                out.stopped = Some(stop(None, book_with));
                return out;
            }
            pending = with;
            out.book.extend(rows.into_iter().map(|(p, _, _)| p));
        }
        out
    }
}

/// Match one currency's statement rows over `months` to the account's book
/// transactions and work out each month's closing on both sides.
fn reconcile(currency: Currency, months: &[jiff::civil::Date], keys: &[String], kept: &BTreeMap<String, BTreeMap<jiff::civil::Date, Option<Vec<StatementRow>>>>, cash_now: &BTreeMap<String, BTreeMap<Currency, Dec>>, own: &[&Transaction]) -> Reconciled {
    // the rows in this currency, each by its place
    let mut rows: Vec<(Place, &StatementRow)> = vec![];
    for k in keys {
        for m in months {
            if let Some(Some(list)) = kept[k].get(m) {
                for (i, r) in list.iter().enumerate().filter(|(_, r)| r.currency == currency) {
                    rows.push(((k.clone(), *m, i), r));
                }
            }
        }
    }
    let txs: Vec<&Transaction> = own.iter().copied().filter(|t| t.cash.is_some_and(|c| c.currency == currency)).collect();
    let matched = match_rows(&rows, &txs);
    // each transaction's day as the statement counts it
    let mut day_of: Vec<jiff::civil::Date> = txs.iter().map(|t| t.trade_date).collect();
    let mut row_matched = vec![false; rows.len()];
    let mut file_rows = BTreeMap::new();
    for (ri, ti) in &matched {
        day_of[*ti] = rows[*ri].1.day;
        row_matched[*ri] = true;
        if crate::csv::source() == source_of(txs[*ti]) {
            file_rows.insert(rows[*ri].0.clone(), txs[*ti].id.record);
        }
    }
    let matched_tx: BTreeSet<usize> = matched.iter().map(|(_, t)| *t).collect();
    let top = months.last().copied();
    let mut book = BTreeMap::new();
    for m in months {
        let end = month_end(*m);
        let mut total = Some(Dec::ZERO);
        for (i, t) in txs.iter().enumerate() {
            // an unmatched fill of the newest month may settle in the next and be
            // on its statement: outstanding, not counted until that is issued.
            // Cash moved is dated alike on both sides, so it is never outstanding.
            if !matched_tx.contains(&i) && matches!(t.kind, Kind::Buy | Kind::Sell) && Some(month_of(t.trade_date)) == top && Some(*m) == top {
                continue;
            }
            if day_of[i] <= end {
                total = total.and_then(|s| s.checked_add(t.cash.expect("filtered to cash").amount).ok());
            }
        }
        book.insert(*m, total);
    }
    let mut unmatched: BTreeMap<jiff::civil::Date, Vec<(Place, Dec, Option<Kind>)>> = BTreeMap::new();
    for (i, (place, r)) in rows.iter().enumerate() {
        if !row_matched[i] {
            unmatched.entry(place.1).or_default().push((place.clone(), r.cash, crate::codes::kind(&r.code)));
        }
    }
    Reconciled { closing: closings(currency, months, keys, kept, cash_now), book, unmatched, file_rows, months: months.to_vec() }
}

fn source_of(t: &Transaction) -> SourceName {
    t.mapping.source.clone()
}

/// Each month's closing balance in `currency`, summed over the broker's
/// accounts: a month with rows closes at its last row's balance; one without
/// carries the month before's closing, or the next month's opening. An account
/// with no row in any month read moved nothing through them: its balance is
/// the cash the broker states it holds now, and unstated where it states none.
fn closings(currency: Currency, months: &[jiff::civil::Date], keys: &[String], kept: &BTreeMap<String, BTreeMap<jiff::civil::Date, Option<Vec<StatementRow>>>>, cash_now: &BTreeMap<String, BTreeMap<Currency, Dec>>) -> BTreeMap<jiff::civil::Date, Option<Dec>> {
    let mut sums: BTreeMap<jiff::civil::Date, Option<Dec>> = months.iter().map(|m| (*m, Some(Dec::ZERO))).collect();
    for k in keys {
        // each month's (opening, closing) where it has rows in this currency
        let known: BTreeMap<jiff::civil::Date, (Option<Dec>, Option<Dec>)> = months
            .iter()
            .filter_map(|m| {
                let rows: Vec<&StatementRow> = kept[k].get(m)?.as_ref()?.iter().filter(|r| r.currency == currency).collect();
                if rows.is_empty() {
                    return None;
                }
                Some((*m, chain_ends(&rows)))
            })
            .collect();
        let still = if known.is_empty() { cash_now.get(k).map(|c| c.get(&currency).copied().unwrap_or(Dec::ZERO)) } else { None };
        for m in months {
            let closing = known.get(m).map(|(_, c)| *c).or_else(|| known.range(..*m).next_back().map(|(_, (_, c))| *c)).or_else(|| known.range(*m..).next().map(|(_, (o, _))| *o)).flatten().or(still);
            let s = sums.get_mut(m).expect("each month is summed");
            *s = match (*s, closing) {
                (Some(a), Some(b)) => a.checked_add(b).ok(),
                _ => None,
            };
        }
    }
    sums
}

/// Days after the feed's day a statement may date a movement: it dates one on
/// the day it posted, the feed on the day it was made. On the owner's
/// statements (September 2023 to August 2026, 1,028 pairs) 840 posted the
/// same day, 156 a day later and 32 two to six days later; none earlier.
pub const POSTS_WITHIN_DAYS: i64 = 7;

/// A month's opening and closing balance from its rows' running balance. The
/// statement lists a day's rows in an order its running balance need not
/// follow (the owner's July 2024: the last row listed is not the month's
/// close), so the ends are found from the chain itself: the closing is the one
/// balance after a row that is no row's balance before, the opening the one
/// balance before a row that is no row's balance after. None where the rows do
/// not make one chain.
fn chain_ends(rows: &[&StatementRow]) -> (Option<Dec>, Option<Dec>) {
    let mut after: Vec<Dec> = rows.iter().map(|r| r.balance).collect();
    let mut before: Vec<Dec> = Vec::new();
    for r in rows {
        match r.balance.checked_sub(r.cash) {
            Ok(b) => before.push(b),
            Err(_) => return (None, None),
        }
    }
    // take away each balance that is both some row's after and another's before
    let mut i = 0;
    while i < after.len() {
        match before.iter().position(|b| *b == after[i]) {
            Some(j) => {
                before.remove(j);
                after.remove(i);
            }
            None => i += 1,
        }
    }
    match (before.as_slice(), after.as_slice()) {
        ([o], [c]) => (Some(*o), Some(*c)),
        // every balance both before one row and after another: the month ends
        // where it began, and the first row's balance before it is both
        ([], []) => {
            let o = rows[0].balance.checked_sub(rows[0].cash).ok();
            (o, o)
        }
        _ => (None, None),
    }
}

/// Match rows to transactions one-to-one: the same signed amount, the row
/// dated on the transaction's day or up to `POSTS_WITHIN_DAYS` after it (the
/// row's day being the day it states it was executed, where it states one).
/// The nearest day is taken first; among as near, a transaction whose kind
/// agrees, and one of the broker's before an imported file's, so the kind only
/// breaks a tie and the broker's own rows come before a file's.
fn match_rows(rows: &[(Place, &StatementRow)], txs: &[&Transaction]) -> Vec<(usize, usize)> {
    let lag = |r: &StatementRow, t: &Transaction| (r.book_day() - t.trade_date).get_days() as i64;
    let fits = |r: &StatementRow, t: &Transaction| t.cash.is_some_and(|c| c.amount == r.cash) && (0..=POSTS_WITHIN_DAYS).contains(&lag(r, t));
    let kind_agrees = |r: &StatementRow, t: &Transaction| crate::codes::kind(&r.code) == Some(t.kind);
    let broker = |t: &Transaction| source_of(t) != crate::csv::source();
    let edges: Vec<Vec<usize>> = rows.iter().map(|(_, r)| (0..txs.len()).filter(|&t| fits(r, txs[t])).collect()).collect();
    let mut tx_of: Vec<Option<usize>> = vec![None; txs.len()];
    let mut row_to: Vec<Option<usize>> = vec![None; rows.len()];
    for days in 0..=POSTS_WITHIN_DAYS {
        let near = |r: usize, t: usize| lag(rows[r].1, txs[t]) <= days;
        let phases: [&dyn Fn(usize, usize) -> bool; 4] = [
            &|r, t| near(r, t) && kind_agrees(rows[r].1, txs[t]) && broker(txs[t]),
            &|r, t| near(r, t) && broker(txs[t]),
            &|r, t| near(r, t) && kind_agrees(rows[r].1, txs[t]),
            &|r, t| near(r, t),
        ];
        for allowed in phases {
            for r in 0..rows.len() {
                if row_to[r].is_none() {
                    let mut seen = vec![false; txs.len()];
                    augment(r, &edges, allowed, &mut seen, &mut tx_of, &mut row_to);
                }
            }
        }
    }
    row_to.iter().enumerate().filter_map(|(r, t)| t.map(|t| (r, t))).collect()
}

/// One augmenting path from row `r` (Kuhn's algorithm), over the edges a
/// phase allows; rows matched in an earlier phase stay matched.
fn augment(r: usize, edges: &[Vec<usize>], allowed: &dyn Fn(usize, usize) -> bool, seen: &mut [bool], tx_of: &mut [Option<usize>], row_to: &mut [Option<usize>]) -> bool {
    for &t in &edges[r] {
        if seen[t] || !allowed(r, t) {
            continue;
        }
        seen[t] = true;
        if tx_of[t].is_none_or(|other| augment(other, edges, allowed, seen, tx_of, row_to)) {
            tx_of[t] = Some(r);
            row_to[r] = Some(t);
            return true;
        }
    }
    false
}

/// Join the two sides of each move between the person's own accounts among
/// the rows booked now and before: money out of one account and in to
/// another, on one day, of exactly the same amount, where exactly one pair
/// fits. A pair that does not join stays two movements.
fn join_moves(book: &Book, source: &SourceName, booked_now: &[RecordId]) -> Result<usize> {
    if booked_now.is_empty() {
        return Ok(0);
    }
    let mut booked: Vec<Transaction> = vec![];
    for r in book.live_records(source)? {
        booked.extend(book.transactions_of(r)?);
    }
    let linked: BTreeSet<_> = book.transfer_links()?.into_iter().flat_map(|(a, b)| [a, b]).collect();
    let free: Vec<&Transaction> = booked.iter().filter(|t| !linked.contains(&t.id) && t.cash.is_some()).collect();
    let out_kinds = |k: Kind| matches!(k, Kind::Withdrawal | Kind::TransferOut);
    let in_kinds = |k: Kind| matches!(k, Kind::TransferIn | Kind::Deposit);
    let mut joined = 0;
    for o in free.iter().filter(|t| out_kinds(t.kind)) {
        let oc = o.cash.expect("filtered");
        let fits = |i: &&&Transaction| in_kinds(i.kind) && i.account != o.account && i.trade_date == o.trade_date && i.cash.is_some_and(|c| c.currency == oc.currency && c.amount == oc.amount.neg());
        let into: Vec<&&Transaction> = free.iter().filter(fits).collect();
        let outs = free.iter().filter(|x| out_kinds(x.kind) && x.trade_date == o.trade_date && x.cash == o.cash).count();
        if let ([i], 1) = (into.as_slice(), outs) {
            book.link_transfer(&o.id, &i.id)?;
            joined += 1;
        }
    }
    Ok(joined)
}
