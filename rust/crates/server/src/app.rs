//! The running app: where it keeps its data, what it knows about the
//! Wealthsimple session and the work in flight, the derived model it serves,
//! and the small tools every part of the server shares.

use rusqlite::Connection;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use bagholder_model::base::Base;

pub const APP_VERSION: &str = "1.46.3";
/// Bumped whenever the page and the server change together.
pub const PROTOCOL: &str = "2026-09-19.1";
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
pub struct Watched<T>(Mutex<T>);

pub struct WatchedGuard<'a, T> {
    guard: std::sync::MutexGuard<'a, T>,
    written: bool,
}

impl<T> Watched<T> {
    pub fn new(v: T) -> Watched<T> {
        Watched(Mutex::new(v))
    }
    /// Never fails: a writer that panicked left a state still worth showing.
    pub fn lock(&self) -> Result<WatchedGuard<'_, T>, std::convert::Infallible> {
        Ok(WatchedGuard { guard: self.0.lock().unwrap_or_else(|e| e.into_inner()), written: false })
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
            crate::events::signal();
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
    model: crate::model_cache::ModelCache,
    jobs: Mutex<HashMap<String, Job>>,
}

static APP: OnceLock<App> = OnceLock::new();

pub fn app() -> &'static App {
    APP.get().expect("the app is set up at start")
}

pub fn init(home: PathBuf, root: PathBuf, bind_host: String) -> &'static App {
    APP.get_or_init(|| App {
        home,
        root,
        bind_host,
        port: Mutex::new(0),
        started_at: now_iso(),
        state: Watched::new(State::default()),
        stop: AtomicBool::new(false),
        stop_bell: (Mutex::new(()), std::sync::Condvar::new()),
        exit_code: AtomicI32::new(0),
        model: crate::model_cache::ModelCache::new(),
        jobs: Mutex::new(HashMap::new()),
    })
}

impl App {
    pub fn db_path(&self) -> PathBuf {
        self.home.join("bagholder.db")
    }

    /// A connection to the store, ready.
    pub fn open(&self) -> rusqlite::Result<Connection> {
        // every connection passes here: a test never opens the live database
        bagholder_store::connect(&self.home)
    }

    pub fn ws_home(&self) -> bagholder_ws::session::Home {
        bagholder_ws::session::Home::new(&self.home)
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
        let _ = c.wait_timeout_while(g, d, |_| !self.stopping());
        self.stopping()
    }

    /// Wait until the app stops, however long that is.
    pub fn wait_stop(&self) {
        let (m, c) = &self.stop_bell;
        let g = m.lock().unwrap_or_else(|e| e.into_inner());
        drop(c.wait_while(g, |_| !self.stopping()));
    }

    /// Stop: every waiter wakes at once.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _g = self.stop_bell.0.lock().unwrap_or_else(|e| e.into_inner());
        self.stop_bell.1.notify_all();
        drop(_g);
        crate::events::signal(); // the streams and anything parked on a change, too
    }

    /// The model as it stands: each layer rebuilt only when something it reads
    /// has changed (`model_cache`).
    pub fn base(&self) -> rusqlite::Result<std::sync::Arc<Base>> {
        let conn = self.open()?;
        let today = bagholder_model::clock::today_local();
        let base = self.model.base(&conn, &today)?;
        // Once a run: notes an older version kept per trade are carried into the
        // journal. The save moves the journal's counter, so the next build has them.
        static CARRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !CARRIED.swap(true, Ordering::SeqCst) {
            let notes = bagholder_store::snapshot::notes_part(&conn)?;
            if !notes.is_empty() && bagholder_store::snapshot::journal(&conn)?.is_empty() {
                let groups = bagholder_store::snapshot::groups_part(&conn)?;
                let migrated = bagholder_model::symbols_of::migrate_legacy_notes(&base.book.fifo.closed, &groups, &notes);
                if !migrated.is_empty() {
                    bagholder_store::admin::save_journal(&conn, Some(&Value::Object(migrated)))?;
                    return self.model.base(&conn, &today);
                }
            }
        }
        Ok(base)
    }

    /// The model as the page is sent it, under these filters and with this
    /// trade's detail: built once for a base, then shared.
    pub fn view(&self, filters: Option<&Value>, detail: Option<&str>) -> rusqlite::Result<std::sync::Arc<Value>> {
        let base = self.base()?;
        Ok(self.model.view(&base, filters, detail))
    }

    /// Forget the model. Nothing in the app needs this -- the store's counters say
    /// exactly what changed -- it is for a test that swaps the database under it.
    #[cfg(test)]
    pub fn invalidate(&self) {
        self.model.clear();
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
    pub fn kick<F: FnOnce() + Send + 'static>(&'static self, name: &str, f: F) -> bool {
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
    let _ = std::thread::Builder::new().name(name.to_string()).spawn(f);
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

pub fn today_utc() -> String {
    now_iso()[..10].to_string()
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

/// An absent or unreadable number is the default.
pub fn num(v: Option<&Value>, default: Option<f64>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => default,
        Some(Value::String(t)) if t.is_empty() => default,
        Some(Value::Number(n)) => n.as_f64().or(default),
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        Some(Value::String(t)) => bagholder_model::textrules::parse_float(t).or(default),
        _ => default,
    }
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

pub fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

pub fn log(line: &str) {
    eprintln!("{}", line);
}

/// `qty_text`: whole numbers bare, fractions without trailing zeros.
pub fn qty_text(q: f64) -> String {
    if q.fract() == 0.0 {
        format!("{}", q as i64)
    } else {
        let t = format!("{:.6}", q);
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

