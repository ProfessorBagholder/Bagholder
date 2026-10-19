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
use crate::ledger::{match_family, match_lots, Direction, MatchState, Matched, TripKey};
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

/// A cash dividend of an instrument: a row its payer's rate is worked out from.
fn pays_cash(r: &CashRow) -> bool {
    r.instrument.is_some() && r.kind == crate::cashflow::Payment::Dividend && r.in_units.is_none()
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
    /// Each instrument's cash dividends among `cash`, in its order: what its
    /// payer's rate is worked out from, read by key rather than by scanning
    /// every cash row.
    paid: BTreeMap<InstrumentId, Vec<CashRow>>,
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
    /// Each instrument by every symbol it has been called (in ASCII capitals) and
    /// its currency: what a trade typed in by symbol is found by, on its day or now.
    symbols: BTreeMap<(String, bagholder_core::Currency), BTreeSet<InstrumentId>>,
    /// Each listed instrument by every listing it has been (`ListingKey`) and its
    /// currency: what a trade typed in by symbol and exchange is found by.
    listings: BTreeMap<(bagholder_core::ListingKey, bagholder_core::Currency), BTreeSet<InstrumentId>>,
    /// Each family's match as it stood after its last walk: what a change that
    /// only adds rows after it continues from (`ledger::continue_family`).
    kept: BTreeMap<Node, MatchState>,
    /// The round trips the saved groups hold.
    group_held: BTreeSet<TripKey>,
    /// Each family matched, and how, over the engine's life: what a test reads,
    /// on whichever thread the change ran.
    #[cfg(feature = "oracles")]
    paths: Vec<(Node, Path)>,
}

/// How a change matched a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// Every row of it, from its first.
    Whole,
    /// From its kept state, for rows after everything it matched.
    Continued,
    /// A round trip placed inside one of its flat stretches
    /// (`ledger::place_in_stretch`).
    Placed,
}

#[cfg(feature = "oracles")]
thread_local! {
    /// Each family a change to the record matched, and how: what a test
    /// compares with the path its own steps expect.
    static PATHS: std::cell::RefCell<Vec<(Node, Path)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The families matched since last asked on this thread, and how.
#[cfg(feature = "oracles")]
pub fn take_paths() -> Vec<(Node, Path)> {
    PATHS.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

#[cfg(feature = "oracles")]
fn path(log: &mut Vec<(Node, Path)>, f: Node, how: Path) {
    log.push((f, how));
    PATHS.with(|p| p.borrow_mut().push((f, how)));
}

/// The instruments a name finds on a day (`Engine::named_on`, `listed_on`), and
/// whether they bore it on that day; otherwise each first bore it after the day,
/// its history beginning then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalledOn {
    pub ids: Vec<InstrumentId>,
    pub on_the_day: bool,
}

/// What an instrument is found by when named under `n`: the symbol in ASCII
/// capitals, and its currency.
fn symbol_key(i: &crate::input::InstrumentInfo, n: &bagholder_core::instrument::Name) -> (String, bagholder_core::Currency) {
    (n.symbol.to_ascii_uppercase(), i.instrument.currency)
}

/// The listing `n` makes an instrument, where it names one, and its currency.
fn listing_key(i: &crate::input::InstrumentInfo, n: &bagholder_core::instrument::Name) -> Option<(bagholder_core::ListingKey, bagholder_core::Currency)> {
    bagholder_core::ListingKey::of(i.instrument.kind, n).ok().map(|k| (k, i.instrument.currency))
}

/// Every symbol an instrument has been called, each once.
fn symbol_keys(i: &crate::input::InstrumentInfo) -> BTreeSet<(String, bagholder_core::Currency)> {
    i.names.iter().map(|n| symbol_key(i, n)).collect()
}

/// Every listing an instrument has been, each once.
fn listing_keys(i: &crate::input::InstrumentInfo) -> BTreeSet<(bagholder_core::ListingKey, bagholder_core::Currency)> {
    i.names.iter().filter_map(|n| listing_key(i, n)).collect()
}

/// Each account's cash a family's match gives, by family.
type ByFamily = BTreeMap<Node, BTreeMap<AccountId, BTreeMap<bagholder_core::Currency, Fig<bagholder_core::Dec>>>>;

/// The match of each family named, put together in the match's order, each
/// one's share of the accounts' cash, and what each keeps to be continued from.
fn match_families(inputs: &Inputs, families: &Families, names: impl IntoIterator<Item = Node>) -> (Matched, ByFamily, BTreeMap<Node, MatchState>) {
    let mut all = Matched::default();
    let mut shares = ByFamily::new();
    let mut kept = BTreeMap::new();
    for f in names {
        let txs = families.transactions(&inputs.ledger, f);
        if txs.is_empty() {
            continue;
        }
        let (mut m, state) = match_family(inputs, txs);
        shares.insert(f, std::mem::take(&mut m.cash));
        kept.insert(f, state);
        all.absorb(m);
    }
    all.in_match_order(&inputs.ledger);
    (all, shares, kept)
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

/// The first day whose past holdings a change to the record can move (#527): a
/// past day is worked out from the rows dated on or before it (`Engine::past`),
/// so a change made of rows alone moves the days from the earliest date any row
/// it removes, adds or links states (its trade date, its settlement date); a
/// change to anything else every past day reads (an account, an instrument, a
/// trade, a group, the journal, an adjustment) moves them all.
fn first_day_touched(ledger: &Ledger, delta: &crate::input::LedgerDelta) -> Date {
    if !delta.accounts.is_empty() || !delta.instruments.is_empty() || !delta.trades.is_empty() || !delta.groups.is_empty() || !delta.journal.is_empty() || !delta.adjustments.is_empty() {
        return Date::MIN;
    }
    let days = |t: &bagholder_core::transaction::Transaction| [Some(t.trade_date), t.settle_date];
    let gone = delta.records.keys().flat_map(|r| ledger.transactions.of_record(*r));
    let come = delta.records.values().flatten().flat_map(|(_, txs)| txs.iter());
    let linked = delta.links.iter().flat_map(|(a, b)| [a, b]).filter_map(|id| ledger.transactions.get(id));
    gone.chain(come).chain(linked).flat_map(days).flatten().min().unwrap_or(Date::MAX)
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
            paid: BTreeMap::new(),
            payers: BTreeMap::new(),
            equity: BTreeMap::new(),
            checks: vec![],
            benchmarks: BTreeMap::new(),
            past: PastCache::default(),
            families,
            shares: BTreeMap::new(),
            symbols: BTreeMap::new(),
            listings: BTreeMap::new(),
            kept: BTreeMap::new(),
            group_held: BTreeSet::new(),
            #[cfg(feature = "oracles")]
            paths: Vec::new(),
        };
        for (id, info) in &e.inputs.ledger.instruments {
            for k in symbol_keys(info) {
                e.symbols.entry(k).or_default().insert(*id);
            }
            for k in listing_keys(info) {
                e.listings.entry(k).or_default().insert(*id);
            }
        }
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
        self.group_held = self.trades.iter().filter_map(|t| if let TradeKey::Group(_) = t.key { Some(t.trips.clone()) } else { None }).flatten().collect();
        self.positions = build_positions(i, &self.matched, &self.identity, None, None);
        self.cash = build_cashflow(i, &self.matched);
        self.paid = BTreeMap::new();
        for r in self.cash.iter().filter(|r| pays_cash(r)) {
            self.paid.entry(r.instrument.expect("a dividend names its payer")).or_default().push(r.clone());
        }
        self.payers = payer_rates(i, &self.cash, &self.matched);
        self.equity = build_equity(i, None);
        self.checks = broker_checks(i, &self.matched, None);
    }

    /// Every family matched again (the day turned: any contract's expiry).
    fn match_all(&mut self) {
        let names: Vec<Node> = self.families.all().map(|(n, _)| *n).collect();
        let (matched, by_family, kept) = match_families(&self.inputs, &self.families, names);
        #[cfg(feature = "oracles")]
        for f in kept.keys() {
            path(&mut self.paths, *f, Path::Whole);
        }
        self.matched = matched;
        self.kept = kept;
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
        let (fresh, by_family, kept) = match_families(&self.inputs, &self.families, g.families.iter().copied());
        // the families that went keep nothing; the ones that took their place keep their own
        for f in &g.gone {
            self.kept.remove(f);
        }
        #[cfg(feature = "oracles")]
        for f in &g.families {
            path(&mut self.paths, *f, Path::Whole);
        }
        self.kept.extend(kept);
        // what waits on the person in the region, before and after
        let in_region = |m: &Matched, l: &crate::input::Ledger| -> Vec<(bagholder_core::TransactionId, crate::ledger::Waiting)> {
            m.waiting.iter().filter(|(tx, w)| g.region.contains(&Node::Instrument(w.instrument)) || l.transactions.get(tx).is_none_or(|t| g.region.contains(&node_of(t)))).map(|(t, w)| (t.clone(), w.clone())).collect()
        };
        let waiting_was = in_region(&self.matched, &self.inputs.ledger);
        // every account holding an instrument of the region before the change: one
        // that holds none of it after is rechecked too
        let mut touched: BTreeSet<AccountId> = g.region.iter().filter_map(|n| if let Node::Instrument(i) = n { Some(i) } else { None }).flat_map(|i| self.matched.held_by.get(i).into_iter().flatten().copied()).collect();
        self.matched.replace(&self.inputs.ledger, &g.region, fresh);
        let waiting_moved = waiting_was != in_region(&self.matched, &self.inputs.ledger);
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

    /// Each family matched over the engine's life, and whether its match was
    /// continued.
    #[cfg(feature = "oracles")]
    pub fn paths(&self) -> &[(Node, Path)] {
        &self.paths
    }

    /// The family a node is matched in (`crate::families`), by name.
    pub fn family_of(&self, n: Node) -> Node {
        self.families.of(n)
    }

    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// The instruments called `symbol` now (any case of its ASCII letters) in
    /// `currency`.
    pub fn named(&self, symbol: &str, currency: bagholder_core::Currency) -> impl Iterator<Item = InstrumentId> + '_ {
        let key = (symbol.to_ascii_uppercase(), currency);
        self.symbols.get(&key).into_iter().flatten().copied().filter(move |id| self.inputs.ledger.instruments.get(id).and_then(|i| i.current_name()).is_some_and(|n| n.symbol.to_ascii_uppercase() == key.0))
    }

    /// The instruments of a kind `of_kind` takes called `symbol` on `day`, in
    /// `currency` (`called_on`): the kind is part of the match, as a broker names a
    /// contract by its underlying's symbol.
    pub fn named_on(&self, symbol: &str, currency: bagholder_core::Currency, day: Date, of_kind: impl Fn(bagholder_core::instrument::InstrumentKind) -> bool) -> CalledOn {
        let symbol = symbol.to_ascii_uppercase();
        let instruments = &self.inputs.ledger.instruments;
        let ids = self.symbols.get(&(symbol.clone(), currency)).into_iter().flatten().copied().filter(|id| instruments.get(id).is_some_and(|i| of_kind(i.instrument.kind)));
        self.called_on(ids, day, |_, n| n.symbol.to_ascii_uppercase() == symbol)
    }

    /// Of `ids`, those `is` takes by the name each had on `day`: the name in force
    /// that day where the instrument's history had begun by then. An instrument whose
    /// history begins after `day` is taken by its first name only where none of
    /// `ids` had the name on that day: a file older than the broker's history still
    /// finds it, and a day another instrument held the name is never given to it.
    fn called_on(&self, ids: impl Iterator<Item = InstrumentId>, day: Date, is: impl Fn(&crate::input::InstrumentInfo, &bagholder_core::instrument::Name) -> bool) -> CalledOn {
        let infos: Vec<(InstrumentId, &crate::input::InstrumentInfo)> = ids.filter_map(|id| self.inputs.ledger.instruments.get(&id).map(|i| (id, i))).collect();
        let then: Vec<InstrumentId> = infos.iter().filter(|(_, i)| i.names.iter().rev().find(|n| n.first_seen <= day).is_some_and(|n| is(i, n))).map(|(id, _)| *id).collect();
        if !then.is_empty() {
            return CalledOn { ids: then, on_the_day: true };
        }
        let ids = infos.iter().filter(|(_, i)| i.names.first().is_some_and(|n| n.first_seen > day && is(i, n))).map(|(id, _)| *id).collect();
        CalledOn { ids, on_the_day: false }
    }

    /// The instruments that are the listing `key` now, in `currency`.
    pub fn listed(&self, key: &bagholder_core::ListingKey, currency: bagholder_core::Currency) -> impl Iterator<Item = InstrumentId> + '_ {
        let whole = (key.clone(), currency);
        self.listings.get(&whole).into_iter().flatten().copied().filter(move |id| {
            self.inputs.ledger.instruments.get(id).is_some_and(|i| i.current_name().and_then(|n| listing_key(i, n)).is_some_and(|k| k == whole))
        })
    }

    /// The instruments that were the listing `key` on `day`, in `currency` (`called_on`).
    pub fn listed_on(&self, key: &bagholder_core::ListingKey, currency: bagholder_core::Currency, day: Date) -> CalledOn {
        let whole = (key.clone(), currency);
        let ids = self.listings.get(&whole).into_iter().flatten().copied();
        self.called_on(ids, day, |i, n| listing_key(i, n).is_some_and(|k| k == whole))
    }

    /// The contracts written on `underlying`.
    pub fn contracts_on(&self, underlying: InstrumentId) -> impl Iterator<Item = InstrumentId> + '_ {
        self.families.contracts_on(underlying)
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
            Change::Ledger(delta) => Some(first_day_touched(&self.inputs.ledger, delta)),
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
                // the families whose match the change continues, with the rows it adds
                let mut continued = if matching { self.continuable(&delta) } else { BTreeMap::new() };
                let continued_nodes: BTreeSet<Node> = continued.keys().flat_map(|f| self.families.members(*f)).collect();
                let mut seeds: BTreeSet<Node> = BTreeSet::new();
                let records: Vec<bagholder_core::RecordId> = delta.records.keys().copied().collect();
                let instruments: Vec<InstrumentId> = delta.instruments.keys().copied().collect();
                let accounts: Vec<AccountId> = delta.accounts.keys().copied().collect();
                let gone: Vec<bagholder_core::transaction::Transaction> = records.iter().flat_map(|r| self.inputs.ledger.transactions.of_record(*r).cloned().collect::<Vec<_>>()).collect();
                let was: BTreeMap<InstrumentId, Option<InstrumentId>> =
                    instruments.iter().map(|i| (*i, self.inputs.ledger.instruments.get(i).and_then(|x| x.terms.as_ref()).map(|t| t.underlying))).collect();
                self.seed(&mut seeds, &records, &was, &accounts);
                // what the person keeps on the trades: the round trips they are on, before and after
                let mut reach = Reach::default();
                for (t, now) in &delta.trades {
                    reach.trips.extend(self.identity.trip_of.get(t).cloned());
                    // the transactions anchored on, before and after: every trade on one
                    // competes with this one for its round trip
                    for trade in self.inputs.ledger.trades.get(t).into_iter().chain(now.as_ref().map(|(_, x)| x)) {
                        reach.trips.extend(self.trips_anchored(trade));
                        reach.anchors.extend(crate::input::Keyed::anchor(trade));
                    }
                    reach.trades.insert(*t);
                }
                reach.groups = !delta.groups.is_empty();
                for subject in delta.journal.keys() {
                    match subject {
                        bagholder_core::journal::JournalSubject::Trade(t) => {
                            reach.trades.insert(*t);
                            reach.trips.extend(self.identity.trip_of.get(t).cloned());
                        }
                        // a group's figure carries its journal: the groups are worked out again (brief 30, C2)
                        bagholder_core::journal::JournalSubject::Group(_) => reach.groups = true,
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
                for i in &instruments {
                    for k in self.inputs.ledger.instruments.get(i).map(symbol_keys).unwrap_or_default() {
                        if let Some(s) = self.symbols.get_mut(&k) {
                            s.remove(i);
                            if s.is_empty() {
                                self.symbols.remove(&k);
                            }
                        }
                    }
                    for k in self.inputs.ledger.instruments.get(i).map(listing_keys).unwrap_or_default() {
                        if let Some(s) = self.listings.get_mut(&k) {
                            s.remove(i);
                            if s.is_empty() {
                                self.listings.remove(&k);
                            }
                        }
                    }
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
                for i in &instruments {
                    for k in self.inputs.ledger.instruments.get(i).map(symbol_keys).unwrap_or_default() {
                        self.symbols.entry(k).or_default().insert(*i);
                    }
                    for k in self.inputs.ledger.instruments.get(i).map(listing_keys).unwrap_or_default() {
                        self.listings.entry(k).or_default().insert(*i);
                    }
                }
                let come: Vec<bagholder_core::transaction::Transaction> = records.iter().flat_map(|r| self.inputs.ledger.transactions.of_record(*r).cloned().collect::<Vec<_>>()).collect();
                reach.transactions.extend(come.iter().map(|t| t.id.clone()));
                reach.accounts.extend(come.iter().map(|t| t.account));
                reach.accounts.extend(accounts.iter().copied());
                self.families.reindex(&self.inputs.ledger, &was, gone.iter(), come.iter(), &adjusted_was, &self.inputs.facts.adjustments);
                let mut waiting_moved = false;
                if matching {
                    // a continued family's own rows are walked on from its state; what
                    // is left, and every node of a family the change reaches otherwise,
                    // is matched whole, as is a continued family the whole match takes in
                    seeds.retain(|n| !continued_nodes.contains(n) && (self.families.has(*n) || self.holds_node(*n)));
                    if !seeds.is_empty() {
                        let (g, cash_moved, waited) = self.rematch_families(seeds);
                        waiting_moved = waited;
                        reach.accounts.extend(cash_moved);
                        for n in &g.region {
                            if let Node::Instrument(i) = n {
                                reach.instruments.insert(*i);
                            }
                        }
                        continued.retain(|f, _| !g.gone.contains(f));
                    }
                    for (f, (rows, stretch)) in continued {
                        let (c, waited) = match stretch {
                            None => self.continue_family(f, &rows),
                            Some(start) => match self.place_family(f, &rows, &start) {
                                Some(placed) => placed,
                                // the rows did not leave the family as they found it: matched whole
                                None => {
                                    let (g, cash_moved, waited) = self.rematch_families(self.families.members(f));
                                    reach.accounts.extend(cash_moved);
                                    reach.instruments.extend(g.region.iter().filter_map(|n| if let Node::Instrument(i) = n { Some(*i) } else { None }));
                                    waiting_moved |= waited;
                                    continue;
                                }
                            },
                        };
                        waiting_moved |= waited;
                        reach.accounts.extend(c.cash.keys().copied());
                        reach.accounts.extend(c.holdings.iter().map(|(a, _)| *a));
                        reach.holdings.extend(c.holdings);
                        reach.trips.extend(c.trips);
                    }
                    reach.matched = true;
                }
                // a trade's round trip after the change: the one holding its anchor
                for t in &reach.trades {
                    if let Some(trade) = self.inputs.ledger.trades.get(t) {
                        reach.trips.extend(self.trips_anchored(trade));
                    }
                }
                // every instrument of a family reached whole: a round trip keyed on one holds lots of the others
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
                let live_before = self.live_rates_stated();
                self.inputs.clock = c;
                self.rematch();
                self.benchmarks = build_benchmarks(&self.inputs.market.benchmarks, &self.inputs.facts.rates, &self.inputs.clock);
                self.compare(&before, &mut moved);
                self.live_rates_moved(&live_before, &mut moved);
            }
            Change::Rates(r) => {
                let before = self.snapshot(Parts::ALL);
                let live_before = self.live_rates_stated();
                self.inputs.facts.rates = r;
                // a coin's remainder is judged dust at the rate of its row's day: every
                // family with a coin and a row from the first day that changed is
                // matched again, its kept state with it (#513)
                if let Some(day) = stale {
                    let coin = |n: &Node| matches!(n, Node::Instrument(i) if self.inputs.ledger.instruments.get(i).is_some_and(|x| x.instrument.kind == bagholder_core::instrument::InstrumentKind::Crypto));
                    let seeds: BTreeSet<Node> = self
                        .kept
                        .iter()
                        .filter(|(_, k)| k.last.as_ref().is_some_and(|l| l.0 >= day))
                        .map(|(f, _)| self.families.members(*f))
                        .filter(|m| m.iter().any(coin))
                        .flatten()
                        .collect();
                    if !seeds.is_empty() {
                        self.rematch_families(seeds);
                        self.identity = identify(&self.inputs.ledger, &self.matched);
                    }
                }
                self.derive();
                self.benchmarks = build_benchmarks(&self.inputs.market.benchmarks, &self.inputs.facts.rates, &self.inputs.clock);
                self.compare(&before, &mut moved);
                self.live_rates_moved(&live_before, &mut moved);
            }
            Change::Declared(i, d) => {
                match d {
                    Some(d) => self.inputs.facts.declared.insert(i, d),
                    None => self.inputs.facts.declared.remove(&i),
                };
                // that payer's rate, from its own payments
                self.payer_again(i, &mut moved);
            }
            Change::Frequency(i, s) => {
                match s {
                    Some(s) => self.inputs.facts.frequencies.insert(i, s),
                    None => self.inputs.facts.frequencies.remove(&i),
                };
                // that payer's rate, from its own payments
                self.payer_again(i, &mut moved);
            }
            Change::ExDividend(i, d) => {
                match d {
                    Some(d) => self.inputs.market.ex_dividends.insert(i, d),
                    None => self.inputs.market.ex_dividends.remove(&i),
                };
                // that payer's rate, from its own payments
                self.payer_again(i, &mut moved);
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

    /// Whether the ledger holds rows of a node.
    fn holds_node(&self, n: Node) -> bool {
        let l = &self.inputs.ledger;
        match n {
            Node::Instrument(i) => l.instruments.contains_key(&i) || l.transactions.of_instrument(i).next().is_some(),
            Node::Cash(a) => l.transactions.cash_of(a).next().is_some(),
        }
    }

    /// The round trips a trade is on: the one its anchor opens, and each holding
    /// its anchor as a fill of its instrument.
    fn trips_anchored(&self, trade: &bagholder_core::journal::Trade) -> BTreeSet<TripKey> {
        let bagholder_core::journal::Anchor::Opening(o) = &trade.anchor else { return BTreeSet::new() };
        let m = &self.matched;
        let mut out: BTreeSet<TripKey> = m.trips_by_fill.get(&o.transaction).into_iter().flatten().filter(|k| m.trips.get(k).is_some_and(|t| t.instruments.contains(&o.instrument))).cloned().collect();
        let own = TripKey { opening: o.transaction.clone(), instrument: o.instrument };
        if m.trips.contains_key(&own) {
            out.insert(own);
        }
        out
    }

    /// The families whose match a change continues from their kept state, each
    /// with the rows it adds (`docs/plans/stage-3-a-change-on-a-deep-history.md`,
    /// B): only records added, every leg placed after everything the family
    /// matched, no family joined or split, no row that decides an earlier one
    /// (an expiry, an assignment, an exercise, an event, a transfer linked to a
    /// row walked, a coin's finer place), nothing the match reads of an
    /// instrument or an account changed, and the family's last expiry up to today
    /// quiet. Every other family the change reaches is matched whole. Decided
    /// from the change alone: no row of a family is read to decide it.
    /// The families a change continues, each with the rows it adds and, where
    /// they are a round trip placed inside one of its flat stretches, the
    /// stretch's start (`docs/plans/matching-from-a-flat-point.md`, B).
    fn continuable(&self, delta: &LedgerDelta) -> BTreeMap<Node, (Vec<TransactionId>, Option<Option<crate::ledger::OrderKey>>)> {
        use bagholder_core::transaction::{Effect, Kind};
        let l = &self.inputs.ledger;
        let fams = &self.families;
        let family = |n: Node| -> Option<Node> { fams.has(n).then(|| fams.of(n)) };
        let mut blocked: BTreeSet<Node> = BTreeSet::new();
        let mut added: BTreeMap<Node, Vec<TransactionId>> = BTreeMap::new();
        // the families whose rows are placed inside a flat stretch, by its start
        let mut placed: BTreeMap<Node, BTreeSet<Option<crate::ledger::OrderKey>>> = BTreeMap::new();
        let mut went_on: BTreeSet<Node> = BTreeSet::new();
        let is_new = |r: &bagholder_core::RecordId| !l.records.contains_key(r) && l.transactions.of_record(*r).next().is_none();
        let new_row = |id: &TransactionId| -> Option<&bagholder_core::transaction::Transaction> {
            if !is_new(&id.record) {
                return None;
            }
            delta.records.get(&id.record).and_then(|x| x.as_ref()).and_then(|(_, ts)| ts.iter().find(|t| t.id == *id))
        };
        for (r, now) in &delta.records {
            if !is_new(r) || now.is_none() {
                // 1: a record revised or removed
                blocked.extend(l.transactions.of_record(*r).filter_map(|t| family(node_of(t))));
                if let Some((_, txs)) = now {
                    blocked.extend(txs.iter().filter_map(|t| family(node_of(t))));
                }
                continue;
            }
            let Some((info, txs)) = now else { continue };
            let legs: Vec<&bagholder_core::transaction::Transaction> = txs.iter().collect();
            // the walk applies a record whose legs share a day at its earliest leg,
            // whichever family that leg is in: every family it touches must have
            // walked nothing after that place (brief 30, B1)
            let earliest = legs.iter().map(|t| crate::ledger::order_key_of(t, &legs, &info.source_key)).min();
            let unstated = info.problems.iter().any(|p| p.code == "leg-unstated");
            for t in txs {
                // a node no family holds yet is matched whole, as a family of its own
                let Some(fam) = family(node_of(t)) else { continue };
                #[derive(PartialEq)]
                enum Way {
                    Continue,
                    Place(Option<crate::ledger::OrderKey>),
                    Whole,
                }
                let way = (|| {
                    use Way::Whole;
                    // 3: a record whose legs are not stated can make its account one family
                    if unstated {
                        return Whole;
                    }
                    // 4: a row that decides how an earlier one ends, or an event
                    if matches!(t.kind, Kind::OptionExpiry | Kind::OptionAssignment | Kind::OptionExercise | Kind::CorporateEvent) {
                        return Whole;
                    }
                    // 3 and 4: a transfer's other side is added too, in this family
                    for (out, inn) in &delta.links {
                        let other = if *out == t.id {
                            inn
                        } else if *inn == t.id {
                            out
                        } else {
                            continue;
                        };
                        if new_row(other).map(node_of).and_then(family) != Some(fam) {
                            return Whole;
                        }
                    }
                    // 3: an account made one family holds it in that family already
                    if fams.is_whole(t.account) && family(Node::Cash(t.account)) != Some(fam) {
                        return Whole;
                    }
                    let Some(k) = self.kept.get(&fam) else { return Whole };
                    // 7
                    if !k.ended_quiet {
                        return Whole;
                    }
                    // 2: after everything the family walked, and not a close the
                    // look-ahead from a stated open at that instant would take first;
                    // a record whose every leg the walk passes over (no instrument, or
                    // a distribution no adjustment names) is placed anywhere: it moves
                    // no lot, so the match after it is the match before it with its
                    // cash (#527)
                    let Some(last) = &k.last else { return Whole };
                    let key = crate::ledger::order_key_of(t, &legs, &info.source_key);
                    let passed_over = legs.iter().all(|l| self.walk_passes_over(l));
                    if !passed_over && (key <= *last || earliest.as_ref().is_some_and(|e| e <= last)) {
                        return self.placeable(fam, k, t, &key).map_or(Whole, Way::Place);
                    }
                    let same_instant = (key.0, key.1, key.2) == (last.0, last.1, last.2);
                    if same_instant && t.effect == Some(Effect::Close) && txs.len() == 1 && t.instrument.is_some_and(|i| k.opens_at_last.contains(&(t.account, i))) {
                        return Whole;
                    }
                    // 5: a coin's row no finer than its source's rows were
                    if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
                        let coin = l.instruments.get(&i).is_some_and(|x| x.instrument.kind == bagholder_core::instrument::InstrumentKind::Crypto);
                        if coin && k.places.get(&(i, t.mapping.source.clone())).is_some_and(|p| q.places() > *p) {
                            return Whole;
                        }
                    }
                    Way::Continue
                })();
                match way {
                    Way::Continue => {
                        added.entry(fam).or_default().push(t.id.clone());
                        went_on.insert(fam);
                    }
                    Way::Place(start) => {
                        added.entry(fam).or_default().push(t.id.clone());
                        placed.entry(fam).or_default().insert(start);
                    }
                    Way::Whole => {
                        blocked.insert(fam);
                    }
                }
            }
        }
        // 6: what the match reads of an instrument: its kind, currency and terms
        for (i, now) in &delta.instruments {
            let was = l.instruments.get(i);
            let same = match (was, now) {
                (Some(a), Some(b)) => a.instrument.kind == b.instrument.kind && a.instrument.currency == b.instrument.currency && a.terms == b.terms,
                _ => false,
            };
            if !same {
                blocked.extend(family(Node::Instrument(*i)));
                for u in [was.and_then(|x| x.terms.as_ref()), now.as_ref().and_then(|x| x.terms.as_ref())].into_iter().flatten() {
                    blocked.extend(family(Node::Instrument(u.underlying)));
                }
            }
        }
        // 6: an account that held rows before the change: anything of it changed
        for (a, now) in &delta.accounts {
            if let Some(was) = l.accounts.get(a) {
                if now.as_ref() != Some(was) {
                    blocked.extend(family(Node::Cash(*a)));
                    blocked.extend(l.transactions.instruments_in(*a).filter_map(|i| family(Node::Instrument(i))));
                }
            }
        }
        // 1: an adjustment set or cleared
        for (tx, list) in &delta.adjustments {
            let was = self.inputs.facts.adjustments.get(tx);
            if was.is_some() || !list.is_empty() {
                blocked.extend(l.transactions.get(tx).map(node_of).and_then(family));
                blocked.extend(was.into_iter().chain(list.iter()).flat_map(|x| x.legs.iter().flat_map(|g| [g.from, g.to])).flatten().filter_map(|i| family(Node::Instrument(i))));
            }
        }
        added.retain(|f, _| !blocked.contains(f));
        added
            .into_iter()
            .filter_map(|(f, rows)| match placed.get(&f) {
                None => Some((f, (rows, None))),
                // every row placed, all in one stretch: a family is continued one way
                Some(starts) if starts.len() == 1 && !went_on.contains(&f) => Some((f, (rows, starts.iter().next().cloned()))),
                Some(_) => None,
            })
            .collect()
    }

    /// The start of the flat stretch `t`, a row placed before what its family
    /// matched, falls strictly inside, where it can be placed there
    /// (`docs/plans/matching-from-a-flat-point.md`, B): a security's row (a coin's
    /// rows read its source's places and its dust, a contract's the expiry
    /// calendar), in a family nothing of which waits on a multi-leg order whose
    /// legs are not stated (a taint never clears, so none now is none at any
    /// point), on a day strictly between the days the stretch starts and ends.
    /// Whether the rows then leave the family as they found it is the walk's own
    /// check (`ledger::place_in_stretch`).
    fn placeable(&self, fam: Node, k: &MatchState, t: &bagholder_core::transaction::Transaction, key: &crate::ledger::OrderKey) -> Option<Option<crate::ledger::OrderKey>> {
        let i = t.instrument?;
        if self.inputs.ledger.instruments.get(&i).is_none_or(|x| x.instrument.kind != bagholder_core::instrument::InstrumentKind::Security) {
            return None;
        }
        if !self.untainted(fam, k) {
            return None;
        }
        let (start, end) = k.stretches.range(..=Some(key.clone())).next_back()?;
        let day = key.0;
        if start.as_ref().is_some_and(|s| s.0 >= day) || end.as_ref().is_some_and(|e| e.0 <= day) {
            return None;
        }
        Some(start.clone())
    }

    /// Whether nothing of the family waits on a multi-leg order whose legs are
    /// not stated: its kept state, and every book of its holdings.
    fn untainted(&self, fam: Node, k: &MatchState) -> bool {
        k.untainted()
            && self.families.members(fam).into_iter().all(|n| {
                let Node::Instrument(i) = n else { return true };
                self.matched.held_by.get(&i).into_iter().flatten().all(|a| self.matched.books.get(&(*a, i)).is_none_or(|b| b.taint.is_empty()))
            })
    }

    /// Whether the match walks past `t` moving nothing but cash: a row naming no
    /// instrument, or a distribution no adjustment names (a return of capital is
    /// an adjustment, and moves the lots). Its place in the walk then decides
    /// nothing (`ledger::Matcher::apply`).
    fn walk_passes_over(&self, t: &bagholder_core::transaction::Transaction) -> bool {
        crate::ledger::passes_over(&self.inputs, t)
    }

    /// A family's match continued from its kept state for `rows`, its share of
    /// each account's cash brought with it. What it wrote, and whether what waits
    /// on the person moved.
    fn continue_family(&mut self, family: Node, rows: &[TransactionId]) -> (crate::ledger::Continued, bool) {
        let mut state = self.kept.remove(&family).expect("a family continued keeps its state");
        let txs: Vec<&bagholder_core::transaction::Transaction> = rows.iter().filter_map(|id| self.inputs.ledger.transactions.get(id)).collect();
        let waiting = self.matched.waiting.len();
        let c = crate::ledger::continue_family(&self.inputs, &mut self.matched, &mut state, txs);
        self.kept.insert(family, state);
        self.add_shares(family, &c);
        #[cfg(feature = "oracles")]
        {
            path(&mut self.paths, family, Path::Continued);
            self.same_as_whole(family);
        }
        let moved = self.matched.waiting.len() != waiting;
        (c, moved)
    }

    /// A family's round trip placed inside the flat stretch starting at `start`
    /// (`ledger::place_in_stretch`), its share of each account's cash brought with
    /// it: what it wrote, and whether what waits on the person moved. None where
    /// the walk left the family otherwise than it found it, and nothing written.
    fn place_family(&mut self, family: Node, rows: &[TransactionId], start: &Option<crate::ledger::OrderKey>) -> Option<(crate::ledger::Continued, bool)> {
        let mut state = self.kept.remove(&family).expect("a family placed into keeps its state");
        let txs: Vec<&bagholder_core::transaction::Transaction> = rows.iter().filter_map(|id| self.inputs.ledger.transactions.get(id)).collect();
        let c = crate::ledger::place_in_stretch(&self.inputs, &mut self.matched, &mut state, txs, start);
        self.kept.insert(family, state);
        let c = c?;
        self.add_shares(family, &c);
        #[cfg(feature = "oracles")]
        {
            path(&mut self.paths, family, Path::Placed);
            self.same_as_whole(family);
        }
        Some((c, false))
    }

    /// The rows' cash added to the family's share of each account's.
    fn add_shares(&mut self, family: Node, c: &crate::ledger::Continued) {
        for (a, cash) in &c.cash {
            let share = self.shares.entry(*a).or_default().entry(family).or_default();
            for (cur, v) in cash {
                let e = share.entry(*cur).or_insert(Ok(bagholder_core::Dec::ZERO));
                if let Ok(t) = e {
                    *e = match v {
                        Ok(v) => t.checked_add(*v).map_err(crate::gap::Gaps::from),
                        Err(g) => Err(g.clone()),
                    };
                }
            }
            let t = total(&self.shares[a]);
            self.matched.cash.insert(*a, t);
        }
    }

    /// Under the oracles: the family's match as it stands equals matching it whole.
    #[cfg(feature = "oracles")]
    fn same_as_whole(&self, family: Node) {
        {
            if !crate::oracles_off() {
                let txs = self.families.transactions(&self.inputs.ledger, family);
                let (whole, whole_state) = match_family(&self.inputs, txs);
                assert_eq!(&whole_state, &self.kept[&family], "the match of {family:?} as it stands keeps what a whole match keeps");
                let mut expect = self.matched.clone();
                let region: BTreeSet<Node> = self.families.members(family);
                let shares = whole.cash.clone();
                expect.replace(&self.inputs.ledger, &region, whole);
                expect.cash = self.matched.cash.clone();
                assert!(expect == self.matched, "the match of {family:?} as it stands equals a whole one");
                for (a, s) in shares {
                    assert_eq!(self.shares.get(&a).and_then(|x| x.get(&family)), Some(&s), "{family:?}'s share of {a}'s cash");
                }
            }
        }
    }

    /// The figures a change to the record reached, worked out again in place, and
    /// what moved said (`docs/plans/stage-3-the-record-by-change.md`, B, and
    /// `docs/plans/stage-3-a-change-on-a-deep-history.md`, C): the trade
    /// identity, round trips and positions of `reach.instruments` (a family
    /// matched whole), and of the round trips `reach.trips` and the holdings
    /// `reach.holdings` beyond them (a continued match, a trade or its journal);
    /// the saved groups, where a round trip or a trade of theirs is reached or a
    /// group changed (their claims are settled over every group, in the book's
    /// order); the cashflow rows of `reach.transactions` and of the stock
    /// dividends of those instruments; the payers' rates of what those rows and
    /// instruments are, and of the holdings reached; and the checks of
    /// `reach.accounts`.
    fn rederive(&mut self, reach: &Reach, moved: &mut Moved) {
        let instruments = &reach.instruments;
        let fresh = crate::identity::identify_in(&self.inputs.ledger, &self.matched, instruments);
        self.identity.replace(&self.inputs.ledger, instruments, fresh);
        // the round trips reached beyond the instruments reached whole, and the
        // trades whose round trip can move: the ones named, those sharing their
        // anchor, and every one on a round trip reached; closed over each other,
        // so every round trip such a trade holds or could claim is worked out with it
        let whole = |t: &bagholder_core::TradeId| self.inputs.ledger.trades.get(t).and_then(crate::input::Keyed::instrument).is_some_and(|i| instruments.contains(&i));
        let on_anchors = reach.anchors.iter().flat_map(|a| self.inputs.ledger.trades.anchored_on(a).map(|t| t.id));
        let mut named: BTreeSet<bagholder_core::TradeId> = reach.trades.iter().copied().chain(on_anchors).filter(|t| !whole(t)).collect();
        let mut narrow: BTreeSet<TripKey> = reach.trips.iter().filter(|k| !instruments.contains(&k.instrument)).cloned().collect();
        loop {
            let mut more: BTreeSet<TripKey> = BTreeSet::new();
            for t in &named {
                more.extend(self.identity.trip_of.get(t).cloned());
                if let Some(trade) = self.inputs.ledger.trades.get(t) {
                    more.extend(self.trips_anchored(trade));
                }
            }
            let mut trades: BTreeSet<bagholder_core::TradeId> = BTreeSet::new();
            for k in narrow.iter().chain(more.iter()) {
                for f in self.matched.trips.get(k).into_iter().flat_map(|t| t.fills.iter()) {
                    trades.extend(self.inputs.ledger.trades.anchored_on(f).map(|t| t.id).filter(|t| !whole(t)));
                }
            }
            let before = (narrow.len(), named.len());
            narrow.extend(more.into_iter().filter(|k| !instruments.contains(&k.instrument)));
            named.extend(trades);
            if (narrow.len(), named.len()) == before {
                break;
            }
        }
        // the trades whose round trip can move: the ones named, and every one on a
        // round trip reached
        let mut identified: BTreeSet<bagholder_core::TradeId> = reach.trades.clone();
        if !narrow.is_empty() || !named.is_empty() {
            let (fresh, trades) = crate::identity::identify_trips(&self.inputs.ledger, &self.matched, &narrow, &named);
            self.identity.replace_trips(&self.inputs.ledger, &narrow, &trades, fresh);
            identified.extend(trades);
        }
        // the groups: which round trips each holds turns on the earlier ones, so all
        // of them, where a round trip or a trade of one is reached or one changed
        let member = |t: &bagholder_core::TradeId| self.inputs.ledger.groups.iter().any(|g| g.members.contains(t));
        let groups_reached = !instruments.is_empty() || reach.groups || narrow.iter().any(|k| self.group_held.contains(k)) || identified.iter().any(member);
        let held_was = self.group_held.clone();
        let (groups, held) = if groups_reached {
            let (g, h) = crate::trades::groups_shown(&self.inputs, &self.matched, &self.identity);
            (Some(g), h)
        } else {
            (None, held_was.clone())
        };
        let mut trips: BTreeSet<TripKey> = instruments.iter().flat_map(|i| self.matched.trips_of.get(i).into_iter().flatten().cloned()).collect();
        trips.extend(narrow.iter().cloned());
        trips.extend(held_was.symmetric_difference(&held).cloned());
        let rebuilt = |k: &TradeKey| match k {
            TradeKey::Group(_) => groups.is_some(),
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
        let mut now: Vec<TradeFig> = groups.unwrap_or_default();
        for k in &trips {
            if held.contains(k) {
                continue;
            }
            now.extend(crate::trades::trip_fig(&self.inputs, &self.matched, &self.identity, k));
        }
        self.group_held = held;
        // each after every row it ties with: as a fresh build's stable sort leaves them
        for t in &now {
            let at = self.trades.partition_point(|x| crate::trades::shown_order(x, t) != std::cmp::Ordering::Greater);
            self.trades.insert(at, t.clone());
        }
        diff(moved, was.iter().map(|t| (trade_entity(t), t)), now.iter().map(|t| (trade_entity(t), t)));
        // positions: of the instruments reached whole, of every holding with lots of
        // a round trip reached (a roll's spans two), and of the holdings reached
        let mut shown: BTreeSet<InstrumentId> = instruments.clone();
        for k in &narrow {
            if let Some(t) = self.matched.trips.get(k) {
                shown.extend(t.instruments.iter().copied());
            }
            shown.insert(k.instrument);
        }
        shown.extend(reach.holdings.iter().map(|(_, i)| *i));
        let mut was: Vec<PositionFig> = Vec::new();
        self.positions.retain(|p| {
            if shown.contains(&p.instrument) {
                was.push(p.clone());
                false
            } else {
                true
            }
        });
        let now: Vec<PositionFig> = shown.iter().flat_map(|i| build_positions(&self.inputs, &self.matched, &self.identity, Some(*i), None)).collect();
        let order = |p: &PositionFig| (p.account, p.instrument, p.direction);
        for p in &now {
            let at = self.positions.partition_point(|x| order(x) <= order(p));
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
            let at = self.cash.partition_point(|x| crate::cashflow::shown_order(x, r) != std::cmp::Ordering::Greater);
            self.cash.insert(at, r.clone());
        }
        for r in was.iter().filter(|r| pays_cash(r)) {
            let i = r.instrument.expect("a dividend names its payer");
            if let Some(rows) = self.paid.get_mut(&i) {
                rows.retain(|x| x.id != r.id);
                if rows.is_empty() {
                    self.paid.remove(&i);
                }
            }
        }
        for r in now.iter().filter(|r| pays_cash(r)) {
            let rows = self.paid.entry(r.instrument.expect("a dividend names its payer")).or_default();
            let at = rows.partition_point(|x| crate::cashflow::shown_order(x, r) != std::cmp::Ordering::Greater);
            rows.insert(at, r.clone());
        }
        let payers: BTreeSet<InstrumentId> = was.iter().chain(now.iter()).filter_map(|r| r.instrument).chain(instruments.iter().copied()).chain(reach.holdings.iter().map(|(_, i)| *i)).collect();
        diff(moved, was.iter().map(|r| (Entity::CashRow(r.id.clone()), r)), now.iter().map(|r| (Entity::CashRow(r.id.clone()), r)));
        // the payers' rates
        for i in payers {
            self.payer_again(i, moved);
        }
        // the checks with the brokers
        for a in &reach.accounts {
            self.recheck(*a, moved);
        }
    }

    /// One payer's rate worked out again from its own payments, and what moved said.
    fn payer_again(&mut self, i: InstrumentId, moved: &mut Moved) {
        let payments: Vec<&CashRow> = self.paid.get(&i).into_iter().flatten().collect();
        let rate = crate::cashflow::payer_rate(&self.inputs, i, payments, &self.matched);
        let before = self.payers.get(&i).cloned();
        match rate.clone() {
            Some(r) => self.payers.insert(i, r),
            None => self.payers.remove(&i),
        };
        diff(moved, before.iter().map(|p| (Entity::Payer(i), p)), rate.iter().map(|p| (Entity::Payer(i), p)));
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

    /// Per account, the live rate of each currency other than CAD its broker
    /// states something in now (its value, cash, what it can borrow, what its
    /// holdings are worth): what every figure that adds an account's statements
    /// in CAD is converted at. None of the snapshot's entities holds it, so a
    /// rates or clock change compares it here.
    fn live_rates_stated(&self) -> BTreeMap<AccountId, BTreeMap<bagholder_core::Currency, Fig<(bagholder_core::Dec, Option<Date>)>>> {
        let (rates, clock) = (&self.inputs.facts.rates, &self.inputs.clock);
        self.inputs
            .market
            .brokers
            .iter()
            .map(|(a, b)| {
                let currencies = b.net_value_now.map(|m| m.currency).into_iter().chain(b.cash.keys().copied()).chain(b.buying_power.iter().filter_map(|r| r.as_ref().ok().map(|m| m.currency))).chain(b.held_value.values().map(|m| m.currency));
                (*a, currencies.filter(|c| *c != bagholder_core::Currency::CAD).map(|c| (c, crate::fx::live_rate(rates, clock, c))).collect())
            })
            .collect()
    }

    /// Each account whose statements are now converted at another live rate (or
    /// wait where they did not, or no longer wait) moved: the views that add its
    /// statements in CAD read it again.
    fn live_rates_moved(&self, before: &BTreeMap<AccountId, BTreeMap<bagholder_core::Currency, Fig<(bagholder_core::Dec, Option<Date>)>>>, m: &mut Moved) {
        for (a, now) in self.live_rates_stated() {
            if before.get(&a) != Some(&now) {
                m.0.entry(Entity::Broker(a)).or_default().insert("rate");
            }
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
        // what the page shows in order, in that order (brief 27, 11)
        if self.trades != other.trades {
            return Some("the trades differ in order or content".into());
        }
        if self.positions != other.positions {
            return Some("the positions differ in order or content".into());
        }
        if self.cash != other.cash {
            return Some("the cashflow rows differ in order or content".into());
        }
        if self.families != other.families {
            return Some(format!("the families differ: {}", self.families.differences(&other.families)));
        }
        if self.symbols != other.symbols {
            return Some("the instruments by symbol differ".into());
        }
        if self.listings != other.listings {
            return Some("the instruments by listing differ".into());
        }
        if self.shares != other.shares {
            return Some("the cash shares by family differ".into());
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
    /// Instruments whose figures are worked out whole: a family matched whole.
    instruments: BTreeSet<InstrumentId>,
    /// Round trips beyond them: a continued match wrote them, or a trade or its
    /// journal is on them.
    trips: BTreeSet<TripKey>,
    /// Holdings a continued match walked.
    holdings: BTreeSet<(AccountId, InstrumentId)>,
    transactions: BTreeSet<bagholder_core::TransactionId>,
    accounts: BTreeSet<AccountId>,
    trades: BTreeSet<bagholder_core::TradeId>,
    /// The transactions a trade the change names is anchored on, before and after.
    anchors: BTreeSet<bagholder_core::TransactionId>,
    /// A saved group changed.
    groups: bool,
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
