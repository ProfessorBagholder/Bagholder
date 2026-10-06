//! The engine's entry point (`docs/plans/stage-2-engine.md`, "The entry point"):
//! build everything from the inputs, apply one change recomputing only what it
//! touches, and say which figures moved, by entity and field.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use bagholder_core::jiff::civil::Date;
use bagholder_core::journal::{Group, JournalEntry, JournalSubject, Trade};
use bagholder_core::{AccountId, InstrumentId, Money, TransactionId};

use crate::stat::benchmark::{build_benchmarks, total_return_cad};
use crate::cashflow::{build_cashflow, payer_rates, CashRow, PayerRate};
use crate::equity::{broker_checks, build_equity, AccountEquity, BrokerCheck};
use crate::identity::{identify, Identity};
use crate::input::{Adjustments, BenchmarkSeries, BrokerAccount, Clock, DeclaredRead, Inputs, Ledger, Quote, Rates, Sourced};
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
    Adjustments(Adjustments),
    Rates(Rates),
    Declared(InstrumentId, Option<DeclaredRead>),
    Frequency(InstrumentId, Option<Sourced<u32>>),
    Quote(InstrumentId, Option<Quote>),
    /// The ex-dividend date a listing's quote states.
    ExDividend(InstrumentId, Option<Date>),
    Closes(InstrumentId, BTreeMap<Date, Money>),
    Benchmark(String, Option<BenchmarkSeries>),
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
    /// A benchmark's total return in CAD, by its key.
    Benchmark(String),
    /// What the broker states of an account now: its cash, what it can borrow,
    /// its value.
    Broker(AccountId),
    /// What the record is as a whole: its accounts and instruments, the facts it
    /// waits on the person for, how many transactions it holds, and today.
    Book,
    /// An instrument's quote, held or not: what a screen showing a listing's own
    /// price reads (a watched listing, a tile).
    Quote(InstrumentId),
    /// The holdings as of a past day a screen shows (`Past`): what they are
    /// worked out from moved.
    Past,
}

/// The holdings as they stood at the close of a past day (`SPEC.md` §5, a range
/// ending before today): the ledger matched to that day, as though it were
/// today, and its positions marked at that day's close and that day's rate.
#[derive(Debug, PartialEq)]
pub struct Past {
    pub day: Date,
    pub matched: Matched,
    pub positions: Vec<PositionFig>,
}

/// Each past day's holdings, kept while a screen holds them and until what
/// they are worked out from moves (`Engine::apply`): matching the ledger again
/// for every reading would cost a whole match each time.
#[derive(Default)]
struct PastCache {
    days: std::sync::Mutex<BTreeMap<Date, std::sync::Weak<Past>>>,
    /// A day was worked out since `Engine::past_built` last asked.
    built: std::sync::atomic::AtomicBool,
}

impl PastCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Date, std::sync::Weak<Past>>> {
        self.days.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The days a screen holds.
    fn held(&self) -> Vec<std::sync::Arc<Past>> {
        self.lock().values().filter_map(std::sync::Weak::upgrade).collect()
    }

    /// Every day from `from` on dropped; whether a screen held one of them.
    fn forget_from(&self, from: Date) -> bool {
        let mut c = self.lock();
        let held = c.range(from..).any(|(_, w)| w.strong_count() > 0);
        c.retain(|d, w| *d < from && w.strong_count() > 0);
        held
    }
}

/// A copy of the engine works its own past days out again.
impl Clone for PastCache {
    fn clone(&self) -> PastCache {
        PastCache::default()
    }
}

impl Debug for PastCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PastCache({:?})", self.lock().keys().collect::<Vec<_>>())
    }
}

/// The first key at which two maps differ.
fn first_difference<K: Ord + Copy, V: PartialEq>(a: &BTreeMap<K, V>, b: &BTreeMap<K, V>) -> Option<K> {
    let missing = |x: &BTreeMap<K, V>, y: &BTreeMap<K, V>| x.iter().find(|(k, v)| y.get(k) != Some(v)).map(|(k, _)| *k);
    [missing(a, b), missing(b, a)].into_iter().flatten().min()
}

/// The entities whose figures moved, each with the fields that did; an entity
/// that appeared or went is listed with the field `*`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Moved(pub BTreeMap<Entity, BTreeSet<&'static str>>);

impl Moved {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Everything `more` moved, added to this.
    pub fn merge(&mut self, more: Moved) {
        for (entity, fields) in more.0 {
            self.0.entry(entity).or_default().extend(fields);
        }
    }
}

/// An entity's figures, field by field, for finding what moved: the names of
/// the fields whose values differ from `other`'s, compared by value.
pub trait Fields {
    fn changed(&self, other: &Self) -> BTreeSet<&'static str>;
}

/// `Fields` over the named fields of a struct, each compared with `==`.
macro_rules! fields {
    ($t:ty; $($f:ident),* $(,)?) => {
        impl Fields for $t {
            fn changed(&self, other: &Self) -> BTreeSet<&'static str> {
                let mut out = BTreeSet::new();
                $(if self.$f != other.$f {
                    out.insert(stringify!($f));
                })*
                out
            }
        }
    };
}

fields!(TradeFig; trade, account, instrument, instruments, direction, qty, opened_on, status, closed_on, last_on, realized, hold_days, entry, exit, basis, pnl, pnl_cad, fees, fees_cad, slices, fills, flags, journal, locked);
fields!(PositionFig; key, trade, qty, multiplier, book, fees, avg, mark, market, unrealized, day_change, book_cad, market_cad, unrealized_cad, day_change_cad, held_days, opened_on, lots, fills, gaps, flags, journal, broker_qty);
fields!(CashRow; day, kind, account, instrument, qty, per, amount, amount_cad, in_units);
fields!(PayerRate; per, source, per_year, frequency_source, next_ex, next_pay);
fields!(AccountEquity; points, returns, gaps);
fields!(BrokerCheck; differences, pending);

/// A benchmark's levels, compared whole.
struct BenchmarkLevels<'a>(&'a crate::stat::benchmark::Levels);
impl Fields for BenchmarkLevels<'_> {
    fn changed(&self, other: &Self) -> BTreeSet<&'static str> {
        if self.0 != other.0 {
            BTreeSet::from(["levels"])
        } else {
            BTreeSet::new()
        }
    }
}

/// The record as a whole, as `Entity::Book` names it.
#[derive(Clone, PartialEq)]
struct BookFig {
    accounts: BTreeMap<AccountId, crate::input::AccountInfo>,
    instruments: BTreeMap<InstrumentId, crate::input::InstrumentInfo>,
    waiting: BTreeMap<TransactionId, crate::ledger::Waiting>,
    activity: usize,
    today: Date,
}
fields!(BookFig; accounts, instruments, waiting, activity, today);

/// What the broker states of an account, compared whole.
struct Stated<'a>(&'a BrokerAccount);
impl Fields for Stated<'_> {
    fn changed(&self, other: &Self) -> BTreeSet<&'static str> {
        if self.0 != other.0 {
            BTreeSet::from(["stated"])
        } else {
            BTreeSet::new()
        }
    }
}

fn diff<'a, T: Fields + 'a>(moved: &mut Moved, before: impl IntoIterator<Item = (Entity, &'a T)>, after: impl IntoIterator<Item = (Entity, &'a T)>) {
    let before: BTreeMap<Entity, &T> = before.into_iter().collect();
    let mut after: BTreeMap<Entity, &T> = after.into_iter().collect();
    for (k, old) in before {
        match after.remove(&k) {
            None => {
                moved.0.entry(k).or_default().insert("*");
            }
            Some(new) => {
                let changed = old.changed(new);
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
    /// Each benchmark's total return in CAD, as a level per session.
    benchmarks: BTreeMap<String, crate::stat::benchmark::Levels>,
    past: PastCache,
}

/// The figures, read.
pub struct Figures<'a> {
    pub trades: &'a [TradeFig],
    pub positions: &'a [PositionFig],
    pub cash: &'a [CashRow],
    pub payers: &'a BTreeMap<InstrumentId, PayerRate>,
    pub equity: &'a BTreeMap<AccountId, AccountEquity>,
    pub checks: &'a [BrokerCheck],
    pub benchmarks: &'a BTreeMap<String, crate::stat::benchmark::Levels>,
    pub matched: &'a Matched,
}

impl Engine {
    pub fn build(inputs: Inputs) -> Engine {
        let matched = match_lots(&inputs);
        let identity = identify(&inputs.ledger, &matched);
        let mut e = Engine { inputs, matched, identity, trades: vec![], positions: vec![], cash: vec![], payers: BTreeMap::new(), equity: BTreeMap::new(), checks: vec![], benchmarks: BTreeMap::new(), past: PastCache::default() };
        e.benchmarks = build_benchmarks(&e.inputs.market.benchmarks, &e.inputs.facts.rates, &e.inputs.clock);
        e.derive();
        e
    }

    /// Everything that follows from the match.
    fn derive(&mut self) {
        let i = &self.inputs;
        self.trades = build_trades(i, &self.matched, &self.identity);
        self.positions = build_positions(i, &self.matched, &self.identity, None, None);
        self.cash = build_cashflow(i, &self.matched);
        self.payers = payer_rates(i, &self.cash, &self.matched);
        self.equity = build_equity(i, None);
        self.checks = broker_checks(i, &self.matched);
    }

    fn rematch(&mut self) {
        self.matched = match_lots(&self.inputs);
        self.identity = identify(&self.inputs.ledger, &self.matched);
        self.derive();
    }

    /// The holdings at the close of `day`, a day before today.
    pub fn past(&self, day: Date) -> std::sync::Arc<Past> {
        let mut cache = self.past.lock();
        if let Some(p) = cache.get(&day).and_then(std::sync::Weak::upgrade) {
            return p;
        }
        cache.retain(|_, w| w.strong_count() > 0);
        let i = &self.inputs;
        let kept: Vec<_> = i.ledger.transactions.iter().filter(|t| t.trade_date <= day).cloned().collect();
        let ids: BTreeSet<&TransactionId> = kept.iter().map(|t| &t.id).collect();
        let ledger = Ledger {
            accounts: i.ledger.accounts.clone(),
            instruments: i.ledger.instruments.clone(),
            transfer_links: i.ledger.transfer_links.iter().filter(|(o, n)| ids.contains(o) && ids.contains(n)).cloned().collect(),
            records: i.ledger.records.clone(),
            trades: i.ledger.trades.clone(),
            groups: i.ledger.groups.clone(),
            journal: i.ledger.journal.clone(),
            transactions: kept,
        };
        // that day as today: nothing after it is known, and no quote is of it
        let mut market = i.market.clone();
        market.quotes.clear();
        let inputs = Inputs { ledger, facts: i.facts.clone(), market, clock: Clock { today: day, ..i.clock.clone() } };
        let matched = match_lots(&inputs);
        let positions = build_positions(&inputs, &matched, &self.identity, None, Some(day));
        let p = std::sync::Arc::new(Past { day, matched, positions });
        cache.insert(day, std::sync::Arc::downgrade(&p));
        self.past.built.store(true, std::sync::atomic::Ordering::SeqCst);
        p
    }

    /// The holdings `filters` shows on the Portfolio: those held now, or, under
    /// dates that end before today, those held at the close of their last day.
    pub fn holdings(&self, filters: &Filters) -> crate::scope::Portfolio {
        match filters.past_end(self.inputs.clock.today) {
            Some(day) => crate::scope::portfolio_as_of(filters, &self.inputs, self.past(day)),
            None => self.portfolio(filters),
        }
    }

    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The facts the figures use: what the readers read.
    pub fn needs(&self) -> crate::needs::FactNeeds {
        let mut n = crate::needs::fact_needs(&self.inputs, &self.matched);
        // each past day a screen holds: the closes its holdings are priced by
        for p in self.past.held() {
            for h in &p.positions {
                n.spans.entry(h.instrument).or_default().insert((h.opened_on, p.day));
            }
        }
        n
    }

    /// Whether a past day's holdings were worked out since this was last asked:
    /// their closes are then among the needs (`needs`), to be read.
    pub fn past_built(&self) -> bool {
        self.past.built.swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    pub fn figures(&self) -> Figures<'_> {
        Figures { trades: &self.trades, positions: &self.positions, cash: &self.cash, payers: &self.payers, equity: &self.equity, checks: &self.checks, benchmarks: &self.benchmarks, matched: &self.matched }
    }

    pub fn scope(&self, filters: &Filters) -> Scoped {
        scope(filters, &self.inputs, &self.trades, &self.positions, &self.cash, &self.payers, &self.equity, &self.benchmarks)
    }

    /// The trades `filters` shows, in the engine's order.
    pub fn trades_in_scope(&self, filters: &Filters) -> Vec<TradeKey> {
        crate::scope::trades_in_scope(filters, &self.inputs, &self.trades)
    }

    /// The dashboard's part of `filters`' scope.
    pub fn dashboard(&self, filters: &Filters) -> crate::scope::Dashboard {
        crate::scope::dashboard(filters, &self.inputs, &self.trades, &self.equity, &self.benchmarks)
    }

    /// The holdings' part of `filters`' scope.
    pub fn portfolio(&self, filters: &Filters) -> crate::scope::Portfolio {
        crate::scope::portfolio_in_scope(filters, &self.inputs, &self.positions)
    }

    /// The cashflow's part of `filters`' scope, over the holdings' part.
    pub fn cashflow(&self, filters: &Filters, portfolio: &crate::scope::Portfolio) -> crate::scope::Cashflow {
        crate::scope::cashflow_in_scope(filters, &self.inputs, &self.trades, &self.positions, &self.cash, &self.payers, portfolio)
    }

    /// Whether a close of this instrument can decide a contract's expiry.
    fn is_underlying(&self, instrument: InstrumentId) -> bool {
        self.inputs.ledger.instruments.values().any(|i| i.terms.as_ref().is_some_and(|t| t.underlying == instrument))
    }

    /// Apply one change, recomputing only what it touches, and say what moved:
    /// only the figures recomputed are compared.
    pub fn apply(&mut self, change: Change) -> Moved {
        let mut moved = Moved::default();
        // the first day whose past holdings this change can move: the record and
        // the trades and notes move every day's; rates and closes the days from
        // the first they changed on
        let stale: Option<Date> = match &change {
            Change::Ledger(_) | Change::Adjustments(_) | Change::Trades(_) | Change::Journal(_) => Some(Date::MIN),
            Change::Rates(r) => {
                let was = &self.inputs.facts.rates;
                if r.covered != was.covered || r.series != was.series || r.holidays != was.holidays {
                    Some(Date::MIN)
                } else {
                    let currencies: BTreeSet<_> = r.by_currency.keys().chain(was.by_currency.keys()).collect();
                    let empty = BTreeMap::new();
                    currencies.into_iter().filter_map(|c| first_difference(r.by_currency.get(c).unwrap_or(&empty), was.by_currency.get(c).unwrap_or(&empty))).min()
                }
            }
            Change::Closes(i, c) => first_difference(c, self.inputs.market.closes.get(i).unwrap_or(&BTreeMap::new())),
            _ => None,
        };
        if stale.is_some_and(|d| self.past.forget_from(d)) {
            moved.0.entry(Entity::Past).or_default().insert("*");
        }
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
                self.benchmarks = build_benchmarks(&self.inputs.market.benchmarks, &self.inputs.facts.rates, &self.inputs.clock);
                self.compare(&before, &mut moved);
            }
            Change::Rates(r) => {
                let before = self.snapshot(Parts::ALL);
                self.inputs.facts.rates = r;
                self.derive();
                self.benchmarks = build_benchmarks(&self.inputs.market.benchmarks, &self.inputs.facts.rates, &self.inputs.clock);
                self.compare(&before, &mut moved);
            }
            Change::Trades(t) => {
                let before = self.snapshot(Parts { trades: true, positions: true, ..Parts::NONE });
                self.inputs.ledger.trades = t;
                self.identity = identify(&self.inputs.ledger, &self.matched);
                self.trades = build_trades(&self.inputs, &self.matched, &self.identity);
                self.positions = build_positions(&self.inputs, &self.matched, &self.identity, None, None);
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
                self.positions = build_positions(&self.inputs, &self.matched, &self.identity, None, None);
                self.compare(&before, &mut moved);
            }
            Change::Declared(i, d) => {
                let before = self.snapshot(Parts { payers: true, ..Parts::NONE });
                match d {
                    Some(d) => self.inputs.facts.declared.insert(i, d),
                    None => self.inputs.facts.declared.remove(&i),
                };
                self.payers = payer_rates(&self.inputs, &self.cash, &self.matched);
                self.compare(&before, &mut moved);
            }
            Change::Frequency(i, s) => {
                let before = self.snapshot(Parts { payers: true, ..Parts::NONE });
                match s {
                    Some(s) => self.inputs.facts.frequencies.insert(i, s),
                    None => self.inputs.facts.frequencies.remove(&i),
                };
                self.payers = payer_rates(&self.inputs, &self.cash, &self.matched);
                self.compare(&before, &mut moved);
            }
            Change::ExDividend(i, d) => {
                let before = self.snapshot(Parts { payers: true, ..Parts::NONE });
                match d {
                    Some(d) => self.inputs.market.ex_dividends.insert(i, d),
                    None => self.inputs.market.ex_dividends.remove(&i),
                };
                self.payers = payer_rates(&self.inputs, &self.cash, &self.matched);
                self.compare(&before, &mut moved);
            }
            Change::Quote(i, q) => {
                let was = match q.clone() {
                    Some(q) => self.inputs.market.quotes.insert(i, q),
                    None => self.inputs.market.quotes.remove(&i),
                };
                quote_moved(i, was.as_ref(), q.as_ref(), &mut moved);
                self.reprice(i, &mut moved);
                // a coin's price decides whether a difference from the broker is dust
                let before_checks = self.checks.clone();
                self.checks = broker_checks(&self.inputs, &self.matched);
                diff(&mut moved, before_checks.iter().map(|c| (Entity::BrokerCheck(c.account), c)), self.checks.iter().map(|c| (Entity::BrokerCheck(c.account), c)));
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
            Change::Benchmark(k, series) => {
                // read only by the scoped returns
                let before = self.snapshot(Parts { benchmarks: true, ..Parts::NONE });
                match series {
                    Some(s) => {
                        self.benchmarks.insert(k.clone(), total_return_cad(&s, &self.inputs.facts.rates, &self.inputs.clock));
                        self.inputs.market.benchmarks.insert(k, s);
                    }
                    None => {
                        self.benchmarks.remove(&k);
                        self.inputs.market.benchmarks.remove(&k);
                    }
                }
                self.compare(&before, &mut moved);
            }
            Change::Broker(a, b) => {
                let before_positions: Vec<PositionFig> = self.positions.iter().filter(|p| p.account == a).cloned().collect();
                let before_equity: Option<AccountEquity> = self.equity.get(&a).cloned();
                let before_check: Vec<BrokerCheck> = self.checks.iter().filter(|c| c.account == a).cloned().collect();
                let before_stated: Option<BrokerAccount> = self.inputs.market.brokers.get(&a).cloned();
                match b {
                    Some(b) => self.inputs.market.brokers.insert(a, b),
                    None => self.inputs.market.brokers.remove(&a),
                };
                for p in self.positions.iter_mut().filter(|p| p.account == a) {
                    p.broker_qty = self.inputs.market.brokers.get(&a).and_then(|b| b.held.get(&p.instrument)).copied();
                }
                self.equity.remove(&a);
                self.equity.extend(build_equity(&self.inputs, Some(&BTreeSet::from([a]))));
                self.checks = broker_checks(&self.inputs, &self.matched);
                diff(&mut moved, before_positions.iter().map(|p| (position_entity(p), p)), self.positions.iter().filter(|p| p.account == a).map(|p| (position_entity(p), p)));
                diff(&mut moved, before_equity.iter().map(|e| (Entity::Equity(a), e)), self.equity.get(&a).map(|e| (Entity::Equity(a), e)));
                diff(&mut moved, before_check.iter().map(|c| (Entity::BrokerCheck(c.account), c)), self.checks.iter().filter(|c| c.account == a).map(|c| (Entity::BrokerCheck(c.account), c)));
                let now_stated = self.inputs.market.brokers.get(&a);
                diff(&mut moved, before_stated.as_ref().map(|b| (Entity::Broker(a), Stated(b))).iter().map(|(e, s)| (e.clone(), s)), now_stated.map(|b| (Entity::Broker(a), Stated(b))).iter().map(|(e, s)| (e.clone(), s)));
            }
        }
        moved
    }

    /// A price moved: that instrument's positions.
    fn reprice(&mut self, instrument: InstrumentId, moved: &mut Moved) {
        let before_positions: Vec<PositionFig> = self.positions.iter().filter(|p| p.instrument == instrument).cloned().collect();
        let fresh = build_positions(&self.inputs, &self.matched, &self.identity, Some(instrument), None);
        self.positions.retain(|p| p.instrument != instrument);
        self.positions.extend(fresh);
        self.positions.sort_by(|a, b| (a.account, a.instrument, a.direction).cmp(&(b.account, b.instrument, b.direction)));
        diff(moved, before_positions.iter().map(|p| (position_entity(p), p)), self.positions.iter().filter(|p| p.instrument == instrument).map(|p| (position_entity(p), p)));
    }

    fn snapshot(&self, parts: Parts) -> Snapshot {
        Snapshot {
            trades: parts.trades.then(|| self.trades.clone()),
            positions: parts.positions.then(|| self.positions.clone()),
            cash: parts.cash.then(|| self.cash.clone()),
            payers: parts.payers.then(|| self.payers.clone()),
            equity: parts.equity.then(|| self.equity.clone()),
            checks: parts.checks.then(|| self.checks.clone()),
            benchmarks: parts.benchmarks.then(|| self.benchmarks.clone()),
            brokers: parts.book.then(|| self.inputs.market.brokers.clone()),
            book: parts.book.then(|| self.book_fig()),
        }
    }

    /// The record as a whole, as `Entity::Book` names it.
    fn book_fig(&self) -> BookFig {
        let l = &self.inputs.ledger;
        BookFig { accounts: l.accounts.clone(), instruments: l.instruments.clone(), waiting: self.matched.waiting.clone(), activity: l.transactions.len(), today: self.inputs.clock.today }
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
        if let Some(b) = &before.benchmarks {
            let (was, now): (Vec<_>, Vec<_>) = (b.iter().map(|(k, l)| (k, BenchmarkLevels(l))).collect(), self.benchmarks.iter().map(|(k, l)| (k, BenchmarkLevels(l))).collect());
            diff(m, was.iter().map(|(k, l)| (Entity::Benchmark((*k).clone()), l)), now.iter().map(|(k, l)| (Entity::Benchmark((*k).clone()), l)));
        }
        if let Some(b) = &before.brokers {
            let (was, now): (Vec<_>, Vec<_>) = (b.iter().map(|(a, s)| (*a, Stated(s))).collect(), self.inputs.market.brokers.iter().map(|(a, s)| (*a, Stated(s))).collect());
            diff(m, was.iter().map(|(a, s)| (Entity::Broker(*a), s)), now.iter().map(|(a, s)| (Entity::Broker(*a), s)));
        }
        if let Some(b) = &before.book {
            let now = self.book_fig();
            diff(m, [(Entity::Book, b)], [(Entity::Book, &now)]);
        }
    }

    /// What differs between this engine's figures and another's, field for
    /// field: empty when an incremental `apply` agrees with a fresh build.
    pub fn differences(&self, other: &Engine) -> Moved {
        let mut m = Moved::default();
        self.compare(&other.snapshot(Parts::ALL), &mut m);
        let (mine, theirs) = (&self.inputs.market.quotes, &other.inputs.market.quotes);
        for i in mine.keys().chain(theirs.keys()).collect::<BTreeSet<_>>() {
            quote_moved(*i, theirs.get(i), mine.get(i), &mut m);
        }
        m
    }
}

/// An instrument's quote as reported moved: `*` when it came or went, else the
/// fields that differ.
fn quote_moved(i: InstrumentId, was: Option<&Quote>, now: Option<&Quote>, moved: &mut Moved) {
    let fields: BTreeSet<&'static str> = match (was, now) {
        (None, None) => BTreeSet::new(),
        (Some(_), None) | (None, Some(_)) => BTreeSet::from(["*"]),
        (Some(a), Some(b)) => {
            // every field named, so a field added to a quote is compared too
            let Quote { price, change, change_pct, at, source } = a;
            [("price", *price != b.price), ("change", *change != b.change), ("changePct", *change_pct != b.change_pct), ("at", *at != b.at), ("source", *source != b.source)]
                .into_iter()
                .filter(|(_, differs)| *differs)
                .map(|(f, _)| f)
                .collect()
        }
    };
    if !fields.is_empty() {
        moved.0.entry(Entity::Quote(i)).or_default().extend(fields);
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
    benchmarks: bool,
    book: bool,
}

impl Parts {
    const ALL: Parts = Parts { trades: true, positions: true, cash: true, payers: true, equity: true, checks: true, benchmarks: true, book: true };
    const NONE: Parts = Parts { trades: false, positions: false, cash: false, payers: false, equity: false, checks: false, benchmarks: false, book: false };
}

/// The figures of the collections a change recomputes, as they were before it.
struct Snapshot {
    trades: Option<Vec<TradeFig>>,
    positions: Option<Vec<PositionFig>>,
    cash: Option<Vec<CashRow>>,
    payers: Option<BTreeMap<InstrumentId, PayerRate>>,
    equity: Option<BTreeMap<AccountId, AccountEquity>>,
    checks: Option<Vec<BrokerCheck>>,
    benchmarks: Option<BTreeMap<String, crate::stat::benchmark::Levels>>,
    brokers: Option<BTreeMap<AccountId, BrokerAccount>>,
    book: Option<BookFig>,
}
