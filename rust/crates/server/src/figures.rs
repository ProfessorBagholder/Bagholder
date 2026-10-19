//! The figure path (`docs/plans/stage-3c-switch.md`, §2): the book, the market
//! cache and the engine, held by the server and kept current by applying each
//! change to the engine, which recomputes only what the change touches.
//!
//! The book and the cache are SQLite files in the data folder; each thread that
//! reads or writes them opens its own connection (SQLite serializes the writes),
//! so a reader waiting on the network never holds another writer up. The engine
//! is one, behind a lock: a writer stores its change, then applies the part of
//! the inputs it changed, read back from the book or the cache.
//!
//! No figure is built before a zone is known: the zone is the one the page in
//! use states (`docs/decisions.md`, 2026-09-25, time), kept in the book, and the
//! engine's days, months and "today" are in it. The server's own zone is never
//! read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use bagholder_book::Book;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::InstrumentId;
use bagholder_engine::engine::{Change, Moved};
use bagholder_engine::input::{Clock, Inputs, Market};
use bagholder_engine::Engine;
use bagholder_sources::cache::MarketCache;

use crate::engine_inputs;

/// The market cache's file in the data folder.
pub const CACHE_FILE: &str = "market.db";
/// The old store's file, imported once into a new book.
pub const OLD_FILE: &str = "bagholder.db";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The zone the Bank of Canada publishes its rates in (its 16:30 Eastern), as
/// the Bank states it: a fact of the source, not the person's zone.
pub fn bank_zone() -> Result<TimeZone, String> {
    TimeZone::get("America/Toronto").map_err(err)
}

/// The engine's clock at `now`, with the person's days in `zone`.
pub fn clock(zone: &TimeZone, now: Timestamp) -> Result<Clock, String> {
    Ok(Clock { today: now.to_zoned(zone.clone()).date(), now, home: zone.clone(), bank: bank_zone()? })
}

/// The market as the cache and the book hold it: quotes, closes and the
/// benchmarks' trackers from the cache, what each broker states from the book.
pub fn market(book: &Book, cache: &MarketCache) -> Result<Market, String> {
    let mut m = crate::read_sources::market_from_cache(cache, book)?;
    m.brokers = engine_inputs::brokers(book)?;
    Ok(m)
}

/// Every input, read whole, and the engine built from it.
pub fn build(book: &Book, cache: &MarketCache, clock: Clock) -> Result<Engine, String> {
    Ok(Engine::build(Inputs { ledger: engine_inputs::ledger(book)?, facts: engine_inputs::facts(book)?, market: market(book, cache)?, clock }))
}

/// `f` run in one read transaction of the book (`Book::snapshot`): every read
/// in it sees the book at one moment.
fn in_snapshot<T>(book: &Book, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let mut out = None;
    book.snapshot(|| {
        out = Some(f());
        Ok(())
    })
    .map_err(err)?;
    out.expect("the snapshot ran its reads")
}

/// What is kept beside the engine about the record: the sequence of its change
/// log applied, and its rows by movement.
pub(crate) struct Held<'a> {
    pub applied: &'a std::sync::atomic::AtomicI64,
    pub movements: &'a std::sync::Mutex<crate::csv_import::Movements>,
    /// The instruments and accounts every catch-up through this read: whose
    /// broker names the page holds are read again (brief 27, 5).
    pub named: std::cell::RefCell<Named>,
    /// Whether a catch-up through this read touched a withdrawal: only then can a
    /// repayment of a debt held outside have moved (`pulled`).
    pub withdrawals: std::cell::Cell<bool>,
}

impl<'a> Held<'a> {
    pub(crate) fn new(applied: &'a std::sync::atomic::AtomicI64, movements: &'a std::sync::Mutex<crate::csv_import::Movements>) -> Held<'a> {
        Held { applied, movements, named: Default::default(), withdrawals: Default::default() }
    }
}

/// What the record's change log names past `applied`, read in one snapshot and
/// applied to the engine, and the sequence the engine has applied moved past it
/// (`docs/plans/stage-3-the-record-by-change.md`, A). Called with the engine's
/// lock held: the snapshot is taken after the lock, so it holds everything an
/// earlier holder applied, and the sequence only rises.
fn catch_up(book: &Book, engine: &mut Engine, held: &Held) -> Result<Moved, String> {
    let applied = held.applied;
    let after = applied.load(std::sync::atomic::Ordering::SeqCst);
    let (changes, delta) = in_snapshot(book, || {
        let changes = book.changes_since(after).map_err(err)?;
        #[cfg(test)]
        BETWEEN_READS.with(|h| {
            let mut h = h.borrow_mut();
            match h.as_mut() {
                Some((0, _)) => {
                    if let Some((_, f)) = h.take() {
                        f();
                    }
                }
                Some((n, _)) => *n -= 1,
                None => {}
            }
        });
        let delta = engine_inputs::delta(book, &changes)?;
        Ok((changes, delta))
    })?;
    let mut moved = Moved::default();
    if !delta.is_empty() {
        let records: Vec<bagholder_core::RecordId> = delta.records.keys().copied().collect();
        let rows = |e: &Engine| -> Vec<bagholder_core::transaction::Transaction> { records.iter().flat_map(|r| e.inputs().ledger.transactions.of_record(*r).cloned().collect::<Vec<_>>()).collect() };
        let gone = rows(engine);
        merge(&mut moved, engine.apply(Change::Ledger(delta)));
        let now = rows(engine);
        let withdrawal = |t: &bagholder_core::transaction::Transaction| t.kind == bagholder_core::transaction::Kind::Withdrawal;
        if gone.iter().chain(now.iter()).any(withdrawal) {
            held.withdrawals.set(true);
        }
        held.movements.lock().unwrap_or_else(|e| e.into_inner()).changed(&gone, &now);
    }
    applied.store(changes.upto, std::sync::atomic::Ordering::SeqCst);
    // what is applied is dropped from the log as it is applied: the log holds only
    // what the engine has not read yet. The sequence applied already keeps it from
    // being read twice, so a drop that fails waits for the next one (brief 27, 7)
    if !changes.is_empty() {
        forget(book, changes.upto);
    }
    let mut named = held.named.borrow_mut();
    named.instruments.extend(changes.instruments);
    named.accounts.extend(changes.accounts);
    Ok(moved)
}

#[cfg(test)]
thread_local! {
    /// A write another writer commits between the log's read and the keyed reads
    /// of a catch-up on this thread, after as many catch-ups as the count says
    /// (`tests::two_writers_racing…`).
    static BETWEEN_READS: std::cell::RefCell<Option<(usize, Box<dyn FnOnce()>)>> = const { std::cell::RefCell::new(None) };
    /// A write another writer commits between the engine naming trades to move or
    /// orphan and the writes that do it, given those trades and the openings the
    /// engine saw them on (`tests::a_trade_another_writer_settled…`).
    static BEFORE_SETTLE_WRITES: std::cell::RefCell<Option<Box<dyn FnOnce(Vec<(bagholder_core::TradeId, bagholder_core::journal::Opening)>)>>> = const { std::cell::RefCell::new(None) };
    /// A failure planted after a pull's first catch-up (`tests::a_pull_that_fails…`).
    static AFTER_CATCH_UP: std::cell::RefCell<Option<Box<dyn FnOnce() -> Result<(), String>>>> = const { std::cell::RefCell::new(None) };
}

/// The log dropped up to `upto`, its failure said and not raised: the engine has
/// applied it, and a later drop takes it.
fn forget(book: &Book, upto: i64) {
    if let Err(e) = book.forget_changes(upto) {
        crate::app::log(&format!("bagholder: the record's change log could not be trimmed (it is trimmed at the next change): {e}"));
    }
}

/// The instruments and accounts a change named: whose names the page holds are
/// read again.
#[derive(Default)]
pub(crate) struct Named {
    instruments: BTreeSet<bagholder_core::InstrumentId>,
    accounts: BTreeSet<bagholder_core::AccountId>,
}

thread_local! {
    /// The trades whose identity this thread's settlings changed (opened, moved,
    /// orphaned): what a change did to the trade identity, which the capacity job
    /// measures a change's work by (#527).
    static SETTLED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The trades whose identity this thread's settlings have changed so far.
pub(crate) fn settled() -> u64 {
    SETTLED.with(std::cell::Cell::get)
}

/// Every round trip given its trade in the book: one opened for each that has
/// none, a trade whose anchor a correction made a fill of another opening's round
/// trip moved to it, a trade a correction joined into another's orphaned, and one
/// whose opening no longer opens anything orphaned, its notes kept; then what was
/// written read back through the change log (`catch_up`). What moved.
///
/// Each move and orphaning is done only on the anchor the engine saw (the book
/// may have moved it since, as a pull does), so what a stale reading named is
/// never written over the book's own; the trades then read back are settled again
/// until a pass writes nothing.
pub(crate) fn settle_trades(book: &Book, engine: &mut Engine, held: &Held, at: Timestamp) -> Result<Moved, String> {
    use bagholder_core::journal::{Anchor, Opening};
    let mut moved = Moved::default();
    loop {
        // only what the book has to do, never the whole identity
        let i = engine.identity();
        if i.needs_trade.is_empty() && i.moved.is_empty() && i.joined.is_empty() && i.unclaimed.is_empty() {
            return Ok(moved);
        }
        let identity = bagholder_engine::identity::Identity { needs_trade: i.needs_trade.clone(), moved: i.moved.clone(), joined: i.joined.clone(), unclaimed: i.unclaimed.clone(), ..Default::default() };
        let seen = |t: &bagholder_core::TradeId| match engine.inputs().ledger.trades.get(t).map(|t| &t.anchor) {
            Some(Anchor::Opening(o)) => Some(o.clone()),
            _ => None,
        };
        #[cfg(test)]
        if !identity.moved.is_empty() || !identity.unclaimed.is_empty() || !identity.joined.is_empty() {
            let named: Vec<_> = identity.moved.iter().map(|(t, _)| t).chain(identity.unclaimed.iter()).chain(identity.joined.iter().map(|(t, _)| t)).filter_map(|t| seen(t).map(|s| (*t, s))).collect();
            BEFORE_SETTLE_WRITES.with(|h| h.borrow_mut().take().map(|f| f(named)));
        }
        let before = book.changes_upto().map_err(err)?;
        // every write of one settling in one transaction: a change that joins many
        // round trips costs one commit (and one settling of names), not one each
        // (#527); each write still checks the anchor the engine saw, inside it
        let mut done = 0u64;
        book.atomically(|| {
            for (trade, key) in &identity.moved {
                if let Some(s) = seen(trade) {
                    done += u64::from(book.move_trade(*trade, &s, &Opening { transaction: key.opening.clone(), instrument: key.instrument })?);
                }
            }
            for key in &identity.needs_trade {
                book.open_trade(&Opening { transaction: key.opening.clone(), instrument: key.instrument }, None, at)?;
                done += 1;
            }
            // the trades orphaned, in set writes: each still only where it is
            // anchored as the engine saw it (#538)
            let orphaned: Vec<(bagholder_core::TradeId, String, Opening)> = identity
                .joined
                .iter()
                .filter_map(|(joined, keeps)| seen(joined).map(|s| (*joined, Book::joined_reason(*keeps), s)))
                .chain(identity.unclaimed.iter().filter_map(|t| seen(t).map(|s| (*t, Book::UNCLAIMED_REASON.to_string(), s))))
                .collect();
            done += book.orphan_seen(&orphaned)?;
            Ok(())
        })
        .map_err(err)?;
        SETTLED.with(|c| c.set(c.get() + done));
        // nothing written: what the engine named was already moved by another
        if book.changes_upto().map_err(err)? == before {
            return Ok(moved);
        }
        merge(&mut moved, catch_up(book, engine, held)?);
    }
}

/// The engine built whole from the book and the cache in `zone` at `now`, in one
/// snapshot, with everything the change log holds then taken as applied (and
/// dropped from it); every round trip given its trade, and the notes carried
/// from the earlier app put back on the round trips an earlier build orphaned
/// them from (`legacy_import::reattach_carried_notes`, once per book).
fn built(home: &Path, book: &Book, cache: &MarketCache, zone: &TimeZone, now: Timestamp) -> Result<Built, String> {
    // the debts' fingerprint read with them, so the first change after the build
    // reads them again only where they moved
    let (mut e, upto, debts_print) = in_snapshot(book, || Ok((build(book, cache, clock(zone, now)?)?, book.changes_upto().map_err(err)?, book.debts_print().map_err(err)?)))?;
    // a sequence and an index of the build's own: nothing the server holds moves
    // until the engine is in place (brief 27, 6)
    let applied = std::sync::atomic::AtomicI64::new(upto);
    let movements = std::sync::Mutex::new(crate::csv_import::Movements::build(&e.inputs().ledger));
    let held = Held::new(&applied, &movements);
    settle_trades(book, &mut e, &held, now)?;
    if crate::legacy_import::reattach_carried_notes(home, book, &e, now)? {
        catch_up(book, &mut e, &held)?;
        settle_trades(book, &mut e, &held, now)?;
    }
    let applied = applied.load(std::sync::atomic::Ordering::SeqCst);
    let movements = movements.into_inner().unwrap_or_else(|e| e.into_inner());
    Ok(Built { engine: e, applied, movements, debts_print })
}

/// An engine built whole, with the sequence of the change log it holds and its
/// rows by movement: put in place by `Figures::install`.
struct Built {
    engine: Engine,
    applied: i64,
    movements: crate::csv_import::Movements,
    debts_print: String,
}

/// Why a journal was not written.
#[derive(Debug)]
pub enum JournalRefused {
    /// No trade or group of the figures has this id.
    Unknown(String),
    Failed(String),
}

/// What the server holds of the figure path.
pub struct Figures {
    home: PathBuf,
    /// Built once a zone is known.
    engine: RwLock<Option<Engine>>,
    /// Something changed that can make a read due (a zone stated, the record
    /// changed): the scheduler looks again at once (`due`).
    wake: std::sync::atomic::AtomicBool,
    /// Counts each change to the figures.
    version: std::sync::atomic::AtomicU64,
    /// What each of the latest changes moved, by the version it made: what a
    /// page's subscriptions are brought forward by (`moved_since`). `None` is a
    /// change that moved everything (the engine built again).
    log: std::sync::Mutex<std::collections::VecDeque<(u64, Option<Moved>)>>,
    /// What the page calls each instrument by the broker, read once per record.
    names: RwLock<Option<crate::wire::build::Names>>,
    /// Told of every commit to the cache on a connection `cache` hands out (the
    /// app's bus, `App::set_figures`): a source's outcome recorded is a change
    /// the header may show.
    heard: std::sync::OnceLock<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// Told after every commit to the book, on any connection `book` opens.
    book_heard: std::sync::OnceLock<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// The connection the header's source failures are read on, opened once.
    failures_conn: std::sync::Mutex<Option<MarketCache>>,
    /// The last sequence of the record's change log the engine holds: written only
    /// with the engine's lock held (`catch_up`).
    applied: std::sync::atomic::AtomicI64,
    /// What the outside debts and the answers about them were the last time the
    /// repayments were matched (`Book::debts_print`).
    debts_print: std::sync::Mutex<Option<String>>,
    /// The ledger's rows by movement, and the movements touched since linking
    /// last looked (`csv_import::link`): kept with the engine.
    movements: std::sync::Mutex<crate::csv_import::Movements>,
    /// The records were derived again under this build's mappings and the prices
    /// kept under a guessed venue forgotten (`prepared`): false until both succeed,
    /// at the open or at a later pass, and nothing is built before.
    rederived: std::sync::atomic::AtomicBool,
}

/// What the cache holds of the prices of every share or fund whose records name no
/// venue a listing is keyed by, forgotten at start: no source asks for it, so a
/// quote kept from before (read under a venue guessed from its currency by an
/// earlier build, perhaps another company's) would stand as its price at any age.
/// The cache asks again once a record names its venue; that costs only time.
fn forget_unkeyed_prices(book: &Book, cache: &MarketCache) -> Result<(), String> {
    let mut unkeyed = Vec::new();
    for i in book.instruments().map_err(err)? {
        if i.kind != bagholder_core::instrument::InstrumentKind::Security {
            continue;
        }
        let keyed = book.current_name(i.id).map_err(err)?.is_some_and(|n| bagholder_core::ListingKey::share(&n.symbol, n.venue_mic.as_deref(), n.venue_name.as_deref()).is_ok());
        if !keyed {
            unkeyed.push(i.id);
        }
    }
    let had = cache.forget_prices(&unkeyed).map_err(|e| format!("the market cache could not be written: {e}"))?;
    if had > 0 {
        crate::app::log(&format!("bagholder: the prices kept of {had} instruments whose records name no venue were forgotten; each is read again once a record names its venue"));
    }
    Ok(())
}

/// What must hold before anything is built from the book: its records derived
/// again under this build's mappings, and no price kept under a guessed venue.
fn prepared(book: &Book, cache: &MarketCache, at: Timestamp) -> Result<(), String> {
    rederive_all(book, at)?;
    forget_unkeyed_prices(book, cache)
}

/// Why the figure path could not be opened: the market cache, said under the
/// cache's own name as every other borrow of it is, or the book (the book itself,
/// or the earlier database carried into it), said as the book's.
#[derive(Clone, Debug, PartialEq)]
pub enum OpenFailure {
    Cache(String),
    Book(String),
}

impl std::fmt::Display for OpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenFailure::Cache(s) | OpenFailure::Book(s) => f.write_str(s),
        }
    }
}

impl From<OpenFailure> for String {
    fn from(e: OpenFailure) -> String {
        e.to_string()
    }
}

impl Figures {
    /// Open the figure path in `home` at `at`: the old store imported into a new
    /// book the first time (its journal, groups and notes carried; the first pull
    /// then supersedes its rows), both files brought to this build's schema, and
    /// the engine built when the book holds a zone. A book or cache this build
    /// cannot open is said by what failed, and the open is tried again later.
    pub fn open(home: &Path, at: Timestamp) -> Result<Figures, OpenFailure> {
        let book_path = home.join(bagholder_book::BOOK_FILE);
        let old = home.join(OLD_FILE);
        if !book_path.exists() && old.exists() {
            crate::legacy_import::import(&old, home, at).map_err(|e| OpenFailure::Book(format!("The earlier database could not be carried into the book: {e}")))?;
        }
        let f = Figures { home: home.to_path_buf(), engine: RwLock::new(None), wake: std::sync::atomic::AtomicBool::new(false), version: std::sync::atomic::AtomicU64::new(1), log: std::sync::Mutex::new(Default::default()), names: RwLock::new(None), heard: std::sync::OnceLock::new(), book_heard: std::sync::OnceLock::new(), failures_conn: std::sync::Mutex::new(None), applied: std::sync::atomic::AtomicI64::new(0), debts_print: std::sync::Mutex::new(None), movements: Default::default(), rederived: std::sync::atomic::AtomicBool::new(false) };
        let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(|e| OpenFailure::Book(format!("The book could not be opened: {e}")))?;
        let (cache, _) = MarketCache::open(&home.join(CACHE_FILE), crate::app::APP_VERSION, at).map_err(|e| OpenFailure::Cache(format!("The market cache could not be opened: {e}")))?;
        // the records derived again and the prices kept under a guessed venue forgotten,
        // both before anything is built: either failing is said on the page and tried
        // again at the next pass; it never stops the server, which a service manager
        // would only start again into the same failure (#516)
        match prepared(&book, &cache, at) {
            Ok(()) => f.rederived.store(true, std::sync::atomic::Ordering::SeqCst),
            Err(e) => crate::app::log(&format!("bagholder: {e}")),
        }
        if !f.rederived.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(f);
        }
        if let Some(z) = book.zone().map_err(|e| OpenFailure::Book(format!("The book could not be read: {e}")))? {
            match built(home, &book, &cache, &z.zone, at) {
                Ok(b) => {
                    let mut engine = f.engine.write().unwrap_or_else(|e| e.into_inner());
                    f.install(&mut engine, &book, b);
                }
                Err(e) => crate::app::log(&format!("bagholder: the figures could not be built: {e}")),
            }
        }
        Ok(f)
    }

    /// The engine built where a zone is known and it is not built yet: what a
    /// pass needs first. Its failure is the pass's, said on the page.
    pub fn ensure_built(&self, now: Timestamp) -> Result<(), String> {
        // prepared first, where that failed when the book opened
        if !self.rederived.load(std::sync::atomic::Ordering::SeqCst) {
            prepared(&self.book()?, &self.cache()?, now)?;
            self.rederived.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if self.engine.read().unwrap_or_else(|e| e.into_inner()).is_some() {
            return Ok(());
        }
        let book = self.book()?;
        let Some(z) = book.zone().map_err(err)? else { return Ok(()) };
        let mut engine = self.engine.write().unwrap_or_else(|e| e.into_inner());
        if engine.is_none() {
            let b = built(&self.home, &book, &self.cache()?, &z.zone, now)?;
            self.install(&mut engine, &book, b);
            drop(engine);
            self.logged(None);
            self.wake();
        }
        Ok(())
    }

    /// Build the engine again from what the book and the cache hold now: after
    /// Clear data, where what changed is everything. Nothing is built before a
    /// page has stated its zone.
    pub fn rebuild(&self, now: Timestamp) -> Result<(), String> {
        let book = self.book()?;
        let Some(z) = book.zone().map_err(err)? else { return Ok(()) };
        let mut engine = self.engine.write().unwrap_or_else(|e| e.into_inner());
        match self.cache().and_then(|c| built(&self.home, &book, &c, &z.zone, now)) {
            Ok(b) => self.install(&mut engine, &book, b),
            Err(e) => {
                // what the engine held is no longer the record: the next zone a
                // page states builds it whole
                *engine = None;
                return Err(e);
            }
        }
        drop(engine);
        self.logged(None);
        self.wake();
        Ok(())
    }

    /// A connection to the book, of this thread's own.
    pub fn book(&self) -> Result<Book, String> {
        let (b, _) = Book::open_in(&self.home, crate::app::APP_VERSION, Timestamp::now()).map_err(err)?;
        // every write to the book reaches the screens once it has landed: a note, a
        // grade, a trade typed in, an import (`docs/plans/stage-money.md`, part E)
        if let Some(heard) = self.book_heard.get() {
            b.on_commit(heard.clone()).map_err(|e| format!("the book's commits could not be heard: {e}"))?;
        }
        Ok(b)
    }

    /// Have `heard` told after every commit to the book from now on. Once: the app's bus.
    pub fn hear_book(&self, heard: std::sync::Arc<dyn Fn() + Send + Sync>) {
        match self.book_heard.set(heard) {
            Ok(()) => {}
            // a second call leaves the first
            Err(_second) => {}
        }
    }

    /// A connection to the market cache, of this thread's own; its commits are
    /// heard (`hear`).
    pub fn cache(&self) -> Result<MarketCache, String> {
        let (c, _) = MarketCache::open(&self.home.join(CACHE_FILE), crate::app::APP_VERSION, Timestamp::now()).map_err(err)?;
        if let Some(heard) = self.heard.get() {
            c.on_commit(heard.clone()).map_err(|e| format!("the market cache's commits could not be heard: {e}"))?;
        }
        Ok(c)
    }

    /// Have `heard` told of every commit to the cache from now on. Once: the
    /// app's bus.
    pub fn hear(&self, heard: std::sync::Arc<dyn Fn() + Send + Sync>) {
        match self.heard.set(heard) {
            Ok(()) => {}
            // once: a second listener leaves the first
            Err(_second) => {}
        }
    }

    /// Each market source failing now, one sentence each (`health::failures`):
    /// read from what the cache recorded of each source, the one record of
    /// them, so a failure shows exactly until its own subject next answers.
    pub fn source_failures(&self, started: Timestamp) -> Result<Vec<String>, String> {
        let mut conn = self.failures_conn.lock().unwrap_or_else(|e| e.into_inner());
        if conn.is_none() {
            // only read on: it has nothing of its own to be heard
            *conn = Some(MarketCache::open(&self.home.join(CACHE_FILE), crate::app::APP_VERSION, Timestamp::now()).map_err(err)?.0);
        }
        let cache = conn.as_ref().expect("opened above");
        let newest = cache.newest_counted_by_subject().map_err(err)?;
        // a subject's failures in a row; one the cache cannot count is counted as said
        let in_a_row = |o: &bagholder_sources::cache::OutcomeRow| cache.failures_in_a_row(&o.source, o.kind, o.instrument).unwrap_or(bagholder_sources::health::SAID_AFTER);
        Ok(bagholder_sources::health::failures(&newest, started, in_a_row))
    }

    /// A built engine put in place, under the engine's lock: then, and only
    /// then, the sequence it holds, its rows by movement (every movement holding a
    /// file's row pending, so the first link after it looks at them all), the log
    /// dropped up to it, and the broker names read again.
    fn install(&self, engine: &mut Option<Engine>, book: &Book, b: Built) {
        let Built { engine: e, applied, mut movements, debts_print } = b;
        movements.pend_every_file_row(&e.inputs().ledger);
        *engine = Some(e);
        *self.debts_print.lock().unwrap_or_else(|e| e.into_inner()) = Some(debts_print);
        self.applied.store(applied, std::sync::atomic::Ordering::SeqCst);
        *self.movements.lock().unwrap_or_else(|e| e.into_inner()) = movements;
        forget(book, applied);
        *self.names.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The movements a change touched since linking last looked, taken.
    pub fn links_pending(&self) -> std::collections::BTreeSet<crate::csv_import::Movement> {
        self.movements.lock().unwrap_or_else(|e| e.into_inner()).take_pending()
    }

    /// `keys` pending again.
    pub fn links_pend(&self, keys: std::collections::BTreeSet<crate::csv_import::Movement>) {
        self.movements.lock().unwrap_or_else(|e| e.into_inner()).pend(keys);
    }

    /// The movements of `records`' rows pending: a file read again whose rows were
    /// all kept before still has each looked at (brief 27, 4b).
    pub fn links_pend_records(&self, records: &std::collections::BTreeSet<bagholder_core::RecordId>) {
        let keys: Vec<crate::csv_import::Movement> = self
            .read(|e| records.iter().flat_map(|r| e.inputs().ledger.transactions.of_record(*r).filter_map(crate::csv_import::movement_of).collect::<Vec<_>>()).collect())
            .unwrap_or_default();
        self.links_pend(keys.into_iter().collect());
    }

    /// Read the ledger's rows by movement.
    pub fn with_movements<T>(&self, f: impl FnOnce(&crate::csv_import::Movements) -> T) -> T {
        f(&self.movements.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Read the engine, once it is built.
    pub fn read<T>(&self, f: impl FnOnce(&Engine) -> T) -> Option<T> {
        self.engine.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(f)
    }

    /// Which change the figures are at.
    pub fn version(&self) -> u64 {
        self.version.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn moved(&self, m: Moved) -> Moved {
        if !m.is_empty() {
            self.logged(Some(m.clone()));
        }
        m
    }

    /// A change made: the next version, and what it moved kept for the pages.
    fn logged(&self, m: Option<Moved>) {
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let v = self.version.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        log.push_back((v, m));
        while log.len() > LOGGED {
            log.pop_front();
        }
    }

    /// What moved after version `v`: nothing, the entities the engine reported, or
    /// everything (the engine was built again, or `v` is older than what is kept).
    pub fn moved_since(&self, v: u64) -> Since {
        let log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.version();
        if v >= now {
            return Since::Nothing;
        }
        match log.front() {
            Some((first, _)) if *first <= v + 1 => {}
            _ => return Since::Everything,
        }
        let mut all = Moved::default();
        for (_, m) in log.iter().filter(|(at, _)| *at > v) {
            match m {
                Some(m) => all.merge(m.clone()),
                None => return Since::Everything,
            }
        }
        Since::Moved(all)
    }

    /// The broker's names for the instruments, read from the book once per record.
    pub fn names(&self) -> Result<crate::wire::build::Names, String> {
        if let Some(n) = self.names.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(n.clone());
        }
        let book = self.book()?;
        // the engine's read lock held across the cache's write: an install or a
        // pull cannot land between the read and the write and be overwritten
        let engine = self.engine.read().unwrap_or_else(|e| e.into_inner());
        let n = match engine.as_ref() {
            Some(e) => crate::wire::build::Names::load(&book, e.inputs())?,
            None => Default::default(),
        };
        *self.names.write().unwrap_or_else(|e| e.into_inner()) = Some(n.clone());
        drop(engine);
        Ok(n)
    }

    /// Apply one change: what moved, nothing before the engine is built.
    fn apply(&self, change: Change) -> Moved {
        let m = match self.engine.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(e) => e.apply(change),
            None => Moved::default(),
        };
        self.moved(m)
    }

    /// Apply a change only where it differs from what the engine holds: a read
    /// that stored nothing new computes nothing.
    fn apply_if(&self, differs: impl FnOnce(&Inputs) -> bool, change: impl FnOnce() -> Change) -> Moved {
        let m = match self.engine.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(e) if differs(e.inputs()) => e.apply(change()),
            _ => Moved::default(),
        };
        self.moved(m)
    }

    /// Ask the scheduler to look again now.
    pub fn wake(&self) {
        self.wake.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether the scheduler was asked to look again, clearing it.
    pub fn woken(&self) -> bool {
        self.wake.swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    /// A page states its browser's zone at `now`: kept, and a zone other than the
    /// one held moves "today", months and years to it. The first zone stated
    /// builds the engine.
    pub fn state_zone(&self, name: &str, now: Timestamp) -> Result<Moved, String> {
        let book = self.book()?;
        let changed = book.state_zone(name, now).map_err(err)?;
        if changed {
            self.wake();
        }
        let zone = book.zone().map_err(err)?.ok_or("the zone stated was not kept")?.zone;
        let mut engine = self.engine.write().unwrap_or_else(|e| e.into_inner());
        match engine.as_mut() {
            None => {
                let b = built(&self.home, &book, &self.cache()?, &zone, now)?;
                self.install(&mut engine, &book, b);
                drop(engine);
                self.logged(None);
                // the scheduler's next pass reads for the figures, and clears a
                // failure the header holds from a build that did not succeed
                self.wake();
                Ok(Moved::default())
            }
            Some(e) if changed => {
                let m = e.apply(Change::Clock(clock(&zone, now)?));
                drop(engine);
                Ok(self.moved(m))
            }
            Some(_) => Ok(Moved::default()),
        }
    }

    /// The instant moved (the day turned in the person's zone, the Bank's 16:30
    /// passed): the clock, in the zone the book holds.
    pub fn clock_moved(&self, now: Timestamp) -> Result<Moved, String> {
        let Some(zone) = self.book()?.zone().map_err(err)? else { return Ok(Moved::default()) };
        Ok(self.apply(Change::Clock(clock(&zone.zone, now)?)))
    }

    /// The person wrote a trade's or a group's journal: kept in the book, then the
    /// journal applied. `id` is the trade's id as the page has it; one the figures
    /// do not know, or a round trip not yet given its trade, is refused by name.
    pub fn write_journal(&self, id: &str, entry: &bagholder_core::journal::JournalEntry, now: Timestamp) -> Result<Moved, JournalRefused> {
        use bagholder_core::journal::JournalSubject;
        use bagholder_engine::trades::TradeKey;
        let subject = self
            .read(|e| {
                e.figures().trades.iter().find_map(|t| match &t.key {
                    TradeKey::Group(g) if g.to_string() == id => Some(JournalSubject::Group(*g)),
                    TradeKey::Trip(_) if t.trade.is_some_and(|x| x.to_string() == id) => t.trade.map(JournalSubject::Trade),
                    _ => None,
                })
            })
            .ok_or_else(|| JournalRefused::Failed("the figures are not built yet".into()))?
            .ok_or_else(|| JournalRefused::Unknown(id.to_string()))?;
        let book = self.book().map_err(JournalRefused::Failed)?;
        book.set_journal(subject, entry, now).map_err(|e| JournalRefused::Failed(e.to_string()))?;
        self.record_changed(now).map_err(JournalRefused::Failed)
    }

    /// The record changed (an entry, an import, a fill giving way): the ledger
    /// and the adjustments applied, each round trip given its trade. What the
    /// broker states beside the rows is not read: nothing here wrote it.
    pub fn record_changed(&self, now: Timestamp) -> Result<Moved, String> {
        self.pulled(now, &BTreeSet::new())
    }

    /// A pull stored, revised or removed records and read the broker's statements
    /// of `stated_for`: the ledger, the adjustments and those accounts' statements
    /// applied in one write, under one lock and one version, so no page is ever
    /// sent the rows without the statements that go with them (a cash difference
    /// that shows for a moment and goes: `docs/decisions.md`, 2026-10-09).
    pub fn pulled(&self, now: Timestamp, stated_for: &BTreeSet<bagholder_core::AccountId>) -> Result<Moved, String> {
        let book = self.book()?;
        let mut guard = self.engine.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = guard.as_mut() else {
            // figures that could not be built may build from the record as it is
            // now: the scheduler's next pass tries again
            drop(guard);
            self.wake();
            return Ok(Moved::default());
        };
        // the record by what changed in it, then each round trip given its trade:
        // a pull that supersedes a record moves the trades on it to the broker's
        // rows in the same write, read back with it
        let held = Held::new(&self.applied, &self.movements);
        let mut moved = catch_up(&book, e, &held)?;
        // once a catch-up has applied, what it named must reach the names the page
        // holds, whatever fails after it: on a failure the names are read whole
        // again when next asked (brief 28, E)
        let rest = (|| -> Result<(), String> {
            merge(&mut moved, settle_trades(&book, e, &held, now)?);
            #[cfg(test)]
            AFTER_CATCH_UP.with(|h| h.borrow_mut().take().map_or(Ok(()), |f| f()))?;
            let mut brokers = std::collections::BTreeMap::new();
            for a in stated_for {
                brokers.insert(*a, engine_inputs::broker_account(&book.stated(*a).map_err(|e| e.to_string())?));
            }
            for (a, now_stated) in brokers {
                if e.inputs().market.brokers.get(&a) != now_stated.as_ref() {
                    merge(&mut moved, e.apply(Change::Broker(a, now_stated)));
                }
            }
            // the debts held outside, and which withdrawals repaid them: read again
            // only where a withdrawal was touched or the debts or the answers moved,
            // so a change costs what it changed
            let print = book.debts_print().map_err(|e| e.to_string())?;
            let mut last = self.debts_print.lock().unwrap_or_else(|e| e.into_inner());
            if held.withdrawals.get() || *last != Some(print.clone()) {
                let debts = engine_inputs::debts(&book)?;
                if e.inputs().facts.debts != debts {
                    merge(&mut moved, e.apply(Change::Debts(debts)));
                }
                *last = Some(print);
            }
            Ok(())
        })();
        let named = held.named.take();
        let mut names = self.names.write().unwrap_or_else(|e| e.into_inner());
        // the broker's names of what the change named read again, where the page
        // holds them; on a failure they are read whole again when next asked
        let done = rest.and_then(|()| names.as_mut().map_or(Ok(()), |n| n.reload(&book, e.inputs(), named.instruments, named.accounts)));
        if done.is_err() {
            *names = None;
        }
        drop(names);
        drop(guard);
        // what the applied catch-up moved reaches the pages whatever failed after
        // it, and the scheduler looks again (brief 30, C6)
        self.wake();
        let moved = self.moved(moved);
        done.map(|()| moved)
    }

    /// What the broker states of an account now (its cash, what it can borrow,
    /// what it is worth) changed: that account's statements read through their
    /// indexes, its day history left as the figures hold it (a balances read never
    /// states a day), and that account's figures alone moved.
    pub fn broker_changed(&self, account: bagholder_core::AccountId) -> Result<Moved, String> {
        let now = self.book()?.stated_latest(account).map_err(|e| e.to_string())?;
        let mut guard = self.engine.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = guard.as_mut() else { return Ok(Moved::default()) };
        let held = e.inputs().market.brokers.get(&account);
        let mut stated = engine_inputs::broker_account(&now);
        if let Some(h) = held.filter(|h| !h.net_value.is_empty() || !h.net_deposits.is_empty()) {
            let b = stated.get_or_insert_with(Default::default);
            b.net_value = h.net_value.clone();
            b.net_deposits = h.net_deposits.clone();
        }
        let m = if held != stated.as_ref() { e.apply(Change::Broker(account, stated)) } else { Moved::default() };
        drop(guard);
        Ok(self.moved(m))
    }

    /// The Bank's rates or its calendar changed.
    pub fn rates_changed(&self) -> Result<Moved, String> {
        let rates = engine_inputs::facts(&self.book()?)?.rates;
        Ok(self.apply_if(|i| i.facts.rates != rates, || Change::Rates(rates.clone())))
    }

    /// A payer's declared record or its stated schedule changed.
    pub fn payer_changed(&self, instrument: InstrumentId) -> Result<Moved, String> {
        let facts = engine_inputs::facts(&self.book()?)?;
        let (declared, frequency) = (facts.declared.get(&instrument).cloned(), facts.frequencies.get(&instrument).cloned());
        let mut moved = self.apply_if(|i| i.facts.declared.get(&instrument) != declared.as_ref(), || Change::Declared(instrument, declared.clone()));
        merge(&mut moved, self.apply_if(|i| i.facts.frequencies.get(&instrument) != frequency.as_ref(), || Change::Frequency(instrument, frequency.clone())));
        Ok(moved)
    }

    /// An instrument's quote (with the ex-dividend date it states) or daily
    /// closes changed.
    pub fn price_changed(&self, instrument: InstrumentId) -> Result<Moved, String> {
        // that instrument's prices, read by its key: nothing of any other instrument
        // and nothing of the book (its kind is the engine's own)
        let kind = self.read(|e| e.inputs().ledger.instruments.get(&instrument).map(|i| i.instrument.kind)).flatten();
        let (quote, ex_dividend, closes) = crate::read_sources::prices_of(&self.cache()?, instrument, kind)?;
        let mut moved = self.apply_if(|i| i.market.quotes.get(&instrument) != quote.as_ref(), || Change::Quote(instrument, quote.clone()));
        merge(&mut moved, self.apply_if(|i| i.market.ex_dividends.get(&instrument).copied() != ex_dividend, || Change::ExDividend(instrument, ex_dividend)));
        merge(&mut moved, self.apply_if(|i| i.market.closes.get(&instrument).cloned().unwrap_or_default() != closes, || Change::Closes(instrument, closes.clone())));
        Ok(moved)
    }

    /// A benchmark's tracker changed.
    pub fn benchmark_changed(&self, key: &str) -> Result<Moved, String> {
        let series = crate::read_sources::benchmark_of(&self.cache()?, key)?;
        Ok(self.apply_if(|i| i.market.benchmarks.get(key) != series.as_ref(), || Change::Benchmark(key.to_string(), series.clone())))
    }
}

fn merge(into: &mut Moved, more: Moved) {
    into.merge(more)
}

/// How many changes are kept for the pages to be brought forward by: a page
/// further behind is sent its subscriptions whole.
const LOGGED: usize = 256;

/// What moved after a version (`Figures::moved_since`).
#[derive(Debug, PartialEq)]
pub enum Since {
    Nothing,
    Moved(Moved),
    Everything,
}


/// Every source's mapping, as the book stores records under it.
pub fn mappings() -> [&'static dyn bagholder_book::mapping::Mapping; 5] {
    [&bagholder_wealthsimple::mapping::WealthsimpleMapping, &bagholder_book::import::mapping::ImportMapping, &bagholder_book::person::PersonMapping, &bagholder_broker::csv::CsvMapping, &bagholder_book::fills::FillMapping]
}

/// Derive again every record a newer version of its mapping reads differently: a
/// mapping's version moves exactly when what it makes of a stored row changes,
/// so the rows already stored follow it, not only the ones read after.
fn rederive_all(book: &Book, at: Timestamp) -> Result<(), String> {
    // instruments an earlier build merged for a succession, put apart once
    book.repair_merged_successions(&mappings(), at).map_err(|e| format!("the instruments an earlier build merged could not be put apart: {e}"))?;
    for m in mappings() {
        let changes = book.rederive(m, at).map_err(|e| format!("{}'s records could not be derived again: {e}", m.source()))?;
        if !changes.is_empty() {
            crate::app::log(&format!("bagholder: {}'s records derived again under version {}", m.source(), m.version()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn pulled(home: &Path) {
        crate::tests_common::pulled_book(home)
    }

    #[test]
    fn records_a_mapping_now_reads_differently_are_derived_again_when_the_book_opens() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let book_file = home.path().join(bagholder_book::BOOK_FILE);
        let versions = || -> Vec<(String, i64)> {
            let c = rusqlite::Connection::open(&book_file).unwrap();
            let mut st = c.prepare("SELECT source, derived_version FROM source_records WHERE state = 'live'").unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        assert!(!versions().is_empty());
        // every record as an older version of its mapping left it
        rusqlite::Connection::open(&book_file).unwrap().execute("UPDATE source_records SET derived_version = 0", []).unwrap();
        Figures::open(home.path(), at("2025-11-20T12:00:00Z")).unwrap();
        for (source, v) in versions() {
            let m = mappings().into_iter().find(|m| m.source().as_str() == source).unwrap_or_else(|| panic!("no mapping for {source}"));
            assert_eq!(v, m.version() as i64, "{source}'s records follow its mapping's version");
        }
    }

    /// Records that cannot be derived again when the book opens never stop the
    /// start (#516): the figure path opens, nothing is built from records left
    /// as an older mapping read them, and the next pass derives them again and
    /// builds, its failure the pass's, said in the header until then.
    #[test]
    fn records_that_cannot_be_derived_again_at_the_open_are_tried_again_by_the_pass() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let book_file = home.path().join(bagholder_book::BOOK_FILE);
        let now = at("2025-11-20T12:00:00Z");
        {
            let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            book.state_zone("America/Toronto", now).unwrap();
        }
        // a record no reader can name, left for this build's mapping to derive
        let c = rusqlite::Connection::open(&book_file).unwrap();
        let source: String = c.query_row("SELECT source FROM source_records LIMIT 1", [], |r| r.get(0)).unwrap();
        c.execute("INSERT INTO source_records(id, source, source_key, state, derived_version, first_received_at, state_changed_at) VALUES ('not a record id', ?, 'k', 'live', 0, '2025-11-19T20:00:00Z', '2025-11-19T20:00:00Z')", [&source]).unwrap();
        let f = Figures::open(home.path(), now).expect("the open never stops on it");
        assert!(f.read(|_| ()).is_none(), "nothing is built from records not derived again");
        assert!(f.ensure_built(now).unwrap_err().contains("could not be derived again"), "the pass's failure says why");
        c.execute("DELETE FROM source_records WHERE id = 'not a record id'", []).unwrap();
        f.ensure_built(now).unwrap();
        assert!(f.read(|_| ()).is_some(), "the pass that derived them built the figures");
        same_as_fresh(&f, now);
    }

    /// Nothing is built from records not derived again, even where the build could
    /// read them (brief 50, 5): a record this build's mapping refuses (an id the
    /// book reads, with nothing it can map) leaves the build able to run, so only
    /// the open's own guard keeps the figures unbuilt until the pass derives it.
    #[test]
    fn nothing_is_built_from_records_not_derived_again_where_the_build_could_read_them() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let book_file = home.path().join(bagholder_book::BOOK_FILE);
        let now = at("2025-11-20T12:00:00Z");
        {
            let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            book.state_zone("America/Toronto", now).unwrap();
        }
        let c = rusqlite::Connection::open(&book_file).unwrap();
        let source: String = c.query_row("SELECT source FROM source_records LIMIT 1", [], |r| r.get(0)).unwrap();
        c.execute("INSERT INTO source_records(id, source, source_key, state, derived_version, first_received_at, state_changed_at) VALUES ('01893c4a-0000-7000-8000-000000000001', ?, 'planted', 'live', 0, '2025-11-19T20:00:00Z', '2025-11-19T20:00:00Z')", [&source]).unwrap();
        // the build itself reads the book with the record in it
        let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
        assert!(rederive_all(&book, now).is_err(), "the mapping refuses the record");
        let cache = MarketCache::open(&home.path().join(CACHE_FILE), crate::app::APP_VERSION, now).unwrap().0;
        assert!(build(&book, &cache, clock(&book.zone().unwrap().unwrap().zone, now).unwrap()).is_ok(), "the build alone would run");
        drop((book, cache));
        let f = Figures::open(home.path(), now).expect("the open never stops on it");
        assert!(f.read(|_| ()).is_none(), "nothing is built from records not derived again");
    }

    /// The engine the figure path holds, against one built fresh from the files.
    #[track_caller]
    fn same_as_fresh(f: &Figures, now: Timestamp) {
        let book = f.book().unwrap();
        let zone = book.zone().unwrap().unwrap().zone;
        let fresh = build(&book, &f.cache().unwrap(), clock(&zone, now).unwrap()).unwrap();
        let held = f.read(|e| e.differences(&fresh)).expect("built");
        assert!(held.is_empty(), "{held:?}");
        // in the order shown, and in the match and the trades' identity (brief 28, E)
        if let Some(why) = f.read(|e| e.match_disagrees(&fresh)).expect("built") {
            panic!("{why}");
        }
        // what the change log brought in is the record as a whole read gives it
        let (ledger, adjustments) = f.read(|e| (e.inputs().ledger.clone(), e.inputs().facts.adjustments.clone())).unwrap();
        assert!(ledger == engine_inputs::ledger(&book).unwrap(), "the ledger applied by key is not the ledger read whole");
        assert!(adjustments == engine_inputs::facts(&book).unwrap().adjustments, "the adjustments applied by key are not those read whole");
        // the broker's names the page holds, kept by key, are those read whole
        let held = f.names().unwrap();
        let whole = f.read(|e| crate::wire::build::Names::load(&book, e.inputs())).unwrap().unwrap();
        assert!(held == whole, "the names kept by key are not those read whole");
    }

    /// At start, what the cache kept of the prices of a share whose records name no
    /// venue is forgotten, and a keyed one's is kept: a quote read under a guessed
    /// venue never stands as a price at any age.
    #[test]
    fn the_prices_kept_of_a_share_with_no_venue_are_forgotten_at_start() {
        use bagholder_book::person::{Entry, Side, Traded};
        use bagholder_core::{Currency, Dec, Money};
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
        let account = book.accounts().unwrap()[0].id;
        let trade = |venue: Option<&str>, symbol: &str| {
            let r = book.enter(&Entry::Trade { account, instrument: Traded::Named { symbol: symbol.into(), currency: Currency::USD, contract: None, venue: venue.map(str::to_string) }, day: "2025-11-03".parse().unwrap(), side: Side::Buy, quantity: Dec::ONE, price: Money::new(Dec::ONE, Currency::USD), fee: None }, now).unwrap();
            book.transactions_of(r.record).unwrap()[0].instrument.unwrap()
        };
        let (unkeyed, keyed) = (trade(None, "ZQNV"), trade(Some("XNYS"), "ZQKV"));
        let (cache, _) = MarketCache::open(&home.path().join(CACHE_FILE), crate::app::APP_VERSION, now).unwrap();
        for id in [unkeyed, keyed] {
            let q = bagholder_sources::cache::StoredQuote { instrument: id, source: bagholder_core::SourceName::named("yahoo"), price: Money::new(Dec::parse("9.5").unwrap(), Currency::USD), change: None, change_pct: None, quoted_at: at("2025-10-01T20:00:00Z"), allowance: std::time::Duration::ZERO, received_at: at("2025-10-01T20:00:00Z") };
            cache.store_quote(&q).unwrap();
            cache.store_closes(id, &[("2025-10-01".parse().unwrap(), Dec::parse("9.5").unwrap())], Currency::USD, &bagholder_core::SourceName::named("yahoo"), now).unwrap();
        }
        drop((book, cache));
        let _f = Figures::open(home.path(), now).unwrap();
        let (cache, _) = MarketCache::open(&home.path().join(CACHE_FILE), crate::app::APP_VERSION, now).unwrap();
        assert!(cache.quotes_of(unkeyed).unwrap().is_empty() && cache.closes_of(unkeyed).unwrap().is_empty(), "the share with no venue keeps no price");
        assert_eq!(cache.quotes_of(keyed).unwrap().len(), 1, "the keyed share keeps its quote");
        assert_eq!(cache.closes_of(keyed).unwrap().len(), 1, "and its closes");
    }

    /// A balances read states no day (brief 22, 9; brief 25): what the broker states
    /// of an account now is applied without one statement on the day history, which
    /// a start and a pull read.
    #[test]
    fn a_balances_read_runs_no_statement_on_the_day_history() {
        bagholder_sqlite::work::start_counting();
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let book = f.book().unwrap();
        let account = f.read(|e| e.figures().positions[0].account).unwrap();
        let read = book.broker_read(book.connections().unwrap()[0].id, "cash", now).unwrap();
        book.store_cash(account, now, &[(bagholder_core::Currency::CAD, bagholder_core::Dec::parse("42.00").unwrap())].into_iter().collect(), &read).unwrap();
        let (moved, days) = bagholder_sqlite::work::statements_naming("account_days", || f.broker_changed(account).unwrap());
        assert!(!moved.is_empty(), "the cash moved its account");
        assert_eq!(days, 0, "a balances read ran statements on the day history");
        // the counter sees the read it refuses: the whole read with days names the table
        let (_, whole) = bagholder_sqlite::work::statements_naming("account_days", || engine_inputs::brokers(&book).unwrap());
        assert!(whole > 0);
    }

    /// A write kept under an instrument a succession joins into another reaches
    /// the engine under the one it is read as (brief 27, 1): its name, a broker's
    /// id for it and a record under it removed, each applied as a whole read
    /// gives it.
    #[test]
    fn a_write_under_a_joined_id_reaches_the_instrument_it_is_read_as() {
        let home = tempfile::tempdir().unwrap();
        let size = bagholder_wealthsimple::generated::Size { accounts: 6, instruments: 16, trades: 60, months: 4, seed: 3, deep: 0 };
        crate::capacity::build(home.path(), size, 2).unwrap();
        let now = Timestamp::now();
        let f = Figures::open(home.path(), now).unwrap();
        f.names().unwrap();
        let book = f.book().unwrap();
        let (member, head): (String, String) = book.conn_for_tests().query_row("SELECT instrument_id, into_id FROM instrument_joins LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).expect("the made-up broker states a succession");
        same_as_fresh(&f, now);
        // its name, as a record under the member states it
        book.conn_for_tests().execute("UPDATE instrument_sightings SET name = 'Renamed Under The Member', day = '2099-01-01' WHERE rowid = (SELECT s.rowid FROM instrument_sightings s JOIN source_records r ON r.id = s.record_id AND r.state = 'live' WHERE s.instrument_id = ?1 LIMIT 1)", [&member]).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        assert!(f.read(|e| e.inputs().ledger.instruments[&head.parse().unwrap()].names.last().and_then(|n| n.name.clone())).unwrap().as_deref() == Some("Renamed Under The Member"), "the head kept its old name");
        // a broker's id for it, under the member, that the page names it by: the
        // first of its ids (brief 28, E), so what is shown changes
        book.conn_for_tests().execute("INSERT INTO instrument_refs (scheme, value, instrument_id) VALUES ('broker-security:wealthsimple', '0-sec-under-the-member', ?1)", [&member]).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        let head_id: bagholder_core::InstrumentId = head.parse().unwrap();
        assert_eq!(f.names().unwrap().security.get(&head_id).map(String::as_str), Some("0-sec-under-the-member"), "the page does not name the head by the member's broker id");
        // a record under the member: a sighting of a new name, the member's alone
        let record: String = book.conn_for_tests().query_row("SELECT t.record_id FROM transactions t JOIN source_records r ON r.id = t.record_id WHERE t.instrument_id = ?1 AND r.state = 'live' LIMIT 1", [&member], |r| r.get(0)).unwrap();
        book.conn_for_tests().execute("INSERT INTO instrument_sightings (record_id, leg, instrument_id, symbol, day) VALUES (?1, 'extra', ?2, 'NEWSYM', '2099-02-01')", [&record, &member]).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        assert_eq!(f.read(|e| e.inputs().ledger.instruments[&head_id].names.last().map(|n| n.symbol.clone())).unwrap().as_deref(), Some("NEWSYM"), "the head kept its old symbol");
        // and that record removed
        book.mark_removed(record.parse().unwrap(), now).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
    }

    /// Two writers racing (brief 26, 3; brief 27, 13): another connection
    /// supersedes and removes rows between the log's read and the keyed reads.
    /// The catch-up applies the book as it stood at its read, never a mix, and
    /// the next one applies what the other writer did: then the engine is a
    /// fresh build's, and no trade was orphaned that a fresh settle keeps.
    #[test]
    fn two_writers_racing_leave_the_engine_as_a_fresh_build_would_and_orphan_no_trade() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let book = f.book().unwrap();
        let orphaned = |b: &Book| b.trades().unwrap().iter().filter(|t| t.orphaned_reason().is_some()).count();
        let orphaned_before = orphaned(&book);
        // a record the catch-up will read, and two the other writer changes under it
        let live = book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap();
        let (read, gone, revised) = (live[0], live[1], live[2]);
        book.mark_removed(read, now).unwrap();
        // a problem on `revised` the catch-up reads, and one the other writer adds to it after
        book.conn_for_tests().execute("INSERT INTO record_problems (record_id, code, detail) VALUES (?1, 'before', 'written before the read')", [revised.to_string()]).unwrap();
        let home_path = home.path().to_path_buf();
        BETWEEN_READS.with(|h| {
            *h.borrow_mut() = Some((0, Box::new(move || {
                // the other writer, on a connection of its own
                let (other, _) = Book::open_in(&home_path, crate::app::APP_VERSION, now).unwrap();
                other.mark_removed(gone, now).unwrap();
                other.conn_for_tests().execute("INSERT INTO record_problems (record_id, code, detail) VALUES (?1, 'raced', 'written between the reads')", [revised.to_string()]).unwrap();
            })))
        });
        // one catch-up alone: it saw `read` removed and nothing the other writer did
        {
            let mut guard = f.engine.write().unwrap();
            let e = guard.as_mut().unwrap();
            catch_up(&book, e, &Held::new(&f.applied, &f.movements)).unwrap();
        }
        let held = f.read(|e| e.inputs().ledger.clone()).unwrap();
        assert!(!held.records.contains_key(&read), "the change read is not applied");
        assert!(held.records.contains_key(&gone), "a write committed after the log's read was applied in part");
        let codes: Vec<String> = held.records[&revised].problems.iter().map(|p| p.code.clone()).collect();
        assert!(codes.contains(&"before".to_string()) && !codes.contains(&"raced".to_string()), "a write committed after the log's read was applied in part: {codes:?}");
        // the next change applies what the other writer did
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        // the same writes made one after another, no race: as many trades orphaned
        let twin = tempfile::tempdir().unwrap();
        pulled(twin.path());
        let g = Figures::open(twin.path(), now).unwrap();
        g.state_zone("America/Toronto", now).unwrap();
        let tb = g.book().unwrap();
        assert_eq!(orphaned(&tb), orphaned_before);
        let twin_live = tb.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap();
        for r in [twin_live[0], twin_live[1]] {
            tb.mark_removed(r, now).unwrap();
        }
        g.record_changed(now).unwrap();
        assert_eq!(orphaned(&f.book().unwrap()), orphaned(&tb), "the race orphaned otherwise than the same writes made in turn");
    }

    /// What every catch-up of one pull named is read again, not only the first's
    /// (brief 27, 5): a broker id another writer commits between two catch-ups
    /// reaches the names the page holds.
    #[test]
    fn a_broker_id_written_between_two_catch_ups_reaches_the_names() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        f.names().unwrap();
        let book = f.book().unwrap();
        let (instrument, record): (String, String) = book.conn_for_tests().query_row("SELECT instrument_id, record_id FROM transactions WHERE instrument_id IS NOT NULL LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let held = Held::new(&f.applied, &f.movements);
        {
            let mut guard = f.engine.write().unwrap();
            let e = guard.as_mut().unwrap();
            book.conn_for_tests().execute("INSERT INTO record_problems (record_id, code, detail) VALUES (?1, 'first', 'the first catch-up')", [&record]).unwrap();
            catch_up(&book, e, &held).unwrap();
            // the other writer: a broker id for the instrument, then the second catch-up
            let (other, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            other.conn_for_tests().execute("DELETE FROM instrument_refs WHERE instrument_id = ?1 AND scheme LIKE 'broker-security:%'", [&instrument]).unwrap();
            other.conn_for_tests().execute("INSERT INTO instrument_refs (scheme, value, instrument_id) VALUES ('broker-security:wealthsimple', 'sec-s-written-between', ?1)", [&instrument]).unwrap();
            catch_up(&book, e, &held).unwrap();
        }
        assert!(held.named.borrow().instruments.contains(&instrument.parse().unwrap()), "the second catch-up's names were dropped");
        // read again as a pull reads them: then they are a whole read's
        let named = held.named.take();
        f.read(|e| f.names.write().unwrap().as_mut().unwrap().reload(&book, e.inputs(), named.instruments, named.accounts).unwrap()).unwrap();
        same_as_fresh(&f, now);
    }

    /// The broker's own row for an order, as its pull stores it: what a read-back
    /// fill gives way to.
    struct BrokerRow;

    impl bagholder_book::mapping::Mapping for BrokerRow {
        fn source(&self) -> bagholder_core::SourceName {
            bagholder_core::SourceName::named("wealthsimple")
        }
        fn version(&self) -> u32 {
            1
        }
        fn map(&self, _ctx: &bagholder_book::mapping::MapContext, _payload: &str) -> bagholder_book::mapping::Mapped {
            use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
            let usd = bagholder_core::Currency::USD;
            bagholder_book::mapping::Mapped {
                legs: vec![bagholder_book::mapping::Draft {
                    leg: bagholder_core::Leg::named("trade"),
                    account: bagholder_core::account::AccountRef::new(bagholder_core::Broker::named("wealthsimple"), "acct-race"),
                    occurred_at: None,
                    trade_date: bagholder_core::jiff::civil::date(2025, 11, 19),
                    settle_date: None,
                    kind: bagholder_core::transaction::Kind::Buy,
                    effect: None,
                    instrument: Some(bagholder_book::mapping::InstrumentDraft { refs: vec![Reference::new(RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), "sec-race")], kind: InstrumentKind::Security, currency: usd, name: None, option: None, standing: None }),
                    quantity: Some(bagholder_core::Dec::from_int(10)),
                    price: None,
                    cash: Some(bagholder_core::Money::new(bagholder_core::Dec::parse("-1012.5").unwrap(), usd)),
                    fee: None,
                    fx_rate: None,
                    paid_on: None,
                    value: None,
                }],
                problems: vec![],
                adjustments: vec![],
                hold: None,
                orders: vec![],
            }
        }
    }

    /// An order's read-back fill racing the broker's own row (brief 26, 3; brief
    /// 28, E): the fill opens a round trip, and while the trade it is given is
    /// read back, another connection stores the broker's row and the fill gives
    /// way to it. The stale-anchor guards of `settle_trades` meet the move: after
    /// the next change the engine is a fresh build's, and the round trip keeps one
    /// trade, orphaned by nothing.
    #[test]
    fn a_fill_giving_way_while_its_trade_is_read_back_orphans_no_trade() {
        use bagholder_core::order::{Asker, BrokerStatus, OrderEvent, OrderKind, Reading, Side, TimeInForce};
        let run = |race: bool| -> (usize, usize) {
            let home = tempfile::tempdir().unwrap();
            let now = at("2025-11-19T21:00:00Z");
            {
                let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
                let ws = bagholder_core::Broker::named("wealthsimple");
                let conn = book.add_connection(&ws, "Wealthsimple", now).unwrap();
                let kind = bagholder_core::account::AccountType::Known { kind: bagholder_core::account::AccountKind::Margin, registration: bagholder_core::account::Registration::Unregistered, managed: false, joint: false };
                book.add_account(conn, &[bagholder_core::account::AccountRef::new(ws, "acct-race")], &kind, bagholder_core::account::AccountStatus::Open, Some("Trading"), now).unwrap();
            }
            let f = Figures::open(home.path(), now).unwrap();
            f.state_zone("America/Toronto", now).unwrap();
            let book = f.book().unwrap();
            let order = bagholder_book::orders::OrderRequest {
                venue_mic: None,
                venue_name: None,
                id: "order-race".into(),
                broker: "wealthsimple".into(),
                broker_account: "acct-race".into(),
                broker_security: "sec-race".into(),
                symbol: "RACE".into(),
                currency: bagholder_core::Currency::USD,
                side: Side::Buy,
                kind: OrderKind::Limit,
                quantity: bagholder_core::Dec::from_int(10),
                limit_price: Some(bagholder_core::Dec::from_int(102)),
                stop_price: None,
                time_in_force: TimeInForce::Day,
                bracket: None,
                request: serde_json::json!({}),
            };
            book.write_order(&order, false, &Asker::Person, now).unwrap();
            book.order_event("order-race", &Asker::Person, now, &OrderEvent::Accepted { broker_id: "ws-race".into() }).unwrap().unwrap();
            let _ = book.order_event("order-race", &Asker::Engine, now, &OrderEvent::Read(Reading::of(BrokerStatus::Filled, bagholder_core::Dec::from_int(10), Some(bagholder_core::Dec::parse("101.25").unwrap())))).unwrap();
            let give_way = {
                let home = home.path().to_path_buf();
                move || {
                    // the pull, on a connection of its own: the broker's row, and the fill giving way to it
                    let (other, _) = Book::open_in(&home, crate::app::APP_VERSION, now).unwrap();
                    let conn = other.connections().unwrap()[0].id;
                    other.store(&BrokerRow, &bagholder_book::records::Incoming { connection: Some(conn), source_key: "ws-race", payload: "{}", refs: vec![] }, now).unwrap();
                    assert_eq!(other.fills_give_way(now).unwrap(), 1);
                }
            };
            if race {
                // between the reads of the catch-up that reads back the trade opened
                // on the fill: the second on this thread (the first applies the fill)
                BETWEEN_READS.with(|h| *h.borrow_mut() = Some((1, Box::new(give_way))));
                f.record_changed(now).unwrap();
                assert!(BETWEEN_READS.with(|h| h.borrow().is_none()), "the other writer ran inside the read-back");
            } else {
                f.record_changed(now).unwrap();
                give_way();
            }
            f.record_changed(now).unwrap();
            same_as_fresh(&f, now);
            let trades = book.trades().unwrap();
            (trades.iter().filter(|t| t.orphaned_reason().is_none()).count(), trades.iter().filter(|t| t.orphaned_reason().is_some()).count())
        };
        let (kept, orphaned) = run(true);
        assert_eq!((kept, orphaned), (1, 0), "the round trip keeps one trade, and none is orphaned");
        assert_eq!(run(false), (kept, orphaned), "the race left the trades otherwise than the same writes made in turn");
    }

    /// A sale of what `BrokerRow` buys, a day before it: a short the buy then closes.
    struct EarlierRow;

    impl bagholder_book::mapping::Mapping for EarlierRow {
        fn source(&self) -> bagholder_core::SourceName {
            BrokerRow.source()
        }
        fn version(&self) -> u32 {
            BrokerRow.version()
        }
        fn map(&self, ctx: &bagholder_book::mapping::MapContext, payload: &str) -> bagholder_book::mapping::Mapped {
            let mut m = BrokerRow.map(ctx, payload);
            for l in &mut m.legs {
                l.trade_date = bagholder_core::jiff::civil::date(2025, 11, 18);
                l.kind = bagholder_core::transaction::Kind::Sell;
                l.effect = Some(bagholder_core::transaction::Effect::Open);
                l.quantity = l.quantity.map(|q| q.neg());
                l.cash = l.cash.map(|c| c.neg());
            }
            m
        }
    }

    /// A trade the engine names to move, settled by another writer between the
    /// naming and the write (brief 30, C1): the write is refused, since the trade
    /// is no longer anchored where the engine saw it, and the next change decides
    /// again, leaving the engine a fresh build's.
    #[test]
    fn a_trade_another_writer_settled_first_is_not_written_over() {
        let home = tempfile::tempdir().unwrap();
        let now = at("2025-11-19T21:00:00Z");
        let conn = {
            let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            let ws = bagholder_core::Broker::named("wealthsimple");
            let conn = book.add_connection(&ws, "Wealthsimple", now).unwrap();
            let kind = bagholder_core::account::AccountType::Known { kind: bagholder_core::account::AccountKind::Margin, registration: bagholder_core::account::Registration::Unregistered, managed: false, joint: false };
            book.add_account(conn, &[bagholder_core::account::AccountRef::new(ws, "acct-race")], &kind, bagholder_core::account::AccountStatus::Open, Some("Trading"), now).unwrap();
            conn
        };
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let book = f.book().unwrap();
        // the broker's buy, its round trip given a trade; then a short sale the day
        // before, which the buy closes: the buy opens no round trip any more, and the
        // engine names its trade to move to the short's
        book.store(&BrokerRow, &bagholder_book::records::Incoming { connection: Some(conn), source_key: "ws-b", payload: "{}", refs: vec![] }, now).unwrap();
        f.record_changed(now).unwrap();
        book.store(&EarlierRow, &bagholder_book::records::Incoming { connection: Some(conn), source_key: "ws-a", payload: "{}", refs: vec![] }, now).unwrap();
        let settled = std::rc::Rc::new(std::cell::RefCell::new(None));
        {
            let (home, settled) = (home.path().to_path_buf(), settled.clone());
            BEFORE_SETTLE_WRITES.with(|h| {
                *h.borrow_mut() = Some(Box::new(move |named: Vec<(bagholder_core::TradeId, bagholder_core::journal::Opening)>| {
                    // the other writer, on a connection of its own, settles the trade first
                    let (other, _) = Book::open_in(&home, crate::app::APP_VERSION, now).unwrap();
                    let (trade, seen) = named.into_iter().next().expect("the engine names a trade");
                    assert!(other.orphan_unclaimed(trade, &seen).unwrap());
                    *settled.borrow_mut() = Some(trade);
                }))
            });
        }
        f.record_changed(now).unwrap();
        let trade = settled.borrow().expect("the other writer ran between the naming and the write");
        let t = book.trade(trade).unwrap();
        assert!(t.orphaned_reason().is_some() && !matches!(t.anchor, bagholder_core::journal::Anchor::Opening(_)), "the move was written over the other writer's settling: {t:?}");
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
    }

    /// A hand-entered round trip of one unit on `day_in`, sold on `day_out`, of a
    /// listing the book does not hold, in the Manual account.
    fn round_trip(f: &Figures, now: Timestamp, day_in: &str, day_out: Option<&str>) {
        let trade = |day: &str, side: &str| crate::entries::EntryRequest::Trade {
            account: String::new(),
            instrument: None,
            symbol: "JOINQ".into(),
            exchange: "XNYS".into(),
            currency: "USD".into(),
            day: day.into(),
            side: side.into(),
            quantity: "1".into(),
            price: "10".into(),
            fee: "0".into(),
        };
        crate::entries::enter(f, &trade(day_in, "buy"), now).unwrap();
        if let Some(out) = day_out {
            crate::entries::enter(f, &trade(out, "sell"), now).unwrap();
        }
    }

    /// The trades a back-dated buy's round trip joined, orphaned together in set
    /// writes (#538): one settling leaves the figures a fresh build's, with no
    /// second change needed, and each joined trade keeps its journal.
    #[test]
    fn trades_a_back_dated_buy_joined_are_orphaned_in_one_settling() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        for (a, b) in [("2025-11-03", "2025-11-04"), ("2025-11-05", "2025-11-06"), ("2025-11-07", "2025-11-10"), ("2025-11-11", "2025-11-12")] {
            round_trip(&f, now, a, Some(b));
        }
        let book = f.book().unwrap();
        let trades = |f: &Figures| f.read(|e| e.inputs().ledger.trades.iter().filter(|t| matches!(t.anchor, bagholder_core::journal::Anchor::Opening(_))).count()).unwrap();
        let before = trades(&f);
        round_trip(&f, now, "2025-11-01", None);
        // one round trip from the first buy on: the first trade moves to its
        // opening, and the three after it are joined into it
        let orphaned = book.trades().unwrap().into_iter().filter(|t| t.orphaned_reason().is_some_and(|r| r.starts_with("its round trip joined"))).count();
        assert_eq!(orphaned, 3, "every later trade joined");
        assert_eq!(trades(&f), before - 3);
        same_as_fresh(&f, now);
    }

    /// A joined trade another writer settles between the engine naming it and the
    /// set write (#538, as brief 30's C1 for a move): the set write leaves it as
    /// that writer left it, orphans the others, and the next change leaves the
    /// figures a fresh build's.
    #[test]
    fn a_joined_trade_another_writer_settled_first_is_not_written_over() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        for (a, b) in [("2025-11-03", "2025-11-04"), ("2025-11-05", "2025-11-06"), ("2025-11-07", "2025-11-10")] {
            round_trip(&f, now, a, Some(b));
        }
        let settled = std::rc::Rc::new(std::cell::RefCell::new(None));
        {
            let (home, settled) = (home.path().to_path_buf(), settled.clone());
            BEFORE_SETTLE_WRITES.with(|h| {
                *h.borrow_mut() = Some(Box::new(move |named: Vec<(bagholder_core::TradeId, bagholder_core::journal::Opening)>| {
                    let (other, _) = Book::open_in(&home, crate::app::APP_VERSION, now).unwrap();
                    // the last named is a joined one (moves, then unclaimed, then joined)
                    let (trade, seen) = named.into_iter().last().expect("the engine names the joined trades");
                    assert!(other.orphan_unclaimed(trade, &seen).unwrap());
                    *settled.borrow_mut() = Some(trade);
                }))
            });
        }
        round_trip(&f, now, "2025-11-01", None);
        let trade = settled.borrow().expect("the other writer ran between the naming and the write");
        let book = f.book().unwrap();
        assert_eq!(book.trade(trade).unwrap().orphaned_reason().as_deref(), Some(Book::UNCLAIMED_REASON), "the set write wrote over the other writer's settling");
        let joined = book.trades().unwrap().into_iter().filter(|t| t.orphaned_reason().is_some_and(|r| r.starts_with("its round trip joined"))).count();
        assert_eq!(joined, 1, "the other orphaned as joined");
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
    }

    /// A pull that fails after its first catch-up (brief 28, E; brief 30, C6):
    /// what that catch-up applied and named still reaches the names the page
    /// holds, which are read whole again when next asked, and what it moved
    /// reaches the pages.
    #[test]
    fn a_pull_that_fails_after_its_catch_up_keeps_the_names_right() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        f.names().unwrap();
        let book = f.book().unwrap();
        let instrument: String = book.conn_for_tests().query_row("SELECT instrument_id FROM transactions WHERE instrument_id IS NOT NULL LIMIT 1", [], |r| r.get(0)).unwrap();
        book.conn_for_tests().execute("DELETE FROM instrument_refs WHERE instrument_id = ?1 AND scheme LIKE 'broker-security:%'", [&instrument]).unwrap();
        book.conn_for_tests().execute("INSERT INTO instrument_refs (scheme, value, instrument_id) VALUES ('broker-security:wealthsimple', 'sec-s-before-the-failure', ?1)", [&instrument]).unwrap();
        // and a record removed: what the catch-up moves on the figures
        let removed = book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap()[0];
        book.mark_removed(removed, now).unwrap();
        let (version, _) = (f.version(), f.woken());
        AFTER_CATCH_UP.with(|h| *h.borrow_mut() = Some(Box::new(|| Err("a failure planted after the catch-up".into()))));
        assert!(f.record_changed(now).is_err(), "the planted failure is the pull's");
        // what the catch-up moved reached the pages, and the scheduler was asked to look (brief 30, C6)
        assert!(f.version() > version, "what the catch-up moved was dropped with the failure");
        assert!(f.woken(), "the scheduler was not asked to look again");
        let id: bagholder_core::InstrumentId = instrument.parse().unwrap();
        assert_eq!(f.names().unwrap().security.get(&id).map(String::as_str), Some("sec-s-before-the-failure"), "the names the page holds lost what the catch-up named");
        same_as_fresh(&f, now);
    }

    /// The change log that cannot be trimmed (brief 27, 7): the pull's work stands,
    /// the failure is logged, and the next change trims it.
    #[test]
    fn a_log_that_cannot_be_trimmed_leaves_the_pulls_work_standing() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let book = f.book().unwrap();
        book.conn_for_tests().execute_batch("CREATE TRIGGER planted_no_trim BEFORE DELETE ON ledger_changes BEGIN SELECT RAISE(ABORT, 'planted'); END;").unwrap();
        let record = book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap()[0];
        book.mark_removed(record, now).unwrap();
        f.record_changed(now).expect("a log that cannot be trimmed does not fail the pull");
        same_as_fresh(&f, now);
        let left: i64 = book.conn_for_tests().query_row("SELECT COUNT(*) FROM ledger_changes", [], |r| r.get(0)).unwrap();
        assert!(left > 0, "the planted fault kept the log");
        book.conn_for_tests().execute_batch("DROP TRIGGER planted_no_trim;").unwrap();
        let other = book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap()[0];
        book.mark_removed(other, now).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        let left: i64 = book.conn_for_tests().query_row("SELECT COUNT(*) FROM ledger_changes", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "the next change trims what the log held");
    }

    /// An account its broker states in US dollars (2.5.6 refused it, and every
    /// start with it failed): the figures are built, and its value is in the
    /// total at the live rate, as every other figure in another currency is.
    #[test]
    fn an_account_stated_in_us_dollars_is_built_and_counted_in_cad() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        {
            let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            book.state_zone("America/Toronto", now).unwrap();
            let account = book.accounts().unwrap()[0].id;
            let conn = book.connections().unwrap()[0].id;
            let read = book.broker_read(conn, "accounts", now).unwrap();
            book.store_net_value(account, now, bagholder_core::Money::new(bagholder_core::Dec::from_int(1000), bagholder_core::Currency::USD), &read).unwrap();
        }
        let f = Figures::open(home.path(), now).expect("a start with an account stated in US dollars");
        assert!(f.read(|_| ()).is_some(), "the figures are built");
        same_as_fresh(&f, now);
    }

    /// A record the figures cannot be built from never stops the server: the
    /// start goes on with no figures, and the next pass says why.
    #[test]
    fn a_book_the_figures_cannot_be_built_from_starts_and_says_why() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        {
            let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            book.state_zone("America/Toronto", now).unwrap();
            let account = book.accounts().unwrap()[0].id;
            let conn = book.connections().unwrap()[0].id;
            let read = book.broker_read(conn, "accounts", now).unwrap();
            book.store_net_value(account, now, bagholder_core::Money::new(bagholder_core::Dec::from_int(1000), bagholder_core::Currency::CAD), &read).unwrap();
            // a value no reader can read, written around the book
            book.conn_for_tests().execute("UPDATE account_values SET amount = 'not a number'", []).unwrap();
        }
        let f = Figures::open(home.path(), now).expect("the server starts");
        assert!(f.read(|_| ()).is_none(), "no figures from a record they cannot be built from");
        let why = f.ensure_built(now).expect_err("the pass's failure");
        assert!(why.contains("not a number"), "{why}");
    }

    #[test]
    fn no_figure_is_built_before_a_page_states_a_zone() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let f = Figures::open(home.path(), at("2025-11-19T21:00:00Z")).unwrap();
        assert!(f.read(|_| ()).is_none());
        f.state_zone("America/Halifax", at("2025-11-19T21:00:00Z")).unwrap();
        assert!(f.read(|e| e.figures().positions.len()).unwrap() > 0);
        same_as_fresh(&f, at("2025-11-19T21:00:00Z"));
        // kept in the book: the next start builds at once
        let again = Figures::open(home.path(), at("2025-11-19T22:00:00Z")).unwrap();
        assert!(again.read(|_| ()).is_some());
    }

    #[test]
    fn each_writer_leaves_the_engine_as_a_fresh_build_would() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("Asia/Kolkata", now).unwrap();
        // the page holds the broker's names: each change keeps them by key
        f.names().unwrap();
        // another zone: "today" moves with it
        f.state_zone("Pacific/Honolulu", now).unwrap();
        same_as_fresh(&f, now);
        let book = f.book().unwrap();
        // a read that stored nothing new computes nothing
        assert!(f.rates_changed().unwrap().is_empty());
        // a quote for something held
        let (held, currency) = f.read(|e| {
            let p = &e.figures().positions[0];
            (p.instrument, e.inputs().ledger.instruments[&p.instrument].instrument.currency)
        }).unwrap();
        let cache = f.cache().unwrap();
        let price = bagholder_core::Money::new(bagholder_core::Dec::parse("12.34").unwrap(), currency);
        cache.store_quote(&bagholder_sources::cache::StoredQuote { instrument: held, source: bagholder_core::SourceName::named("tmx"), price, change: None, change_pct: None, quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now }).unwrap();
        assert!(!f.price_changed(held).unwrap().is_empty(), "its position moved");
        same_as_fresh(&f, now);
        // the ex-dividend date its quote states, kept with the quote
        cache.store_ex_dividend(held, &bagholder_core::SourceName::named("tmx"), Some("2025-12-15".parse().unwrap()), now).unwrap();
        f.price_changed(held).unwrap();
        assert_eq!(f.read(|e| e.inputs().market.ex_dividends.get(&held).copied()).unwrap(), Some("2025-12-15".parse().unwrap()));
        same_as_fresh(&f, now);
        // the payer's declared record
        let row = bagholder_book::facts::DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: "2025-11-03".parse().unwrap(), record_date: None, pay_date: None, amount: bagholder_core::Money::new(bagholder_core::Dec::parse("0.10").unwrap(), currency), reinvested: None };
        book.store_declared(held, &[row], &bagholder_core::SourceName::named("tmx"), now).unwrap();
        f.payer_changed(held).unwrap();
        same_as_fresh(&f, now);
        // a rate the Bank published
        book.store_rates(bagholder_core::Currency::USD, &[("2025-11-18".parse().unwrap(), bagholder_core::Dec::parse("1.4012").unwrap())], ("2025-11-18".parse().unwrap(), "2025-11-18".parse().unwrap()), &bagholder_core::SourceName::named("bank-of-canada"), now).unwrap();
        f.rates_changed().unwrap();
        same_as_fresh(&f, now);
        // the record changed: a row the broker no longer lists removed
        let removed = book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap()[0];
        book.mark_removed(removed, now).unwrap();
        assert!(!f.record_changed(now).unwrap().is_empty());
        same_as_fresh(&f, now);
        // the broker states an account's cash and what it can borrow
        let account = f.read(|e| e.figures().positions[0].account).unwrap();
        let connection = book.connections().unwrap()[0].id;
        let read = book.broker_read(connection, "cash", now).unwrap();
        book.store_cash(account, now, &[(bagholder_core::Currency::CAD, bagholder_core::Dec::parse("42.00").unwrap())].into_iter().collect(), &read).unwrap();
        book.store_buying_power(account, now, &Ok(bagholder_core::Money::new(bagholder_core::Dec::parse("100").unwrap(), bagholder_core::Currency::CAD)), &read).unwrap();
        assert!(!f.broker_changed(account).unwrap().is_empty());
        same_as_fresh(&f, now);
        assert!(f.broker_changed(account).unwrap().is_empty(), "the same statement again moves nothing");
        // an account the broker names by an id of its own arrives
        let t = bagholder_core::account::AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Tfsa, managed: false, joint: false };
        let named = bagholder_core::account::AccountRef::new(bagholder_core::Broker::named("wealthsimple"), "a-new-account");
        book.add_account(connection, &[named], &t, bagholder_core::account::AccountStatus::Open, Some("New"), now).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        // the next day
        let later = at("2025-11-20T21:00:00Z");
        assert!(!f.clock_moved(later).unwrap().is_empty());
        same_as_fresh(&f, later);
    }

    /// A record replaced by another (an imported row by the broker's own, as the
    /// first pull after an import does) moves the trades on it to the new row in
    /// the same write. The engine held its trades as they were before: settling
    /// the round trips against those would call the moved trade's old opening gone
    /// and orphan it over the book's own anchor, cutting its journal loose (the
    /// owner's note, after the first sync). The trade keeps its id and its note.
    #[test]
    fn a_trade_whose_record_is_replaced_keeps_its_id_and_its_note_through_the_change() {
        use bagholder_core::journal::{Anchor, Grade, JournalEntry, JournalSubject};
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2025-11-19T21:00:00Z");
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let book = f.book().unwrap();
        // a trade on a record of its own (the only transaction of its record)
        let trades = book.trades().unwrap();
        let (trade, opening) = trades
            .iter()
            .find_map(|t| match &t.anchor {
                Anchor::Opening(o) if book.transactions_of(o.transaction.record).unwrap().len() == 1 => Some((t.id, o.clone())),
                _ => None,
            })
            .expect("a trade on a one-transaction record");
        let note = JournalEntry { thesis: "long story".into(), grade: Some(Grade::A), tags: vec!["winners".into()] };
        f.write_journal(&trade.to_string(), &note, now).unwrap();
        // the same row, stored again under another key, replaces it
        let payload = book.revisions(opening.transaction.record).unwrap().pop().unwrap().2;
        let connection = book.connections().unwrap()[0].id;
        let incoming = bagholder_book::records::Incoming { connection: Some(connection), source_key: "the same row again", payload: &payload, refs: vec![] };
        let replaced = book.store_superseding(&bagholder_wealthsimple::mapping::WealthsimpleMapping, &incoming, &[opening.transaction.record], "the broker's own row", now).unwrap();
        f.record_changed(now).unwrap();
        let after = book.trade(trade).unwrap();
        match after.anchor {
            Anchor::Opening(o) => assert_eq!(o.transaction.record, replaced.record, "the trade is on the row that replaced its own"),
            Anchor::Orphaned(why) => panic!("the trade was orphaned: {why}"),
        }
        assert_eq!(book.journal(JournalSubject::Trade(trade)).unwrap(), Some(note), "its note is still on it");
        assert!(book.orphaned_journal().unwrap().is_empty());
        let shown = f.read(|e| e.figures().trades.iter().any(|t| t.trade == Some(trade))).unwrap();
        assert!(shown, "the figures show the trade by its id");
        same_as_fresh(&f, now);
    }

    /// A day, month or "today" is in the zone of the page in use, for every zone
    /// a page can state, and a date a source states never moves with it.
    #[test]
    fn every_zone_a_page_can_state_puts_today_in_it_and_moves_no_stated_date() {
        let home = tempfile::tempdir().unwrap();
        pulled(home.path());
        let now = at("2026-03-08T07:30:00Z"); // around a daylight-saving change in several zones
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("UTC", now).unwrap();
        let stated = |f: &Figures| f.read(|e| e.figures().trades.iter().map(|t| (t.opened_on, t.closed_on)).collect::<Vec<_>>()).unwrap();
        let before = stated(&f);
        let mut n = 0;
        for name in bagholder_core::jiff::tz::db().available() {
            let name = name.as_str();
            let Ok(zone) = bagholder_core::jiff::tz::TimeZone::get(name) else { continue };
            if f.state_zone(name, now).is_err() {
                continue; // not a zone (a rules file such as `posixrules`)
            }
            let today = f.read(|e| e.inputs().clock.today).unwrap();
            assert_eq!(today, now.to_zoned(zone.clone()).date(), "{name}");
            assert_eq!(stated(&f), before, "{name}: a stated date moved");
            n += 1;
        }
        assert!(n > 300, "{n} zones");
    }

    /// Nothing on the figure path reads the machine's own zone.
    #[test]
    fn the_machines_own_zone_is_never_read_on_the_figure_path() {
        let forbidden = ["TimeZone::system", "Zoned::now", "today_local", "Local::now", "chrono::Local"];
        let hit = |text: &str| text.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| forbidden.iter().any(|f| l.contains(f)));
        assert!(hit("let z = TimeZone::system();"), "the scan finds what it looks for");
        let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files: Vec<PathBuf> = ["server/src/figures.rs", "server/src/due.rs", "server/src/engine_inputs.rs", "server/src/read_sources.rs", "server/src/compare.rs"].iter().map(|f| crates.join(f)).collect();
        let mut dirs: Vec<PathBuf> = ["engine/src", "sources/src", "book/src", "broker/src", "wealthsimple/src"].iter().map(|d| crates.join(d)).collect();
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    files.push(p);
                }
            }
        }
        let this = std::fs::read_to_string(crates.join("server/src/figures.rs")).unwrap();
        let this = crate::tests_source::production(&this);
        let found: Vec<String> = files
            .iter()
            .filter(|p| {
                let text = std::fs::read_to_string(p).unwrap();
                let text = if p.ends_with("server/src/figures.rs") { this.to_string() } else { text };
                hit(&text)
            })
            .map(|p| p.display().to_string())
            .collect();
        assert!(found.is_empty(), "{found:?}");
    }
}
