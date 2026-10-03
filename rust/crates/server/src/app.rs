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


pub const APP_VERSION: &str = "2.0.13";
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

pub struct App {
    pub home: PathBuf,
    pub root: PathBuf,
    pub bind_host: String,
    pub port: Mutex<u16>,
    pub started_at: String,
    pub state: Watched<State>,
    pub stop: AtomicBool,
    stop_bell: (Mutex<()>, std::sync::Condvar),
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
            cache: Arc::new(bagholder_sqlite::pool::Pool::with_hook(&cache_file, hook).with_schema(bagholder_sources::cache::SCHEMA.latest() as i32, migrate)),
            home,
            root,
            bind_host,
            port: Mutex::new(0),
            started_at: now_iso(),
            state: Watched::new(State::default(), events.clone()),
            stop: AtomicBool::new(false),
            stop_bell: (Mutex::new(()), std::sync::Condvar::new()),
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

    /// Wait `d`, or until the app stops: true when it stopped. The thread sleeps in
    /// the kernel until one or the other -- it does not wake to look. (It used to
    /// sleep in quarter-second slices to check a flag, and with some seventeen loops
    /// waiting that was about seventy wake-ups a second from an app doing nothing.)
    pub fn wait(&self, d: Duration) -> bool {
        let (m, c) = &self.stop_bell;
        let g = m.lock().unwrap_or_else(|e| e.into_inner());
        // woken by the stop or by the time running out: either way the answer is whether it stopped
        let (_g, _timed_out) = c.wait_timeout_while(g, d, |_| !self.stopping()).unwrap_or_else(|e| e.into_inner());
        self.stopping()
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

/// A switch in the environment: set to anything but blanks, it is on.
pub fn env_on(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.to_string_lossy().trim().is_empty())
}

pub fn log(line: &str) {
    eprintln!("{}", line);
}


