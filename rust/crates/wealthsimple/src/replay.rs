//! Wealthsimple answered from recorded replies: a directory of the files
//! `ws-anonymise` writes (`<operation>-<n>.json`, `positions@<account>@<day>.json`),
//! each one reply as Wealthsimple sent it. The adapter's tests read their fixtures
//! through it, and a pull can run on a whole capture of the owner's history
//! (`bagholder pull-broker --replay <dir>`) without signing in, exactly as it runs
//! on the network: the same reading, the same records.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bagholder_broker::{Answer, Failure};
use bagholder_core::json::{self, Value};
use bagholder_sources::reply::Node;

use crate::adapter::Source;
use crate::mapping::ZONE;

/// The replies of a capture, answering as Wealthsimple did.
pub struct Replay {
    pub rows: Vec<Value>,
    accounts: Vec<Value>,
    securities: BTreeMap<String, Value>,
    orders: BTreeMap<String, Value>,
    entitlements: BTreeMap<String, Value>,
    conversions: BTreeMap<String, Value>,
    transfers: BTreeMap<String, Value>,
    cards: BTreeMap<String, Value>,
    positions: BTreeMap<(String, String), Value>,
    /// The balances replies' accounts, each as Wealthsimple sent it.
    balances: Vec<Value>,
    history: BTreeMap<String, Vec<Value>>,
    /// What was asked, in order: what a pull would have sent.
    pub asked: Vec<String>,
    zones: bagholder_book::zones::Zones,
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
            transfers: BTreeMap::new(),
            cards: BTreeMap::new(),
            positions: BTreeMap::new(),
            balances: vec![],
            history: BTreeMap::new(),
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
            } else if let Ok(t) = data.obj("accountTransfer") {
                if let Ok(id) = t.text("id") {
                    // the detail, not the reply asking only for its selected assets
                    if t.field("state").is_ok() {
                        r.transfers.insert(id.to_string(), t.value().clone());
                    }
                }
            } else if let Ok(c) = data.obj("creditCardAccount") {
                if let Ok(id) = c.text("id") {
                    r.cards.insert(id.to_string(), c.value().clone());
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

impl Source for Replay {
    fn accounts(&mut self) -> Answer<Vec<Value>> {
        self.asked.push("accounts".into());
        Ok(self.accounts.clone())
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        self.asked.push(format!("activity {account}"));
        Ok(self.rows.iter().filter(|r| Node::root(r).text("accountId").ok() == Some(account) && from.is_none_or(|f| self.day_of(r).is_some_and(|d| d >= f))).cloned().collect())
    }
    fn securities(&mut self, ids: &[String]) -> Answer<Vec<Value>> {
        self.asked.push(format!("securities {}", ids.len()));
        Ok(ids.iter().filter_map(|i| self.securities.get(i).cloned()).collect())
    }
    fn order(&mut self, batch: &str) -> Answer<Option<Value>> {
        self.asked.push(format!("order {batch}"));
        Ok(self.orders.get(batch).cloned())
    }
    fn entitlements(&mut self, activity: &str) -> Answer<Option<Value>> {
        self.asked.push(format!("entitlements {activity}"));
        Ok(self.entitlements.get(activity).cloned())
    }
    fn conversion(&mut self, id: &str) -> Answer<Option<Value>> {
        self.asked.push(format!("conversion {id}"));
        Ok(self.conversions.get(id).cloned())
    }
    fn transfer(&mut self, id: &str) -> Answer<Option<Value>> {
        self.asked.push(format!("transfer {id}"));
        Ok(self.transfers.get(id).cloned())
    }
    fn card(&mut self, account: &str) -> Answer<Value> {
        self.asked.push(format!("card {account}"));
        self.cards.get(account).cloned().ok_or_else(|| Failure::Refused(format!("no card account {account} in the capture")))
    }
    fn positions(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Value> {
        self.asked.push(format!("positions {account} {day}"));
        self.positions.get(&(account.to_string(), day.to_string())).cloned().ok_or_else(|| Failure::Refused(format!("no positions of {account} as of {day} in the capture")))
    }
    fn balances(&mut self, _accounts: &[String]) -> Answer<Vec<Value>> {
        self.asked.push("balances".into());
        Ok(self.balances.clone())
    }
    fn history(&mut self, account: &str, _from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        self.asked.push(format!("history {account}"));
        Ok(self.history.get(account).cloned().unwrap_or_default())
    }
    fn requests(&self) -> usize {
        self.asked.len()
    }
}
