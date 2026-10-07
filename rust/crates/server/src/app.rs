//! The running app: where it keeps its data, what it knows about the
//! Wealthsimple session and the work in flight, the derived model it serves,
//! and the small tools every part of the server shares.

use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};


pub const APP_VERSION: &str = "2.5.1";
/// Bumped whenever the page and the server change together.
pub const PROTOCOL: &str = "2026-09-26.2";
/// Bump when title/summary logic improves, so a row that is missing a half is
/// read again. A row that has both keeps them: a re-read of everything costs a
/// download and a reading each, which is minutes of a list standing still.
pub const ENRICH_VERSION: i64 = 11;
pub const REPO: &str = "ProfessorBagholder/Bagholder";

/// What the header and the loops know.
#[derive(Default)]
pub struct State {
    pub connected: bool,
    pub email: String,
    pub last_sync: String,
    pub capturing: bool,
    pub syncing: bool,
    pub listings_filling: bool,
    pub sync_step: String,
    pub error: String,
    /// What the reading of balances and buying power between syncs could not read; cleared by the next read that could.
    pub portfolio_error: String,
    /// Why the last pass of the figures failed; cleared by the next pass that succeeds.
    pub figures_error: String,
    /// Each month whose broker statement the book does not reconcile with, as
    /// the last pull found it; cleared by a pull that finds none.
    pub statement_error: String,
    pub sync_fails: i64,
    pub sync_first_fail: String,
    pub login_attempt: i64,
    pub chrome_proc: Option<Child>,
    pub chrome_pid: u32,
    pub updating: String,
    pub update_error: String,
    /// The import running now, and how far it has come; none when none runs.
    pub importing: Option<crate::csv_import::Importing>,
    /// The person asked for the running import to stop.
    pub import_stop: bool,
    /// The last import that ended, and what it did.
    pub imported: Option<crate::csv_import::Imported>,
}

struct Job {
    running: bool,
    until: Option<Instant>,
}

/// A lock whose guard says when it was written through: the page shows this state
/// (the sync step, connected, an update's progress), so a write to it is a change
/// the page is told of, and no writer has to remember to say so. A read is silent.
pub struct Watched<T> {
    inner: Mutex<T>,
    bus: Arc<crate::events::Bus>,
}

pub struct WatchedGuard<'a, T> {
    guard: std::sync::MutexGuard<'a, T>,
    bus: &'a crate::events::Bus,
    written: bool,
}

impl<T> Watched<T> {
    pub fn new(v: T, bus: Arc<crate::events::Bus>) -> Watched<T> {
        Watched { inner: Mutex::new(v), bus }
    }
    /// Never fails: a writer that panicked left a state still worth showing.
    pub fn lock(&self) -> Result<WatchedGuard<'_, T>, std::convert::Infallible> {
        Ok(WatchedGuard { guard: self.inner.lock().unwrap_or_else(|e| e.into_inner()), bus: &self.bus, written: false })
    }
}

impl<T> std::ops::Deref for WatchedGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for WatchedGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.written = true;
        &mut self.guard
    }
}

impl<T> Drop for WatchedGuard<'_, T> {
    fn drop(&mut self) {
        if self.written {
            self.bus.signal();
        }
    }
}

/// The clock the app's waits are measured on: the wall clock, which goes on through
/// a machine's sleep, where every timer an OS offers does not (Linux's monotonic
/// clock, macOS's awake time and Windows' unbiased interrupt time all stop while the
/// machine is suspended). A wait measured by the OS would end however long the
/// machine slept after it was due; measured here, a deadline that passed while the
/// machine slept is met within `check` of its waking, the same on every device.
pub struct Wall {
    /// Added to the system's clock: none outside tests, which move it to stand for
    /// a machine that slept.
    offset_ms: std::sync::atomic::AtomicI64,
    /// How often a wait longer than it looks at the wall clock again, in ms.
    check_ms: std::sync::atomic::AtomicU64,
    /// The last time the machine was seen to have slept: from, to.
    slept: Mutex<Option<(std::time::SystemTime, std::time::SystemTime)>>,
}

/// How often a long wait looks at the wall clock: a minute, as cron looks for the
/// jobs that are due (cronie and Vixie cron wake each minute), the established
/// way a scheduler meets the deadlines a suspend passed over. A deadline that came
/// due while the machine slept is met at most this long after it wakes.
pub const WALL_CHECK: Duration = Duration::from_secs(60);

impl Default for Wall {
    fn default() -> Wall {
        Wall { offset_ms: std::sync::atomic::AtomicI64::new(0), check_ms: std::sync::atomic::AtomicU64::new(WALL_CHECK.as_millis() as u64), slept: Mutex::new(None) }
    }
}

impl Wall {
    pub fn now(&self) -> std::time::SystemTime {
        let off = self.offset_ms.load(Ordering::SeqCst);
        let now = std::time::SystemTime::now();
        if off >= 0 { now + Duration::from_millis(off as u64) } else { now - Duration::from_millis(off.unsigned_abs()) }
    }

    pub fn check(&self) -> Duration {
        Duration::from_millis(self.check_ms.load(Ordering::SeqCst))
    }

    /// What is left of a wait until `until`, and how long to sleep in the kernel for
    /// before looking at the clock again; none once it has come.
    pub fn slice(&self, until: std::time::SystemTime) -> Option<Duration> {
        let left = until.duration_since(self.now()).ok().filter(|l| !l.is_zero())?;
        Some(left.min(self.check()))
    }

    /// A wait of `slice` that began at `before` and ended at `after` on the wall
    /// clock: where the clock moved on by more than the wait and a check besides,
    /// the machine was asleep in between, and the log says so once, whichever of
    /// the waiting threads saw it first.
    pub fn waited(&self, before: std::time::SystemTime, slice: Duration, after: std::time::SystemTime) {
        if after.duration_since(before).is_ok_and(|d| d > slice + self.check()) {
            let mut slept = self.slept.lock().unwrap_or_else(|e| e.into_inner());
            if slept.is_some_and(|(_, to)| after.duration_since(to).map_or(true, |d| d <= self.check())) {
                return;
            }
            *slept = Some((before + slice, after));
            let at = |t: std::time::SystemTime| match bagholder_core::jiff::Timestamp::try_from(t) {
                Ok(t) => t.to_string(),
                Err(e) => format!("a moment the clock cannot state ({e})"),
            };
            log(&format!("bagholder: this machine was asleep from about {} to {}: what came due meanwhile is read now", at(before + slice), at(after)));
        }
    }

    /// The last time the machine was seen to have slept. Tests only.
    #[cfg(test)]
    pub fn last_sleep(&self) -> Option<(std::time::SystemTime, std::time::SystemTime)> {
        *self.slept.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A machine that slept for `d`: the wall clock moves on and no timer does. Tests only.
    #[cfg(test)]
    pub fn sleep_through(&self, d: Duration) {
        self.offset_ms.fetch_add(d.as_millis() as i64, Ordering::SeqCst);
    }

    /// How often waits look at the clock. Tests only.
    #[cfg(test)]
    pub fn set_check(&self, d: Duration) {
        self.check_ms.store(d.as_millis() as u64, Ordering::SeqCst);
    }
}

pub struct App {
    pub home: PathBuf,
    pub root: PathBuf,
    pub bind_host: String,
    pub port: Mutex<u16>,
    pub started_at: String,
    pub state: Watched<State>,
    pub stop: AtomicBool,
    stop_bell: (Mutex<()>, std::sync::Condvar),
    /// The clock every wait is measured on (`Wall`).
    pub wall: Wall,
    pub exit_code: AtomicI32,
    market: crate::market_context::MarketContext,
    /// The market cache (`market.db`), for the earlier readers' tables it holds
    /// (news, filings, exposures, gauges, short interest, universes, chart bars,
    /// and what the readers remember by key): connections kept between uses.
    cache: Arc<bagholder_sqlite::pool::Pool>,
    jobs: Mutex<HashMap<String, Job>>,
    /// Every page's live connection to this app: what changed, who is looking,
    /// what each open stream is showing.
    pub events: Arc<crate::events::Bus>,
    /// The bus told that a store changed, from the commit on: the one signal every
    /// store connection is heard by (`bagholder_sqlite::on_commit`).
    pub store_signal: Arc<dyn Fn() + Send + Sync>,
    /// The open ticket's quote, kept while some page shows it.
    pub docs: crate::docs::DocsState,
    /// The streamed Wealthsimple login window: the socket to it and its casts.
    pub login: crate::login::LoginState,
    /// The market-data loops' own state: what they are reading now, and for whom.
    pub feeds: crate::feeds::FeedsState,
    /// The one thread that shows this app's notifications, and what wakes it.
    pub notify: crate::notify::NotifyState,
    /// The order loops' own state: brackets in flight, and the last readback.
    pub orders: crate::orders::OrdersState,
    /// The figure path: the book, the market cache and the engine, opened when
    /// the server starts (`figures`).
    pub figures: std::sync::OnceLock<crate::figures::Figures>,
    /// The network the figure path's readers and the Wealthsimple session use:
    /// the process's one limiter, on the machine's clock (a test answers it).
    pub net: bagholder_net::Net,
    /// Sync now was asked: the broker's reads pull at once (`broker_reads`).
    pub pull_asked: AtomicBool,
    /// Fills of Bagholder's own orders whose Wealthsimple row the book does not
    /// hold yet, by Wealthsimple's order id, and when each was first seen waiting.
    pub fill_waits: Mutex<std::collections::BTreeMap<String, bagholder_core::jiff::Timestamp>>,
    /// Counts each change to what the person follows (the watchlist, the tile
    /// row), which the book keeps: the Markets tab's documents read it
    /// (`following`).
    following: std::sync::atomic::AtomicU64,
}

impl App {
    pub fn new(home: PathBuf, root: PathBuf, bind_host: String) -> Arc<App> {
        // a test's app never reaches beyond this machine, whether or not the test
        // said so (`tests_common::home` sets the process offline and dry)
        #[cfg(test)]
        crate::tests_common::home();
        App::with_net(home, root, bind_host, bagholder_net::Net::new(Arc::new(bagholder_net::SystemClock), bagholder_net::machine::shared()))
    }

    /// An app on the network given.
    pub fn with_net(home: PathBuf, root: PathBuf, bind_host: String, net: bagholder_net::Net) -> Arc<App> {
        let events = Arc::new(crate::events::Bus::new());
        let hook = {
            let events = events.clone();
            std::sync::Arc::new(move || events.signal_from(crate::events::Source::Store)) as std::sync::Arc<dyn Fn() + Send + Sync>
        };
        let cache_file = home.join(crate::figures::CACHE_FILE);
        // brought to this build's schema on the first borrow, and again on the
        // borrow that finds the file replaced or rolled back under the running app:
        // by the cache's own migrations, on a connection of their own
        let migrate = {
            let path = cache_file.clone();
            Arc::new(move |_: &rusqlite::Connection| {
                bagholder_sources::cache::MarketCache::open(&path, APP_VERSION, bagholder_core::jiff::Timestamp::now())
                    .map(|_| ())
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            }) as bagholder_sqlite::pool::Prepare
        };
        Arc::new(App {
            store_signal: hook.clone(),
            cache: Arc::new(bagholder_sqlite::pool::Pool::with_hook(&cache_file, hook).with_schema(bagholder_sources::cache::SCHEMA.latest() as i32, migrate)),
            home,
            root,
            bind_host,
            port: Mutex::new(0),
            started_at: now_iso(),
            state: Watched::new(State::default(), events.clone()),
            stop: AtomicBool::new(false),
            stop_bell: (Mutex::new(()), std::sync::Condvar::new()),
            wall: Wall::default(),
            exit_code: AtomicI32::new(0),
            market: crate::market_context::MarketContext::new(),
            jobs: Mutex::new(HashMap::new()),
            events,
            docs: crate::docs::DocsState::default(),
            login: crate::login::LoginState::default(),
            feeds: crate::feeds::FeedsState::default(),
            notify: crate::notify::NotifyState::default(),
            orders: crate::orders::OrdersState::from_env(),
            figures: std::sync::OnceLock::new(),
            net,
            pull_asked: AtomicBool::new(false),
            fill_waits: Mutex::new(std::collections::BTreeMap::new()),
            following: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Run `f` on a thread of its own, named `name`, handed this app.
    pub fn spawn_with<F: FnOnce(Arc<App>) + Send + 'static>(self: &Arc<Self>, name: &str, f: F) {
        let app = self.clone();
        spawn(name, move || f(app));
    }

    /// A connection to the market cache, on loan from the pool: used as a
    /// `&Connection` and given back when dropped. Its commits are heard on the
    /// bus (`events::Source::Store`).
    pub fn cache(&self) -> rusqlite::Result<bagholder_sqlite::pool::Pooled<'_>> {
        // every connection passes here: a test never opens the live data folder
        bagholder_store::guard_home(&self.home).map_err(|_| rusqlite::Error::InvalidPath(self.home.clone()))?;
        self.cache.get()
    }

    /// Hold the figure path, its cache's commits heard on this app's bus as the
    /// store's are: a source's outcome recorded reaches the header the same way
    /// any change does. Once; a second call leaves the first.
    pub fn set_figures(&self, f: crate::figures::Figures) {
        let events = self.events.clone();
        f.hear(Arc::new(move || events.signal_from(crate::events::Source::Cache)));
        f.hear_book(self.store_signal.clone());
        match self.figures.set(f) {
            Ok(()) => {}
            // a second call leaves the first, as said: the path already held stays
            Err(_second) => {}
        }
    }

    pub fn ws_home(&self) -> bagholder_ws::session::Home {
        bagholder_ws::session::Home::new(&self.home)
    }

    /// The market cache's pool itself, for a connection opened outside a
    /// request's lifetime (a background fetch on a thread of its own): still a
    /// connection this pool hands out, so its commits are heard the same way.
    pub fn cache_pool(&self) -> rusqlite::Result<Arc<bagholder_sqlite::pool::Pool>> {
        bagholder_store::guard_home(&self.home).map_err(|_| rusqlite::Error::InvalidPath(self.home.clone()))?;
        Ok(self.cache.clone())
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Wait `d` by the wall clock (`Wall`), or until the app stops: true when it
    /// stopped. The thread sleeps in the kernel until one or the other, looking at
    /// the wall clock again once a minute in a longer wait, so a machine that slept
    /// through the deadline meets it on waking. The stop wakes it at once. (It used
    /// to sleep in quarter-second slices to check a flag, and with some seventeen
    /// loops waiting that was about seventy wake-ups a second from an app doing
    /// nothing; a minute is one wake-up each for a loop waiting longer than that.)
    pub fn wait(&self, d: Duration) -> bool {
        let until = self.wall.now() + d;
        let (m, c) = &self.stop_bell;
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.stopping() {
                return true;
            }
            let Some(slice) = self.wall.slice(until) else { return false };
            let before = self.wall.now();
            g = c.wait_timeout_while(g, slice, |_| !self.stopping()).unwrap_or_else(|e| e.into_inner()).0;
            self.wall.waited(before, slice, self.wall.now());
        }
    }

    /// Stop: every waiter wakes at once.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _g = self.stop_bell.0.lock().unwrap_or_else(|e| e.into_inner());
        self.stop_bell.1.notify_all();
        drop(_g);
        self.events.signal(); // the streams and anything parked on a change, too
        self.notify.wake_streams();
    }

    /// What the person follows changed: every page's stream looks.
    pub fn followed(&self) {
        self.following.fetch_add(1, Ordering::SeqCst);
        self.events.signal();
    }

    /// Which change to what the person follows the app is at.
    pub fn following_version(&self) -> u64 {
        self.following.load(Ordering::SeqCst)
    }

    /// The market's context the earlier readers are given (`market_context`).
    pub fn market_base(&self) -> Result<std::sync::Arc<bagholder_model::context::MarketBase>, String> {
        Ok(self.market.get(self)?.base.clone())
    }

    /// The market's context with what the person follows, as the Markets tab's
    /// documents read it (`market_context`).
    pub fn market_context(&self) -> Result<std::sync::Arc<crate::market_context::Built>, String> {
        self.market.get(self)
    }

    /// Run `f` only when no other call of that name
    /// is in flight; otherwise answer `busy` at once.
    pub fn single_flight<T, F: FnOnce() -> T>(&self, name: &str, busy: T, f: F) -> T {
        {
            let mut jobs = self.jobs.lock().unwrap();
            let job = jobs.entry(name.to_string()).or_insert(Job { running: false, until: None });
            if job.running {
                return busy;
            }
            job.running = true;
        }
        struct Done<'a>(&'a App, String);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                let mut jobs = self.0.jobs.lock().unwrap();
                let cooldown = cooldown(&self.1);
                let job = jobs.entry(self.1.clone()).or_insert(Job { running: false, until: None });
                job.running = false;
                job.until = Some(Instant::now() + cooldown);
            }
        }
        let _done = Done(self, name.to_string());
        f()
    }

    /// Start a background job unless one is running or its
    /// cooldown holds. True when a thread was started.
    pub fn kick<F: FnOnce() + Send + 'static>(self: &Arc<Self>, name: &str, f: F) -> bool {
        {
            let jobs = self.jobs.lock().unwrap();
            if let Some(job) = jobs.get(name) {
                if job.running || job.until.map(|u| Instant::now() < u).unwrap_or(false) {
                    return false;
                }
            }
        }
        spawn(&format!("bagholder-{}", name), f);
        true
    }
}

/// How long a kind of refresh waits after one finishes before a request may
/// ask for it again.
fn cooldown(name: &str) -> Duration {
    match name {
        "quotes" => Duration::from_secs(60),
        "market" => Duration::from_secs(300),
        _ => Duration::ZERO,
    }
}

pub fn spawn<F: FnOnce() + Send + 'static>(name: &str, f: F) {
    // a thread the system will not start is what `std::thread::spawn` panics on too
    std::thread::Builder::new().name(name.to_string()).spawn(f).expect("the system could not start a thread");
}

pub fn now_unix() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// UTC, to the second.
pub fn now_iso() -> String {
    stamp_of(now_unix() as i64)
}

pub fn stamp_of(secs: i64) -> String {
    let (y, m, d) = bagholder_model::dates::from_days(secs.div_euclid(86400));
    let r = secs.rem_euclid(86400);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, r / 3600, (r % 3600) / 60, r % 60)
}

/// An ISO instant as unix seconds.
pub fn parse_instant(s: &str) -> Option<f64> {
    bagholder_market::quotes::instant_secs_public(s)
}

pub fn uuid4() -> String {
    bagholder_model::textrules::uuid4()
}

/// `str(v)` of a JSON value, "" for null.
pub fn s(v: Option<&Value>) -> String {
    bagholder_model::value::s(v.filter(|x| !x.is_null()))
}

pub fn f(v: &Value, k: &str) -> String {
    s(v.get(k))
}

pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|x| x != 0.0).unwrap_or(true),
        Some(Value::String(t)) => !t.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
    }
}

/// An environment variable the app reads: unset is `None`, the default by
/// design; a value that is not text is refused, for the caller to say.
pub fn env_text(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is set to something that is not text")),
    }
}

/// The data folder's lock, held for the life of the process.
static HOME_LOCK: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();

/// The file in the data folder whose lock is the app's hold on it.
pub const HOME_LOCK_FILE: &str = "bagholder.lock";

/// Take the data folder for this process alone (`docs/plans/stage-money.md`, part
/// D): an advisory lock on a file in it, held until the process ends, so two apps
/// never replace the same stop or book the same fill. One already held is refused,
/// naming the app that holds it as it wrote itself into the file.
pub fn hold_home(home: &std::path::Path) -> Result<(), String> {
    use std::io::{Read, Seek, Write};
    let path = home.join(HOME_LOCK_FILE);
    let mut f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path).map_err(|e| format!("the data folder's lock {} could not be opened: {e}", path.display()))?;
    match f.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            let mut held = String::new();
            // who holds it is said when it can be read; the refusal stands either way
            if f.read_to_string(&mut held).is_err() {
                held.clear();
            }
            let who = held.trim();
            return Err(format!("another Bagholder is using the data folder {} ({}); stop it first, or give this one its own folder with BAGHOLDER_HOME", home.display(), if who.is_empty() { "it has not said which" } else { who }));
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(format!("the data folder's lock {} could not be taken: {e}", path.display())),
    }
    f.set_len(0).and_then(|_| f.rewind()).and_then(|_| write!(f, "pid {}", std::process::id())).map_err(|e| format!("the data folder's lock {} could not be written: {e}", path.display()))?;
    HOME_LOCK.set(std::sync::Mutex::new(f)).map_err(|_| "the data folder was taken twice in one process".to_string())
}

/// The address this app answers on, written beside its pid in the data folder's lock,
/// so a second start can name it.
pub fn note_home_port(port: u16) {
    use std::io::{Seek, Write};
    if let Some(m) = HOME_LOCK.get() {
        let mut f = m.lock().unwrap_or_else(|e| e.into_inner());
        let wrote = f.set_len(0).and_then(|_| f.rewind()).and_then(|_| write!(f, "pid {}, http://127.0.0.1:{port}", std::process::id()));
        if let Err(e) = wrote {
            log(&format!("bagholder: the data folder's lock could not name this app's address: {e}"));
        }
    }
}

/// An on/off setting in the environment (`bagholder_net::switch`).
pub fn env_on(name: &str) -> bool {
    bagholder_net::switch::switch_on(name)
}

/// A line said by the app as it runs: to its log on disk (`logfile`), never the
/// terminal, where nothing waits to act on it; before the log is open (a start
/// that fails, a command-line tool), to stderr.
pub fn log(line: &str) {
    bagholder_core::log::line(line);
}

/// A line the person needs on the terminal they started the app from: where to
/// open it, whether orders are live. Kept in the log as well. Nothing else is said
/// there (`tests_misc::test_the_terminal_says_only_what_the_person_needs`).
pub fn say(line: &str) {
    println!("{line}");
    crate::logfile::write(line, bagholder_core::jiff::Timestamp::now());
}



#[cfg(test)]
mod wall_tests {
    use super::*;

    /// A deadline the machine slept through is met when it wakes, on every device:
    /// the app's waits are measured on the wall clock, which goes on through a
    /// sleep, where every timer an OS offers stops. Here the wall clock is moved on
    /// an hour with no time passing, as a machine asleep for that hour sees it.
    #[test]
    fn a_wait_whose_deadline_passed_while_the_machine_slept_ends_when_it_wakes() {
        let home = tempfile::tempdir().unwrap();
        let app = App::new(home.path().to_path_buf(), home.path().to_path_buf(), "127.0.0.1".into());
        app.wall.set_check(Duration::from_millis(20));
        for which in ["wait", "park_until_or"] {
            let (tx, rx) = std::sync::mpsc::channel();
            let a = app.clone();
            std::thread::spawn(move || {
                let hour = Duration::from_secs(3600);
                let stopped = if which == "wait" { a.wait(hour) } else { a.events.park_until_or(&a, hour, || false) };
                tx.send(stopped).unwrap();
            });
            assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "{which}: still waiting, its hour not come");
            app.wall.sleep_through(Duration::from_secs(3600));
            let stopped = rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| panic!("{which}: its deadline passed while the machine slept, and it did not end on waking"));
            assert!(!stopped, "{which}: the time came; nothing stopped");
            let (from, to) = app.wall.last_sleep().unwrap_or_else(|| panic!("{which}: the sleep was not seen"));
            assert!(to.duration_since(from).unwrap() >= Duration::from_secs(3500), "{which}: about the hour slept");
            *app.wall.slept.lock().unwrap() = None;
        }
        // and a stop still ends a wait at once
        let a = app.clone();
        let waiting = std::thread::spawn(move || a.wait(Duration::from_secs(3600)));
        app.request_stop();
        assert!(waiting.join().unwrap(), "stopped");
    }
}
