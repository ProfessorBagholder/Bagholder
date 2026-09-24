//! Putting a record together: an activity row and every reply read once for it
//! (`crate::record`). What to read for a row is decided here, from the row alone;
//! where the replies come from (the network in a pull, recorded replies in a
//! test) is the [`Replies`] the caller hands in, so a pull and its tests build a
//! record the same way.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::json::Value;
use bagholder_sources::reply::{Node, Read};

use crate::record::{BookMove, Positions, Record};

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
    /// A currency conversion's detail, by its id.
    pub conversion: Option<String>,
    /// Positions of these accounts on these days.
    pub positions: BTreeSet<(String, String)>,
}

/// Where a record's replies come from.
pub trait Replies {
    /// A security's record, as `securities(ids:)` states it.
    fn security(&mut self, id: &str) -> Option<Value>;
    fn order(&mut self, batch: &str) -> Option<Value>;
    fn entitlements(&mut self, activity: &str) -> Option<Value>;
    fn conversion(&mut self, id: &str) -> Option<Value>;
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
    let mut accounts = vec![account];
    match ty {
        "OPTIONS_MULTILEG" => out.order = n.opt_text("externalCanonicalId")?.map(str::to_string),
        "CORPORATE_ACTION" => out.entitlements = Some(n.text("canonicalId")?.to_string()),
        "FUNDS_CONVERSION" => out.conversion = Some(n.opt_text("externalCanonicalId")?.unwrap_or(n.text("canonicalId")?).to_string()),
        _ => {}
    }
    let moves_holdings = ty == "CORPORATE_ACTION" || ty == "ASSET_MOVEMENT" || ty == "INSTITUTIONAL_TRANSFER_INTENT" || (ty == "INTERNAL_TRANSFER" && transfer_type.contains("in_kind"));
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
