//! The engine's entry point (`docs/plans/stage-2-engine.md`, "The entry point"):
//! build everything from the inputs, apply one change recomputing only what it
//! touches, and say which figures moved, by entity and field.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use bagholder_core::jiff::civil::Date;
use bagholder_core::journal::{Group, JournalEntry, JournalSubject, Trade};
use bagholder_core::{AccountId, Dec, InstrumentId, TransactionId};

use crate::cashflow::{build_cashflow, payer_rates, CashRow, PayerRate};
use crate::equity::{broker_checks, build_equity, AccountEquity, BrokerCheck};
use crate::identity::{identify, Identity};
use crate::input::{Adjustment, BrokerAccount, Clock, DeclaredRead, Inputs, Ledger, Quote, Rates, Sourced};
use crate::ledger::{match_lots, Direction, Matched};
use crate::positions::{build_positions, PositionFig};
use crate::scope::{scope, Filters, Scoped};
use crate::trades::{build_trades, TradeFig, TradeKey};

/// One change to what the engine is given. Every part of `Inputs` is changed by
/// exactly one variant.
#[derive(Clone, Debug)]
pub enum Change {
    /// The record: accounts, instruments with their names and terms, the live
    /// transactions, their records' problems, the transfer links.
    Ledger(Ledger),
    /// The trades the book assigned or orphaned.
    Trades(Vec<Trade>),
    Groups(Vec<Group>),
    Journal(BTreeMap<JournalSubject, JournalEntry>),
    Adjustments(BTreeMap<TransactionId, Adjustment>),
    Rates(Rates),
    Declared(InstrumentId, Option<DeclaredRead>),
    Frequency(InstrumentId, Option<Sourced<u32>>),
    Quote(InstrumentId, Option<Quote>),
    Closes(InstrumentId, BTreeMap<Date, Dec>),
    Benchmark(String, BTreeMap<Date, Dec>),
    Broker(AccountId, Option<BrokerAccount>),
    /// The day turning, the instant moving (16:30 passing), the home zone.
    Clock(Clock),
}

/// What kind of entity a moved figure belongs to, and which one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Entity {
    Trade(TradeKey),
    Position(AccountId, InstrumentId, Direction),
    CashRow(TransactionId),
    Payer(InstrumentId),
    Equity(AccountId),
    BrokerCheck(AccountId),
}

/// The entities whose figures moved, each with the fields that did; an entity
/// that appeared or went is listed with the field `*`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Moved(pub BTreeMap<Entity, BTreeSet<&'static str>>);

impl Moved {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// An entity's figures, field by field, for finding what moved.
pub trait Fields {
    fn fields(&self) -> Vec<(&'static str, String)>;
}

fn f<T: Debug>(name: &'static str, v: &T) -> (&'static str, String) {
    (name, format!("{v:?}"))
}

impl Fields for TradeFig {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            f("trade", &self.trade),
            f("account", &self.account),
            f("instrument", &self.instrument),
            f("instruments", &self.instruments),
            f("direction", &self.direction),
            f("qty", &self.qty),
            f("opened_on", &self.opened_on),
            f("closed_on", &self.closed_on),
            f("hold_days", &self.hold_days),
            f("entry", &self.entry),
            f("exit", &self.exit),
            f("basis", &self.basis),
            f("pnl", &self.pnl),
            f("pnl_cad", &self.pnl_cad),
            f("fees", &self.fees),
            f("fees_cad", &self.fees_cad),
            f("slices", &self.slices),
            f("fills", &self.fills),
            f("flags", &self.flags),
            f("journal", &self.journal),
            f("locked", &self.locked),
        ]
    }
}

impl Fields for PositionFig {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            f("key", &self.key),
            f("trade", &self.trade),
            f("qty", &self.qty),
            f("multiplier", &self.multiplier),
            f("book", &self.book),
            f("fees", &self.fees),
            f("avg", &self.avg),
            f("mark", &self.mark),
            f("market", &self.market),
            f("unrealized", &self.unrealized),
            f("day_change", &self.day_change),
            f("book_cad", &self.book_cad),
            f("market_cad", &self.market_cad),
            f("unrealized_cad", &self.unrealized_cad),
            f("day_change_cad", &self.day_change_cad),
            f("held_days", &self.held_days),
            f("opened_on", &self.opened_on),
            f("lots", &self.lots),
            f("fills", &self.fills),
            f("gaps", &self.gaps),
            f("flags", &self.flags),
            f("journal", &self.journal),
            f("broker_qty", &self.broker_qty),
        ]
    }
}

impl Fields for CashRow {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            f("day", &self.day),
            f("kind", &self.kind),
            f("account", &self.account),
            f("instrument", &self.instrument),
            f("qty", &self.qty),
            f("per", &self.per),
            f("amount", &self.amount),
            f("amount_cad", &self.amount_cad),
            f("in_units", &self.in_units),
        ]
    }
}

impl Fields for PayerRate {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![f("per", &self.per), f("source", &self.source), f("per_year", &self.per_year), f("frequency_source", &self.frequency_source), f("next_ex", &self.next_ex), f("next_pay", &self.next_pay)]
    }
}

impl Fields for AccountEquity {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![f("own", &self.own), f("flows", &self.flows), f("points", &self.points), f("returns", &self.returns)]
    }
}

impl Fields for BrokerCheck {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![f("differences", &self.differences), f("pending", &self.pending), f("value_difference", &self.value_difference)]
    }
}

fn diff<'a, T: Fields + 'a>(moved: &mut Moved, before: impl IntoIterator<Item = (Entity, &'a T)>, after: impl IntoIterator<Item = (Entity, &'a T)>) {
    let before: BTreeMap<Entity, Vec<(&'static str, String)>> = before.into_iter().map(|(k, v)| (k, v.fields())).collect();
    let mut after: BTreeMap<Entity, Vec<(&'static str, String)>> = after.into_iter().map(|(k, v)| (k, v.fields())).collect();
    for (k, old) in before {
        match after.remove(&k) {
            None => {
                moved.0.entry(k).or_default().insert("*");
            }
            Some(new) => {
                let changed: BTreeSet<&'static str> = old.iter().zip(new.iter()).filter(|(a, b)| a.1 != b.1).map(|(a, _)| a.0).collect();
                if !changed.is_empty() {
                    moved.0.entry(k).or_default().extend(changed);
                }
            }
        }
    }
    for (k, _) in after {
        moved.0.entry(k).or_default().insert("*");
    }
}

fn trade_entity(t: &TradeFig) -> Entity {
    Entity::Trade(t.key.clone())
}

fn position_entity(p: &PositionFig) -> Entity {
    Entity::Position(p.account, p.instrument, p.direction)
}

/// Every figure, built from the inputs and kept current by `apply`.
#[derive(Clone, Debug)]
pub struct Engine {
    inputs: Inputs,
    matched: Matched,
    identity: Identity,
    trades: Vec<TradeFig>,
    positions: Vec<PositionFig>,
    cash: Vec<CashRow>,
    payers: BTreeMap<InstrumentId, PayerRate>,
    equity: BTreeMap<AccountId, AccountEquity>,
    checks: Vec<BrokerCheck>,
}

/// The figures, read.
pub struct Figures<'a> {
    pub trades: &'a [TradeFig],
    pub positions: &'a [PositionFig],
    pub cash: &'a [CashRow],
    pub payers: &'a BTreeMap<InstrumentId, PayerRate>,
    pub equity: &'a BTreeMap<AccountId, AccountEquity>,
    pub checks: &'a [BrokerCheck],
    pub matched: &'a Matched,
}

impl Engine {
    pub fn build(inputs: Inputs) -> Engine {
        let matched = match_lots(&inputs);
        let identity = identify(&inputs.ledger, &matched);
        let mut e = Engine { inputs, matched, identity, trades: vec![], positions: vec![], cash: vec![], payers: BTreeMap::new(), equity: BTreeMap::new(), checks: vec![] };
        e.derive();
        e
    }

    /// Everything that follows from the match.
    fn derive(&mut self) {
        let i = &self.inputs;
        self.trades = build_trades(i, &self.matched, &self.identity);
        self.positions = build_positions(i, &self.matched, &self.identity, None);
        self.cash = build_cashflow(i, &self.matched);
        self.payers = payer_rates(i, &self.cash);
        self.equity = build_equity(i, &self.matched, None);
        self.checks = broker_checks(i, &self.matched, &self.equity);
    }

    fn rematch(&mut self) {
        self.matched = match_lots(&self.inputs);
        self.identity = identify(&self.inputs.ledger, &self.matched);
        self.derive();
    }

    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn figures(&self) -> Figures<'_> {
        Figures { trades: &self.trades, positions: &self.positions, cash: &self.cash, payers: &self.payers, equity: &self.equity, checks: &self.checks, matched: &self.matched }
    }

    pub fn scope(&self, filters: &Filters) -> Scoped {
        scope(filters, &self.inputs, &self.trades, &self.positions, &self.cash, &self.payers, &self.equity)
    }

    /// The accounts that hold, or ever held, an instrument.
    fn holders(&self, instrument: InstrumentId) -> BTreeSet<AccountId> {
        self.matched.units.keys().filter(|(_, i)| *i == instrument).map(|(a, _)| *a).collect()
    }

    /// Whether a close of this instrument can decide a contract's expiry.
    fn is_underlying(&self, instrument: InstrumentId) -> bool {
        self.inputs.ledger.instruments.values().any(|i| i.terms.as_ref().is_some_and(|t| t.underlying == instrument))
    }

    /// Apply one change, recomputing only what it touches, and say what moved:
    /// only the figures recomputed are compared.
    pub fn apply(&mut self, change: Change) -> Moved {
        let mut moved = Moved::default();
        match change {
            Change::Ledger(l) => {
                let before = self.snapshot(Parts::ALL);
                self.inputs.ledger = Ledger { trades: std::mem::take(&mut self.inputs.ledger.trades), groups: std::mem::take(&mut self.inputs.ledger.groups), journal: std::mem::take(&mut self.inputs.ledger.journal), ..l };
                self.rematch();
                self.compare(&before, &mut moved);
            }
            Change::Adjustments(a) => {
                let before = self.snapshot(Parts::ALL);
                self.inputs.facts.adjustments = a;
                self.rematch();
                self.compare(&before, &mut moved);
            }
            Change::Clock(c) => {
                let before = self.snapshot(Parts::ALL);
                self.inputs.clock = c;
                self.rematch();
                self.compare(&before, &mut moved);
            }
            Change::Rates(r) => {
                let before = self.snapshot(Parts::ALL);
                self.inputs.facts.rates = r;
                self.derive();
                self.compare(&before, &mut moved);
            }
            Change::Trades(t) => {
                let before = self.snapshot(Parts { trades: true, positions: true, ..Parts::NONE });
                self.inputs.ledger.trades = t;
                self.identity = identify(&self.inputs.ledger, &self.matched);
                self.trades = build_trades(&self.inputs, &self.matched, &self.identity);
                self.positions = build_positions(&self.inputs, &self.matched, &self.identity, None);
                self.compare(&before, &mut moved);
            }
            Change::Groups(g) => {
                let before = self.snapshot(Parts { trades: true, ..Parts::NONE });
                self.inputs.ledger.groups = g;
                self.trades = build_trades(&self.inputs, &self.matched, &self.identity);
                self.compare(&before, &mut moved);
            }
            Change::Journal(j) => {
                let before = self.snapshot(Parts { trades: true, positions: true, ..Parts::NONE });
                self.inputs.ledger.journal = j;
                self.trades = build_trades(&self.inputs, &self.matched, &self.identity);
                self.positions = build_positions(&self.inputs, &self.matched, &self.identity, None);
                self.compare(&before, &mut moved);
            }
            Change::Declared(i, d) => {
                let before = self.snapshot(Parts { payers: true, ..Parts::NONE });
                match d {
                    Some(d) => self.inputs.facts.declared.insert(i, d),
                    None => self.inputs.facts.declared.remove(&i),
                };
                self.payers = payer_rates(&self.inputs, &self.cash);
                self.compare(&before, &mut moved);
            }
            Change::Frequency(i, s) => {
                let before = self.snapshot(Parts { payers: true, ..Parts::NONE });
                match s {
                    Some(s) => self.inputs.facts.frequencies.insert(i, s),
                    None => self.inputs.facts.frequencies.remove(&i),
                };
                self.payers = payer_rates(&self.inputs, &self.cash);
                self.compare(&before, &mut moved);
            }
            Change::Quote(i, q) => {
                match q {
                    Some(q) => self.inputs.market.quotes.insert(i, q),
                    None => self.inputs.market.quotes.remove(&i),
                };
                self.reprice(i, &mut moved);
            }
            Change::Closes(i, c) => {
                if self.is_underlying(i) {
                    // a close can decide whether a contract expired worthless
                    let before = self.snapshot(Parts::ALL);
                    self.inputs.market.closes.insert(i, c);
                    self.rematch();
                    self.compare(&before, &mut moved);
                } else {
                    self.inputs.market.closes.insert(i, c);
                    self.reprice(i, &mut moved);
                }
            }
            Change::Benchmark(k, levels) => {
                // read only by the scoped returns, computed when asked for
                self.inputs.market.benchmarks.insert(k, levels);
            }
            Change::Broker(a, b) => {
                let before_positions: Vec<PositionFig> = self.positions.iter().filter(|p| p.account == a).cloned().collect();
                let before_equity: Option<AccountEquity> = self.equity.get(&a).cloned();
                let before_check: Vec<BrokerCheck> = self.checks.iter().filter(|c| c.account == a).cloned().collect();
                match b {
                    Some(b) => self.inputs.market.brokers.insert(a, b),
                    None => self.inputs.market.brokers.remove(&a),
                };
                for p in self.positions.iter_mut().filter(|p| p.account == a) {
                    p.broker_qty = self.inputs.market.brokers.get(&a).and_then(|b| b.held.get(&p.instrument)).copied();
                }
                self.equity.remove(&a);
                self.equity.extend(build_equity(&self.inputs, &self.matched, Some(&BTreeSet::from([a]))));
                self.checks = broker_checks(&self.inputs, &self.matched, &self.equity);
                diff(&mut moved, before_positions.iter().map(|p| (position_entity(p), p)), self.positions.iter().filter(|p| p.account == a).map(|p| (position_entity(p), p)));
                diff(&mut moved, before_equity.iter().map(|e| (Entity::Equity(a), e)), self.equity.get(&a).map(|e| (Entity::Equity(a), e)));
                diff(&mut moved, before_check.iter().map(|c| (Entity::BrokerCheck(c.account), c)), self.checks.iter().filter(|c| c.account == a).map(|c| (Entity::BrokerCheck(c.account), c)));
            }
        }
        moved
    }

    /// A price moved: that instrument's positions, and the series of every
    /// account that holds or held it.
    fn reprice(&mut self, instrument: InstrumentId, moved: &mut Moved) {
        let holders: BTreeSet<AccountId> = self.holders(instrument);
        let before_positions: Vec<PositionFig> = self.positions.iter().filter(|p| p.instrument == instrument).cloned().collect();
        let before_equity: Vec<(AccountId, AccountEquity)> = holders.iter().filter_map(|a| self.equity.get(a).map(|e| (*a, e.clone()))).collect();
        let before_checks: Vec<BrokerCheck> = self.checks.iter().filter(|c| holders.contains(&c.account)).cloned().collect();
        let fresh = build_positions(&self.inputs, &self.matched, &self.identity, Some(instrument));
        self.positions.retain(|p| p.instrument != instrument);
        self.positions.extend(fresh);
        self.positions.sort_by(|a, b| (a.account, a.instrument, a.direction).cmp(&(b.account, b.instrument, b.direction)));
        self.equity.extend(build_equity(&self.inputs, &self.matched, Some(&holders)));
        self.checks = broker_checks(&self.inputs, &self.matched, &self.equity);
        diff(moved, before_positions.iter().map(|p| (position_entity(p), p)), self.positions.iter().filter(|p| p.instrument == instrument).map(|p| (position_entity(p), p)));
        diff(moved, before_equity.iter().map(|(a, e)| (Entity::Equity(*a), e)), holders.iter().filter_map(|a| self.equity.get(a).map(|e| (Entity::Equity(*a), e))));
        diff(moved, before_checks.iter().map(|c| (Entity::BrokerCheck(c.account), c)), self.checks.iter().filter(|c| holders.contains(&c.account)).map(|c| (Entity::BrokerCheck(c.account), c)));
    }

    fn snapshot(&self, parts: Parts) -> Snapshot {
        Snapshot {
            trades: parts.trades.then(|| self.trades.clone()),
            positions: parts.positions.then(|| self.positions.clone()),
            cash: parts.cash.then(|| self.cash.clone()),
            payers: parts.payers.then(|| self.payers.clone()),
            equity: parts.equity.then(|| self.equity.clone()),
            checks: parts.checks.then(|| self.checks.clone()),
        }
    }

    fn compare(&self, before: &Snapshot, m: &mut Moved) {
        if let Some(b) = &before.trades {
            diff(m, b.iter().map(|t| (trade_entity(t), t)), self.trades.iter().map(|t| (trade_entity(t), t)));
        }
        if let Some(b) = &before.positions {
            diff(m, b.iter().map(|p| (position_entity(p), p)), self.positions.iter().map(|p| (position_entity(p), p)));
        }
        if let Some(b) = &before.cash {
            diff(m, b.iter().map(|r| (Entity::CashRow(r.id.clone()), r)), self.cash.iter().map(|r| (Entity::CashRow(r.id.clone()), r)));
        }
        if let Some(b) = &before.payers {
            diff(m, b.iter().map(|(i, p)| (Entity::Payer(*i), p)), self.payers.iter().map(|(i, p)| (Entity::Payer(*i), p)));
        }
        if let Some(b) = &before.equity {
            diff(m, b.iter().map(|(a, e)| (Entity::Equity(*a), e)), self.equity.iter().map(|(a, e)| (Entity::Equity(*a), e)));
        }
        if let Some(b) = &before.checks {
            diff(m, b.iter().map(|c| (Entity::BrokerCheck(c.account), c)), self.checks.iter().map(|c| (Entity::BrokerCheck(c.account), c)));
        }
    }

    /// What differs between this engine's figures and another's, field for
    /// field: empty when an incremental `apply` agrees with a fresh build.
    pub fn differences(&self, other: &Engine) -> Moved {
        let mut m = Moved::default();
        self.compare(&other.snapshot(Parts::ALL), &mut m);
        m
    }
}

/// Which collections a change recomputes.
#[derive(Clone, Copy)]
struct Parts {
    trades: bool,
    positions: bool,
    cash: bool,
    payers: bool,
    equity: bool,
    checks: bool,
}

impl Parts {
    const ALL: Parts = Parts { trades: true, positions: true, cash: true, payers: true, equity: true, checks: true };
    const NONE: Parts = Parts { trades: false, positions: false, cash: false, payers: false, equity: false, checks: false };
}

/// The figures of the collections a change recomputes, as they were before it.
struct Snapshot {
    trades: Option<Vec<TradeFig>>,
    positions: Option<Vec<PositionFig>>,
    cash: Option<Vec<CashRow>>,
    payers: Option<BTreeMap<InstrumentId, PayerRate>>,
    equity: Option<BTreeMap<AccountId, AccountEquity>>,
    checks: Option<Vec<BrokerCheck>>,
}
