//! The running app: where it keeps its data, what it knows about the
//! Wealthsimple session and the work in flight, the derived model it serves,
//! and the small tools every part of the server shares.

use rusqlite::Connection;
use serde_json::{json, Map, Value};
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

struct Cache {
    version: String,
    base: Option<std::sync::Arc<Base>>,
}

struct Job {
    running: bool,
    until: Option<Instant>,
}

pub struct App {
    pub home: PathBuf,
    pub root: PathBuf,
    pub bind_host: String,
    pub port: Mutex<u16>,
    pub started_at: String,
    pub state: Mutex<State>,
    pub stop: AtomicBool,
    pub exit_code: AtomicI32,
    cache: Mutex<Cache>,
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
        state: Mutex::new(State::default()),
        stop: AtomicBool::new(false),
        exit_code: AtomicI32::new(0),
        cache: Mutex::new(Cache { version: String::new(), base: None }),
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

    /// Sleep that ends early when the app stops; true when it stopped.
    pub fn wait(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if self.stopping() {
                return true;
            }
            std::thread::sleep((until - Instant::now()).min(Duration::from_millis(250)));
        }
        self.stopping()
    }

    /// Rebuilt when the data or the day has changed.
    pub fn base(&self) -> rusqlite::Result<std::sync::Arc<Base>> {
        let conn = self.open()?;
        let today = bagholder_model::clock::today_local();
        let version = format!("{}|{}", crate::versions::data_version(&conn)?, today);
        {
            let cache = self.cache.lock().unwrap();
            if let Some(b) = cache.base.as_ref() {
                if cache.version == version {
                    return Ok(b.clone());
                }
            }
        }
        let snapshot = bagholder_store::snapshot::snapshot(&conn, true)?;
        let market = bagholder_store::market::market_data(&conn)?;
        let mut journal = bagholder_store::snapshot::journal(&conn)?;
        let mut version = version;
        let notes = snapshot.get("notes").and_then(|n| n.as_object()).cloned().unwrap_or_default();
        if journal.is_empty() && !notes.is_empty() {
            let probe = bagholder_model::base::build_base(&snapshot, &market, &journal, Some(&today));
            let groups = snapshot.get("tradeGroups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
            let migrated = bagholder_model::symbols_of::migrate_legacy_notes(&probe.book.fifo.closed, &groups, &notes);
            if !migrated.is_empty() {
                journal = bagholder_store::admin::save_journal(&conn, Some(&Value::Object(migrated)))?;
                version = format!("{}|{}", crate::versions::data_version(&conn)?, today);
            }
        }
        let base = std::sync::Arc::new(bagholder_model::base::build_base(&snapshot, &market, &journal, Some(&today)));
        let mut cache = self.cache.lock().unwrap();
        cache.version = version;
        cache.base = Some(base.clone());
        Ok(base)
    }

    pub fn invalidate(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.version.clear();
        cache.base = None;
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

/// A patch for the store's row updaters.
pub fn patch(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
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

pub fn ok(pairs: Value) -> Value {
    let mut m = obj(pairs);
    m.shift_insert(0, "ok".into(), json!(true));
    Value::Object(m)
}
