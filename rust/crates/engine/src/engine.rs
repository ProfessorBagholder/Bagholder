//! The engine's entry point (`docs/plans/stage-2-engine.md`, "The entry point"):
//! build everything from the inputs, apply one change recomputing only what it
//! touches, and say which figures moved, by entity and field.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, InstrumentId, Money, TransactionId};

use crate::stat::benchmark::{build_benchmarks, total_return_cad};
use crate::cashflow::{build_cashflow, payer_rates, CashRow, PayerRate};
use crate::equity::{broker_checks, build_equity, AccountEquity, BrokerCheck};
use crate::identity::{identify, Identity};
use crate::input::{BenchmarkSeries, BrokerAccount, Clock, DeclaredRead, Inputs, Ledger, LedgerDelta, Quote, Rates, Sourced};
use crate::families::{node_of, Families, Node};
use crate::gap::Fig;
use crate::ledger::{match_lots, match_over, Direction, Matched, TripKey};
use crate::positions::{build_positions, PositionFig};
use crate::scope::{scope, Filters, Scoped};
use crate::trades::{build_trades, TradeFig, TradeKey};

/// One change to what the engine is given. Every part of `Inputs` is changed by
/// exactly one variant.
#[derive(Clone, Debug)]
pub enum Change {
    /// The record, by what changed: accounts, instruments with their names and
    /// terms, records with their transactions and problems, the transfer links,
    /// the adjustments the person or a source states, the trades the book
    /// assigned or orphaned, groups and the journal.
    Ledger(LedgerDelta),
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
    /// The match split into families (`crate::families`): what a change to the
    /// record is matched again in.
    families: Families,
    /// Each account's cash, by the family whose rows moved it: a family's match
    /// gives its share, and an account's total is its shares added up.
    shares: BTreeMap<AccountId, BTreeMap<Node, BTreeMap<bagholder_core::Currency, Fig<bagholder_core::Dec>>>>,
}

/// Each account's cash a family's match gives, by family.
type ByFamily = BTreeMap<Node, BTreeMap<AccountId, BTreeMap<bagholder_core::Currency, Fig<bagholder_core::Dec>>>>;

/// The match of each family named, put together in the match's order, and each
/// one's share of the accounts' cash.
fn match_families(inputs: &Inputs, families: &Families, names: impl IntoIterator<Item = Node>) -> (Matched, ByFamily) {
    let mut all = Matched::default();
    let mut shares = ByFamily::new();
    for f in names {
        let txs = families.transactions(&inputs.ledger, f);
        if txs.is_empty() {
            continue;
        }
        let mut m = match_over(inputs, txs);
        shares.insert(f, std::mem::take(&mut m.cash));
        all.absorb(m);
    }
    all.in_match_order(&inputs.ledger);
    (all, shares)
}

/// An account's cash in each currency: its families' shares added up, in the
/// families' order; a sum that does not fit, or a share that did not, is that
/// failure.
fn total(shares: &BTreeMap<Node, BTreeMap<bagholder_core::Currency, Fig<bagholder_core::Dec>>>) -> BTreeMap<bagholder_core::Currency, Fig<bagholder_core::Dec>> {
    let mut out: BTreeMap<bagholder_core::Currency, Fig<bagholder_core::Dec>> = BTreeMap::new();
    for share in shares.values() {
        for (c, v) in share {
            let e = out.entry(*c).or_insert(Ok(bagholder_core::Dec::ZERO));
            if let Ok(t) = e {
                *e = match v {
                    Ok(v) => t.checked_add(*v).map_err(crate::gap::Gaps::from),
                    Err(g) => Err(g.clone()),
                };
            }
        }
    }
    out
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
        let families = Families::build(&inputs.ledger, &inputs.facts.adjustments);
        let mut e = Engine {
            inputs,
            matched: Matched::default(),
            identity: Identity::default(),
            trades: vec![],
            positions: vec![],
            cash: vec![],
            payers: BTreeMap::new(),
            equity: BTreeMap::new(),
            checks: vec![],
            benchmarks: BTreeMap::new(),
            past: PastCache::default(),
            families,
            shares: BTreeMap::new(),
        };
        e.match_all();
        e.identity = identify(&e.inputs.ledger, &e.matched);
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
        self.checks = broker_checks(i, &self.matched, None);
    }

    /// Every family matched again (the day turned: any contract's expiry).
    fn match_all(&mut self) {
        let names: Vec<Node> = self.families.all().map(|(n, _)| *n).collect();
        let (matched, by_family) = match_families(&self.inputs, &self.families, names);
        self.matched = matched;
        self.shares.clear();
        for (f, accounts) in by_family {
            for (a, cash) in accounts {
                self.shares.entry(a).or_default().insert(f, cash);
            }
        }
        self.matched.cash = self.shares.iter().map(|(a, s)| (*a, total(s))).collect();
    }

    fn rematch(&mut self) {
        self.match_all();
        self.identity = identify(&self.inputs.ledger, &self.matched);
        self.derive();
    }

    /// The families of `seeds` worked out again and matched again: what a change
    /// to the record reaches, and nothing else.
    fn rematch_families(&mut self, seeds: BTreeSet<Node>) -> (crate::families::Regrouped, BTreeSet<AccountId>, bool) {
        let g = self.families.regroup(&self.inputs.ledger, &self.inputs.facts.adjustments, seeds);
        let (fresh, by_family) = match_families(&self.inputs, &self.families, g.families.iter().copied());
        // what waits on the person in the region, before and after
        let in_region = |m: &Matched, l: &crate::input::Ledger| -> Vec<(bagholder_core::TransactionId, crate::ledger::Waiting)> {
            m.waiting.iter().filter(|(tx, w)| g.region.contains(&Node::Instrument(w.instrument)) || l.transactions.get(tx).is_none_or(|t| g.region.contains(&node_of(t)))).map(|(t, w)| (t.clone(), w.clone())).collect()
        };
        let waiting_was = in_region(&self.matched, &self.inputs.ledger);
        self.matched.replace(&self.inputs.ledger, &g.region, fresh);
        let waiting_moved = waiting_was != in_region(&self.matched, &self.inputs.ledger);
        let mut touched: BTreeSet<AccountId> = BTreeSet::new();
        for (a, s) in self.shares.iter_mut() {
            let before = s.len();
            s.retain(|f, _| !g.gone.contains(f));
            if s.len() != before {
                touched.insert(*a);
            }
        }
        for (f, accounts) in by_family {
            for (a, cash) in accounts {
                self.shares.entry(a).or_default().insert(f, cash);
                touched.insert(a);
            }
        }
        for a in &touched {
            match self.shares.get(a).filter(|s| !s.is_empty()) {
                Some(s) => {
                    self.matched.cash.insert(*a, total(s));
                }
                None => {
                    self.shares.remove(a);
                    self.matched.cash.remove(a);
                }
            }
        }
        (g, touched, waiting_moved)
    }

    /// The holdings at the close of `day`, a day before today.
    pub fn past(&self, day: Date) -> std::sync::Arc<Past> {
        let mut cache = self.past.lock();
        if let Some(p) = cache.get(&day).and_then(std::sync::Weak::upgrade) {
            return p;
        }
        cache.retain(|_, w| w.strong_count() > 0);
        let i = &self.inputs;
        let kept: crate::input::Transactions = i.ledger.transactions.iter().filter(|t| t.trade_date <= day).cloned().collect();
        let ids: BTreeSet<&TransactionId> = kept.iter().map(|t| &t.id).collect();
        let ledger = Ledger {
            accounts: i.ledger.accounts.clone(),
            instruments: i.ledger.instruments.clone(),
            transfer_links: i.ledger.transfer_links.iter().filter(|(o, n)| ids.contains(o) && ids.contains(n)).map(|(o, n)| (o.clone(), n.clone())).collect(),
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

    /// Apply one change, recomputing only what it touches, and say what moved:
    /// only the figures recomputed are compared.
    pub fn apply(&mut self, change: Change) -> Moved {
        let mut moved = Moved::default();
        // the first day whose past holdings this change can move: the record and
        // the trades and notes move every day's; rates and closes the days from
        // the first they changed on
        let stale: Option<Date> = match &change {
            Change::Ledger(_) => Some(Date::MIN),
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
            Change::Ledger(mut delta) => {
                let matching = delta.matches();
                let mut seeds: BTreeSet<Node> = BTreeSet::new();
                let records: Vec<bagholder_core::RecordId> = delta.records.keys().copied().collect();
                let instruments: Vec<InstrumentId> = delta.instruments.keys().copied().collect();
                let accounts: Vec<AccountId> = delta.accounts.keys().copied().collect();
                let gone: Vec<bagholder_core::transaction::Transaction> = records.iter().flat_map(|r| self.inputs.ledger.transactions.of_record(*r).cloned().collect::<Vec<_>>()).collect();
                let was: BTreeMap<InstrumentId, Option<InstrumentId>> =
                    instruments.iter().map(|i| (*i, self.inputs.ledger.instruments.get(i).and_then(|x| x.terms.as_ref()).map(|t| t.underlying))).collect();
                self.seed(&mut seeds, &records, &was, &accounts);
                // what the person keeps on the trades: the instruments they are anchored on, before and after
                let mut reach = Reach::default();
                for t in delta.trades.keys() {
                    reach.instruments.extend(self.inputs.ledger.trades.get(t).and_then(crate::input::Keyed::instrument));
                }
                for (t, now) in &delta.trades {
                    reach.instruments.extend(now.as_ref().and_then(|(_, t)| crate::input::Keyed::instrument(t)));
                    reach.trades.insert(*t);
                }
                for subject in delta.journal.keys() {
                    match subject {
                        bagholder_core::journal::JournalSubject::Trade(t) => {
                            reach.trades.insert(*t);
                            reach.instruments.extend(self.identity.trip_of.get(t).map(|k| k.instrument));
                        }
                        bagholder_core::journal::JournalSubject::Group(_) => {}
                    }
                }
                // the adjustments: the rows they apply to, and every instrument their legs name before and after
                let adjustments = std::mem::take(&mut delta.adjustments);
                let mut adjusted_was: BTreeMap<bagholder_core::TransactionId, Option<crate::input::Adjustment>> = BTreeMap::new();
                for (tx, list) in &adjustments {
                    let old = self.inputs.facts.adjustments.get(tx).cloned();
                    seeds.extend(old.iter().chain(list.iter()).flat_map(|x| x.legs.iter().flat_map(|l| [l.from, l.to])).flatten().map(Node::Instrument));
                    adjusted_was.insert(tx.clone(), old);
                }
                let book_was = BookParts { accounts: accounts.iter().map(|a| (*a, self.inputs.ledger.accounts.get(a).cloned())).collect(), instruments: instruments.iter().map(|i| (*i, self.inputs.ledger.instruments.get(i).cloned())).collect(), activity: self.inputs.ledger.transactions.len() };
                reach.transactions.extend(gone.iter().map(|t| t.id.clone()));
                reach.accounts.extend(gone.iter().map(|t| t.account));
                self.inputs.ledger.apply(delta);
                for (tx, list) in adjustments {
                    if let Some(t) = self.inputs.ledger.transactions.get(&tx) {
                        seeds.insert(node_of(t));
                    }
                    self.inputs.facts.adjustments.replace(tx, list);
                }
                let now: BTreeMap<InstrumentId, Option<InstrumentId>> =
                    instruments.iter().map(|i| (*i, self.inputs.ledger.instruments.get(i).and_then(|x| x.terms.as_ref()).map(|t| t.underlying))).collect();
                self.seed(&mut seeds, &records, &now, &accounts);
                let come: Vec<bagholder_core::transaction::Transaction> = records.iter().flat_map(|r| self.inputs.ledger.transactions.of_record(*r).cloned().collect::<Vec<_>>()).collect();
                reach.transactions.extend(come.iter().map(|t| t.id.clone()));
                reach.accounts.extend(come.iter().map(|t| t.account));
                reach.accounts.extend(accounts.iter().copied());
                self.families.reindex(&self.inputs.ledger, &was, gone.iter(), come.iter(), &adjusted_was, &self.inputs.facts.adjustments);
                let mut waiting_moved = false;
                if matching {
                    let (g, cash_moved, waited) = self.rematch_families(seeds);
                    waiting_moved = waited;
                    reach.accounts.extend(cash_moved);
                    for n in &g.region {
                        if let Node::Instrument(i) = n {
                            reach.instruments.insert(*i);
                        }
                    }
                    reach.matched = true;
                }
                // every instrument of a family reached: a round trip keyed on one holds lots of the others
                let fam: BTreeSet<InstrumentId> = reach.instruments.iter().flat_map(|i| self.families.members(self.families.of(Node::Instrument(*i)))).filter_map(|n| if let Node::Instrument(i) = n { Some(i) } else { None }).collect();
                reach.instruments.extend(fam);
                for i in &reach.instruments {
                    reach.accounts.extend(self.matched.held_by.get(i).into_iter().flatten().copied());
                }
                self.rederive(&reach, &mut moved);
                // the record as a whole
                let mut book = BTreeSet::new();
                if book_was.accounts.iter().any(|(a, was)| self.inputs.ledger.accounts.get(a) != was.as_ref()) {
                    book.insert("accounts");
                }
                if book_was.instruments.iter().any(|(i, was)| self.inputs.ledger.instruments.get(i) != was.as_ref()) {
                    book.insert("instruments");
                }
                if waiting_moved {
                    book.insert("waiting");
                }
                if book_was.activity != self.inputs.ledger.transactions.len() {
                    book.insert("activity");
                }
                if !book.is_empty() {
                    moved.0.entry(Entity::Book).or_default().extend(book);
                }
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
                // a coin's price decides whether a difference from the broker is dust:
                // the checks of the accounts the book or the broker says hold it
                let holding: BTreeSet<AccountId> =
                    self.positions.iter().filter(|p| p.instrument == i).map(|p| p.account).chain(self.inputs.market.brokers.iter().filter(|(_, b)| b.held.contains_key(&i)).map(|(a, _)| *a)).collect();
                for a in holding {
                    self.recheck(a, &mut moved);
                }
            }
            Change::Closes(i, c) => {
                if self.families.is_underlying(i) {
                    // a close can decide whether a contract expired worthless:
                    // the underlying's family, its contracts with it
                    self.inputs.market.closes.insert(i, c);
                    let (g, cash_moved, waited) = self.rematch_families(BTreeSet::from([Node::Instrument(i)]));
                    let mut reach = Reach { accounts: cash_moved, ..Reach::default() };
                    reach.instruments = g.region.iter().filter_map(|n| if let Node::Instrument(i) = n { Some(*i) } else { None }).collect();
                    for x in &reach.instruments {
                        reach.accounts.extend(self.matched.held_by.get(x).into_iter().flatten().copied());
                    }
                    self.rederive(&reach, &mut moved);
                    if waited {
                        moved.0.entry(Entity::Book).or_default().insert("waiting");
                    }
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
                // only this account's check moves: the others' inputs are as they were
                self.recheck(a, &mut moved);
                diff(&mut moved, before_positions.iter().map(|p| (position_entity(p), p)), self.positions.iter().filter(|p| p.account == a).map(|p| (position_entity(p), p)));
                diff(&mut moved, before_equity.iter().map(|e| (Entity::Equity(a), e)), self.equity.get(&a).map(|e| (Entity::Equity(a), e)));
                let now_stated = self.inputs.market.brokers.get(&a);
                diff(&mut moved, before_stated.as_ref().map(|b| (Entity::Broker(a), Stated(b))).iter().map(|(e, s)| (e.clone(), s)), now_stated.map(|b| (Entity::Broker(a), Stated(b))).iter().map(|(e, s)| (e.clone(), s)));
            }
        }
        moved
    }

    /// The nodes a change to `records`, the instruments `under` names (each with
    /// its underlying) and `accounts` reaches in the ledger as it stands: their
    /// rows', their links' other sides', their own.
    fn seed(&self, seeds: &mut BTreeSet<Node>, records: &[bagholder_core::RecordId], under: &BTreeMap<InstrumentId, Option<InstrumentId>>, accounts: &[AccountId]) {
        let l = &self.inputs.ledger;
        for r in records {
            for t in l.transactions.of_record(*r) {
                seeds.insert(node_of(t));
                for other in [l.transfer_links.received_by(&t.id), l.transfer_links.sent_by(&t.id)].into_iter().flatten() {
                    if let Some(o) = l.transactions.get(other) {
                        seeds.insert(node_of(o));
                    }
                }
            }
        }
        for (i, u) in under {
            seeds.insert(Node::Instrument(*i));
            if let Some(u) = u {
                seeds.insert(Node::Instrument(*u));
            }
        }
        for a in accounts {
            seeds.insert(Node::Cash(*a));
            seeds.extend(l.transactions.instruments_in(*a).map(Node::Instrument));
        }
    }

    /// The figures a change to the record reached, worked out again in place, and
    /// what moved said (`docs/plans/stage-3-the-record-by-change.md`, B): the
    /// trade identity, round trips and positions of `reach.instruments`, the
    /// saved groups (their claims are settled over every group, in the book's
    /// order), the cashflow rows of `reach.transactions` and of the stock
    /// dividends of those instruments, the payers' rates of what those rows and
    /// instruments are, and the checks of `reach.accounts`.
    fn rederive(&mut self, reach: &Reach, moved: &mut Moved) {
        let instruments = &reach.instruments;
        let fresh = crate::identity::identify_in(&self.inputs.ledger, &self.matched, instruments);
        self.identity.replace(&self.inputs.ledger, instruments, fresh);
        // the groups, every one: which round trips each holds turns on the earlier ones
        let held_was: BTreeSet<TripKey> = self.trades.iter().filter_map(|t| if let TradeKey::Group(_) = t.key { Some(t.trips.clone()) } else { None }).flatten().collect();
        let (groups, held) = crate::trades::groups_shown(&self.inputs, &self.matched, &self.identity);
        let mut trips: BTreeSet<TripKey> = instruments.iter().flat_map(|i| self.matched.trips_of.get(i).into_iter().flatten().cloned()).collect();
        trips.extend(held_was.symmetric_difference(&held).cloned());
        let rebuilt = |k: &TradeKey| match k {
            TradeKey::Group(_) => true,
            TradeKey::Trip(t) => instruments.contains(&t.instrument) || trips.contains(t),
        };
        let mut was: Vec<TradeFig> = Vec::new();
        self.trades.retain(|t| {
            if rebuilt(&t.key) {
                was.push(t.clone());
                false
            } else {
                true
            }
        });
        let mut now: Vec<TradeFig> = groups;
        for k in &trips {
            if held.contains(k) {
                continue;
            }
            now.extend(crate::trades::trip_fig(&self.inputs, &self.matched, &self.identity, k));
        }
        for t in &now {
            let at = self.trades.binary_search_by(|x| crate::trades::shown_order(x, t)).unwrap_or_else(|e| e);
            self.trades.insert(at, t.clone());
        }
        diff(moved, was.iter().map(|t| (trade_entity(t), t)), now.iter().map(|t| (trade_entity(t), t)));
        // positions
        let mut was: Vec<PositionFig> = Vec::new();
        self.positions.retain(|p| {
            if instruments.contains(&p.instrument) {
                was.push(p.clone());
                false
            } else {
                true
            }
        });
        let now: Vec<PositionFig> = instruments.iter().flat_map(|i| build_positions(&self.inputs, &self.matched, &self.identity, Some(*i), None)).collect();
        let order = |p: &PositionFig| (p.account, p.instrument, p.direction);
        for p in &now {
            let at = self.positions.binary_search_by(|x| order(x).cmp(&order(p))).unwrap_or_else(|e| e);
            self.positions.insert(at, p.clone());
        }
        diff(moved, was.iter().map(|p| (position_entity(p), p)), now.iter().map(|p| (position_entity(p), p)));
        // cashflow rows
        let mut was: Vec<CashRow> = Vec::new();
        self.cash.retain(|r| {
            let gone = reach.transactions.contains(&r.id) || (r.in_units.is_some() && r.instrument.is_some_and(|i| instruments.contains(&i)));
            if gone {
                was.push(r.clone());
            }
            !gone
        });
        let mut now: Vec<CashRow> = reach.transactions.iter().filter_map(|id| self.inputs.ledger.transactions.get(id)).filter_map(|t| crate::cashflow::cash_row(&self.inputs, t)).collect();
        now.extend(self.matched.stock_dividends.iter().filter(|s| instruments.contains(&s.instrument)).map(|s| crate::cashflow::stock_dividend_row(&self.inputs, s)));
        for r in &now {
            let at = self.cash.binary_search_by(|x| crate::cashflow::shown_order(x, r)).unwrap_or_else(|e| e);
            self.cash.insert(at, r.clone());
        }
        let payers: BTreeSet<InstrumentId> = was.iter().chain(now.iter()).filter_map(|r| r.instrument).chain(instruments.iter().copied()).collect();
        diff(moved, was.iter().map(|r| (Entity::CashRow(r.id.clone()), r)), now.iter().map(|r| (Entity::CashRow(r.id.clone()), r)));
        // the payers' rates
        for i in payers {
            let payments: Vec<&CashRow> = self.cash.iter().filter(|r| r.instrument == Some(i) && r.kind == crate::cashflow::Payment::Dividend && r.in_units.is_none()).collect();
            let rate = crate::cashflow::payer_rate(&self.inputs, i, payments, &self.matched);
            let before = self.payers.get(&i).cloned();
            match rate.clone() {
                Some(r) => self.payers.insert(i, r),
                None => self.payers.remove(&i),
            };
            diff(moved, before.iter().map(|p| (Entity::Payer(i), p)), rate.iter().map(|p| (Entity::Payer(i), p)));
        }
        // the checks with the brokers
        for a in &reach.accounts {
            self.recheck(*a, moved);
        }
    }

    /// A price moved: that instrument's positions.
    /// One account's check with its broker computed again, kept in its place
    /// among the others (by account), and what moved said.
    fn recheck(&mut self, account: AccountId, moved: &mut Moved) {
        let at = self.checks.binary_search_by_key(&account, |c| c.account);
        let before = at.ok().map(|n| self.checks[n].clone());
        let now = broker_checks(&self.inputs, &self.matched, Some(account)).pop();
        match (at, now.clone()) {
            (Ok(n), Some(c)) => self.checks[n] = c,
            (Ok(n), None) => {
                self.checks.remove(n);
            }
            (Err(n), Some(c)) => self.checks.insert(n, c),
            (Err(_), None) => {}
        }
        diff(moved, before.iter().map(|c| (Entity::BrokerCheck(c.account), c)), now.iter().map(|c| (Entity::BrokerCheck(c.account), c)));
    }

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

    /// Where this engine and another disagree, beyond the figures `differences`
    /// compares: the match whole (every lot, round trip and what waits) and the
    /// trades' identity. None when they agree.
    pub fn match_disagrees(&self, other: &Engine) -> Option<String> {
        if self.matched != other.matched {
            let m = (&self.matched, &other.matched);
            let parts = [
                ("trips", m.0.trips != m.1.trips),
                ("books", m.0.books != m.1.books),
                ("beyond", m.0.beyond != m.1.beyond),
                ("dust", m.0.dust != m.1.dust),
                ("arrived", m.0.arrived != m.1.arrived),
                ("last_price", m.0.last_price != m.1.last_price),
                ("stock_dividends", m.0.stock_dividends != m.1.stock_dividends),
                ("disagreements", m.0.disagreements != m.1.disagreements),
                ("unit_disagreements", m.0.unit_disagreements != m.1.unit_disagreements),
                ("unapplied", m.0.unapplied != m.1.unapplied),
                ("units", m.0.units != m.1.units),
                ("roles", m.0.roles != m.1.roles),
                ("waiting", m.0.waiting != m.1.waiting),
                ("cash", m.0.cash != m.1.cash),
            ];
            return Some(format!("the match differs in {:?}", parts.iter().filter(|(_, d)| *d).map(|(n, _)| *n).collect::<Vec<_>>()));
        }
        if self.identity != other.identity {
            return Some(format!("the trades' identity differs: {:?} against {:?}", self.identity, other.identity));
        }
        None
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

/// What a change to the record reached (`Engine::rederive`).
#[derive(Default)]
struct Reach {
    instruments: BTreeSet<InstrumentId>,
    transactions: BTreeSet<bagholder_core::TransactionId>,
    accounts: BTreeSet<AccountId>,
    trades: BTreeSet<bagholder_core::TradeId>,
    matched: bool,
}

/// What the record as a whole was, for the parts a change names.
struct BookParts {
    accounts: BTreeMap<AccountId, Option<crate::input::AccountInfo>>,
    instruments: BTreeMap<InstrumentId, Option<crate::input::InstrumentInfo>>,
    activity: usize,
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
