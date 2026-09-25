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

/// What the server holds of the figure path.
pub struct Figures {
    home: PathBuf,
    /// Built once a zone is known.
    engine: RwLock<Option<Engine>>,
    /// Something changed that can make a read due (a zone stated, the record
    /// changed): the scheduler looks again at once (`due`).
    wake: std::sync::atomic::AtomicBool,
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
        let f = Figures { home: home.to_path_buf(), engine: RwLock::new(None), wake: std::sync::atomic::AtomicBool::new(false) };
        let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(|e| format!("the book could not be opened: {e}"))?;
        let (cache, _) = MarketCache::open(&home.join(CACHE_FILE), crate::app::APP_VERSION, at).map_err(|e| format!("the market cache could not be opened: {e}"))?;
        if let Some(z) = book.zone().map_err(err)? {
            let mut e = build(&book, &cache, clock(&z.zone, at)?)?;
            settle_trades(&book, &mut e, at)?;
            *f.engine.write().unwrap_or_else(|e| e.into_inner()) = Some(e);
        }
        Ok(f)
    }

    /// A connection to the book, of this thread's own.
    pub fn book(&self) -> Result<Book, String> {
        Book::open_in(&self.home, crate::app::APP_VERSION, Timestamp::now()).map(|(b, _)| b).map_err(err)
    }

    /// A connection to the market cache, of this thread's own.
    pub fn cache(&self) -> Result<MarketCache, String> {
        MarketCache::open(&self.home.join(CACHE_FILE), crate::app::APP_VERSION, Timestamp::now()).map(|(c, _)| c).map_err(err)
    }

    /// Read the engine, once it is built.
    pub fn read<T>(&self, f: impl FnOnce(&Engine) -> T) -> Option<T> {
        self.engine.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(f)
    }

    /// Apply one change: what moved, nothing before the engine is built.
    fn apply(&self, change: Change) -> Moved {
        match self.engine.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(e) => e.apply(change),
            None => Moved::default(),
        }
    }

    /// Apply a change only where it differs from what the engine holds: a read
    /// that stored nothing new computes nothing.
    fn apply_if(&self, differs: impl FnOnce(&Inputs) -> bool, change: impl FnOnce() -> Change) -> Moved {
        match self.engine.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(e) if differs(e.inputs()) => e.apply(change()),
            _ => Moved::default(),
        }
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
                Ok(Moved::default())
            }
            Some(e) if changed => Ok(e.apply(Change::Clock(clock(&zone, now)?))),
            Some(_) => Ok(Moved::default()),
        }
    }

    /// The instant moved (the day turned in the person's zone, the Bank's 16:30
    /// passed): the clock, in the zone the book holds.
    pub fn clock_moved(&self, now: Timestamp) -> Result<Moved, String> {
        let Some(zone) = self.book()?.zone().map_err(err)? else { return Ok(Moved::default()) };
        Ok(self.apply(Change::Clock(clock(&zone.zone, now)?)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_broker::pull::pull;
    use bagholder_core::Broker;
    use bagholder_wealthsimple::adapter::Wealthsimple;
    use bagholder_wealthsimple::replay::Replay;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// A book with one account's month from the recorded Wealthsimple replies.
    fn pulled(home: &Path) {
        let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at("2025-11-19T20:00:00Z")).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", at("2025-11-19T20:00:00Z")).unwrap();
        let replies = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wealthsimple/tests/replies/wealthsimple-pull");
        let mut ws = Wealthsimple::new(Replay::read(&replies).unwrap());
        let r = pull(&book, &mut ws, connection, "2025-11-19".parse().unwrap(), at("2025-11-19T20:00:00Z")).unwrap();
        assert!(r.failures.is_empty(), "{:?}", r.failures);
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
        let row = bagholder_book::facts::DeclaredRow { ex_date: "2025-11-03".parse().unwrap(), record_date: None, pay_date: None, amount: bagholder_core::Money::new(bagholder_core::Dec::parse("0.10").unwrap(), currency), reinvested: None };
        book.store_declared(held, &[row], &bagholder_core::SourceName::named("tmx"), now).unwrap();
        f.payer_changed(held).unwrap();
        same_as_fresh(&f, now);
        // a rate the Bank published
        book.store_rates(bagholder_core::Currency::USD, &[("2025-11-18".parse().unwrap(), bagholder_core::Dec::parse("1.4012").unwrap())], ("2025-11-18".parse().unwrap(), "2025-11-18".parse().unwrap()), &bagholder_core::SourceName::named("bank-of-canada"), now).unwrap();
        f.rates_changed().unwrap();
        same_as_fresh(&f, now);
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
