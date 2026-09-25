//! Putting a record together: an activity row and every reply read once for it
//! (`crate::record`). What to read for a row is decided here, from the row alone;
//! where the replies come from (the network in a pull, recorded replies in a
//! test) is the [`Replies`] the caller hands in, so a pull and its tests build a
//! record the same way.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::json::Value;
use bagholder_sources::reply::{Node, Read};

use crate::record::{BookMove, Deposits, Positions, Record};

/// What a row needs read beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// Security ids whose records the row needs (its own; a multi-leg order's
    /// legs and each option's underlying are found once those are read).
    pub securities: BTreeSet<String>,
    /// A multi-leg row's order, by its batch id.
    pub order: Option<String>,
    /// A corporate action's entitlements, by the row's id.
    pub entitlements: Option<String>,
    /// A currency conversion's detail, or the detail of a move between accounts
    /// whose row states no amount, by its id.
    pub conversion: Option<String>,
    /// Positions of these accounts on these days.
    pub positions: BTreeSet<(String, String)>,
    /// A transfer in from another institution's detail, by its id: its
    /// positions are read once the detail states when it completed.
    pub transfer: Option<String>,
    /// The accounts whose net deposits show when a move that states no amount
    /// moved its cash.
    pub deposits: Vec<String>,
    /// The id a withdrawal shares with the tax withheld from it.
    pub withheld: Option<String>,
}

impl Needs {
    /// Whether the row is read against positions (its moves are not the book's own).
    pub fn reads_positions(&self) -> bool {
        !self.positions.is_empty() || self.transfer.is_some()
    }
}

/// Where a record's replies come from.
pub trait Replies {
    /// A security's record, as `securities(ids:)` states it.
    fn security(&mut self, id: &str) -> Option<Value>;
    fn order(&mut self, batch: &str) -> Option<Value>;
    fn entitlements(&mut self, activity: &str) -> Option<Value>;
    fn conversion(&mut self, id: &str) -> Option<Value>;
    fn transfer(&mut self, id: &str) -> Option<Value>;
    /// An account's `historicalDaily` nodes from `from`.
    fn deposits(&mut self, account: &str, from: &str) -> Option<Vec<Value>>;
    /// The account's rows read against positions (moves of holdings, corporate
    /// actions) filed from `first` to `last`, other than `except`.
    fn others(&mut self, account: &str, first: &str, last: &str, except: &str) -> Vec<Value>;
    /// The account's tax withheld rows that share this id.
    fn withheld(&mut self, account: &str, id: &str) -> Vec<Value>;
    /// The completed moves between these two accounts, filed from `first` to
    /// `last`, that state their amount, other than `except`.
    fn stated_moves(&mut self, account: &str, other: &str, first: &str, last: &str, except: &str) -> Vec<Value>;
    /// The position nodes of one account as of one day.
    fn positions(&mut self, account: &str, day: &str) -> Option<Value>;
    /// The moves of holdings between the same two accounts as `row` (or, from
    /// another institution, into the same account) on the days next to its own,
    /// itself among them.
    fn siblings(&mut self, row: &Value) -> Vec<Value>;
    /// What the book's own transactions moved in these accounts on these days.
    fn book(&mut self, accounts: &[String], days: &[String]) -> Vec<BookMove>;
}

fn around(day: &str) -> Vec<String> {
    let Ok(d) = day.parse::<jiff::civil::Date>() else { return vec![] };
    [d.yesterday().ok(), Some(d), d.tomorrow().ok()].into_iter().flatten().map(|d| d.to_string()).collect()
}

/// What the row needs, from the row alone. `day` is the day the row is filed
/// under.
pub fn needs(row: &Value, day: &str) -> Read<Needs> {
    let n = Node::root(row);
    let ty = n.text("type")?;
    let mut out = Needs::default();
    if let Some(id) = n.opt_text("securityId")? {
        if !id.starts_with("sec-c-") {
            out.securities.insert(id.to_string());
        }
    }
    let account = n.text("accountId")?.to_string();
    let transfer_type = n.opt_text("transferType")?.unwrap_or("");
    let completed = n.text("unifiedStatus")? == "COMPLETED";
    let mut accounts = vec![account];
    match ty {
        "OPTIONS_MULTILEG" => out.order = n.opt_text("externalCanonicalId")?.map(str::to_string),
        "CORPORATE_ACTION" => out.entitlements = Some(n.text("canonicalId")?.to_string()),
        "FUNDS_CONVERSION" => out.conversion = Some(n.opt_text("externalCanonicalId")?.unwrap_or(n.text("canonicalId")?).to_string()),
        // a move between accounts whose row states no amount: its detail may,
        // and the accounts' net deposits show the cash it moved
        "INTERNAL_TRANSFER" if n.opt_text("amount")?.is_none() => {
            out.conversion = n.opt_text("externalCanonicalId")?.map(str::to_string);
            if completed {
                out.deposits = [Some(n.text("accountId")?), n.opt_text("opposingAccountId")?].into_iter().flatten().map(str::to_string).collect();
            }
        }
        // a withdrawal from a registered account states its gross amount; the
        // tax withheld from it is its own row, sharing its id
        "INTERNAL_TRANSFER" if completed && n.opt_text("subType")? == Some("SOURCE") => out.withheld = n.opt_text("externalCanonicalId")?.map(str::to_string),
        // a transfer in from another institution: the row states the value
        // asked for; what arrived, and when, its detail and the positions do
        "INSTITUTIONAL_TRANSFER_INTENT" if completed => out.transfer = Some(n.text("externalCanonicalId")?.to_string()),
        _ => {}
    }
    let moves_holdings = ty == "CORPORATE_ACTION" || ty == "ASSET_MOVEMENT" || (ty == "INTERNAL_TRANSFER" && transfer_type.contains("in_kind"));
    if moves_holdings {
        if let Some(other) = n.opt_text("opposingAccountId")? {
            accounts.push(other.to_string());
        }
        for a in &accounts {
            for d in around(day) {
                out.positions.insert((a.clone(), d));
            }
        }
    }
    Ok(out)
}

/// The first and last days of a row's group (its own day alone for a row that
/// is not a move of holdings).
fn span(siblings: &[Value], day: &str) -> (String, String) {
    let zones = bagholder_book::zones::Zones::default();
    let mut days: Vec<String> = siblings
        .iter()
        .filter_map(|s| Node::root(s).text("occurredAt").ok()?.parse::<jiff::Timestamp>().ok())
        .filter_map(|at| zones.day(at, crate::mapping::ZONE).ok())
        .map(|d| d.to_string())
        .collect();
    days.push(day.to_string());
    days.sort();
    (days[0].clone(), days[days.len() - 1].clone())
}

fn shift(day: &str, n: i8) -> Option<String> {
    let d: jiff::civil::Date = day.parse().ok()?;
    let d = if n < 0 { d.yesterday().ok()? } else { d.tomorrow().ok()? };
    Some(d.to_string())
}

/// The record of a row: the row, and every reply its needs name. A reply the
/// source did not give is left out, and the mapping says what it waits on.
pub fn assemble(row: Value, day: &str, replies: &mut dyn Replies) -> Read<Record> {
    let needs = needs(&row, day)?;
    let mut r = Record::of(row);
    if let Some(b) = &needs.order {
        r.order = replies.order(b);
    }
    if let Some(a) = &needs.entitlements {
        r.entitlements = replies.entitlements(a);
    }
    if let Some(c) = &needs.conversion {
        r.conversion = replies.conversion(c);
    }
    if let Some(id) = &needs.withheld {
        r.withheld = replies.withheld(Node::root(&r.activity).text("accountId")?, id);
    }
    // a transfer from another institution: the account's positions the day
    // before its row and on the day its detail says it completed
    let mut arrival: Option<(String, String)> = None;
    if let Some(t) = &needs.transfer {
        r.transfer = replies.transfer(t);
        if let Some(done) = r.transfer.as_ref().map(|t| crate::mapping::completed_on(&Node::root(t))).transpose()?.flatten() {
            let account = Node::root(&r.activity).text("accountId")?.to_string();
            let before = shift(day, -1).unwrap_or_else(|| day.to_string());
            let done = done.to_string();
            r.siblings = replies.others(&account, day, &done, Node::root(&r.activity).text("canonicalId")?);
            for d in [&before, &done] {
                if let Some(nodes) = replies.positions(&account, d) {
                    r.positions.push(Positions { account: account.clone(), day: d.clone(), nodes });
                }
            }
            arrival = Some((account, done));
        }
    }
    // a move that states no amount: each account's net deposits from the day
    // before it to the first day either's changed
    if !needs.deposits.is_empty() {
        let from = shift(day, -1).unwrap_or_else(|| day.to_string());
        let mut read: Vec<(String, Vec<Value>)> = Vec::new();
        for a in &needs.deposits {
            if let Some(mut nodes) = replies.deposits(a, &from) {
                nodes.sort_by(|x, y| Node::root(x).text("date").unwrap_or("").cmp(Node::root(y).text("date").unwrap_or("")));
                read.push((a.clone(), nodes));
            }
        }
        // kept over the days its cash can move in: from the day before it to
        // `SETTLES_WITHIN` days after
        let date = |v: &Value| Node::root(v).text("date").unwrap_or("").to_string();
        let last = day.parse::<jiff::civil::Date>().ok().and_then(|d| d.checked_add(jiff::Span::new().days(crate::mapping::SETTLES_WITHIN)).ok()).map(|d| d.to_string()).unwrap_or_else(|| day.to_string());
        for (account, nodes) in read {
            let kept: Vec<Value> = nodes.into_iter().filter(|n| date(n) >= from && date(n) <= last).collect();
            r.deposits.push(Deposits { account, nodes: Value::Array(kept) });
        }
        // the moves between the same two accounts over those days that state
        // their amount: a day one of them shows is theirs
        if let [a, b] = needs.deposits.as_slice() {
            let except = Node::root(&r.activity).opt_text("externalCanonicalId")?.unwrap_or("").to_string();
            r.stated_moves = replies.stated_moves(a, b, &from, &last, &except);
        }
    }
    // a move of holdings is read with its siblings, over the days they span
    let mut positions = needs.positions.clone();
    if !needs.positions.is_empty() {
        let ty = Node::root(&r.activity).text("type")?;
        if ty != "CORPORATE_ACTION" {
            r.siblings = replies.siblings(&r.activity);
        }
        let (first, last) = span(&r.siblings, day);
        let accounts: BTreeSet<String> = needs.positions.iter().map(|(a, _)| a.clone()).collect();
        positions = accounts.iter().flat_map(|a| [shift(&first, -1), shift(&last, 1)].into_iter().flatten().map(move |d| (a.clone(), d))).collect();
    }
    for (account, d) in &positions {
        if let Some(nodes) = replies.positions(account, d) {
            r.positions.push(Positions { account: account.clone(), day: d.clone(), nodes });
        }
    }
    // securities: the row's, its legs', and each option's underlying, and each
    // security the kept positions changed in
    let mut wanted: Vec<String> = needs.securities.into_iter().collect();
    if let Some(order) = &r.order {
        for l in Node::root(order).list("legs")? {
            wanted.push(l.text("securityId")?.to_string());
        }
    }
    // of the positions kept, only the securities whose units changed across the
    // days: the rest moved nothing
    let mut held: BTreeMap<(&str, &str), BTreeMap<String, String>> = BTreeMap::new();
    for p in &r.positions {
        let e = held.entry((p.account.as_str(), p.day.as_str())).or_default();
        for node in Node::root(&p.nodes).as_list()? {
            e.insert(node.obj("security")?.text("id")?.to_string(), node.field("quantity")?.value().canonical());
        }
    }
    let accounts: BTreeSet<&str> = held.keys().map(|(a, _)| *a).collect();
    for a in accounts {
        let days: Vec<&BTreeMap<String, String>> = held.iter().filter(|((x, _), _)| *x == a).map(|(_, m)| m).collect();
        let ids: BTreeSet<&String> = days.iter().flat_map(|m| m.keys()).collect();
        for id in ids {
            let first = days[0].get(id);
            if days.iter().any(|m| m.get(id) != first) && !id.starts_with("sec-c-") {
                wanted.push(id.clone());
            }
        }
    }
    let mut securities: BTreeMap<String, Value> = BTreeMap::new();
    while let Some(id) = wanted.pop() {
        if securities.contains_key(&id) {
            continue;
        }
        let Some(s) = replies.security(&id) else { continue };
        let node = Node::root(&s);
        if let Ok(o) = node.obj("optionDetails") {
            if let Ok(u) = o.obj("underlyingSecurity") {
                wanted.push(u.text("id")?.to_string());
            }
        }
        securities.insert(id, s);
    }
    r.securities = securities;
    if let Some((account, done)) = &arrival {
        let mut days = vec![];
        let mut d = Some(day.to_string());
        while let Some(x) = d {
            if x > *done {
                break;
            }
            d = shift(&x, 1);
            days.push(x);
        }
        r.book = replies.book(std::slice::from_ref(account), &days);
    }
    if !needs.positions.is_empty() {
        let accounts: Vec<String> = needs.positions.iter().map(|(a, _)| a.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        let (first, last) = span(&r.siblings, day);
        let mut days = vec![];
        let mut d = Some(first);
        while let Some(x) = d {
            if Some(&x) == shift(&last, 1).as_ref() {
                days.push(x);
                break;
            }
            d = shift(&x, 1);
            days.push(x);
        }
        r.book = replies.book(&accounts, &days);
    }
    Ok(r)
}

/// Whether a row moves holdings read against positions, in a group with others.
fn moves_holdings(r: &Node) -> bool {
    matches!(r.text("type").unwrap_or(""), "INTERNAL_TRANSFER" | "ASSET_MOVEMENT")
        && r.opt_text("transferType").ok().flatten().is_none_or(|t| !t.contains("in_cash"))
        && r.text("unifiedStatus").ok() == Some("COMPLETED")
}

/// Whether a row is read against positions: a move of holdings, a corporate
/// action, a transfer from another institution.
pub fn read_against_positions(r: &Value) -> bool {
    let n = Node::root(r);
    moves_holdings(&n) || (matches!(n.text("type").unwrap_or(""), "CORPORATE_ACTION" | "INSTITUTIONAL_TRANSFER_INTENT") && n.text("unifiedStatus").ok() == Some("COMPLETED"))
}

/// A move of holdings' group: every such move connected to it by a shared
/// account and days no more than one apart, followed until none is added
/// (`crate::mapping`, "transfer"). `day` is the day a row is filed under.
pub fn group(row: &Value, rows: &[Value], day: impl Fn(&Value) -> Option<jiff::civil::Date>) -> Vec<Value> {
    let accounts = |r: &Node| -> Vec<String> { [r.text("accountId").ok(), r.opt_text("opposingAccountId").ok().flatten()].into_iter().flatten().map(str::to_string).collect() };
    let id = |r: &Value| Node::root(r).text("canonicalId").ok().map(str::to_string);
    let candidates: Vec<&Value> = rows.iter().filter(|r| moves_holdings(&Node::root(r))).collect();
    let mut group: Vec<Value> = vec![row.clone()];
    loop {
        let before = group.len();
        for c in &candidates {
            if group.iter().any(|g| id(g) == id(c)) {
                continue;
            }
            let Some(cd) = day(c) else { continue };
            let ca = accounts(&Node::root(c));
            let joined = group.iter().any(|g| day(g).is_some_and(|gd| (gd - cd).get_days().abs() <= 1) && accounts(&Node::root(g)).iter().any(|a| ca.contains(a)));
            if joined {
                group.push((*c).clone());
            }
        }
        if group.len() == before {
            break;
        }
    }
    group
}
