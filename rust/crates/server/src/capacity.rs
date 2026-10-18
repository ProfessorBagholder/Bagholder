//! `bagholder capacity`: the server held to its cost budget at the owner's size
//! (`docs/architecture.md` §16; `docs/plans/stage-p-capacity-and-process.md`).
//!
//! - `capacity build <folder> [--times N] [--seed S]` makes a data folder the
//!   way the app makes one: a made-up Wealthsimple of the owner's size (times N)
//!   pulled through the real adapter and pull, a year of balances reads at the
//!   owner's rate, the market facts through the cache's own writers, and the
//!   figures settled as a start settles them.
//! - `capacity run <work folder>` builds (or reuses) the owner's size and four
//!   times it under the work folder, measures each operation in a process of
//!   its own, prints the table, and fails when the budget or the growth rule is
//!   broken (or when an operation listed over budget is now within it).
//! - `capacity probe <operation> <folder> [--depth <rows>]` is one measured run
//!   (on a book of the depth pair, its deep listing built with `<rows>`), printed as one
//!   line; `run` starts it.
//!
//! A subcommand of the server, as `demo-facts` and `pull-broker` are: what it
//! measures is the server's own code, which only this binary holds.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_broker::pull;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Broker, Currency, Dec, Money, SourceName};
use bagholder_sources::cache::{MarketCache, StoredQuote};
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{Generated, Size};

// ---- the budget (docs/architecture.md §16; a test holds the two equal) ----

/// RAIL: load "in 5 seconds or less on mid-range mobile devices" (web.dev/articles/rail).
pub const STARTUP_MS: u64 = 5_000;
/// RAIL: "complete a transition initiated by user input within 100 ms".
pub const ANSWER_MS: u64 = 100;
/// RAIL: past a second the person loses the thread of the task.
pub const TO_FIGURES_MS: u64 = 1_000;
/// How much slower a Raspberry Pi 5 is than the runner, on one core: Raspberry
/// Pi's own Geekbench 6 single-core average for the Pi 5 (764) against published
/// runs of the runner's CPU, Azure Cobalt 100 (1,620 and 1,639). CPU only, not
/// measured on hardware.
pub const PI_SLOWDOWN: f64 = 2.1;
/// Work through an index grows by a B-tree's depth when the book is four times
/// the size; work proportional to the book grows about four times. Under twice
/// tells the two apart.
pub const GROWTH_LIMIT: f64 = 2.0;
/// The balances cadence (`broker_reads::BALANCES_EVERY`), in milliseconds.
pub const BALANCES_EVERY_MS: u64 = crate::broker_reads::BALANCES_EVERY.as_millis() as u64;
/// The quotes cadence (`due::QUOTES_EVERY`), in milliseconds.
pub const QUOTES_EVERY_MS: u64 = crate::due::QUOTES_EVERY.as_millis() as u64;
/// How many times the median is taken over.
pub const RUNS: usize = 7;

/// The owner's size, counted on a copy of the owner's book on 2026-10-08: 29
/// accounts, 210 instruments of which 53 option contracts (each option round
/// trip here makes its own), 646 trades, three years of activity.
pub const OWNER: Size = Size { accounts: 29, instruments: 157, trades: 646, months: 36, seed: 1, deep: 0 };
/// How the owner's balances move, counted on a copy of the owner's book on
/// 2026-10-10 over the 12.6 days from 2026-09-27 to 2026-10-09: 664 balances
/// reads, each stating the cash of 16 accounts; 87 periods of cash, 816 of value
/// now (16 accounts) and 533 of buying power (one margin account). A balances read
/// a day, and each kind's changes a day per account (per margin account for
/// buying power), as (count, over days × accounts).
pub const BALANCES_READS: (u64, f64) = (664, 12.6);
pub const CASH_CHANGES: (u64, f64) = (87, 12.6 * 16.0);
pub const VALUE_CHANGES: (u64, f64) = (816, 12.6 * 16.0);
pub const POWER_CHANGES: (u64, f64) = (533, 12.6);
/// The most rows one instrument holds on the owner's book: 349 of 7,151 live
/// transactions, counted on a copy of the owner's book on 2026-10-12. The depth
/// pair carries one listing this deep, and four times it (brief 26, 12).
pub const DEEPEST_ROWS: usize = 349;
/// A year of balances reads: the size the book reaches in a year of use.
pub const BALANCES_DAYS: u32 = 365;
/// splitmix64: the same seed, the same balances.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n.max(1)
    }
}

/// The day the made-up activity ends on (fixed, so the same seed makes the same book).
pub const MADE_UP_TODAY: &str = "2026-10-08";

/// The two ways a book grows four times, each a pair of books the growth rule
/// is judged on: every count four times (more accounts, instruments and trades,
/// each instrument as deep as before), or one instrument's history four times as
/// deep, the rest as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pair {
    Breadth,
    Depth,
}

impl Pair {
    pub fn name(self) -> &'static str {
        match self {
            Pair::Breadth => "breadth",
            Pair::Depth => "depth",
        }
    }
    /// The operations measured on this pair: depth is what a change to one
    /// instrument's history costs.
    fn ops(self) -> &'static [Op] {
        match self {
            Pair::Breadth => &[Op::Startup, Op::AddTrade, Op::Import, Op::Brokers, Op::PriceChanged, Op::FeedStep],
            // a change dated today, and one dated in the middle of the history: a
            // record that states no venue reads as on the one before it, so a
            // back-dated one re-forms the names after it (brief 44, 2)
            Pair::Depth => &[Op::AddTrade, Op::Import, Op::BackdatedTrade, Op::BackdatedFill, Op::BackdatedRecord, Op::BackdatedFileRow],
        }
    }
    /// What a probe is told of the pair it measures.
    fn flag(self, min_rows: u64) -> Vec<String> {
        match self {
            Pair::Breadth => vec![],
            Pair::Depth => vec!["--depth".into(), min_rows.to_string()],
        }
    }

    /// The rows the deep listing of a book `times` the owner's was built with.
    fn min_rows(self, times: usize) -> u64 {
        match self {
            Pair::Breadth => 0,
            // two rows a round trip, the last still open
            Pair::Depth => (self.size(times).deep * 2).saturating_sub(1) as u64,
        }
    }

    /// The pair's two books, under the work folder.
    fn folders(self, work: &Path) -> (PathBuf, PathBuf) {
        match self {
            Pair::Breadth => (work.join("owner"), work.join("owner-x4")),
            Pair::Depth => (work.join("owner-deep"), work.join("owner-deep-x4")),
        }
    }
    /// The size of its book `times` the owner's.
    fn size(self, times: usize) -> Size {
        match self {
            Pair::Breadth => sized(times, OWNER.seed),
            // two rows a round trip
            Pair::Depth => Size { deep: DEEPEST_ROWS * times / 2, ..OWNER },
        }
    }
}

/// Each operation measured, whether a person waits on it, and whether the
/// growth rule holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Startup,
    AddTrade,
    Import,
    Brokers,
    PriceChanged,
    FeedStep,
    /// A trade entered by hand on the holding, dated in the middle of its history.
    BackdatedTrade,
    /// One of the broker's buys of the holding, from the middle of its history,
    /// stored again under a new id: every later trade of the holding is derived again.
    BackdatedFill,
    /// One of the broker's distribution records of the holding, from the middle of
    /// its history, stored again under a new id, as a pull stores a row the broker adds late.
    BackdatedRecord,
    /// A file's row, dated in the middle of the history of a listing whose rows
    /// name its exchange by code and state no venue in words (`CODED`).
    BackdatedFileRow,
}

impl Op {
    pub const ALL: [Op; 10] = [Op::Startup, Op::AddTrade, Op::Import, Op::Brokers, Op::PriceChanged, Op::FeedStep, Op::BackdatedTrade, Op::BackdatedFill, Op::BackdatedRecord, Op::BackdatedFileRow];
    pub fn name(self) -> &'static str {
        match self {
            Op::Startup => "startup",
            Op::AddTrade => "add-trade",
            Op::Import => "import",
            Op::Brokers => "brokers",
            Op::PriceChanged => "price-changed",
            Op::FeedStep => "feed-step",
            Op::BackdatedTrade => "backdated-trade",
            Op::BackdatedFill => "backdated-fill",
            Op::BackdatedRecord => "backdated-record",
            Op::BackdatedFileRow => "backdated-file-row",
        }
    }
    fn parse(s: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.name() == s)
    }
    /// A person waits on it: its wall time fails the run.
    fn waited_on(self) -> bool {
        matches!(self, Op::Startup | Op::AddTrade | Op::Import | Op::BackdatedTrade | Op::BackdatedFileRow)
    }
    /// It answers one change: the growth rule holds it.
    fn per_change(self) -> bool {
        self != Op::Startup
    }
}

/// The rows the import probe's file holds, by pair. By depth, more than 100: the
/// import then commits them in groups of two or more (`csv_import`, a group per
/// hundredth of the file), so each row after a group's first is looked up inside
/// a transaction that has written sightings not yet settled, on the deepest
/// listing, where a lookup that folds the listing's history would show (brief 30,
/// A3). By breadth, a file small beside the book, so one read of every row the
/// book's files imported stands out against the import's own work (brief 27, 9).
fn import_rows(pair: Pair) -> u64 {
    match pair {
        Pair::Depth => 150,
        Pair::Breadth => 20,
    }
}

/// Where the engine's log of matches stood before a probe's window, and the
/// family the probe's listing is matched in.
#[cfg(test)]
fn paths_mark(f: &crate::figures::Figures, instrument: bagholder_core::InstrumentId) -> (usize, Option<bagholder_engine::families::Node>) {
    f.read(|e| (e.paths().len(), Some(e.family_of(bagholder_engine::families::Node::Instrument(instrument))))).unwrap_or((0, None))
}

/// The window's matches of the probe's family, kept for the test that reads them.
#[cfg(test)]
fn paths_taken(f: &crate::figures::Figures, (from, family): (usize, Option<bagholder_engine::families::Node>)) {
    let window = f.read(|e| e.paths()[from.min(e.paths().len())..].iter().filter(|(n, _)| Some(*n) == family).copied().collect()).unwrap_or_default();
    PROBED_PATHS.with(|p| *p.borrow_mut() = window);
}

/// A test's planted read of every row any file imported, inside the import
/// probe's window (brief 28, E).
#[cfg(test)]
static PLANT_WHOLE_READ: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    /// The matches of the target listing's family in the last probe's window, and
    /// whether each was continued.
    static PROBED_PATHS: std::cell::RefCell<Vec<(bagholder_engine::families::Node, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A test's planted fold of the traded instrument's names from every sighting,
/// inside the add-trade probe's window.
#[cfg(test)]
static PLANT_NAMES_FOLD: AtomicBool = AtomicBool::new(false);

/// What is over budget today, each with its issue: the run fails when one off
/// this list misses, and when one on it meets its budget and the growth rule,
/// so the list only shrinks (`docs/architecture.md` §16).
/// Each entry names the measure that is over, so an operation listed for one
/// measure is still held to the others.
pub const OVER_BUDGET: &[(&str, Pair, Measure, &str)] = &[
    // the whole view built and serialised to learn whether it changed
    ("feed-step", Pair::Breadth, Measure::Allocations, "#426"),
    // a change dated in the middle of a holding's history: the engine derives the
    // holding's later trades and its later days again, from that day on
    ("backdated-trade", Pair::Depth, Measure::Time, ENGINE_FROM_A_DAY),
    ("backdated-trade", Pair::Depth, Measure::Steps, ENGINE_FROM_A_DAY),
    ("backdated-trade", Pair::Depth, Measure::Allocations, ENGINE_FROM_A_DAY),
    ("backdated-fill", Pair::Depth, Measure::Steps, ENGINE_FROM_A_DAY),
    ("backdated-fill", Pair::Depth, Measure::Allocations, ENGINE_FROM_A_DAY),
    ("backdated-record", Pair::Depth, Measure::Allocations, ENGINE_FROM_A_DAY),
    ("backdated-file-row", Pair::Depth, Measure::Allocations, ENGINE_FROM_A_DAY),
];

/// The issue of the engine's work after a back-dated change.
const ENGINE_FROM_A_DAY: &str = "#527";

/// What a measurement is judged by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    /// The wall time a person waits, on a Pi, against the budget.
    Time,
    /// SQLite's steps, against the growth rule.
    Steps,
    /// The allocations made, against the growth rule.
    Allocations,
    /// The bytes read from storage, against the growth rule.
    BytesRead,
}

impl Measure {
    fn name(self) -> &'static str {
        match self {
            Measure::Time => "time",
            Measure::Steps => "steps",
            Measure::Allocations => "allocations",
            Measure::BytesRead => "bytes read",
        }
    }
}

/// Whether `op` on `pair` is listed over budget by `measure`.
fn listed(op: Op, pair: Pair, measure: Measure) -> bool {
    OVER_BUDGET.iter().any(|(name, p, m, _)| *name == op.name() && *p == pair && *m == measure)
}

// ---- work counted ----

static COUNT_ALLOCS: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The allocations made on this thread: a measurement sees its own work and
    /// none of another thread's running beside it.
    static ALLOCS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One allocation counted on this thread, where counting is on. A thread being
/// torn down has no count to add to, and its allocation is not counted.
fn count_alloc() {
    if COUNT_ALLOCS.load(Ordering::Relaxed) {
        // a thread being torn down keeps no count: its allocation goes uncounted
        if ALLOCS.try_with(|a| a.set(a.get() + 1)).is_err() {}
    }
}

fn allocs() -> u64 {
    // read on a live thread, where the count always stands
    ALLOCS.with(std::cell::Cell::get)
}

/// The process's allocator, counting allocations only once a capacity probe
/// has turned counting on; otherwise the system's, with one flag read.
pub struct Counting;

// SAFETY: every call is passed straight to the system allocator with the same
// arguments; the count is a relaxed atomic add, which allocates nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_alloc();
        // SAFETY: as the caller's contract for `alloc`
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_alloc();
        // SAFETY: as the caller's contract for `alloc_zeroed`
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as the caller's contract for `dealloc`
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_alloc();
        // SAFETY: as the caller's contract for `realloc`
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Bytes the process has asked the system to read (Linux's `rchar`): SQLite's
/// page reads that its own cache did not hold. None where the system does not say.
fn read_bytes() -> Option<u64> {
    let io = std::fs::read_to_string("/proc/self/io").ok()?;
    io.lines().find_map(|l| l.strip_prefix("rchar:")).and_then(|v| v.trim().parse().ok())
}

/// One measured run.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measured {
    pub wall_ns: u64,
    pub steps: u64,
    pub allocs: u64,
    pub read: Option<u64>,
    /// What the budget divides by at this size: accounts for `brokers`, priced
    /// holdings for a price change and a stream step.
    pub divisor: u64,
    /// The rows of the listing an added trade or an import changed.
    pub rows: Option<u64>,
}

struct Mark {
    at: std::time::Instant,
    steps: u64,
    allocs: u64,
    read: Option<u64>,
}

fn mark() -> Mark {
    bagholder_sqlite::work::set_measuring(true);
    bagholder_engine::set_oracles_off(true);
    Mark { at: std::time::Instant::now(), steps: bagholder_sqlite::work::steps(), allocs: allocs(), read: read_bytes() }
}

fn since(m: &Mark, divisor: u64) -> Measured {
    bagholder_sqlite::work::set_measuring(false);
    bagholder_engine::set_oracles_off(false);
    Measured {
        wall_ns: m.at.elapsed().as_nanos() as u64,
        steps: bagholder_sqlite::work::steps() - m.steps,
        allocs: allocs() - m.allocs,
        read: match (m.read, read_bytes()) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        },
        divisor,
        rows: None,
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn made_up_today() -> Date {
    MADE_UP_TODAY.parse().expect("a day")
}

// ---- build ----

/// The owner's size times `times`.
pub fn sized(times: usize, seed: u64) -> Size {
    Size { accounts: OWNER.accounts * times, instruments: OWNER.instruments * times, trades: OWNER.trades * times, seed, ..OWNER }
}

/// Make the data folder `home` for `size`, with `balances_days` of balances reads.
pub fn build(home: &Path, size: Size, balances_days: u32) -> Result<String, String> {
    std::fs::create_dir_all(home).map_err(err)?;
    let today = made_up_today();
    let at: Timestamp = format!("{today}T20:00:00Z").parse().map_err(err)?;
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(err)?;
    book.state_zone("America/Toronto", at).map_err(err)?;
    let ws = Broker::named("wealthsimple");
    let connection = match book.connections().map_err(err)?.into_iter().find(|c| c.broker == ws) {
        Some(c) => c.id,
        None => book.add_connection(&ws, "Wealthsimple", at).map_err(err)?,
    };
    let mut adapter = Wealthsimple::new(Generated::new(size, today));
    let report = pull::pull(&book, &mut adapter, connection, today, at, &mut |_| {}).map_err(err)?;
    if !report.failures.is_empty() {
        return Err(format!("the pull failed: {:?}", report.failures));
    }
    // a year of balances reads at the owner's rate. One real pass, through the
    // pull, writes what a pass writes; the rest through the book's own store
    // calls, as the pull writes them (`pull::balances`): every read kept, and a
    // statement stored where an account's cash, value or buying power changed, at
    // the rates the owner's book shows (a statement stating again what the last
    // did only extends its period, which the last pass does for every account).
    // The last pass states each account as the broker does, so the newest cash is
    // the book's own and the check reads no difference the activity did not make.
    let start = at.checked_sub(SignedDuration::from_hours(24 * i64::from(balances_days))).map_err(err)?;
    let read = pull::balances(&book, &mut adapter, connection, start).map_err(err)?;
    if !read.failures.is_empty() {
        return Err(format!("the balances read failed: {:?}", read.failures));
    }
    let a_day = (BALANCES_READS.0 as f64 / BALANCES_READS.1).round().max(1.0) as u64;
    let every = SignedDuration::from_secs((86_400 / a_day) as i64);
    let chance = |(n, over): (u64, f64)| ((n as f64 / over / a_day as f64) * 1_000_000.0).round() as u64;
    let (cash_chance, value_chance, power_chance) = (chance(CASH_CHANGES), chance(VALUE_CHANGES), chance(POWER_CHANGES));
    use bagholder_broker::BrokerAdapter as _;
    let stated = adapter.accounts().map_err(|f| f.to_string())?;
    let mut by_account = Vec::new();
    for a in &stated {
        let Some(id) = book.account_by_ref(&bagholder_core::account::AccountRef::new(ws.clone(), &a.key)).map_err(err)? else { continue };
        let margin = matches!(a.account_type, bagholder_core::account::AccountType::Known { kind: bagholder_core::account::AccountKind::Margin, .. });
        by_account.push((id, a.key.clone(), a.net_value, margin));
    }
    let keys: Vec<String> = by_account.iter().map(|(_, k, _, _)| k.clone()).collect();
    let true_cash = adapter.cash(&keys).map_err(|f| f.to_string())?;
    let margins: Vec<String> = by_account.iter().filter(|(_, _, _, m)| *m).map(|(_, k, _, _)| k.clone()).collect();
    let true_power = if margins.is_empty() { Default::default() } else { adapter.buying_power(&margins).map_err(|f| f.to_string())? };
    let mut rng = Rng(size.seed ^ 0xBA1A_0CE5);
    let cent = Dec::new(1, 2).map_err(err)?;
    let moved = |rng: &mut Rng, d: Dec| -> Result<Dec, String> { d.checked_add(cent.mul_to_fit(Dec::from_int(1 + rng.below(5_000) as i64)).map_err(err)?).map_err(err) };
    let passes = u64::from(balances_days) * a_day;
    let mut when = start;
    for k in 1..passes {
        when = when.checked_add(every).map_err(err)?;
        let last = k + 1 == passes;
        // each change drawn before the pass is written
        let mut draws = Vec::new();
        for (_, key, value, margin) in &by_account {
            let cash = match true_cash.get(key) {
                Some(c) if last => Some(c.clone()),
                Some(c) if rng.below(1_000_000) < cash_chance => {
                    let mut c = c.clone();
                    if let Some((_, v)) = c.iter_mut().next() {
                        *v = moved(&mut rng, *v)?;
                    }
                    Some(c)
                }
                _ => None,
            };
            let value = match value {
                Some(v) if last => Some(*v),
                Some(v) if rng.below(1_000_000) < value_chance => Some(Money::new(moved(&mut rng, v.amount)?, v.currency)),
                _ => None,
            };
            let power = match true_power.get(key) {
                Some(Ok(p)) if *margin && last => Some(*p),
                Some(Ok(p)) if *margin && rng.below(1_000_000) < power_chance => Some(moved(&mut rng, *p)?),
                _ => None,
            };
            draws.push((cash, value, power));
        }
        book.atomically(|| {
            let (rv, rc, rp) = (book.broker_read(connection, "net-value", when)?, book.broker_read(connection, "cash", when)?, book.broker_read(connection, "buying-power", when)?);
            for ((id, _, _, _), (cash, value, power)) in by_account.iter().zip(&draws) {
                if let Some(v) = value {
                    book.store_net_value(*id, when, *v, &rv)?;
                }
                if let Some(c) = cash {
                    book.store_cash(*id, when, c, &rc)?;
                }
                if let Some(p) = power {
                    book.store_buying_power(*id, when, &Ok(Money::new(*p, Currency::CAD)), &rp)?;
                }
            }
            Ok(())
        })
        .map_err(err)?;
    }
    let rows = |b: &Book| -> Result<u64, String> { b.conn_for_tests().query_row("SELECT count(*) FROM statements", [], |r| r.get::<_, i64>(0)).map(|n| n as u64).map_err(err) };
    facts(home, &book, at)?;
    let first_account = by_account.first().map(|(id, _, _, _)| *id).ok_or("the made-up broker has no account")?;
    drop(book);
    // settled as a start settles it: the engine built, each round trip given its trade
    let f = crate::figures::Figures::open(home, at)?;
    // the older part of the activity imported again from Wealthsimple's own
    // activity export, through the import, as the owner's book holds it: what an
    // import is linked against
    let export = activity_export(adapter.source.rows())?;
    let open = || Ok(Box::new(std::io::Cursor::new(export.clone().into_bytes())) as Box<dyn std::io::BufRead>);
    crate::csv_import::import_from(&f, "activities.csv", &open, Some(first_account), at, &mut |_| crate::csv_import::Go::On).map_err(|e| format!("the activity export was not imported: {e:?}"))?;
    if size.deep > 0 {
        let coded = coded_history(size.deep / 2)?;
        let open = || Ok(Box::new(std::io::Cursor::new(coded.clone().into_bytes())) as Box<dyn std::io::BufRead>);
        crate::csv_import::import_from(&f, "coded.csv", &open, Some(first_account), at, &mut |_| crate::csv_import::Go::On).map_err(|e| format!("the listing named by code alone was not imported: {e:?}"))?;
    }
    f.record_changed(at)?;
    Ok(format!("{} rows, {} balances reads ({} a day), {} statement rows\n", report.rows_read, passes, a_day, rows(&f.book()?)?))
}

/// A second deep listing, by depth, imported from a file whose rows name its
/// exchange by code and state no venue in words, as a hand entry's rows and other
/// brokers' files do: the words a run reads are then looked up past sightings that
/// state none, which only the index of those that do state words skips (brief 45,
/// 1). Its symbol and the exchange's code.
const CODED: (&str, &str) = ("ZQCODE", "XTSE");
const CODED_HEADER: &str = "Date,Action,Symbol,Exchange,Quantity,Price,Amount,Currency\n";

/// `rounds` round trips of the `CODED` listing, one after another up to the
/// made-up today, each bought one day and sold the next; half the deep listing's
/// depth, so that one stays the deepest.
fn coded_history(rounds: usize) -> Result<String, String> {
    let mut text = String::from(CODED_HEADER);
    let today = made_up_today();
    for k in 0..rounds {
        let bought = today.checked_sub(SignedDuration::from_hours(24 * (3 * (rounds - k) as i64 + 1))).map_err(err)?;
        let sold = bought.checked_add(SignedDuration::from_hours(24)).map_err(err)?;
        text.push_str(&format!("{bought},Buy,{},{},10,20.00,-200.00,CAD\n{sold},Sell,{},{},10,21.00,210.00,CAD\n", CODED.0, CODED.1, CODED.0, CODED.1));
    }
    Ok(text)
}

/// The share of the feed's records the owner's book also holds from
/// Wealthsimple's activity export: 6,156 of 7,994, counted on the same copy.
pub const EXPORTED_SHARE: (usize, usize) = (6_156, 7_994);

/// Wealthsimple's activity export of the oldest `EXPORTED_SHARE` of the made-up
/// activity, written from the same replies the pull read, in the export's own
/// columns and words: its activity types and sub-types, a direction on every
/// row and a currency on every row (CAD on every coin's), as the owner's
/// imported export rows state them.
fn activity_export(rows: &[bagholder_core::json::Value]) -> Result<String, String> {
    use bagholder_core::json::Value;
    let field = |v: &Value, k: &str| match v {
        Value::Object(o) => match o.get(k) {
            Some(Value::String(t) | Value::Number(t)) => t.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    // the activity feed's type and sub-type, as the export writes them
    let words = |ty: &str, sub: &str| -> (String, String) {
        let (t, s) = match (ty, sub) {
            ("DIY_BUY", _) => ("Trade", "BUY"),
            ("DIY_SELL", _) => ("Trade", "SELL"),
            ("DIVIDEND", _) => ("Dividend", "dividend"),
            ("DEPOSIT", _) => ("Deposit", "deposit"),
            ("WITHDRAWAL", _) => ("Withdrawal", "withdrawal"),
            ("FUNDS_CONVERSION", _) => ("FxExchange", "fx"),
            ("INTEREST", _) => ("Interest", "interest"),
            ("INTERNAL_TRANSFER", _) => ("Transfer", "transfer"),
            ("OPTIONS_BUY", _) => ("OPTIONS_BUY", "BUYTOOPEN"),
            ("OPTIONS_SELL", _) => ("OPTIONS_SELL", "SELLTOCLOSE"),
            // the made-up options are bought, so one expiring leaves a long position
            ("OPTIONS_EXPIRY", _) => ("EXPIR", "SELL"),
            ("CRYPTO_STAKING_REWARD", _) | ("CREDIT_CARD_PAYMENT", _) => (ty, "other"),
            _ => (ty, sub),
        };
        (t.to_string(), s.to_string())
    };
    let toronto = bagholder_core::jiff::tz::TimeZone::get("America/Toronto").map_err(err)?;
    let mut rows: Vec<&Value> = rows.iter().collect();
    rows.sort_by_key(|r| field(r, "occurredAt"));
    let take = rows.len() * EXPORTED_SHARE.0 / EXPORTED_SHARE.1;
    let mut out = String::from("transaction_date,activity_type,activity_sub_type,direction,account_id,symbol,currency,quantity,net_cash_amount\n");
    for r in rows.into_iter().take(take) {
        let (ty, sub) = (field(r, "type"), field(r, "subType"));
        let amount = field(r, "amount");
        let out_of = field(r, "amountSign") == "negative" || matches!(ty.as_str(), "DIY_BUY" | "OPTIONS_BUY");
        let signed = if out_of && !amount.is_empty() { format!("-{amount}") } else { amount };
        let currency = match field(r, "currency") {
            c if c.is_empty() => "CAD".to_string(),
            c => c,
        };
        // the export states each day as Toronto's, where Wealthsimple states its days
        let day = field(r, "occurredAt").parse::<Timestamp>().map_err(|e| format!("a made-up row's time: {e}"))?.to_zoned(toronto.clone()).date().to_string();
        let (t, s) = words(&ty, &sub);
        // a contract named as the export names one: `UNDER 21NOV25 35.00 CALL`
        let symbol = match (field(r, "expiryDate").parse::<Date>(), field(r, "strikePrice")) {
            (Ok(expiry), strike) if !strike.is_empty() => format!("{} {} {strike} CALL", field(r, "assetSymbol"), expiry.strftime("%d%b%y").to_string().to_uppercase()),
            _ => field(r, "assetSymbol"),
        };
        let direction = if out_of { "DEBIT" } else { "CREDIT" };
        out.push_str(&[day, t, s, direction.to_string(), field(r, "accountId"), symbol, currency, field(r, "assetQuantity"), signed].join(","));
        out.push('\n');
    }
    Ok(out)
}

/// The market facts the figures read, through the stores' own writers: the
/// Bank's rate for every other currency, a quote for every listing, each
/// payer's record, and the benchmarks.
fn facts(home: &Path, book: &Book, at: Timestamp) -> Result<(), String> {
    let (cache, _) = MarketCache::open(&home.join(crate::figures::CACHE_FILE), crate::app::APP_VERSION, at).map_err(err)?;
    let source = SourceName::named("capacity");
    let bank = SourceName::named("bank-of-canada");
    let transactions = book.transactions().map_err(err)?;
    let first = transactions.iter().map(|t| t.trade_date).min().unwrap_or(made_up_today());
    let from = first.checked_sub(SignedDuration::from_hours(24 * 30)).map_err(err)?;
    // to the day the run is made on, so a value held today converts
    let to = Timestamp::now().to_zoned(bagholder_core::jiff::tz::TimeZone::get("America/Toronto").map_err(err)?).date().max(made_up_today());
    let days = |a: Date, b: Date| {
        let mut out = Vec::new();
        let mut d = a;
        while d <= b {
            if !matches!(d.weekday(), bagholder_core::jiff::civil::Weekday::Saturday | bagholder_core::jiff::civil::Weekday::Sunday) {
                out.push(d);
            }
            d = d.tomorrow().expect("a day after");
        }
        out
    };
    let instruments = book.instruments().map_err(err)?;
    for c in instruments.iter().map(|i| i.currency).filter(|c| *c != Currency::CAD).collect::<std::collections::BTreeSet<_>>() {
        let rates: Vec<(Date, Dec)> = days(from, to).into_iter().map(|d| (d, Dec::parse("1.35").expect("a rate"))).collect();
        book.store_rates(c, &rates, (from, to), &bank, at).map_err(err)?;
        book.store_rate_series(&[bagholder_book::facts::RateSeries { currency: c, source: bank.clone(), first_day: from, last_day: to, ended: false }], at).map_err(err)?;
    }
    // each listing quoted at the last price a fill of it stated, or its cost
    for i in &instruments {
        let price = transactions.iter().rev().find(|t| t.instrument == Some(i.id) && t.price.is_some()).and_then(|t| t.price).map(|p| p.amount).unwrap_or(Dec::ONE);
        cache.store_quote(&StoredQuote { instrument: i.id, source: source.clone(), price: Money::new(price, i.currency), change: None, change_pct: None, quoted_at: at, allowance: std::time::Duration::ZERO, received_at: at }).map_err(err)?;
    }
    // each payer quarterly, every payment it made declared a week before it was paid
    let mut paid: std::collections::BTreeMap<bagholder_core::InstrumentId, Vec<bagholder_book::facts::DeclaredRow>> = Default::default();
    for t in transactions.iter().filter(|t| t.kind == bagholder_core::transaction::Kind::Dividend) {
        let (Some(i), Some(cash)) = (t.instrument, t.cash) else { continue };
        let ex = t.trade_date.checked_sub(SignedDuration::from_hours(24 * 7)).map_err(err)?;
        paid.entry(i).or_default().push(bagholder_book::facts::DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: ex, record_date: Some(ex), pay_date: Some(t.trade_date), amount: Money::new(cash.amount.div_rounded(Dec::from_int(100), 6, bagholder_core::Rounding::HalfEven).map_err(err)?, cash.currency), reinvested: None });
    }
    for (i, rows) in &paid {
        book.store_declared(*i, rows, &source, at).map_err(err)?;
        book.store_frequency(*i, 4, &source, Some(made_up_today()), at).map_err(err)?;
    }
    for b in [bagholder_sources::contract::Benchmark::Sp500, bagholder_sources::contract::Benchmark::Tsx, bagholder_sources::contract::Benchmark::Tx60] {
        let closes: Vec<(Date, Dec)> = days(from, to).into_iter().enumerate().map(|(n, d)| (d, Dec::new(10_000 + n as i64, 2).expect("a close"))).collect();
        cache.store_benchmark_closes(b, &closes, &source, at).map_err(err)?;
    }
    Ok(())
}

// ---- probe: one measured run, in a process of its own ----

/// The app open on `home`, as a start opens it, with nothing running in the background.
fn opened(home: &Path) -> Result<Arc<crate::app::App>, String> {
    let app = crate::app::App::new(home.to_path_buf(), PathBuf::from("."), "127.0.0.1".into());
    let f = crate::figures::Figures::open(&app.home, Timestamp::now())?;
    app.set_figures(f);
    Ok(app)
}

fn figures(app: &crate::app::App) -> Result<&crate::figures::Figures, String> {
    app.figures.get().ok_or_else(|| "the figures are not open".to_string())
}

/// What a page showing every tab watches, as `web/src/lib/live.svelte.ts` says it.
const PAGE: [&str; 9] = ["status", "book", "notifications", "dashboard", "trades", "positions", "exposure", "markets", "cashflow"];

/// One measured run of `op` on the book in `home`. An added trade and an
/// import change the deepest listing on the depth pair's books (what grows
/// there), and the same listing a price change takes on the breadth pair's (a
/// choice that means the same at every size of book).
pub fn probe(op: Op, pair: Pair, min_rows: u64, home: &Path) -> Result<Measured, String> {
    // every connection from here on is counted, and every allocation
    bagholder_sqlite::work::start_counting();
    COUNT_ALLOCS.store(true, Ordering::SeqCst);
    if op == Op::Startup {
        let m = mark();
        let app = opened(home)?;
        figures(&app)?.read(|_| ()).ok_or("the figures were not built")?;
        return Ok(since(&m, 1));
    }
    let app = opened(home)?;
    let f = figures(&app)?;
    let held: Vec<(bagholder_core::InstrumentId, String, Currency)> = f
        .read(|e| {
            // the shares held, each by the name it trades under now: the one with
            // the longest history of closes first, then by name, so the change
            // measured is of the same size at every size of book, whichever ids a
            // build drew (an id is made from the moment it is made)
            let ledger = &e.inputs().ledger;
            let mut held = e
                .figures()
                .positions
                .iter()
                .filter_map(|p| {
                    let info = ledger.instruments.get(&p.instrument)?;
                    (info.instrument.kind == bagholder_core::instrument::InstrumentKind::Security).then_some(())?;
                    let closes = e.inputs().market.closes.get(&p.instrument).map_or(0, |c| c.len());
                    Some((closes, info.names.last()?.symbol.clone(), p.instrument, info.instrument.currency))
                })
                .collect::<Vec<_>>();
            held.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            held.dedup_by(|a, b| a.2 == b.2);
            held.into_iter().map(|(_, symbol, id, currency)| (id, symbol, currency)).collect::<Vec<_>>()
        })
        .ok_or("the figures were not built")?;
    let priced = held.len().max(1) as u64;
    let accounts = f.book()?.accounts().map_err(err)?.len().max(1) as u64;
    match op {
        Op::Startup => unreachable!("measured above"),
        Op::Brokers => {
            // what a balances read costs an account whose cash moved: the statement
            // kept, and that account's figures brought to it
            let book = f.book()?;
            // an account of the broker's, of the middle size among those holding anything, by the
            // number of its holdings, accounts of one size in the order of the
            // broker's own ids for them, which the generated book states: a choice
            // that means the same at every size of book and on every build, whatever
            // ids the build drew (ties broken by those drew a different account, and
            // a different cost, on each run)
            let names = f.names()?;
            let account = f
                .read(|e| {
                    let mut count: std::collections::BTreeMap<bagholder_core::AccountId, usize> = Default::default();
                    // the accounts the broker states (a statement for the manual account,
                    // which no broker states, cannot happen in use: brief 32, change 3)
                    let stated = |a: &bagholder_core::AccountId| e.inputs().ledger.accounts.get(a).is_some_and(|i| i.broker == bagholder_core::Broker::named("wealthsimple")) && names.account.contains_key(a);
                    for p in e.figures().positions.iter().filter(|p| stated(&p.account)) {
                        *count.entry(p.account).or_default() += 1;
                    }
                    let mut by_size: Vec<(usize, String, bagholder_core::AccountId)> = count.into_iter().map(|(a, n)| (n, names.account.get(&a).cloned().unwrap_or_default(), a)).collect();
                    by_size.sort();
                    by_size.get(by_size.len() / 2).map(|(_, _, a)| *a)
                })
                .flatten()
                .ok_or("nothing is held")?;
            let connection = book.connections().map_err(err)?.first().map(|c| c.id).ok_or("no connection")?;
            let at = Timestamp::now();
            let cash = std::collections::BTreeMap::from([(Currency::CAD, Dec::new(1_234_567, 2).map_err(err)?)]);
            let m = mark();
            let read = book.broker_read(connection, "cash", at).map_err(err)?;
            book.store_cash(account, at, &cash, &read).map_err(err)?;
            f.broker_changed(account)?;
            Ok(since(&m, accounts))
        }
        Op::PriceChanged => {
            let (instrument, _, currency) = held.first().cloned().ok_or("nothing is held")?;
            quote_moved(f, instrument, currency)?;
            let m = mark();
            f.price_changed(instrument)?;
            Ok(since(&m, priced))
        }
        Op::FeedStep => {
            let mut feed = crate::events::Feed::open(app.clone());
            let want = |k: &str| (k.to_string(), crate::events::Want { params: serde_json::json!({}), have: None });
            app.events.watch(&app, feed.id(), PAGE.iter().map(|k| want(k)).collect());
            // the page's first state, sent whole, is not the step measured
            feed.step(&crate::status::status);
            let (instrument, _, currency) = held.first().cloned().ok_or("nothing is held")?;
            quote_moved(f, instrument, currency)?;
            f.price_changed(instrument)?;
            app.events.signal();
            let m = mark();
            feed.step(&crate::status::status);
            Ok(since(&m, priced))
        }
        Op::AddTrade => {
            let ((instrument, symbol, currency), rows) = target(f, pair, min_rows, &held)?;
            // the page holds the broker's names: what a change does to them is in the window
            f.names()?;
            #[cfg(test)]
            let window = paths_mark(f, instrument);
            // the holding by its id, as the page names one it holds
            let req = crate::entries::EntryRequest::Trade { account: String::new(), instrument: Some(instrument.to_string()), symbol, exchange: String::new(), currency: currency.to_string(), day: made_up_today().to_string(), side: "buy".into(), quantity: "1".into(), price: "10".into(), fee: "0".into() };
            let m = mark();
            crate::entries::enter(f, &req, Timestamp::now()).map_err(|e| format!("{e:?}"))?;
            // a test's planted fold of the names, which the growth rule must catch
            #[cfg(test)]
            if PLANT_NAMES_FOLD.load(Ordering::SeqCst) {
                let id = target(f, pair, min_rows, &held)?.0 .0;
                f.book()?.names_as_folded(id).map_err(err)?;
            }
            let out = Measured { rows: Some(rows), ..since(&m, 1) };
            #[cfg(test)]
            paths_taken(f, window);
            Ok(out)
        }
        Op::BackdatedTrade | Op::BackdatedFill | Op::BackdatedRecord => {
            let ((instrument, symbol, currency), rows) = target(f, pair, min_rows, &held)?;
            f.names()?;
            let book = f.book()?;
            // the broker's distribution of the holding (a buy, for a back-dated fill) in
            // the middle of its history, by day, in the account whose history is deep:
            // of the accounts with such a record, the one holding most of its rows, never
            // another account that holds the listing too
            let kind = if op == Op::BackdatedFill { "buy" } else { "dividend" };
            let (record, day, account): (String, String, String) = {
                let c = book.conn_for_tests();
                let account: String = c
                    .query_row(
                        "SELECT t.account_id FROM transactions t WHERE t.instrument_id = ?1
                           AND t.account_id IN (SELECT d.account_id FROM transactions d JOIN source_records r ON r.id = d.record_id AND r.state = 'live' AND r.connection_id IS NOT NULL WHERE d.instrument_id = ?1 AND d.kind = ?2)
                         GROUP BY t.account_id ORDER BY count(*) DESC, t.account_id LIMIT 1",
                        [instrument.to_string(), kind.to_string()],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                let n: i64 = c
                    .query_row(
                        "SELECT count(*) FROM transactions t JOIN source_records r ON r.id = t.record_id AND r.state = 'live' AND r.connection_id IS NOT NULL WHERE t.instrument_id = ?1 AND t.kind = ?2 AND t.account_id = ?3",
                        [instrument.to_string(), kind.to_string(), account.clone()],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                let (record, day) = c
                    .query_row(
                        "SELECT t.record_id, t.trade_date FROM transactions t JOIN source_records r ON r.id = t.record_id AND r.state = 'live' AND r.connection_id IS NOT NULL
                         WHERE t.instrument_id = ?1 AND t.kind = ?3 AND t.account_id = ?4 ORDER BY t.trade_date, t.record_id LIMIT 1 OFFSET ?2",
                        rusqlite::params![instrument.to_string(), n / 2, kind, account],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(err)?;
                (record, day, account)
            };
            let m;
            if op == Op::BackdatedTrade {
                // a buy in the holding's own account on that day, its answer measured:
                // the later trades are derived again after it (`OVER_BUDGET`)
                let req = crate::entries::EntryRequest::Trade { account, instrument: Some(instrument.to_string()), symbol, exchange: String::new(), currency: currency.to_string(), day, side: "buy".into(), quantity: "1".into(), price: "10".into(), fee: "0".into() };
                m = mark();
                crate::entries::enter(f, &req, Timestamp::now()).map_err(|e| format!("{e:?}"))?;
            } else {
                // the same record stored again under a new id, as a pull stores a row the
                // broker adds late: a distribution re-forms the names and moves no
                // position; a buy moves it, and every later trade is derived again
                let id = bagholder_core::RecordId::parse(&record).map_err(err)?;
                let rec = book.record(id).map_err(err)?;
                let payload = book.revisions(id).map_err(err)?.pop().map(|(_, _, p)| p).ok_or("the record has no revision")?;
                let key = format!("{}-late", rec.source_key);
                m = mark();
                book.store(&bagholder_wealthsimple::mapping::WealthsimpleMapping, &bagholder_book::records::Incoming { connection: rec.connection, source_key: &key, payload: &payload, refs: vec![] }, Timestamp::now()).map_err(err)?;
                f.record_changed(Timestamp::now())?;
            }
            Ok(Measured { rows: Some(rows), ..since(&m, 1) })
        }
        Op::BackdatedFileRow => {
            f.names()?;
            let book = f.book()?;
            // the listing whose history is file rows stating its exchange by code alone
            let (instrument, rows, day, account): (String, u64, String, String) = {
                let c = book.conn_for_tests();
                let instrument: String = c
                    .query_row("SELECT instrument_id FROM instrument_names WHERE symbol = ?1 AND venue_mic IS NOT NULL LIMIT 1", [CODED.0], |r| r.get(0))
                    .map_err(|e| format!("the listing named by code alone ({}) is not in the book: {e}", CODED.0))?;
                let n: i64 = c.query_row("SELECT count(*) FROM transactions WHERE instrument_id = ?1", [&instrument], |r| r.get(0)).map_err(err)?;
                let (day, account) = c
                    .query_row(
                        "SELECT trade_date, account_id FROM transactions WHERE instrument_id = ?1 ORDER BY trade_date, record_id LIMIT 1 OFFSET ?2",
                        rusqlite::params![instrument, n / 2],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(err)?;
                (instrument, n as u64, day, account)
            };
            if rows < min_rows / 2 {
                return Err(format!("the listing named by code alone has {rows} rows, below the {} the book was built with", min_rows / 2));
            }
            let _ = instrument;
            let account = bagholder_core::AccountId::parse(&account).map_err(err)?;
            // a distribution of it, dated in the middle of that history
            let text = format!("{CODED_HEADER}{day},Dividend,{},{},,,1.00,CAD\n", CODED.0, CODED.1);
            let open = || Ok(Box::new(std::io::Cursor::new(text.clone().into_bytes())) as Box<dyn std::io::BufRead>);
            let m = mark();
            crate::csv_import::import_from(f, "late.csv", &open, Some(account), Timestamp::now(), &mut |_| crate::csv_import::Go::On).map_err(|e| format!("{e:?}"))?;
            Ok(Measured { rows: Some(rows), ..since(&m, 1) })
        }
        Op::Import => {
            let mut text = String::from("Date,Action,Symbol,Quantity,Price,Amount,Currency\n");
            let ((instrument, symbol, currency), rows) = target(f, pair, min_rows, &held)?;
            f.names()?;
            #[cfg(test)]
            let window = paths_mark(f, instrument);
            #[cfg(not(test))]
            let _ = instrument; // what the test's record of the window keys on
            for n in 0..import_rows(pair) {
                text.push_str(&format!("{},Buy,{symbol},1,{}.00,-{}.00,{currency}\n", made_up_today(), 10 + n, 10 + n));
            }
            let open = || Ok(Box::new(std::io::Cursor::new(text.clone().into_bytes())) as Box<dyn std::io::BufRead>);
            let m = mark();
            crate::csv_import::import_from(f, "capacity.csv", &open, None, Timestamp::now(), &mut |_| crate::csv_import::Go::On).map_err(|e| format!("{e:?}"))?;
            // a test's planted read of every file row, which the growth rule must catch
            #[cfg(test)]
            if PLANT_WHOLE_READ.load(Ordering::SeqCst) {
                f.book()?.live_records(&bagholder_broker::csv::source()).map_err(err)?;
            }
            let out = Measured { rows: Some(rows), ..since(&m, 1) };
            #[cfg(test)]
            paths_taken(f, window);
            Ok(out)
        }
    }
}

/// The listing an added trade or an import changes on `pair`'s books, and its
/// rows: on the depth pair's, the deepest held, refused below the planted depth
/// `min_rows`; on the breadth pair's, the held listing of median depth, as deep
/// at every size of book (rows per listing stay as they are).
fn target(f: &crate::figures::Figures, pair: Pair, min_rows: u64, held: &[(bagholder_core::InstrumentId, String, Currency)]) -> Result<((bagholder_core::InstrumentId, String, Currency), u64), String> {
    let rows = f.read(|e| held.iter().map(|h| e.inputs().ledger.transactions.of_instrument(h.0).count() as u64).collect::<Vec<_>>()).ok_or("the figures were not built")?;
    let mut by_depth: Vec<(u64, &(bagholder_core::InstrumentId, String, Currency))> = rows.into_iter().zip(held.iter()).collect();
    by_depth.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1 .1.cmp(&b.1 .1)));
    let (n, h) = match pair {
        Pair::Depth => *by_depth.last().ok_or("nothing is held")?,
        Pair::Breadth => *by_depth.get(by_depth.len() / 2).ok_or("nothing is held")?,
    };
    if n < min_rows {
        return Err(format!("the deepest listing held has {n} rows, below the {min_rows} the book was built with"));
    }
    Ok((h.clone(), n))
}

/// A new quote for one listing, a cent above its last.
fn quote_moved(f: &crate::figures::Figures, instrument: bagholder_core::InstrumentId, currency: Currency) -> Result<(), String> {
    let cache = f.cache()?;
    let last = cache.quotes().map_err(err)?.into_iter().find(|q| q.instrument == instrument).map(|q| q.price.amount).unwrap_or(Dec::ONE);
    let now = Timestamp::now();
    let price = last.checked_add(Dec::new(1, 2).map_err(err)?).map_err(err)?;
    cache.store_quote(&StoredQuote { instrument, source: SourceName::named("capacity"), price: Money::new(price, currency), change: None, change_pct: None, quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now }).map_err(err)
}

// ---- run: both sizes, every operation, the table and the verdict ----

/// The budget of an operation on a Pi, in milliseconds, at the size measured
/// (its divisor), with what it comes from.
pub fn budget_ms(op: Op, divisor: u64) -> (f64, &'static str) {
    match op {
        Op::Startup => (STARTUP_MS as f64, "RAIL load"),
        Op::AddTrade => (ANSWER_MS as f64, "RAIL response (answered with its figures)"),
        Op::Import => (TO_FIGURES_MS as f64, "RAIL flow"),
        Op::BackdatedTrade => (ANSWER_MS as f64, "RAIL response (answered with its figures)"),
        // a row the pull brings: its figures by the next balances read
        Op::BackdatedFill | Op::BackdatedRecord => (BALANCES_EVERY_MS as f64 / divisor as f64, "balances cadence / accounts"),
        Op::BackdatedFileRow => (TO_FIGURES_MS as f64, "RAIL flow"),
        Op::Brokers => (BALANCES_EVERY_MS as f64 / divisor as f64, "balances cadence / accounts"),
        Op::PriceChanged => (QUOTES_EVERY_MS as f64 / divisor as f64, "quotes cadence / priced holdings"),
        Op::FeedStep => (QUOTES_EVERY_MS as f64 / divisor as f64, "quotes cadence / signals a pass sends"),
    }
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn measure(exe: &Path, op: Op, pair: Pair, min_rows: u64, home: &Path, times: usize) -> Result<Measured, String> {
    let mut runs = Vec::new();
    for _ in 0..times {
        // every probe writes (a quote, a trade, an import, what a start settles),
        // so each runs on a fresh copy and the built book stays as it was built
        let copy = home.with_extension("probe");
        copy_folder(home, &copy)?;
        let exec = std::time::Instant::now();
        let out = std::process::Command::new(exe).args(["capacity", "probe", op.name()]).arg(&copy).args(pair.flag(min_rows)).env("BAGHOLDER_OFFLINE", "1").env("BAGHOLDER_DRY_ORDERS", "1").output().map_err(err)?;
        std::fs::remove_dir_all(&copy).map_err(err)?;
        if !out.status.success() {
            return Err(format!("{} on {}: {}", op.name(), home.display(), String::from_utf8_lossy(&out.stderr)));
        }
        let line = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| format!("{}: {e}: {line}", op.name()))?;
        let n = |k: &str| v[k].as_u64();
        // startup is timed from the exec, as a person starting the app waits on it
        let wall_ns = if op == Op::Startup { exec.elapsed().as_nanos() as u64 } else { n("wall_ns").unwrap_or(0) };
        runs.push(Measured { wall_ns, steps: n("steps").unwrap_or(0), allocs: n("allocs").unwrap_or(0), read: n("read"), divisor: n("divisor").unwrap_or(1), rows: n("rows") });
    }
    Ok(Measured {
        wall_ns: median(runs.iter().map(|r| r.wall_ns).collect()),
        steps: median(runs.iter().map(|r| r.steps).collect()),
        allocs: median(runs.iter().map(|r| r.allocs).collect()),
        read: runs.iter().map(|r| r.read).collect::<Option<Vec<_>>>().map(median),
        divisor: runs[0].divisor,
        rows: runs[0].rows,
    })
}

/// A data folder copied whole into `to`, replacing what was there.
fn copy_folder(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        std::fs::remove_dir_all(to).map_err(err)?;
    }
    std::fs::create_dir_all(to).map_err(err)?;
    for entry in std::fs::read_dir(from).map_err(err)? {
        let entry = entry.map_err(err)?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(err)?.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(err)?;
            // on disk before the probe starts: a copy still waiting to be written
            // back is written by the first sync the probe makes, and that is the
            // copy's cost, not the operation's
            std::fs::File::open(&target).and_then(|f| f.sync_all()).map_err(err)?;
        }
    }
    Ok(())
}

/// One operation's verdict.
#[derive(Debug, PartialEq)]
pub struct Verdict {
    pub op: Op,
    pub pair: Pair,
    /// Each measure over, with what it measured.
    pub over: Vec<(Measure, String)>,
}

impl Verdict {
    #[cfg(test)]
    /// Whether the growth rule is broken by steps or allocations: what the process
    /// reads from storage is judged at the owner's size only, by the capacity job.
    fn grew(&self) -> Vec<&(Measure, String)> {
        self.over.iter().filter(|(m, _)| matches!(m, Measure::Steps | Measure::Allocations)).collect()
    }
}

/// What breaks the budget or the growth rule, from the two sizes' measurements.
pub fn judge(op: Op, pair: Pair, one: &Measured, four: &Measured) -> Verdict {
    let mut over = Vec::new();
    let pi_ms = four.wall_ns as f64 / 1e6 * PI_SLOWDOWN;
    let (budget, _) = budget_ms(op, four.divisor);
    if op.waited_on() && pi_ms > budget {
        over.push((Measure::Time, format!("{:.0} ms on a Pi at four times the owner's size, over {budget:.0} ms", pi_ms)));
    }
    if op.per_change() {
        let ratio = |a: u64, b: u64| if a == 0 { if b == 0 { 1.0 } else { f64::INFINITY } } else { b as f64 / a as f64 };
        for (what, a, b) in [(Measure::Steps, Some(one.steps), Some(four.steps)), (Measure::Allocations, Some(one.allocs), Some(four.allocs)), (Measure::BytesRead, one.read, four.read)] {
            if let (Some(a), Some(b)) = (a, b) {
                let r = ratio(a, b);
                if r >= GROWTH_LIMIT {
                    over.push((what, format!("{} grew {r:.2} times with the book", what.name())));
                }
            }
        }
    }
    Verdict { op, pair, over }
}

/// The run fails on an operation off the list that is over, and on one on the
/// list that is not.
pub fn listed_disagree(verdicts: &[Verdict]) -> Vec<String> {
    let mut out = Vec::new();
    for v in verdicts {
        for m in [Measure::Time, Measure::Steps, Measure::Allocations, Measure::BytesRead] {
            let said: Vec<&str> = v.over.iter().filter(|(o, _)| *o == m).map(|(_, s)| s.as_str()).collect();
            match (listed(v.op, v.pair, m), said.is_empty()) {
                (false, false) => out.push(format!("{} ({}) is over budget: {}", v.op.name(), v.pair.name(), said.join("; "))),
                (true, true) => out.push(format!("{} ({}) is within its budget by {} now: take it off OVER_BUDGET and close its issue", v.op.name(), v.pair.name(), m.name())),
                _ => {}
            }
        }
    }
    out
}

pub fn run(work: &Path) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(err)?;
    for pair in [Pair::Breadth, Pair::Depth] {
        let (one, four) = pair.folders(work);
        for (dir, times) in [(&one, 1), (&four, 4)] {
            if !dir.join(bagholder_book::BOOK_FILE).exists() {
                build(dir, pair.size(times), BALANCES_DAYS)?;
            }
        }
    }
    // each row printed as it is measured, so a long run shows where it is
    let header = "| Operation | Grows by | Rows ×1 → ×4 | Runner (ms) | Pi (ms) | Budget (ms) | From | VM steps ×1 → ×4 | Allocations ×1 → ×4 | Bytes read ×1 → ×4 |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n";
    write_out(header)?;
    let mut verdicts = Vec::new();
    for pair in [Pair::Breadth, Pair::Depth] {
        let (one, four) = pair.folders(work);
        for &op in pair.ops() {
            // an operation listed over budget is measured once: its counts are the
            // same every run, and one run over its budget keeps it over; only a run
            // within it is taken to the median of seven, to say it is off the list
            // every operation the median of seven runs, listed or not (§16)
            let a = measure(&exe, op, pair, pair.min_rows(1), &one, RUNS)?;
            let b = measure(&exe, op, pair, pair.min_rows(4), &four, RUNS)?;
            let verdict = judge(op, pair, &a, &b);
            let rows = match (a.rows, b.rows) {
                (Some(x), Some(y)) => format!("{x} → {y}"),
                _ => "".into(),
            };
            let pi = b.wall_ns as f64 / 1e6 * PI_SLOWDOWN;
            let over = if verdict.over.iter().any(|(m, _)| *m == Measure::Time) { " (over)" } else { "" };
            let (budget, from) = budget_ms(op, b.divisor);
            let read = match (a.read, b.read) {
                (Some(x), Some(y)) => format!("{x} → {y}"),
                _ => "not counted here".into(),
            };
            write_out(&format!(
                "| {} | {} | {rows} | {:.1} | {pi:.1}{over} | {budget:.1} | {from} | {} → {} | {} → {} | {read} |\n",
                op.name(),
                pair.name(),
                b.wall_ns as f64 / 1e6,
                a.steps,
                b.steps,
                a.allocs,
                b.allocs
            ))?;
            verdicts.push(verdict);
        }
    }
    let wrong = listed_disagree(&verdicts);
    if wrong.is_empty() {
        Ok(String::from("\nEvery operation is within its budget and the growth rule, or listed over it.\n"))
    } else {
        Err(format!("\n{}", wrong.join("\n")))
    }
}

fn write_out(text: &str) -> Result<(), String> {
    use std::io::Write as _;
    print!("{text}");
    std::io::stdout().flush().map_err(err)
}

pub fn cli(args: &[String]) -> i32 {
    let usage = "usage: bagholder capacity build <folder> [--times N] [--seed S] | run <work folder> | probe <operation> <folder> [--depth <rows>]";
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
    let result = match args.first().map(String::as_str) {
        Some("build") => match args.get(1) {
            Some(dir) => build(Path::new(dir), sized(flag("--times").unwrap_or(1) as usize, flag("--seed").unwrap_or(OWNER.seed)), BALANCES_DAYS),
            None => Err(usage.into()),
        },
        Some("run") => match args.get(1) {
            Some(dir) => run(Path::new(dir)),
            None => Err(usage.into()),
        },
        Some("probe") => match (args.get(1).and_then(|o| Op::parse(o)), args.get(2)) {
            (Some(op), Some(dir)) => {
                let (pair, min_rows) = match args.iter().position(|a| a == "--depth") {
                    Some(i) => (Pair::Depth, args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0)),
                    None => (Pair::Breadth, 0),
                };
                probe(op, pair, min_rows, Path::new(dir)).map(|m| format!("{}\n", serde_json::json!({"wall_ns": m.wall_ns, "steps": m.steps, "allocs": m.allocs, "read": m.read, "divisor": m.divisor, "rows": m.rows})))
            }
            _ => Err(usage.into()),
        },
        _ => Err(usage.into()),
    };
    match result {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Size = Size { accounts: 12, instruments: 16, trades: 40, months: 4, seed: 3, deep: 0 };
    const BROAD: Size = Size { accounts: SMALL.accounts * 32, instruments: SMALL.instruments * 32, trades: SMALL.trades * 32, ..SMALL };
    const DEEP: Size = Size { deep: 320, ..SMALL };

    /// The families a probe of `op` matched, and whether each was continued: the
    /// probe run on a copy, its paths taken from this thread.
    fn probe_paths(op: Op, pair: Pair, min_rows: u64, home: &Path) -> (Vec<(bagholder_engine::families::Node, bool)>, Measured) {
        let c = tempfile::tempdir().unwrap();
        copy_folder(home, c.path()).unwrap();
        let m = probe(op, pair, min_rows, c.path()).unwrap();
        (PROBED_PATHS.with(|p| std::mem::take(&mut *p.borrow_mut())), m)
    }

    fn counts(home: &Path) -> Vec<(String, i64)> {
        let c = rusqlite::Connection::open(home.join(bagholder_book::BOOK_FILE)).unwrap();
        let tables: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        tables.into_iter().map(|t| { let n = c.query_row(&format!("SELECT count(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap(); (t, n) }).collect()
    }

    /// Every trade's and holding's figures, without the ids a build gives anew
    /// (each id is made from the moment it is made), in a fixed order.
    fn figures_digest(home: &Path) -> Vec<String> {
        let f = crate::figures::Figures::open(home, Timestamp::now()).unwrap();
        let mut out = f
            .read(|e| {
                let fig = e.figures();
                let trades = fig.trades.iter().map(|t| format!("trade {:?}", (t.opened_on, t.closed_on, t.currency, t.direction, &t.status, &t.qty, &t.entry, &t.exit, &t.basis, &t.pnl, &t.pnl_cad, &t.fees)));
                let held = fig.positions.iter().map(|p| format!("held {:?}", (p.currency, p.direction, &p.qty, &p.book, &p.avg, &p.market, &p.unrealized, &p.book_cad)));
                trades.chain(held).collect::<Vec<_>>()
            })
            .unwrap();
        out.sort();
        out
    }

    /// The same seed and size make the same book: what the CI's cached books rest on.
    #[test]
    fn test_the_same_seed_and_size_build_the_same_book() {
        let _g = crate::tests_common::guard();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        build(a.path(), SMALL, 3).unwrap();
        build(b.path(), SMALL, 3).unwrap();
        let (ca, cb) = (counts(a.path()), counts(b.path()));
        assert!(ca.iter().any(|(t, n)| t == "transactions" && *n > 0), "{ca:?}");
        assert_eq!(ca, cb);
        // every row, the feed's and the export's, is read with no problem
        assert!(ca.iter().any(|(t, n)| t == "record_problems" && *n == 0), "{ca:?}");
        let (fa, fb) = (figures_digest(a.path()), figures_digest(b.path()));
        assert!(fa.iter().any(|l| l.starts_with("trade")) && fa.iter().any(|l| l.starts_with("held")), "{fa:?}");
        assert_eq!(fa, fb);
    }

    /// The activity export imported over the feed it repeats links row for row:
    /// a share's rows to the share although the broker names each contract on it
    /// by the share's symbol, and a contract's rows, written by its terms, to the
    /// contract the feed made. What stays unlinked is only the asset movements,
    /// which the import reads as cash and the feed as holdings (#405).
    #[test]
    fn test_the_activity_export_links_to_the_feed_it_repeats() {
        let _g = crate::tests_common::guard();
        let home = tempfile::tempdir().unwrap();
        build(home.path(), SMALL, 1).unwrap();
        let c = rusqlite::Connection::open(home.path().join(bagholder_book::BOOK_FILE)).unwrap();
        let n = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        // the case is there: a contract on a share the book also holds, both seen under the share's symbol
        assert!(n("SELECT count(*) FROM option_terms o JOIN instruments u ON u.id = o.underlying_id AND u.kind = 'security'") > 0);
        assert!(n("SELECT count(*) FROM source_records WHERE source = 'csv'") > 0);
        let unlinked: Vec<String> = c
            .prepare("SELECT json_extract(v.payload, '$.cells.activity_type') FROM source_records r JOIN record_revisions v ON v.record_id = r.id WHERE r.source = 'csv' AND r.state = 'live'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(unlinked.iter().all(|t| t == "ASSET_MOVEMENT"), "{unlinked:?}");
        assert_eq!(n("SELECT count(*) FROM instruments i WHERE NOT EXISTS (SELECT 1 FROM instrument_sightings s JOIN source_records r ON r.id = s.record_id WHERE s.instrument_id = i.id AND r.source = 'wealthsimple')"), 0, "the import named an instrument the feed did not");
    }

    /// A pull reaches the figures in one write: its rows and the statements read
    /// with them under one version, so no page is ever sent a state between them
    /// (`docs/decisions.md`, 2026-10-09). And after a pull, a pull of the activity
    /// alone and a balances read, applied as the server applies each, the engine
    /// holds exactly what the whole read of the book's statements gives (brief 22, 5 and 6).
    #[test]
    fn test_a_pull_reaches_the_figures_in_one_write_and_every_read_leaves_them_as_the_book_states() {
        let _g = crate::tests_common::guard();
        let home = tempfile::tempdir().unwrap();
        let today = made_up_today();
        let at: Timestamp = format!("{today}T20:00:00Z").parse().unwrap();
        let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, at).unwrap();
        let ws = Broker::named("wealthsimple");
        let connection = book.add_connection(&ws, "Wealthsimple", at).unwrap();
        drop(book);
        let f = crate::figures::Figures::open(home.path(), at).unwrap();
        f.state_zone("America/Toronto", at).unwrap();
        let book = f.book().unwrap();
        let oracle = |what: &str| {
            let whole = crate::engine_inputs::brokers(&f.book().unwrap()).unwrap();
            let held = f.read(|e| e.inputs().market.brokers.clone()).unwrap();
            assert_eq!(held, whole, "after {what}: the engine holds what the book states");
        };
        let mut adapter = Wealthsimple::new(Generated::new(SMALL, today));
        let before = f.version();
        let report = pull::pull(&book, &mut adapter, connection, today, at, &mut |_| {}).unwrap();
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(!f.pulled(at, &report.stated_for).unwrap().is_empty());
        assert_eq!(f.version(), before + 1, "the rows and their statements in one write");
        oracle("a pull");
        let later = at.checked_add(SignedDuration::from_hours(1)).unwrap();
        let report = pull::pull_activity(&book, &mut adapter, connection, today, later, &mut |_| {}).unwrap();
        f.pulled(later, &report.stated_for).unwrap();
        oracle("a pull of the activity");
        let read = pull::balances(&book, &mut adapter, connection, later.checked_add(SignedDuration::from_mins(5)).unwrap()).unwrap();
        for a in &read.accounts {
            f.broker_changed(*a).unwrap();
        }
        oracle("a balances read");
    }

    /// The broker check's moment for a cash stated again unchanged is its last
    /// (brief 22, 1): the broker states the book's cash plus ten at t1, the activity
    /// is read in full at t2, the broker states the same again at t3. The cash
    /// stated after the read confirms the difference: it is said, not pending.
    #[test]
    fn test_a_difference_the_broker_states_again_after_the_read_is_confirmed_through_a_folded_period() {
        let _g = crate::tests_common::guard();
        let home = tempfile::tempdir().unwrap();
        build(home.path(), SMALL, 1).unwrap();
        let f = crate::figures::Figures::open(home.path(), Timestamp::now()).unwrap();
        let book = f.book().unwrap();
        let (account, own) = f
            .read(|e| e.figures().matched.cash.iter().find_map(|(a, c)| c.get(&Currency::CAD).and_then(|v| v.as_ref().ok()).map(|v| (*a, *v))))
            .flatten()
            .unwrap();
        let connection = book.connections().unwrap()[0].id;
        let stated = std::collections::BTreeMap::from([(Currency::CAD, own.checked_add(Dec::from_int(10)).unwrap())]);
        let (t1, t2, t3): (Timestamp, Timestamp, Timestamp) = ("2026-10-09T10:00:00Z".parse().unwrap(), "2026-10-09T10:05:00Z".parse().unwrap(), "2026-10-09T10:10:00Z".parse().unwrap());
        let r1 = book.broker_read(connection, "cash", t1).unwrap();
        book.store_cash(account, t1, &stated, &r1).unwrap();
        book.note_activity_read(account, t2, true).unwrap();
        let r3 = book.broker_read(connection, "cash", t3).unwrap();
        book.store_cash(account, t3, &stated, &r3).unwrap();
        f.broker_changed(account).unwrap();
        let cad = f
            .read(|e| e.figures().checks.iter().find(|c| c.account == account).and_then(|c| c.differences.iter().find(|d| matches!(d, bagholder_engine::equity::Difference::Cash { currency: Currency::CAD, .. })).cloned()))
            .flatten()
            .expect("the difference of ten");
        assert!(!cad.pending(), "stated again after the read: confirmed, {cad:?}");
    }

    /// Every operation is measured on a small book and sixteen times it (at these
    /// sizes an operation's own work is large beside a whole read, so the larger
    /// step is what lets the growth rule, under twice, see one), by breadth
    /// and by depth, in this process, and none off `OVER_BUDGET` breaks the growth
    /// rule there; the budget itself, and whether a listed one now meets it, is the
    /// CI job's, at the owner's size, on the runner it is stated for.
    #[test]
    fn test_every_operation_is_measured_and_the_growth_rule_judged() {
        let _g = crate::tests_common::guard();
        // a book and thirty-two times it by breadth, and one listing twenty and
        // three hundred and twenty round trips deep: large enough beside an
        // operation's own work that one whole read is past the rule (brief 28, 14
        // and E)
        let sizes = [
            (Pair::Breadth, SMALL, BROAD, (0, 0)),
            (Pair::Depth, Size { deep: 20, ..SMALL }, DEEP, (39, 639)),
        ];
        for (pair, small, large, min) in sizes {
            let (one, four) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            build(one.path(), small, 2).unwrap();
            build(four.path(), large, 2).unwrap();
            for &op in pair.ops().iter().filter(|o| o.per_change()) {
                let a = probe(op, pair, min.0, one.path()).unwrap();
                let paths_a = PROBED_PATHS.with(|p| std::mem::take(&mut *p.borrow_mut()));
                let b = probe(op, pair, min.1, four.path()).unwrap();
                let paths_b = PROBED_PATHS.with(|p| std::mem::take(&mut *p.borrow_mut()));
                assert!(a.steps > 0 || op == Op::FeedStep, "{op:?} counted no work: {a:?}");
                let v = judge(op, pair, &a, &b);
                // steps and allocations: what the process reads from storage at
                // these small sizes is what this run's page cache holds, judged by
                // the capacity job at the owner's size instead
                let growth: Vec<_> = v.grew().into_iter().filter(|(m, _)| !listed(op, pair, *m)).collect();
                assert!(growth.is_empty(), "{op:?} ({pair:?}): {growth:?} ({a:?} then {b:?})");
                // an added trade and an import match the listing's family once, by
                // continuing it, on each book (brief 30, C4)
                if matches!(op, Op::AddTrade | Op::Import) {
                    for (book, paths) in [("small", &paths_a), ("large", &paths_b)] {
                        assert!(paths.len() == 1 && paths[0].1, "{op:?} ({pair:?}, the {book} book): the family's matches in the window were {paths:?}");
                    }
                }
            }
        }
    }

    /// The growth rule catches a whole pass in the import's window (brief 27, 9):
    /// today's whole-ledger linking, planted where the test build's check used to
    /// run, fails it; that check now stays out of the window, so without the plant
    /// the import passes. The books are a small one and sixteen times it.
    #[test]
    fn a_whole_pass_planted_in_the_import_fails_the_growth_rule() {
        let _g = crate::tests_common::guard();
        let (one, four) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        build(one.path(), SMALL, 2).unwrap();
        build(four.path(), BROAD, 2).unwrap();
        let measured = |plant: bool| {
            PLANT_WHOLE_READ.store(plant, Ordering::SeqCst);
            let copy = |d: &Path| {
                let c = tempfile::tempdir().unwrap();
                copy_folder(d, c.path()).unwrap();
                c
            };
            let (a, b) = (copy(one.path()), copy(four.path()));
            let (x, y) = (probe(Op::Import, Pair::Breadth, 0, a.path()).unwrap(), probe(Op::Import, Pair::Breadth, 0, b.path()).unwrap());
            PLANT_WHOLE_READ.store(false, Ordering::SeqCst);
            (!judge(Op::Import, Pair::Breadth, &x, &y).grew().is_empty(), ())
        };
        assert!(!measured(false).0, "the import grows with the book without the plant");
        assert!(measured(true).0, "one read of every file row in the window went unseen");
    }

    /// The growth rule catches the names folded from every sighting in an added
    /// trade's window on the depth pair: kept as runs, they cost the same at any
    /// depth, and the fold planted beside them fails the rule.
    #[test]
    fn the_names_folded_whole_in_an_added_trade_fail_the_growth_rule() {
        let _g = crate::tests_common::guard();
        let (one, deep) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        build(one.path(), Size { deep: 20, ..SMALL }, 2).unwrap();
        build(deep.path(), DEEP, 2).unwrap();
        let measured = |plant: bool| {
            PLANT_NAMES_FOLD.store(plant, Ordering::SeqCst);
            let (_, x) = probe_paths(Op::AddTrade, Pair::Depth, 39, one.path());
            let (_, y) = probe_paths(Op::AddTrade, Pair::Depth, 639, deep.path());
            PLANT_NAMES_FOLD.store(false, Ordering::SeqCst);
            !judge(Op::AddTrade, Pair::Depth, &x, &y).grew().is_empty()
        };
        assert!(!measured(false), "an added trade grows with the listing's depth without the plant");
        assert!(measured(true), "the names folded whole in the window went unseen");
    }

    /// The verdict: a listed operation within its budget fails the run as an
    /// unlisted one over it does.
    #[test]
    fn test_an_operation_over_budget_fails_unless_listed_and_a_listed_one_within_fails_too() {
        let small = Measured { wall_ns: 1_000, steps: 100, allocs: 100, read: Some(1_000), divisor: 10, rows: None };
        let linear = Measured { wall_ns: 1_000, steps: 400, allocs: 100, read: Some(1_000), divisor: 40, rows: None };
        let v = judge(Op::PriceChanged, Pair::Breadth, &small, &linear);
        assert!(v.over.iter().any(|(m, o)| *m == Measure::Steps && o.contains("steps grew 4.00 times")), "{v:?}");
        assert!(judge(Op::PriceChanged, Pair::Breadth, &small, &small).over.is_empty());
        let slow = Measured { wall_ns: 10_000_000_000, ..small };
        assert!(judge(Op::Startup, Pair::Breadth, &small, &slow).over.iter().any(|(m, o)| *m == Measure::Time && o.contains("over 5000 ms")));
        // a background operation's wall time is reported, never judged
        assert!(judge(Op::Brokers, Pair::Breadth, &small, &Measured { wall_ns: 10_000_000_000, ..small }).over.is_empty());
        let off_list = listed_disagree(&[Verdict { op: Op::PriceChanged, pair: Pair::Breadth, over: vec![(Measure::Steps, "x".into())] }]);
        assert_eq!(off_list.len(), usize::from(!listed(Op::PriceChanged, Pair::Breadth, Measure::Steps)));
        // a listing names its measure: one listed by allocations is still held to its steps
        let (op, pair, listed_by, _) = OVER_BUDGET[0];
        let op = Op::parse(op).unwrap();
        let other = [Measure::Steps, Measure::Allocations].into_iter().find(|m| *m != listed_by).unwrap();
        let said = listed_disagree(&[Verdict { op, pair, over: vec![(listed_by, "x".into()), (other, "y".into())] }]);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("y"), "{said:?}");
        // and one listed but no longer over by its measure fails the run
        let said = listed_disagree(&[Verdict { op, pair, over: vec![] }]);
        assert!(said.iter().any(|s| s.contains("within its budget")), "{said:?}");
    }
}
