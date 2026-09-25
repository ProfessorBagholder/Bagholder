//! Wealthsimple behind the broker interface: one adapter, over any [`Source`] of
//! its replies (the network, `crate::client`; a recorded capture,
//! `crate::replay`), so a pull reads, assembles and maps the same way whichever
//! answers it.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::mapping::Mapping;
use bagholder_broker::{AccountStated, Answer, BookMoves, BrokerAdapter, DayValue, Failure, MovedWhat, Row, Units};
use bagholder_core::json::Value;
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
    /// The first failure met while putting a record together.
    failed: Option<Failure>,
}

fn mismatch(m: Mismatch) -> Failure {
    Failure::Mismatch(m.to_string())
}

impl<S: Source> Wealthsimple<S> {
    pub fn new(source: S) -> Wealthsimple<S> {
        Wealthsimple { source, zones: bagholder_book::zones::Zones::default(), rows: vec![], securities: BTreeMap::new(), positions: BTreeMap::new(), moves: vec![], failed: None }
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
    let reads_positions = !assemble::needs(value, &day.to_string())?.positions.is_empty();
    Ok(Row { key: n.text("canonicalId")?.to_string(), account: n.text("accountId")?.to_string(), day, settled: settled(n.text("unifiedStatus")?), reads_positions, value: value.clone() })
}

/// Whether a status is final: a row pending or in progress is read again.
pub fn settled(status: &str) -> bool {
    !matches!(status, "PENDING" | "IN_PROGRESS" | "PROCESSING")
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
        crate::read::accounts(&nodes).map_err(mismatch)
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Row>> {
        let values = self.source.activity(account, from)?;
        let mut out = Vec::new();
        for v in &values {
            let day = self.day_of(v).ok_or_else(|| Failure::Mismatch("a row whose occurredAt is not an instant".into()))?;
            out.push(row_of(v, day).map_err(mismatch)?);
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
        let needs = assemble::needs(&row.value, &row.day.to_string()).map_err(mismatch)?;
        if !needs.positions.is_empty() {
            // the book's moves over the days the row's group spans, a day either side
            let group = self.siblings(&row.value);
            let days: Vec<jiff::civil::Date> = group.iter().filter_map(|g| self.day_of(g)).chain([row.day]).collect();
            let (first, last) = (days.iter().min().copied().unwrap_or(row.day), days.iter().max().copied().unwrap_or(row.day));
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
        let record = assemble::assemble(row.value.clone(), &row.day.to_string(), self).map_err(mismatch);
        self.moves.clear();
        if let Some(f) = self.failed.take() {
            return Err(f);
        }
        Ok(record?.to_value())
    }
    fn reads_positions(&self, payload: &Value) -> bool {
        let Ok(row) = Node::root(payload).obj("activity") else { return false };
        let Some(day) = self.day_of(row.value()) else { return false };
        assemble::needs(row.value(), &day.to_string()).is_ok_and(|n| !n.positions.is_empty())
    }
    fn unsettled(&self, payload: &Value) -> Option<(String, jiff::civil::Date)> {
        let row = Node::root(payload).obj("activity").ok()?;
        if settled(row.text("unifiedStatus").ok()?) {
            return None;
        }
        Some((row.text("accountId").ok()?.to_string(), self.day_of(row.value())?))
    }
    fn day(&self, at: jiff::Timestamp) -> jiff::civil::Date {
        self.zones.day(at, ZONE).unwrap_or_else(|_| at.to_zoned(jiff::tz::TimeZone::UTC).date())
    }
    fn cash(&mut self, accounts: &[String]) -> Answer<BTreeMap<String, BTreeMap<Currency, Dec>>> {
        let nodes = self.source.balances(accounts)?;
        let all = crate::read::cash(&nodes).map_err(mismatch)?;
        Ok(all.into_iter().filter(|(k, _)| accounts.contains(k)).collect())
    }
    fn units(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Vec<Units>> {
        let nodes = self.source.positions(account, day)?;
        crate::read::units(&nodes).map_err(mismatch)
    }
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>> {
        let nodes = self.source.history(account, from)?;
        let days = crate::read::history(&nodes).map_err(mismatch)?;
        Ok(days.into_iter().filter(|d| from.is_none_or(|f| d.day >= f)).collect())
    }
}
