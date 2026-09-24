//! Wealthsimple answered from recorded replies: a directory of the files
//! `ws-anonymise` writes (`<operation>-<n>.json`, `positions@<account>@<day>.json`),
//! each one reply as Wealthsimple sent it. The adapter's tests read their fixtures
//! through it, and a pull can run on a whole capture of the owner's history
//! (`bagholder pull-broker --replay <dir>`) without signing in, exactly as it runs
//! on the network: the same reading, the same records.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bagholder_book::mapping::Mapping;
use bagholder_broker::{AccountStated, Answer, BookMoves, BrokerAdapter, DayValue, Failure, MovedWhat, Row, Units};
use bagholder_core::json::{self, Value};
use bagholder_core::{Broker, Currency, Dec};
use bagholder_sources::reply::{Mismatch, Node};

use crate::assemble::{self, Replies};
use crate::mapping::{broker, WealthsimpleMapping, ZONE};
use crate::record::BookMove;

pub struct Replay {
    pub rows: Vec<Value>,
    accounts: Vec<Value>,
    securities: BTreeMap<String, Value>,
    orders: BTreeMap<String, Value>,
    entitlements: BTreeMap<String, Value>,
    conversions: BTreeMap<String, Value>,
    positions: BTreeMap<(String, String), Value>,
    /// The balances replies' accounts, each as Wealthsimple sent it.
    balances: Vec<Value>,
    history: BTreeMap<String, Vec<Value>>,
    /// The book's own moves for the record being put together.
    moves: Vec<BookMove>,
    /// What was asked, in order: what a pull would have sent.
    pub asked: Vec<String>,
    zones: bagholder_book::zones::Zones,
}

fn mismatch(m: Mismatch) -> Failure {
    Failure::Mismatch(m.to_string())
}

impl Replay {
    pub fn read(dir: &Path) -> std::io::Result<Replay> {
        let mut r = Replay {
            rows: vec![],
            accounts: vec![],
            securities: BTreeMap::new(),
            orders: BTreeMap::new(),
            entitlements: BTreeMap::new(),
            conversions: BTreeMap::new(),
            positions: BTreeMap::new(),
            balances: vec![],
            history: BTreeMap::new(),
            moves: vec![],
            asked: vec![],
            zones: bagholder_book::zones::Zones::default(),
        };
        let mut seen_rows = BTreeSet::new();
        let mut seen_accounts = BTreeSet::new();
        let mut names: Vec<String> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json") && !n.starts_with("wrong-")).collect();
        names.sort();
        for name in names {
            let text = std::fs::read_to_string(dir.join(&name))?;
            let Ok(v) = json::parse(&text) else { continue };
            let root = Node::root(&v);
            // a refused read answers nothing
            if root.field("errors").is_ok_and(|e| !matches!(e.value(), Value::Null)) {
                continue;
            }
            let Ok(data) = root.obj("data") else { continue };
            if let Some(rest) = name.strip_prefix("positions@").and_then(|n| n.strip_suffix(".json")) {
                let Some((account, day)) = rest.rsplit_once('@') else { continue };
                let mut nodes = Vec::new();
                for a in data.list("accounts").unwrap_or_default() {
                    for e in a.obj("financials").and_then(|f| f.obj("current")).and_then(|c| c.obj("positionsAsOfDate")).and_then(|p| p.list("edges")).unwrap_or_default() {
                        if let Ok(n) = e.obj("node") {
                            nodes.push(n.value().clone());
                        }
                    }
                }
                r.positions.insert((account.to_string(), day.to_string()), Value::Array(nodes));
            } else if let Ok(f) = data.obj("activityFeedItems") {
                for e in f.list("edges").unwrap_or_default() {
                    if let Ok(node) = e.obj("node") {
                        if let Ok(id) = node.text("canonicalId") {
                            if seen_rows.insert(id.to_string()) {
                                r.rows.push(node.value().clone());
                            }
                        }
                    }
                }
            } else if let Ok(list) = data.list("securities") {
                for s in list {
                    if let Ok(id) = s.text("id") {
                        r.securities.insert(id.to_string(), s.value().clone());
                    }
                }
            } else if let Ok(o) = data.obj("soOrdersMultilegOrder") {
                if let Ok(b) = o.text("orderBatchId") {
                    r.orders.insert(b.to_string(), o.value().clone());
                }
            } else if let Ok(c) = data.obj("corporateActionChildActivities") {
                if let Some(first) = c.list("nodes").unwrap_or_default().first() {
                    if let Ok(a) = first.text("activityCanonicalId") {
                        r.entitlements.insert(a.to_string(), c.value().clone());
                    }
                }
            } else if let Ok(t) = data.obj("internalTransfer") {
                if let Ok(id) = t.text("id") {
                    r.conversions.insert(id.to_string(), t.value().clone());
                }
            } else if let Ok(f) = data.obj("searchFundingIntents") {
                for e in f.list("edges").unwrap_or_default() {
                    if let Ok(n) = e.obj("node") {
                        if let Ok(id) = n.text("id") {
                            r.conversions.insert(id.to_string(), n.value().clone());
                        }
                    }
                }
            } else if let Ok(i) = data.obj("identity") {
                for e in i.obj("accounts").and_then(|a| a.list("edges")).unwrap_or_default() {
                    // the accounts list's nodes (a reply asking only for ids is not one)
                    if let Ok(n) = e.obj("node").and_then(|n| n.field("unifiedAccountType").map(|_| n)) {
                        if n.text("id").is_ok_and(|id| seen_accounts.insert(id.to_string())) {
                            r.accounts.push(n.value().clone());
                        }
                    }
                }
            } else if let Ok(a) = data.obj("account") {
                if let (Ok(id), Ok(edges)) = (a.text("id"), a.obj("financials").and_then(|f| f.obj("historicalDaily")).and_then(|h| h.list("edges"))) {
                    r.history.entry(id.to_string()).or_default().extend(edges.into_iter().filter_map(|e| e.obj("node").ok().map(|n| n.value().clone())));
                }
            } else if let Ok(list) = data.list("accounts") {
                r.balances.extend(list.into_iter().map(|a| a.value().clone()));
            }
        }
        Ok(r)
    }

    fn day_of(&self, row: &Value) -> Option<jiff::civil::Date> {
        let at: jiff::Timestamp = Node::root(row).text("occurredAt").ok()?.parse().ok()?;
        self.zones.day(at, ZONE).ok()
    }
}

impl Replies for Replay {
    fn security(&mut self, id: &str) -> Option<Value> {
        self.asked.push(format!("security {id}"));
        self.securities.get(id).cloned()
    }
    fn order(&mut self, batch: &str) -> Option<Value> {
        self.asked.push(format!("order {batch}"));
        self.orders.get(batch).cloned()
    }
    fn entitlements(&mut self, activity: &str) -> Option<Value> {
        self.asked.push(format!("entitlements {activity}"));
        self.entitlements.get(activity).cloned()
    }
    fn conversion(&mut self, id: &str) -> Option<Value> {
        self.asked.push(format!("conversion {id}"));
        self.conversions.get(id).cloned()
    }
    fn positions(&mut self, account: &str, day: &str) -> Option<Value> {
        self.asked.push(format!("positions {account} {day}"));
        self.positions.get(&(account.to_string(), day.to_string())).cloned()
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
    Ok(Row {
        key: n.text("canonicalId")?.to_string(),
        account: n.text("accountId")?.to_string(),
        day,
        settled: settled(n.text("unifiedStatus")?),
        reads_positions,
        value: value.clone(),
    })
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

impl BrokerAdapter for Replay {
    fn broker(&self) -> Broker {
        broker()
    }
    fn mapping(&self) -> &dyn Mapping {
        &WealthsimpleMapping
    }
    fn accounts(&mut self) -> Answer<Vec<AccountStated>> {
        self.asked.push("accounts".into());
        crate::read::accounts(&self.accounts).map_err(mismatch)
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Row>> {
        self.asked.push(format!("activity {account}"));
        let mut out = Vec::new();
        for r in &self.rows {
            if Node::root(r).text("accountId").ok() != Some(account) {
                continue;
            }
            let day = self.day_of(r).ok_or_else(|| Failure::Mismatch("a row whose instant is not a time".into()))?;
            if from.is_some_and(|f| day < f) {
                continue;
            }
            out.push(row_of(r, day).map_err(mismatch)?);
        }
        Ok(out)
    }
    fn record(&mut self, row: &Row, book: &mut dyn BookMoves) -> Answer<Value> {
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
        let record = assemble::assemble(row.value.clone(), &row.day.to_string(), self).map_err(mismatch)?;
        self.moves.clear();
        Ok(record.to_value())
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
        self.asked.push("balances".into());
        let all = crate::read::cash(&self.balances).map_err(mismatch)?;
        Ok(all.into_iter().filter(|(k, _)| accounts.contains(k)).collect())
    }
    fn units(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Vec<Units>> {
        self.asked.push(format!("units {account} {day}"));
        let Some(nodes) = self.positions.get(&(account.to_string(), day.to_string())) else {
            return Err(Failure::Refused(format!("no positions of {account} as of {day} in the capture")));
        };
        crate::read::units(nodes).map_err(mismatch)
    }
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>> {
        self.asked.push(format!("history {account}"));
        let nodes = self.history.get(account).cloned().unwrap_or_default();
        let days = crate::read::history(&nodes).map_err(mismatch)?;
        Ok(days.into_iter().filter(|d| from.is_none_or(|f| d.day >= f)).collect())
    }
}
