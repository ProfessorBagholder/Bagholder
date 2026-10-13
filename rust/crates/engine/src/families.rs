//! The match split into families (`docs/plans/stage-3-the-record-by-change.md`,
//! B): sets of holdings no other holding's matching reaches, so a change is
//! matched again in the families it touches and nowhere else.
//!
//! A family is a set of nodes joined by every way the match couples holdings
//! (`ledger::Matcher`):
//! 1. every account's holdings of one instrument (a coin's places and its last
//!    stated price are the instrument's): one node per instrument;
//! 2. a contract and its underlying (an assignment or an exercise moves the
//!    underlying; an order whose legs are not stated waits on every contract of
//!    the underlying);
//! 3. a corporate event: the instrument of the row its adjustment applies to,
//!    and every leg's `from` and `to`;
//! 4. a transfer between accounts: its two sides' instruments (or an account's
//!    cash, where a side names none);
//! 5. every holding of an account, where a row naming no instrument, of a record
//!    whose legs are not stated, makes everything the account holds wait.
//!
//! A row naming no instrument is its account's cash node: it moves the account's
//! cash totals and nothing a lot is matched on, unless 4 or 5 join it to more.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, InstrumentId, TransactionId};

use crate::input::{Adjustments, Ledger};

/// What a family is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    /// Every account's holdings of an instrument.
    Instrument(InstrumentId),
    /// An account's rows that name no instrument.
    Cash(AccountId),
}

/// The node a transaction is matched in.
pub fn node_of(t: &Transaction) -> Node {
    match t.instrument {
        Some(i) => Node::Instrument(i),
        None => Node::Cash(t.account),
    }
}

/// Whether a row makes everything its account holds wait: it names no
/// instrument, its record's legs are not stated, and it is not an event's row.
pub fn waits_the_account(ledger: &Ledger, t: &Transaction) -> bool {
    t.instrument.is_none() && t.kind != Kind::CorporateEvent && ledger.records.get(&t.id.record).is_some_and(|r| r.problems.iter().any(|p| p.code == "leg-unstated"))
}

/// What a regrouping did: the families that went, the ones that took their
/// place, and every node of either.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Regrouped {
    pub gone: BTreeSet<Node>,
    pub families: BTreeSet<Node>,
    pub region: BTreeSet<Node>,
}

/// The families, each named by its least node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Families {
    family_of: BTreeMap<Node, Node>,
    members: BTreeMap<Node, BTreeSet<Node>>,
    /// Each instrument's contracts (the instruments whose terms name it).
    contracts: BTreeMap<InstrumentId, BTreeSet<InstrumentId>>,
    /// Each instrument, the transactions whose adjustments name it in a leg.
    named_by: BTreeMap<InstrumentId, BTreeSet<TransactionId>>,
    /// Each account made one family, by the rows that make it so (5).
    whole: BTreeMap<AccountId, BTreeSet<TransactionId>>,
}

impl Families {
    /// Every family of the ledger.
    pub fn build(ledger: &Ledger, adjustments: &Adjustments) -> Families {
        let mut f = Families::default();
        for (id, info) in &ledger.instruments {
            if let Some(t) = &info.terms {
                f.contracts.entry(t.underlying).or_default().insert(*id);
            }
        }
        for (tx, a) in adjustments.iter() {
            f.name_legs(tx, a, true);
        }
        let mut nodes = BTreeSet::new();
        for t in &ledger.transactions {
            nodes.insert(node_of(t));
            if waits_the_account(ledger, t) {
                f.whole.entry(t.account).or_default().insert(t.id.clone());
            }
        }
        nodes.extend(ledger.instruments.keys().map(|i| Node::Instrument(*i)));
        f.regroup(ledger, adjustments, nodes);
        f
    }

    /// Every transaction of the family named `family`, in the ledger's order.
    pub fn transactions<'a>(&self, ledger: &'a Ledger, family: Node) -> Vec<&'a Transaction> {
        let mut out: Vec<&Transaction> = Vec::new();
        for n in self.members(family) {
            match n {
                Node::Instrument(i) => out.extend(ledger.transactions.of_instrument(i)),
                Node::Cash(a) => out.extend(ledger.transactions.cash_of(a)),
            }
        }
        out.sort_by(|a, b| ledger.transactions.place_of(&a.id).cmp(&ledger.transactions.place_of(&b.id)));
        out
    }

    fn name_legs(&mut self, tx: &TransactionId, a: &crate::input::Adjustment, on: bool) {
        for i in a.legs.iter().flat_map(|l| [l.from, l.to]).flatten() {
            if on {
                self.named_by.entry(i).or_default().insert(tx.clone());
            } else if let Some(s) = self.named_by.get_mut(&i) {
                s.remove(tx);
                if s.is_empty() {
                    self.named_by.remove(&i);
                }
            }
        }
    }

    /// Where two sets of families differ, said for a test.
    pub fn differences(&self, other: &Families) -> String {
        let mut out = Vec::new();
        for (n, f) in &self.family_of {
            if other.family_of.get(n) != Some(f) {
                out.push(format!("{n:?} in {f:?} here, {:?} there", other.family_of.get(n)));
            }
        }
        for (n, f) in &other.family_of {
            if !self.family_of.contains_key(n) {
                out.push(format!("{n:?} in none here, {f:?} there"));
            }
        }
        if self.contracts != other.contracts {
            out.push("contracts".into());
        }
        if self.named_by != other.named_by {
            out.push(format!("named_by {:?} against {:?}", self.named_by, other.named_by));
        }
        if self.whole != other.whole {
            out.push(format!("whole {:?} against {:?}", self.whole, other.whole));
        }
        out.join("; ")
    }

    /// The contracts written on `i`.
    pub fn contracts_on(&self, i: InstrumentId) -> impl Iterator<Item = InstrumentId> + '_ {
        self.contracts.get(&i).into_iter().flatten().copied()
    }

    /// Whether any contract is written on `i`: a close of it can decide a
    /// contract's expiry.
    pub fn is_underlying(&self, i: InstrumentId) -> bool {
        self.contracts.contains_key(&i)
    }

    /// The family a node is in (a node in none is a family of its own).
    pub fn of(&self, n: Node) -> Node {
        self.family_of.get(&n).copied().unwrap_or(n)
    }

    /// The nodes of the family named `family`.
    pub fn members(&self, family: Node) -> BTreeSet<Node> {
        self.members.get(&family).cloned().unwrap_or_else(|| BTreeSet::from([family]))
    }

    /// Every family, by name.
    pub fn all(&self) -> impl Iterator<Item = (&Node, &BTreeSet<Node>)> {
        self.members.iter()
    }

    /// What a node is joined to by the match's couplings.
    fn neighbours(&self, ledger: &Ledger, adjustments: &Adjustments, n: Node) -> BTreeSet<Node> {
        let mut out = BTreeSet::new();
        let txs: Vec<&Transaction> = match n {
            Node::Instrument(i) => {
                // 2: its underlying and its contracts
                if let Some(t) = ledger.instruments.get(&i).and_then(|x| x.terms.as_ref()) {
                    out.insert(Node::Instrument(t.underlying));
                }
                out.extend(self.contracts.get(&i).into_iter().flatten().map(|c| Node::Instrument(*c)));
                // 3: the events whose legs name it, their rows and their other legs
                for tx in self.named_by.get(&i).into_iter().flatten() {
                    if let Some(t) = ledger.transactions.get(tx) {
                        out.insert(node_of(t));
                    }
                    if let Some(a) = adjustments.get(tx) {
                        out.extend(a.legs.iter().flat_map(|l| [l.from, l.to]).flatten().map(Node::Instrument));
                    }
                }
                ledger.transactions.of_instrument(i).collect()
            }
            Node::Cash(a) => ledger.transactions.cash_of(a).collect(),
        };
        for t in txs {
            // 3: the legs of the event this row is the anchor of
            if let Some(a) = adjustments.get(&t.id) {
                out.extend(a.legs.iter().flat_map(|l| [l.from, l.to]).flatten().map(Node::Instrument));
            }
            // 4: the other side of a transfer
            for other in [ledger.transfer_links.received_by(&t.id), ledger.transfer_links.sent_by(&t.id)].into_iter().flatten() {
                if let Some(o) = ledger.transactions.get(other) {
                    out.insert(node_of(o));
                }
            }
            // 5: everything an account made one family holds
            if self.whole.contains_key(&t.account) {
                out.insert(Node::Cash(t.account));
                out.extend(ledger.transactions.instruments_in(t.account).map(Node::Instrument));
            }
        }
        out.remove(&n);
        out
    }

    /// The families of `seeds` (and of every node a coupling reaches from them)
    /// worked out again from the ledger as it is now; the families they make,
    /// by name. Every node of a family of a seed is in one of them.
    pub fn regroup(&mut self, ledger: &Ledger, adjustments: &Adjustments, seeds: BTreeSet<Node>) -> Regrouped {
        // the region: every node of every family a seed is in, and the seeds
        let mut region: BTreeSet<Node> = BTreeSet::new();
        let take = |region: &mut BTreeSet<Node>, f: &Families, n: Node| {
            for m in f.members(f.of(n)) {
                region.insert(m);
            }
            region.insert(n);
        };
        for n in &seeds {
            take(&mut region, self, *n);
        }
        // components of the region, taking in the family of any node a coupling
        // reaches outside it
        let mut seen: BTreeSet<Node> = BTreeSet::new();
        let mut parts: Vec<BTreeSet<Node>> = Vec::new();
        // a node with nothing left (an account's cash with no row, an instrument
        // the ledger no longer holds and nothing names) is a family only where a
        // coupling reaches it, as in a fresh build
        let exists = |n: &Node| match n {
            Node::Instrument(i) => ledger.instruments.contains_key(i) || ledger.transactions.of_instrument(*i).next().is_some(),
            Node::Cash(a) => ledger.transactions.cash_of(*a).next().is_some(),
        };
        let mut pending: Vec<Node> = region.iter().copied().filter(exists).collect();
        while let Some(start) = pending.pop() {
            if seen.contains(&start) {
                continue;
            }
            let mut part = BTreeSet::new();
            let mut stack = vec![start];
            seen.insert(start);
            while let Some(n) = stack.pop() {
                part.insert(n);
                for m in self.neighbours(ledger, adjustments, n) {
                    if !region.contains(&m) {
                        let before = region.len();
                        take(&mut region, self, m);
                        if region.len() > before {
                            pending.extend(self.members(self.of(m)));
                        }
                    }
                    if seen.insert(m) {
                        stack.push(m);
                    }
                }
            }
            parts.push(part);
        }
        // the old families of the region go; the parts take their place
        let mut gone = BTreeSet::new();
        for n in &region {
            if let Some(f) = self.family_of.remove(n) {
                self.members.remove(&f);
                gone.insert(f);
            }
        }
        let mut names = BTreeSet::new();
        for part in parts {
            let name = *part.iter().next().expect("a part holds its start");
            for n in &part {
                self.family_of.insert(*n, name);
            }
            self.members.insert(name, part);
            names.insert(name);
        }
        Regrouped { gone, families: names, region }
    }

    /// The indexes kept beside the families brought to `ledger` and
    /// `adjustments` after a change: the instruments named (`instruments`), the
    /// transactions gone and come (`gone`, `come`) and the adjustments of `txs`.
    pub fn reindex<'a>(
        &mut self,
        ledger: &Ledger,
        was: &BTreeMap<InstrumentId, Option<InstrumentId>>,
        gone: impl IntoIterator<Item = &'a Transaction>,
        come: impl IntoIterator<Item = &'a Transaction>,
        adjustments_was: &BTreeMap<TransactionId, Option<crate::input::Adjustment>>,
        adjustments: &Adjustments,
    ) {
        for (i, under) in was {
            if let Some(u) = under {
                if let Some(s) = self.contracts.get_mut(u) {
                    s.remove(i);
                    if s.is_empty() {
                        self.contracts.remove(u);
                    }
                }
            }
            if let Some(t) = ledger.instruments.get(i).and_then(|x| x.terms.as_ref()) {
                self.contracts.entry(t.underlying).or_default().insert(*i);
            }
        }
        for t in gone {
            if let Some(s) = self.whole.get_mut(&t.account) {
                s.remove(&t.id);
                if s.is_empty() {
                    self.whole.remove(&t.account);
                }
            }
        }
        for t in come {
            if waits_the_account(ledger, t) {
                self.whole.entry(t.account).or_default().insert(t.id.clone());
            }
        }
        for (tx, was) in adjustments_was {
            if let Some(a) = was {
                self.name_legs(tx, a, false);
            }
            if let Some(a) = adjustments.get(tx) {
                self.name_legs(tx, a, true);
            }
        }
    }
}
