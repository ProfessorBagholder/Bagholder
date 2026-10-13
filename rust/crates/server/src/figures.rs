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
}

impl<'a> Held<'a> {
    pub(crate) fn new(applied: &'a std::sync::atomic::AtomicI64, movements: &'a std::sync::Mutex<crate::csv_import::Movements>) -> Held<'a> {
        Held { applied, movements, named: Default::default() }
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
        let delta = engine_inputs::delta(book, &changes)?;
        Ok((changes, delta))
    })?;
    let mut moved = Moved::default();
    if !delta.is_empty() {
        let records: Vec<bagholder_core::RecordId> = delta.records.keys().copied().collect();
        let rows = |e: &Engine| -> Vec<bagholder_core::transaction::Transaction> { records.iter().flat_map(|r| e.inputs().ledger.transactions.of_record(*r).cloned().collect::<Vec<_>>()).collect() };
        let gone = rows(engine);
        merge(&mut moved, engine.apply(Change::Ledger(delta)));
        held.movements.lock().unwrap_or_else(|e| e.into_inner()).changed(&gone, &rows(engine));
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
        let before = book.changes_upto().map_err(err)?;
        for (trade, key) in &identity.moved {
            if let Some(s) = seen(trade) {
                book.move_trade(*trade, &s, &Opening { transaction: key.opening.clone(), instrument: key.instrument }).map_err(err)?;
            }
        }
        for key in &identity.needs_trade {
            book.open_trade(&Opening { transaction: key.opening.clone(), instrument: key.instrument }, None, at).map_err(err)?;
        }
        for (joined, keeps) in &identity.joined {
            if let Some(s) = seen(joined) {
                book.orphan_joined(*joined, *keeps, &s).map_err(err)?;
            }
        }
        for trade in &identity.unclaimed {
            if let Some(s) = seen(trade) {
                book.orphan_unclaimed(*trade, &s).map_err(err)?;
            }
        }
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
    let (mut e, upto) = in_snapshot(book, || Ok((build(book, cache, clock(zone, now)?)?, book.changes_upto().map_err(err)?)))?;
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
    Ok(Built { engine: e, applied, movements })
}

/// An engine built whole, with the sequence of the change log it holds and its
/// rows by movement: put in place by `Figures::install`.
struct Built {
    engine: Engine,
    applied: i64,
    movements: crate::csv_import::Movements,
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
    /// The ledger's rows by movement, and the movements touched since linking
    /// last looked (`csv_import::link`): kept with the engine.
    movements: std::sync::Mutex<crate::csv_import::Movements>,
}

impl Figures {
    /// Open the figure path in `home` at `at`: the old store imported into a new
    /// book the first time (its journal, groups and notes carried; the first pull
    /// then supersedes its rows), both files brought to this build's schema, and
    /// the engine built when the book holds a zone. A book or cache this build
    /// cannot open stops the server, saying why.
    pub fn open(home: &Path, at: Timestamp) -> Result<Figures, String> {
        let book_path = home.join(bagholder_book::BOOK_FILE);
        let old = home.join(OLD_FILE);
        if !book_path.exists() && old.exists() {
            crate::legacy_import::import(&old, home, at).map_err(|e| format!("the earlier database could not be carried into the book: {e}"))?;
        }
        let f = Figures { home: home.to_path_buf(), engine: RwLock::new(None), wake: std::sync::atomic::AtomicBool::new(false), version: std::sync::atomic::AtomicU64::new(1), log: std::sync::Mutex::new(Default::default()), names: RwLock::new(None), heard: std::sync::OnceLock::new(), book_heard: std::sync::OnceLock::new(), failures_conn: std::sync::Mutex::new(None), applied: std::sync::atomic::AtomicI64::new(0), movements: Default::default() };
        let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(|e| format!("the book could not be opened: {e}"))?;
        let (cache, _) = MarketCache::open(&home.join(CACHE_FILE), crate::app::APP_VERSION, at).map_err(|e| format!("the market cache could not be opened: {e}"))?;
        rederive_all(&book, at)?;
        if let Some(z) = book.zone().map_err(err)? {
            let b = built(home, &book, &cache, &z.zone, at)?;
            let mut engine = f.engine.write().unwrap_or_else(|e| e.into_inner());
            f.install(&mut engine, &book, b);
        }
        Ok(f)
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
        let Built { engine: e, applied, mut movements } = b;
        movements.pend_every_file_row(&e.inputs().ledger);
        *engine = Some(e);
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
        let n = self.read(|e| crate::wire::build::Names::load(&book, e.inputs())).unwrap_or_else(|| Ok(Default::default()))?;
        *self.names.write().unwrap_or_else(|e| e.into_inner()) = Some(n.clone());
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
        let Some(e) = guard.as_mut() else { return Ok(Moved::default()) };
        // the record by what changed in it, then each round trip given its trade:
        // a pull that supersedes a record moves the trades on it to the broker's
        // rows in the same write, read back with it
        let held = Held::new(&self.applied, &self.movements);
        let mut moved = catch_up(&book, e, &held)?;
        merge(&mut moved, settle_trades(&book, e, &held, now)?);
        let named = held.named.take();
        let mut brokers = std::collections::BTreeMap::new();
        for a in stated_for {
            brokers.insert(*a, engine_inputs::broker_account(*a, &book.stated(*a).map_err(|e| e.to_string())?)?);
        }
        for (a, now_stated) in brokers {
            if e.inputs().market.brokers.get(&a) != now_stated.as_ref() {
                merge(&mut moved, e.apply(Change::Broker(a, now_stated)));
            }
        }
        // the broker's names of what the change named read again, where the page holds them
        if let Some(n) = self.names.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
            n.reload(&book, e.inputs(), named.instruments, named.accounts)?;
        }
        drop(guard);
        self.wake();
        Ok(self.moved(moved))
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
        let mut stated = engine_inputs::broker_account(account, &now)?;
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

    /// The engine the figure path holds, against one built fresh from the files.
    #[track_caller]
    fn same_as_fresh(f: &Figures, now: Timestamp) {
        let book = f.book().unwrap();
        let zone = book.zone().unwrap().unwrap().zone;
        let fresh = build(&book, &f.cache().unwrap(), clock(&zone, now).unwrap()).unwrap();
        let held = f.read(|e| e.differences(&fresh)).expect("built");
        assert!(held.is_empty(), "{held:?}");
        // what the change log brought in is the record as a whole read gives it
        let (ledger, adjustments) = f.read(|e| (e.inputs().ledger.clone(), e.inputs().facts.adjustments.clone())).unwrap();
        assert!(ledger == engine_inputs::ledger(&book).unwrap(), "the ledger applied by key is not the ledger read whole");
        assert!(adjustments == engine_inputs::facts(&book).unwrap().adjustments, "the adjustments applied by key are not those read whole");
        // the broker's names the page holds, kept by key, are those read whole
        let held = f.names().unwrap();
        let whole = f.read(|e| crate::wire::build::Names::load(&book, e.inputs())).unwrap().unwrap();
        assert!(held == whole, "the names kept by key are not those read whole");
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
        // a broker's id for it, under the member
        book.conn_for_tests().execute("INSERT INTO instrument_refs (scheme, value, instrument_id) VALUES ('broker-security:wealthsimple', 'sec-s-another-id', ?1)", [&member]).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
        // a record under the member removed
        let record: String = book.conn_for_tests().query_row("SELECT t.record_id FROM transactions t JOIN source_records r ON r.id = t.record_id WHERE t.instrument_id = ?1 AND r.state = 'live' LIMIT 1", [&member], |r| r.get(0)).unwrap();
        book.mark_removed(record.parse().unwrap(), now).unwrap();
        f.record_changed(now).unwrap();
        same_as_fresh(&f, now);
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
