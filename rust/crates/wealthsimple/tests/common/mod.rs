//! Recorded replies (`tests/replies/wealthsimple/`, or a capture elsewhere): the
//! files `ws-anonymise` writes, `<operation>-<n>.json`, each one reply as
//! Wealthsimple sent it with the owner's identifiers replaced. [`Recorded`]
//! answers a record's needs from them, as the network does in a pull.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bagholder_book::mapping::{MapContext, Mapped, Mapping};
use bagholder_book::zones::Zones;
use bagholder_core::json::{self, Value};
use bagholder_core::RecordId;
use bagholder_sources::reply::Node;
use bagholder_wealthsimple::assemble::{assemble, Replies};
use bagholder_wealthsimple::mapping::{WealthsimpleMapping, ZONE};
use bagholder_wealthsimple::record::BookMove;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple")
}

/// Every reply in a directory, by operation.
pub struct Recorded {
    pub rows: Vec<Value>,
    securities: BTreeMap<String, Value>,
    orders: BTreeMap<String, Value>,
    entitlements: BTreeMap<String, Value>,
    conversions: BTreeMap<String, Value>,
    positions: BTreeMap<(String, String), Value>,
    /// What the book's own transactions moved, per account, day and security
    /// (`sec-c-<currency>` for cash), as a pull finds it in the book.
    pub moves: BTreeMap<(String, String, String), bagholder_core::Dec>,
    /// Each reply asked for, in order: what a pull would have sent.
    pub asked: Vec<String>,
}

fn replies(dir: &Path) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.starts_with("wrong-") || name.starts_with("positions@") {
            continue;
        }
        let op = name.rsplit_once('-').map(|(a, _)| a.to_string()).unwrap_or(name.clone());
        out.push((op, json::parse(&std::fs::read_to_string(&p).unwrap()).unwrap()));
    }
    out
}

impl Recorded {
    pub fn read(dir: &Path) -> Recorded {
        let mut r = Recorded { rows: vec![], securities: BTreeMap::new(), orders: BTreeMap::new(), entitlements: BTreeMap::new(), conversions: BTreeMap::new(), positions: BTreeMap::new(), moves: BTreeMap::new(), asked: vec![] };
        let mut seen_rows = std::collections::BTreeSet::new();
        for (_, v) in replies(dir) {
            // each reply read by what it answers
            let Ok(data) = Node::root(&v).obj("data") else { continue };
            if let Ok(f) = data.obj("activityFeedItems") {
                for e in f.list("edges").unwrap() {
                    let node = e.obj("node").unwrap();
                    if seen_rows.insert(node.text("canonicalId").unwrap().to_string()) {
                        r.rows.push(node.value().clone());
                    }
                }
            } else if let Ok(list) = data.list("securities") {
                for s in list {
                    r.securities.insert(s.text("id").unwrap().to_string(), s.value().clone());
                }
            } else if let Ok(o) = data.obj("soOrdersMultilegOrder") {
                r.orders.insert(o.text("orderBatchId").unwrap().to_string(), o.value().clone());
            } else if let Ok(c) = data.obj("corporateActionChildActivities") {
                if let Some(first) = c.list("nodes").unwrap().first() {
                    r.entitlements.insert(first.text("activityCanonicalId").unwrap().to_string(), c.value().clone());
                }
            } else if let Ok(t) = data.obj("internalTransfer") {
                r.conversions.insert(t.text("id").unwrap().to_string(), t.value().clone());
            } else if let Ok(f) = data.obj("searchFundingIntents") {
                for e in f.list("edges").unwrap() {
                    let n = e.obj("node").unwrap();
                    r.conversions.insert(n.text("id").unwrap().to_string(), n.value().clone());
                }
            }
        }
        r
    }

    /// Positions replies carry no account and day of their own beyond the
    /// account's id: the capture's variables named the day, so a positions
    /// fixture is `positions-<account>-<day>.json`, read here.
    pub fn with_positions(mut self, dir: &Path) -> Recorded {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("positions@").and_then(|n| n.strip_suffix(".json")) else { continue };
            let (account, day) = rest.rsplit_once('@').unwrap();
            let v = json::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
            // a refused read keeps no positions
            if Node::root(&v).field("errors").is_ok_and(|e| !matches!(e.value(), Value::Null)) {
                continue;
            }
            let nodes: Vec<Value> = Node::root(&v).obj("data").unwrap().list("accounts").unwrap().iter().flat_map(|a| a.obj("financials").unwrap().obj("current").unwrap().obj("positionsAsOfDate").unwrap().list("edges").unwrap().into_iter().map(|e| e.obj("node").unwrap().value().clone())).collect();
            self.positions.insert((account.to_string(), day.to_string()), Value::Array(nodes));
        }
        self
    }
}

impl Replies for Recorded {
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
        // every move of holdings connected to this one by a shared account and
        // days no more than one apart, followed until none is added
        let moves = |r: &Node| matches!(r.text("type").unwrap_or(""), "INTERNAL_TRANSFER" | "ASSET_MOVEMENT" | "INSTITUTIONAL_TRANSFER_INTENT") && r.opt_text("transferType").ok().flatten().is_none_or(|t| !t.contains("in_cash")) && r.text("unifiedStatus").ok() == Some("COMPLETED");
        let accounts = |r: &Node| -> Vec<String> { [r.text("accountId").ok(), r.opt_text("opposingAccountId").ok().flatten()].into_iter().flatten().map(str::to_string).collect() };
        let id = |r: &Value| Node::root(r).text("canonicalId").unwrap().to_string();
        let candidates: Vec<Value> = self.rows.iter().filter(|r| moves(&Node::root(r))).cloned().collect();
        let mut group: Vec<Value> = vec![row.clone()];
        loop {
            let before = group.len();
            for c in &candidates {
                if group.iter().any(|g| id(g) == id(c)) {
                    continue;
                }
                let cd: jiff::civil::Date = day_of(c).parse().unwrap();
                let ca = accounts(&Node::root(c));
                let joined = group.iter().any(|g| {
                    let gd: jiff::civil::Date = day_of(g).parse().unwrap();
                    (gd - cd).get_days().abs() <= 1 && accounts(&Node::root(g)).iter().any(|a| ca.contains(a))
                });
                if joined {
                    group.push(c.clone());
                }
            }
            if group.len() == before {
                break;
            }
        }
        group
    }
    fn book(&mut self, accounts: &[String], days: &[String]) -> Vec<BookMove> {
        self.moves
            .iter()
            .filter(|((a, d, _), _)| accounts.contains(a) && days.contains(d))
            .map(|((a, d, s), q)| BookMove { account: a.clone(), day: d.clone(), security: s.clone(), quantity: q.to_string() })
            .collect()
    }
}

/// The day Wealthsimple files a row under.
pub fn day_of(row: &Value) -> String {
    let at: jiff::Timestamp = Node::root(row).text("occurredAt").unwrap().parse().unwrap();
    Zones::default().day(at, ZONE).unwrap().to_string()
}

/// A row mapped as the book would map its record.
pub fn map_row(recorded: &mut Recorded, row: &Value) -> Mapped {
    let record = assemble(row.clone(), &day_of(row), recorded).unwrap();
    let zones = Zones::default();
    let ctx = MapContext { connection: None, record: RecordId::parse("01923e6a-7b1c-7def-8123-456789abcdef").unwrap(), zones: &zones };
    WealthsimpleMapping.map(&ctx, &record.to_value().canonical())
}

/// Whether a row's record reads positions: it is mapped after the rest, once
/// the book holds what the other rows moved.
pub fn reads_positions(row: &Value) -> bool {
    !bagholder_wealthsimple::assemble::needs(row, &day_of(row)).unwrap().positions.is_empty()
}

/// Every row mapped as a pull stores them: first the rows that move by
/// themselves, their moves kept as the book's, then the rows read against
/// positions, net of those moves.
pub fn map_all(rec: &mut Recorded) -> Vec<(Value, Mapped)> {
    use bagholder_core::instrument::RefScheme;
    let rows = rec.rows.clone();
    let mut out = Vec::new();
    for row in rows.iter().filter(|r| !reads_positions(r)) {
        let m = map_row(rec, row);
        let account = Node::root(row).text("accountId").unwrap().to_string();
        for d in &m.legs {
            let day = d.trade_date.to_string();
            if let (Some(i), Some(q)) = (&d.instrument, d.quantity) {
                if let Some(r) = i.refs.iter().find(|r| matches!(r.scheme, RefScheme::BrokerSecurity(_))) {
                    let e = rec.moves.entry((account.clone(), day.clone(), r.value.clone())).or_insert(bagholder_core::Dec::ZERO);
                    *e = e.checked_add(q).unwrap();
                }
            }
            if let Some(c) = d.cash {
                let e = rec.moves.entry((account.clone(), day.clone(), format!("sec-c-{}", c.currency.to_string().to_lowercase()))).or_insert(bagholder_core::Dec::ZERO);
                *e = e.checked_add(c.amount).unwrap();
            }
        }
        out.push((row.clone(), m));
    }
    for row in rows.iter().filter(|r| reads_positions(r)) {
        let m = map_row(rec, row);
        out.push((row.clone(), m));
    }
    out
}
