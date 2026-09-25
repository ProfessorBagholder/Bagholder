//! Wealthsimple behind the broker interface: one adapter, over any [`Source`] of
//! its replies (the network, `crate::client`; a recorded capture,
//! `crate::replay`), so a pull reads, assembles and maps the same way whichever
//! answers it.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::mapping::Mapping;
use bagholder_broker::{AccountStated, Activity, Answer, BookMoves, BrokerAdapter, DayValue, Failure, MovedWhat, Row, Units};
use bagholder_core::json::Value;
use bagholder_book::mapping::InstrumentDraft;
use bagholder_core::instrument::Reference;
use bagholder_core::{Broker, Currency, Dec};
use bagholder_sources::reply::{Mismatch, Node};

use crate::assemble::{self, Replies};
use crate::mapping::{broker, WealthsimpleMapping, ZONE};
use crate::record::BookMove;

/// What Wealthsimple answers, as it sends it.
pub trait Source {
    /// The accounts list's nodes.
    fn accounts(&mut self) -> Answer<Vec<Value>>;
    /// An account's activity rows from `from` (all of them when `None`).
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>>;
    /// The records of these securities.
    fn securities(&mut self, ids: &[String]) -> Answer<Vec<Value>>;
    fn order(&mut self, batch: &str) -> Answer<Option<Value>>;
    fn entitlements(&mut self, activity: &str) -> Answer<Option<Value>>;
    /// A conversion's detail: a funding intent's node or an internal transfer.
    fn conversion(&mut self, id: &str) -> Answer<Option<Value>>;
    /// A transfer in from another institution's detail.
    fn transfer(&mut self, id: &str) -> Answer<Option<Value>>;
    /// A credit card account (`creditCardAccount`): its balance.
    fn card(&mut self, account: &str) -> Answer<Value>;
    /// The position nodes of an account as of a day.
    fn positions(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Value>;
    /// The balances replies' accounts.
    fn balances(&mut self, accounts: &[String]) -> Answer<Vec<Value>>;
    /// An account's `historicalDaily` nodes from `from`.
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>>;
    /// How many requests have been sent.
    fn requests(&self) -> usize;
}

pub struct Wealthsimple<S: Source> {
    pub source: S,
    zones: bagholder_book::zones::Zones,
    /// Every row read in this pull, for a move's siblings.
    rows: Vec<Value>,
    securities: BTreeMap<String, Option<Value>>,
    positions: BTreeMap<(String, String), Option<Value>>,
    moves: Vec<BookMove>,
    transfers: BTreeMap<String, Option<Value>>,
    /// The credit card accounts, by key, and the currency each is held in:
    /// their balance is stated apart from the others'.
    cards: BTreeMap<String, Currency>,
    /// The first failure met while putting a record together.
    failed: Option<Failure>,
}

fn mismatch(m: Mismatch) -> Failure {
    Failure::Mismatch(m.to_string())
}

impl<S: Source> Wealthsimple<S> {
    pub fn new(source: S) -> Wealthsimple<S> {
        Wealthsimple { source, zones: bagholder_book::zones::Zones::default(), rows: vec![], securities: BTreeMap::new(), positions: BTreeMap::new(), moves: vec![], transfers: BTreeMap::new(), cards: BTreeMap::new(), failed: None }
    }

    fn day_of(&self, row: &Value) -> Option<jiff::civil::Date> {
        let at: jiff::Timestamp = Node::root(row).text("occurredAt").ok()?.parse().ok()?;
        self.zones.day(at, ZONE).ok()
    }

    fn note(&mut self, f: Failure) {
        if self.failed.is_none() {
            self.failed = Some(f);
        }
    }

    /// Read these securities' records into the cache, as few requests as the
    /// source takes (a batch at a time).
    fn prefetch(&mut self, ids: impl IntoIterator<Item = String>) {
        let wanted: Vec<String> = ids.into_iter().filter(|id| !id.starts_with("sec-c-") && !self.securities.contains_key(id)).collect::<BTreeSet<_>>().into_iter().collect();
        for batch in wanted.chunks(50) {
            match self.source.securities(batch) {
                Ok(list) => {
                    for s in list {
                        if let Ok(id) = Node::root(&s).text("id") {
                            self.securities.insert(id.to_string(), Some(s.clone()));
                        }
                    }
                    for id in batch {
                        self.securities.entry(id.clone()).or_insert(None);
                    }
                }
                Err(f) => self.note(f),
            }
        }
    }
}

impl<S: Source> Replies for Wealthsimple<S> {
    fn security(&mut self, id: &str) -> Option<Value> {
        if !self.securities.contains_key(id) {
            self.prefetch([id.to_string()]);
        }
        self.securities.get(id).cloned().flatten()
    }
    fn order(&mut self, batch: &str) -> Option<Value> {
        match self.source.order(batch) {
            Ok(o) => o,
            Err(f) => {
                self.note(f);
                None
            }
        }
    }
    fn entitlements(&mut self, activity: &str) -> Option<Value> {
        match self.source.entitlements(activity) {
            Ok(e) => e,
            Err(f) => {
                self.note(f);
                None
            }
        }
    }
    fn conversion(&mut self, id: &str) -> Option<Value> {
        match self.source.conversion(id) {
            Ok(c) => c,
            Err(f) => {
                self.note(f);
                None
            }
        }
    }
    fn transfer(&mut self, id: &str) -> Option<Value> {
        if let Some(t) = self.transfers.get(id) {
            return t.clone();
        }
        let got = match self.source.transfer(id) {
            Ok(t) => t,
            Err(f) => {
                self.note(f);
                None
            }
        };
        self.transfers.insert(id.to_string(), got.clone());
        got
    }
    fn deposits(&mut self, account: &str, from: &str) -> Option<Vec<Value>> {
        let from: jiff::civil::Date = from.parse().ok()?;
        match self.source.history(account, Some(from)) {
            // the days asked for: a source may answer more
            Ok(nodes) => Some(nodes.into_iter().filter(|n| Node::root(n).day("date").is_ok_and(|d| d >= from)).collect()),
            Err(f) => {
                self.note(f);
                None
            }
        }
    }
    fn others(&mut self, account: &str, first: &str, last: &str, except: &str) -> Vec<Value> {
        let (Ok(first), Ok(last)) = (first.parse::<jiff::civil::Date>(), last.parse::<jiff::civil::Date>()) else { return vec![] };
        self.rows
            .iter()
            .filter(|r| {
                let n = Node::root(r);
                n.text("canonicalId").ok() != Some(except)
                    && (n.text("accountId").ok() == Some(account) || n.opt_text("opposingAccountId").ok().flatten() == Some(account))
                    && assemble::read_against_positions(r)
                    && self.day_of(r).is_some_and(|d| d >= first && d <= last)
            })
            .cloned()
            .collect()
    }
    fn stated_moves(&mut self, account: &str, other: &str, first: &str, last: &str, except: &str) -> Vec<Value> {
        let (Ok(first), Ok(last)) = (first.parse::<jiff::civil::Date>(), last.parse::<jiff::civil::Date>()) else { return vec![] };
        self.rows
            .iter()
            .filter(|r| {
                let n = Node::root(r);
                n.text("type").ok() == Some("INTERNAL_TRANSFER")
                    && n.text("unifiedStatus").ok() == Some("COMPLETED")
                    && n.opt_text("amount").ok().flatten().is_some()
                    && n.text("accountId").ok() == Some(account)
                    && n.opt_text("opposingAccountId").ok().flatten() == Some(other)
                    && n.opt_text("externalCanonicalId").ok().flatten() != Some(except)
                    && self.day_of(r).is_some_and(|d| d >= first && d <= last)
            })
            .cloned()
            .collect()
    }
    fn withheld(&mut self, account: &str, id: &str) -> Vec<Value> {
        self.rows
            .iter()
            .filter(|r| {
                let n = Node::root(r);
                n.text("type").ok() == Some("WITHHOLDING_TAX") && n.text("accountId").ok() == Some(account) && n.opt_text("externalCanonicalId").ok().flatten() == Some(id)
            })
            .cloned()
            .collect()
    }
    fn positions(&mut self, account: &str, day: &str) -> Option<Value> {
        let key = (account.to_string(), day.to_string());
        if let Some(p) = self.positions.get(&key) {
            return p.clone();
        }
        let got = match day.parse() {
            Ok(d) => match self.source.positions(account, d) {
                Ok(v) => Some(v),
                Err(Failure::Refused(_)) => None,
                Err(f) => {
                    self.note(f);
                    None
                }
            },
            Err(_) => None,
        };
        self.positions.insert(key, got.clone());
        got
    }
    fn siblings(&mut self, row: &Value) -> Vec<Value> {
        let zones = &self.zones;
        assemble::group(row, &self.rows, |r| {
            let at: jiff::Timestamp = Node::root(r).text("occurredAt").ok()?.parse().ok()?;
            zones.day(at, ZONE).ok()
        })
    }
    fn book(&mut self, accounts: &[String], days: &[String]) -> Vec<BookMove> {
        self.moves.iter().filter(|m| accounts.contains(&m.account) && days.contains(&m.day)).cloned().collect()
    }
}

/// The adapter's reading of one row, for the pull.
pub fn row_of(value: &Value, day: jiff::civil::Date) -> Result<Row, Mismatch> {
    let n = Node::root(value);
    let reads_positions = assemble::needs(value, &day.to_string())?.reads_positions();
    let status = n.text("unifiedStatus")?;
    let settled = settled(status).ok_or_else(|| n.field("unifiedStatus").map(|f| f.mismatch(format!("a status Wealthsimple's web app does not list: {status:?}"))).unwrap_or_else(|m| m))?;
    Ok(Row { key: n.text("canonicalId")?.to_string(), account: n.text("accountId")?.to_string(), day: Some(day), settled, reads_positions, unread: None, value: value.clone() })
}

/// A row read from an account's activity: one the adapter cannot read is kept
/// with why, not final, so the rest of the account is read and it is read
/// again; one with no id to keep it by is None.
fn read_row(value: &Value, account: &str, day: Option<jiff::civil::Date>) -> Option<Row> {
    let key = Node::root(value).text("canonicalId").ok()?.to_string();
    let unread = |why: String, day: Option<jiff::civil::Date>| Row { key: key.clone(), account: account.to_string(), day, settled: false, reads_positions: false, unread: Some(why), value: value.clone() };
    Some(match day {
        None => unread("a row whose occurredAt is not an instant".to_string(), None),
        Some(d) => match row_of(value, d) {
            Ok(r) => r,
            Err(m) => unread(m.to_string(), Some(d)),
        },
    })
}

/// Whether a status is final: a row pending or in progress is read again.
/// Whether a status is final, as Wealthsimple's web app (release 0.3.668812)
/// lists its statuses; None for one it does not list. A row not final is read
/// again until it is; only a completed one moves anything (`crate::mapping`).
pub fn settled(status: &str) -> Option<bool> {
    match status {
        "COMPLETED" | "CANCELLED" | "DECLINED" | "EXPIRED" | "FAILED" | "REJECTED" | "REFUNDED" | "REVERSED" => Some(true),
        "PENDING" | "IN_PROGRESS" | "IN_REVIEW" | "PARTIALLY_FILLED" | "ACTION_REQUIRED" | "CANCEL_PENDING" | "TRANSFERRING" => Some(false),
        _ => None,
    }
}

/// The book's moves in the form a record keeps (`sec-c-<currency>` for cash).
pub fn book_moves(book: &mut dyn BookMoves, accounts: &[String], days: &[String]) -> Vec<BookMove> {
    let days: Vec<jiff::civil::Date> = days.iter().filter_map(|d| d.parse().ok()).collect();
    book.moves(accounts, &days)
        .into_iter()
        .map(|m| BookMove {
            account: m.account,
            day: m.day.to_string(),
            security: match m.what {
                MovedWhat::Instrument(r) => r.value,
                MovedWhat::Cash(c) => format!("sec-c-{}", c.to_string().to_lowercase()),
            },
            quantity: m.quantity.to_text(),
        })
        .collect()
}

impl<S: Source> BrokerAdapter for Wealthsimple<S> {
    fn broker(&self) -> Broker {
        broker()
    }
    fn mapping(&self) -> &dyn Mapping {
        &WealthsimpleMapping
    }
    fn accounts(&mut self) -> Answer<Vec<AccountStated>> {
        let nodes = self.source.accounts()?;
        let stated = crate::read::accounts(&nodes).map_err(mismatch)?;
        for n in &nodes {
            let n = Node::root(n);
            if n.text("unifiedAccountType").map_err(mismatch)? == "CREDIT_CARD" {
                let currency = Currency::parse(n.text("currency").map_err(mismatch)?).map_err(|e| Failure::Mismatch(e.to_string()))?;
                self.cards.insert(n.text("id").map_err(mismatch)?.to_string(), currency);
            }
        }
        Ok(stated)
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Activity> {
        let values = self.source.activity(account, from)?;
        let mut out = Activity::default();
        for v in &values {
            match read_row(v, account, self.day_of(v)) {
                Some(r) => out.rows.push(r),
                None => {
                    let n = Node::root(v);
                    out.unkeyed.push(format!("{} {}", n.text("type").unwrap_or("a row"), n.text("occurredAt").unwrap_or("of no stated time")));
                }
            }
        }
        self.rows.extend(values);
        Ok(out)
    }
    fn prepare(&mut self, rows: &[&Row]) {
        // the securities these rows name, read now a batch at a time
        let ids: Vec<String> = rows.iter().filter_map(|r| Node::root(&r.value).opt_text("securityId").ok().flatten().map(str::to_string)).collect();
        self.prefetch(ids);
    }
    fn holds(&self, payload: &Value, row: &Row) -> bool {
        Node::root(payload).obj("activity").is_ok_and(|a| a.value() == &row.value)
    }
    fn record(&mut self, row: &Row, book: &mut dyn BookMoves) -> Answer<Value> {
        self.failed = None;
        // a row it could not read is kept as it came, with why
        let (Some(day), None) = (row.day, &row.unread) else {
            let why = row.unread.clone().unwrap_or_else(|| "a row whose day is not known".to_string());
            return Ok(Value::Object(BTreeMap::from([("activity".to_string(), row.value.clone()), ("unread".to_string(), Value::String(why))])));
        };
        let needs = assemble::needs(&row.value, &day.to_string()).map_err(mismatch)?;
        if let Some(t) = &needs.transfer {
            // the book's moves in the account from the row's day to the day the
            // transfer completed
            let done = self.transfer(t).and_then(|v| crate::mapping::completed_on(&Node::root(&v)).ok().flatten());
            if let Some(done) = done {
                let mut span = Vec::new();
                let mut d = day;
                while d <= done {
                    span.push(d.to_string());
                    match d.tomorrow() {
                        Ok(n) => d = n,
                        Err(_) => break,
                    }
                }
                self.moves = book_moves(book, std::slice::from_ref(&row.account), &span);
            }
        }
        if !needs.positions.is_empty() {
            // the book's moves over the days the row's group spans, a day either side
            let group = self.siblings(&row.value);
            let days: Vec<jiff::civil::Date> = group.iter().filter_map(|g| self.day_of(g)).chain([day]).collect();
            let (first, last) = (days.iter().min().copied().unwrap_or(day), days.iter().max().copied().unwrap_or(day));
            let mut accounts: BTreeSet<String> = needs.positions.iter().map(|(a, _)| a.clone()).collect();
            for g in &group {
                let n = Node::root(g);
                accounts.extend([n.text("accountId").ok(), n.opt_text("opposingAccountId").ok().flatten()].into_iter().flatten().map(str::to_string));
            }
            let mut span = Vec::new();
            let mut d = first.yesterday().unwrap_or(first);
            let end = last.tomorrow().unwrap_or(last);
            while d <= end {
                span.push(d.to_string());
                match d.tomorrow() {
                    Ok(n) => d = n,
                    Err(_) => break,
                }
            }
            self.moves = book_moves(book, &accounts.into_iter().collect::<Vec<_>>(), &span);
        }
        let record = assemble::assemble(row.value.clone(), &day.to_string(), self).map_err(mismatch);
        self.moves.clear();
        if let Some(f) = self.failed.take() {
            return Err(f);
        }
        Ok(record?.to_value())
    }
    fn reads_positions(&self, payload: &Value) -> bool {
        let Ok(row) = Node::root(payload).obj("activity") else { return false };
        let Some(day) = self.day_of(row.value()) else { return false };
        assemble::needs(row.value(), &day.to_string()).is_ok_and(|n| n.reads_positions())
    }
    fn unsettled(&self, payload: &Value) -> Option<(String, Option<jiff::civil::Date>)> {
        let root = Node::root(payload);
        let row = root.obj("activity").ok()?;
        let unread = root.field("unread").is_ok();
        if !unread && row.text("unifiedStatus").ok().and_then(settled) != Some(false) {
            return None;
        }
        Some((row.text("accountId").ok()?.to_string(), self.day_of(row.value())))
    }
    fn placed(&self, payload: &Value) -> Option<(String, jiff::civil::Date)> {
        let row = Node::root(payload).obj("activity").ok()?;
        Some((row.text("accountId").ok()?.to_string(), self.day_of(row.value())?))
    }
    fn day(&self, at: jiff::Timestamp) -> jiff::civil::Date {
        self.zones.day(at, ZONE).unwrap_or_else(|_| at.to_zoned(jiff::tz::TimeZone::UTC).date())
    }
    fn cash(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, BTreeMap<Currency, Dec>>> {
        let nodes = self.source.balances(accounts)?;
        let mut all = crate::read::cash(&nodes).map_err(mismatch)?;
        // a card's balance is what is owed on it: the account's cash below zero
        for (key, currency) in self.cards.clone() {
            if accounts.contains(&key) {
                let owed = crate::read::card_balance(&self.source.card(&key)?, &key).map_err(mismatch)?;
                all.insert(key, BTreeMap::from([(currency, owed.neg())]));
            }
        }
        Ok(all.into_iter().filter(|(k, _)| accounts.contains(k)).collect())
    }
    fn units(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Vec<Units>> {
        let nodes = self.source.positions(account, day)?;
        crate::read::units(&nodes).map_err(mismatch)
    }
    fn instruments(&mut self, refs: &[Reference], day: jiff::civil::Date) -> Vec<(Reference, Answer<InstrumentDraft>)> {
        // their securities' records a batch at a time, then options' underlyings
        self.failed = None;
        self.prefetch(refs.iter().map(|r| r.value.clone()));
        let underlyings: Vec<String> = refs
            .iter()
            .filter_map(|r| self.securities.get(&r.value).cloned().flatten())
            .filter_map(|s| Node::root(&s).obj("optionDetails").and_then(|o| o.obj("underlyingSecurity")).and_then(|u| u.text("id").map(str::to_string)).ok())
            .collect();
        self.prefetch(underlyings);
        let failed = self.failed.take();
        refs.iter()
            .map(|r| {
                let mut records: BTreeMap<String, Value> = BTreeMap::new();
                let mut wanted = vec![r.value.clone()];
                while let Some(id) = wanted.pop() {
                    if records.contains_key(&id) {
                        continue;
                    }
                    let Some(s) = self.securities.get(&id).cloned().flatten() else {
                        let f = failed.clone().unwrap_or_else(|| Failure::Mismatch(format!("security {id}, which a position names, is not answered")));
                        return (r.clone(), Err(f));
                    };
                    if let Ok(u) = Node::root(&s).obj("optionDetails").and_then(|o| o.obj("underlyingSecurity")).and_then(|u| u.text("id").map(str::to_string)) {
                        wanted.push(u);
                    }
                    records.insert(id, s);
                }
                (r.clone(), crate::mapping::draft_of(&Value::Object(records), &r.value, day).map_err(Failure::Mismatch))
            })
            .collect()
    }
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>> {
        let nodes = self.source.history(account, from)?;
        let days = crate::read::history(&nodes).map_err(mismatch)?;
        Ok(days.into_iter().filter(|d| from.is_none_or(|f| d.day >= f)).collect())
    }
}
