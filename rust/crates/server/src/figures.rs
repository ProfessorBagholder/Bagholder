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

/// Every round trip given its trade in the book: one opened for each that has
/// none, a trade a correction joined into another's orphaned, and one whose
/// opening no longer opens anything orphaned, its notes kept; then the trades
/// applied. What moved.
fn settle_trades(book: &Book, engine: &mut Engine, at: Timestamp) -> Result<Moved, String> {
    let identity = engine.identity().clone();
    if identity.needs_trade.is_empty() && identity.joined.is_empty() && identity.unclaimed.is_empty() {
        return Ok(Moved::default());
    }
    for key in &identity.needs_trade {
        book.open_trade(&bagholder_core::journal::Opening { transaction: key.opening.clone(), instrument: key.instrument }, None, at).map_err(err)?;
    }
    for (joined, keeps) in &identity.joined {
        book.orphan_joined(*joined, *keeps).map_err(err)?;
    }
    for trade in &identity.unclaimed {
        book.orphan_unclaimed(*trade).map_err(err)?;
    }
    Ok(engine.apply(Change::Trades(book.trades().map_err(err)?)))
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
    /// Counts each change to the figures, so a page's stream rebuilds its
    /// document only when one moved.
    version: std::sync::atomic::AtomicU64,
    /// What the page calls each instrument by the broker, read once per record.
    names: RwLock<Option<crate::wire::build::Names>>,
    /// Told of every commit to the cache on a connection `cache` hands out (the
    /// app's bus, `App::set_figures`): a source's outcome recorded is a change
    /// the header may show.
    heard: std::sync::OnceLock<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// The connection the header's source failures are read on, opened once.
    failures_conn: std::sync::Mutex<Option<MarketCache>>,
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
        let f = Figures { home: home.to_path_buf(), engine: RwLock::new(None), wake: std::sync::atomic::AtomicBool::new(false), version: std::sync::atomic::AtomicU64::new(1), names: RwLock::new(None), heard: std::sync::OnceLock::new(), failures_conn: std::sync::Mutex::new(None) };
        let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(|e| format!("the book could not be opened: {e}"))?;
        let (cache, _) = MarketCache::open(&home.join(CACHE_FILE), crate::app::APP_VERSION, at).map_err(|e| format!("the market cache could not be opened: {e}"))?;
        rederive_all(&book, at)?;
        if let Some(z) = book.zone().map_err(err)? {
            let mut e = build(&book, &cache, clock(&z.zone, at)?)?;
            settle_trades(&book, &mut e, at)?;
            *f.engine.write().unwrap_or_else(|e| e.into_inner()) = Some(e);
        }
        Ok(f)
    }

    /// Build the engine again from what the book and the cache hold now: after
    /// Clear data, where what changed is everything. Nothing is built before a
    /// page has stated its zone.
    pub fn rebuild(&self, now: Timestamp) -> Result<(), String> {
        let book = self.book()?;
        let Some(z) = book.zone().map_err(err)? else { return Ok(()) };
        let mut e = build(&book, &self.cache()?, clock(&z.zone, now)?)?;
        settle_trades(&book, &mut e, now)?;
        *self.engine.write().unwrap_or_else(|e| e.into_inner()) = Some(e);
        *self.names.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.version.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.wake();
        Ok(())
    }

    /// A connection to the book, of this thread's own.
    pub fn book(&self) -> Result<Book, String> {
        Book::open_in(&self.home, crate::app::APP_VERSION, Timestamp::now()).map(|(b, _)| b).map_err(err)
    }

    /// A connection to the market cache, of this thread's own; its commits are
    /// heard (`hear`).
    pub fn cache(&self) -> Result<MarketCache, String> {
        let (c, _) = MarketCache::open(&self.home.join(CACHE_FILE), crate::app::APP_VERSION, Timestamp::now()).map_err(err)?;
        if let Some(heard) = self.heard.get() {
            c.on_commit(heard.clone());
        }
        Ok(c)
    }

    /// Have `heard` told of every commit to the cache from now on. Once: the
    /// app's bus.
    pub fn hear(&self, heard: std::sync::Arc<dyn Fn() + Send + Sync>) {
        let _ = self.heard.set(heard);
    }

    /// Each market source failing now, one sentence each (`health::failures`):
    /// read from what the cache recorded of each source, the one record of
    /// them, so a failure shows exactly until that source next answers.
    pub fn source_failures(&self) -> Result<Vec<String>, String> {
        let mut conn = self.failures_conn.lock().unwrap_or_else(|e| e.into_inner());
        if conn.is_none() {
            // only read on: it has nothing of its own to be heard
            *conn = Some(MarketCache::open(&self.home.join(CACHE_FILE), crate::app::APP_VERSION, Timestamp::now()).map_err(err)?.0);
        }
        let newest = conn.as_ref().expect("opened above").newest_counted().map_err(err)?;
        Ok(bagholder_sources::health::failures(&newest))
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
            self.version.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        m
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
                let mut e = build(&book, &self.cache()?, clock(&zone, now)?)?;
                settle_trades(&book, &mut e, now)?;
                *engine = Some(e);
                self.version.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        let journal = book.journal_entries().map_err(|e| JournalRefused::Failed(e.to_string()))?.into_iter().collect();
        Ok(self.apply(Change::Journal(journal)))
    }

    /// The record changed (a pull stored, revised or removed records; an entry, an
    /// import): the ledger and the adjustments applied, each round trip given its
    /// trade, and what the broker stated beside the rows.
    pub fn record_changed(&self, now: Timestamp) -> Result<Moved, String> {
        let book = self.book()?;
        let ledger = engine_inputs::ledger(&book)?;
        let adjustments = engine_inputs::facts(&book)?.adjustments;
        let brokers = engine_inputs::brokers(&book)?;
        let mut guard = self.engine.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = guard.as_mut() else { return Ok(Moved::default()) };
        let mut moved = Moved::default();
        if e.inputs().ledger != ledger {
            merge(&mut moved, e.apply(Change::Ledger(ledger)));
        }
        if e.inputs().facts.adjustments != adjustments {
            merge(&mut moved, e.apply(Change::Adjustments(adjustments)));
        }
        merge(&mut moved, settle_trades(&book, e, now)?);
        let accounts: BTreeSet<bagholder_core::AccountId> = brokers.keys().chain(e.inputs().market.brokers.keys()).copied().collect();
        for a in accounts {
            let now_stated = brokers.get(&a).cloned();
            if e.inputs().market.brokers.get(&a) != now_stated.as_ref() {
                merge(&mut moved, e.apply(Change::Broker(a, now_stated)));
            }
        }
        drop(guard);
        // the broker's names for what the record holds are read again
        *self.names.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.wake();
        Ok(self.moved(moved))
    }

    /// What the broker states of an account now (its cash, what it can borrow)
    /// changed.
    pub fn broker_changed(&self, account: bagholder_core::AccountId) -> Result<Moved, String> {
        let stated = engine_inputs::brokers(&self.book()?)?.remove(&account);
        Ok(self.apply_if(|i| i.market.brokers.get(&account) != stated.as_ref(), || Change::Broker(account, stated.clone())))
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

    /// An instrument's quote or daily closes changed.
    pub fn price_changed(&self, instrument: InstrumentId) -> Result<Moved, String> {
        let m = market(&self.book()?, &self.cache()?)?;
        let (quote, closes) = (m.quotes.get(&instrument).cloned(), m.closes.get(&instrument).cloned().unwrap_or_default());
        let mut moved = self.apply_if(|i| i.market.quotes.get(&instrument) != quote.as_ref(), || Change::Quote(instrument, quote.clone()));
        merge(&mut moved, self.apply_if(|i| i.market.closes.get(&instrument).cloned().unwrap_or_default() != closes, || Change::Closes(instrument, closes.clone())));
        Ok(moved)
    }

    /// A benchmark's tracker changed.
    pub fn benchmark_changed(&self, key: &str) -> Result<Moved, String> {
        let m = market(&self.book()?, &self.cache()?)?;
        let series = m.benchmarks.get(key).cloned();
        Ok(self.apply_if(|i| i.market.benchmarks.get(key) != series.as_ref(), || Change::Benchmark(key.to_string(), series.clone())))
    }
}

fn merge(into: &mut Moved, more: Moved) {
    for (entity, fields) in more.0 {
        into.0.entry(entity).or_default().extend(fields);
    }
}


/// Every source's mapping, as the book stores records under it.
pub fn mappings() -> [&'static dyn bagholder_book::mapping::Mapping; 4] {
    [&bagholder_wealthsimple::mapping::WealthsimpleMapping, &bagholder_book::import::mapping::ImportMapping, &bagholder_book::person::PersonMapping, &bagholder_broker::csv::CsvMapping]
}

/// Derive again every record a newer version of its mapping reads differently: a
/// mapping's version moves exactly when what it makes of a stored row changes,
/// so the rows already stored follow it, not only the ones read after.
fn rederive_all(book: &Book, at: Timestamp) -> Result<(), String> {
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
        // the next day
        let later = at("2025-11-20T21:00:00Z");
        assert!(!f.clock_moved(later).unwrap().is_empty());
        same_as_fresh(&f, later);
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
        let this = &this[..this.find("#[cfg(test)]").unwrap()];
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
