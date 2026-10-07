//! The page's one connection to the server: its data once, then only what
//! changes in it (docs/architecture.md, rules 0 and 2).
//!
//! Nothing polls. A change reaches the page because it happened:
//!
//! - every database commit, on any connection, signals here (the store's commit
//!   hook, wired to this bus when the app is built);
//! - every write to the app's in-memory state signals here (`app::Watched`).
//!
//! Each open page has a stream (`GET /api/events`). On a signal the stream asks
//! for the page's view again -- the model cache makes that a counter read when
//! nothing the model reads moved, and a rebuild of just the layers that did
//! otherwise -- and compares it with what that page was last sent. What it sends
//! is the entities and the fields that differ (`model::patch`): a price moving on
//! one holding is that holding's figures and the totals that include it. The page
//! writes them into the objects it already shows, so the elements bound to them
//! update and nothing else is touched.
//!
//! The first message is the whole view (`snapshot`); the page asks for a whole
//! view again only by connecting again, which it does when its filters or its
//! open trade change. An idle stream carries a comment every fifteen seconds so
//! a dead connection is noticed; that is the transport's keepalive, not a poll.
//!
//! A stream is a task on the runtime, not a thread: it sleeps on a channel until
//! a signal (`changed`), and only the comparison itself (`Feed::step`) runs on a
//! blocking thread, for as long as it takes.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;



use crate::app::App;

/// What a page is showing: each subscription's key, its parameters, and the
/// version of it the page already holds (opened again with what it kept).
type Wanted = std::collections::BTreeMap<String, Want>;

#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub params: Value,
    pub have: Option<String>,
}

/// Every page's live connection to this app: the bell that wakes a stream (and
/// anything parked) when something changed, how many pages are looking now, and
/// what each open stream is showing beyond the model. One per app.
pub struct Bus {
    bell: (Mutex<u64>, Condvar),
    /// The same bell for whoever waits without a thread of its own (a page's
    /// stream is a task on the runtime): a channel that holds the count.
    ticker: tokio::sync::watch::Sender<u64>,
    watchers: AtomicUsize,
    /// Pages opened since the app started: what a read due "when a page opens"
    /// compares against the count it last served.
    opened: AtomicU64,
    streams:Mutex<HashMap<u64, Arc<Mutex<Wanted>>>>,
    next_stream: AtomicU64,
    /// Streams whose page saw a message out of its order: each is sent its whole
    /// state again at its next step.
    resync: Mutex<std::collections::HashSet<u64>>,
    /// Changes counted by where they came from (`Source`): what tells a document
    /// that reads none of what moved that it need not be read again.
    counts: [AtomicU64; 3],
}

/// Where a change came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A commit to the book on any connection the app opens (a note, a grade, a
    /// trade typed in, an import, the notices, the settings) and to what the
    /// earlier store kept in the market cache (feeds, the market's context tables,
    /// on the app's pool of it), told once the commit has landed
    /// (`bagholder_sqlite::on_commit`).
    Store = 0,
    /// A commit to the market cache by the figure path (`market.db`): quotes,
    /// closes, sources' outcomes.
    Cache = 1,
    /// A write to the app's own state in memory (orders read back, a ticket's
    /// quote, a sync's step), and any change said by hand.
    State = 2,
}

/// The count of changes from each source, as `Bus::stamp` reads them.
pub type Stamp = [u64; 3];

impl Bus {
    pub fn new() -> Bus {
        Bus {
            bell: (Mutex::new(0), Condvar::new()),
            ticker: tokio::sync::watch::channel(0).0,
            watchers: AtomicUsize::new(0),
            opened: AtomicU64::new(0),
            streams: Mutex::new(HashMap::new()),
            next_stream: AtomicU64::new(1),
            resync: Mutex::new(std::collections::HashSet::new()),
            counts: Default::default(),
        }
    }

    /// Something changed in the app's own state. Cheap, and safe to call from
    /// anywhere: it counts and wakes, no more.
    pub fn signal(&self) {
        self.signal_from(Source::State)
    }

    /// Something changed at `source`: counted, and whoever waits woken. Safe to
    /// call from SQLite's commit hook.
    pub fn signal_from(&self, source: Source) {
        self.counts[source as usize].fetch_add(1, Ordering::SeqCst);
        let (m, c) = &self.bell;
        *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        c.notify_all();
        self.ticker.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// The changes counted from each source so far.
    pub fn stamp(&self) -> Stamp {
        [0, 1, 2].map(|i| self.counts[i].load(Ordering::SeqCst))
    }

    /// A receiver that is told of every signal from now on.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.ticker.subscribe()
    }

    /// Pages connected now. What a periodic read of an outside source consults
    /// before it runs: nobody watching, nothing to fetch.
    pub fn watchers(&self) -> usize {
        self.watchers.load(Ordering::SeqCst)
    }

    /// Pages opened since the app started, each opening counted, a second page
    /// beside an open one included.
    pub fn opened(&self) -> u64 {
        self.opened.load(Ordering::SeqCst)
    }

    /// Sleep until `ready` says so, looking again only when something signals: no
    /// clock is consulted while it waits. False when the app is stopping instead. What
    /// background work that has nothing to do parks on -- a loop with no one watching
    /// its data, an engine with nothing armed.
    pub fn park_until(&self, app: &App, ready: impl Fn() -> bool) -> bool {
        let (m, c) = &self.bell;
        loop {
            if app.stopping() {
                return false;
            }
            let seen = *m.lock().unwrap_or_else(|e| e.into_inner());
            if ready() {
                return true;
            }
            let g = m.lock().unwrap_or_else(|e| e.into_inner());
            drop(c.wait_while(g, |n| *n == seen && !app.stopping()));
        }
    }

    /// As `park_until`, but no longer than `most`: true when `ready`, false when the
    /// time ran out or the app is stopping.
    /// `most` is measured on the wall clock (`crate::app::Wall`), so a deadline a
    /// machine slept through is met on its waking.
    pub fn park_until_or(&self, app: &App, most: Duration, ready: impl Fn() -> bool) -> bool {
        let (m, c) = &self.bell;
        let until = app.wall.now() + most;
        loop {
            if app.stopping() {
                return false;
            }
            let seen = *m.lock().unwrap_or_else(|e| e.into_inner());
            if ready() {
                return true;
            }
            let Some(slice) = app.wall.slice(until) else { return false };
            let g = m.lock().unwrap_or_else(|e| e.into_inner());
            let before = app.wall.now();
            drop(c.wait_timeout_while(g, slice, |n| *n == seen && !app.stopping()));
            app.wall.waited(before, slice, app.wall.now());
        }
    }

    /// What the page on stream `id` is showing now, beyond the model. Replaces what it
    /// said before. False when there is no such stream (it closed; the page will
    /// connect again and say so again).
    pub fn watch(&self, app: &Arc<App>, id: u64, docs: Wanted) -> bool {
        let Some(wanted) = self.streams.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned() else { return false };
        let fresh: Vec<String> = {
            let mut w = wanted.lock().unwrap_or_else(|e| e.into_inner());
            let fresh = docs.keys().filter(|k| !w.contains_key(*k)).cloned().collect();
            // a subscription kept with the same parameters keeps what it was sent
            *w = docs.into_iter().map(|(k, want)| match w.get(&k) {
                Some(was) if was.params == want.params => (k, was.clone()),
                _ => (k, want),
            }).collect();
            fresh
        };
        // a document just opened is worth reading fresh: once, now, in the background
        for key in &fresh {
            crate::docs::opened(app, key);
        }
        self.signal();
        true
    }

    /// Whether any page is showing the document `key` now.
    /// A page missed a message of stream `id`: its whole state goes again. False
    /// for a stream that is not open.
    pub fn resync(&self, id: u64) -> bool {
        if !self.streams.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id) {
            return false;
        }
        self.resync.lock().unwrap_or_else(|e| e.into_inner()).insert(id);
        self.signal();
        true
    }

    fn take_resync(&self, id: u64) -> bool {
        self.resync.lock().unwrap_or_else(|e| e.into_inner()).remove(&id)
    }

    /// Whether any page is showing a subscription of one of `kinds` (the part of a
    /// key before `:`): what a read made only for what is on screen asks.
    pub fn showing(&self, kinds: &[&str]) -> bool {
        self.streams.lock().unwrap_or_else(|e| e.into_inner()).values().any(|w| w.lock().unwrap_or_else(|e| e.into_inner()).keys().any(|k| kinds.contains(&k.split(':').next().unwrap_or(k))))
    }

    pub fn watched(&self, key: &str) -> bool {
        self.streams.lock().unwrap_or_else(|e| e.into_inner()).values().any(|w| w.lock().unwrap_or_else(|e| e.into_inner()).contains_key(key))
    }
}

impl Default for Bus {
    fn default() -> Bus {
        Bus::new()
    }
}

/// Wait for the next signal; then gather the signals that follow it for `GATHER`,
/// so a burst reaches the page as one message. The window is counted from the first
/// signal, never pushed back by the next: signals that never stop (a long import
/// telling its progress) still reach the page once a window.
pub async fn changed(rx: &mut tokio::sync::watch::Receiver<u64>) {
    if rx.changed().await.is_err() {
        return std::future::pending().await; // the sender lives as long as its app: this cannot happen
    }
    tokio::time::sleep(GATHER).await;
}

pub(crate) struct Watching(Arc<Bus>);
impl Watching {
    pub(crate) fn new(bus: &Arc<Bus>) -> Watching {
        bus.watchers.fetch_add(1, Ordering::SeqCst);
        bus.opened.fetch_add(1, Ordering::SeqCst);
        bus.signal(); // work that waits for someone to look can start
        Watching(bus.clone())
    }
}
impl Drop for Watching {
    fn drop(&mut self) {
        self.0.watchers.fetch_sub(1, Ordering::SeqCst);
        self.0.signal();
    }
}

/// How long signals are gathered before the page is told. A quote pass commits a
/// dozen rows one after another; they reach the page as one message.
const GATHER: Duration = Duration::from_millis(40);
/// How often an idle stream carries a comment, so a dead connection is noticed.
pub const KEEPALIVE: Duration = Duration::from_secs(15);

// --- documents --------------------------------------------------------------------
//
// The model is what every page shows. Some things are shown only some of the time
// -- the orders while their panel is open, the short-interest table while the
// Markets tab is, one listing's filings while its page is -- and those are sent
// only while they are: the page says what it is showing (`POST /api/events/watch`)
// and the stream carries that document too, whole once and then by change, exactly
// as it carries the model. When the page stops showing it, it stops being sent.
// This is what replaces each panel asking again every few seconds.

struct Registered(u64, Arc<Bus>);
impl Registered {
    fn new(bus: &Arc<Bus>) -> (Registered, Arc<Mutex<Wanted>>) {
        let id = bus.next_stream.fetch_add(1, Ordering::SeqCst);
        let wanted = Arc::new(Mutex::new(Wanted::new()));
        bus.streams.lock().unwrap_or_else(|e| e.into_inner()).insert(id, wanted.clone());
        (Registered(id, bus.clone()), wanted)
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        self.1.streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// The book's own id: what a page files what it keeps under. Empty, and said,
/// when it cannot be read.
pub fn book_id(app: &Arc<App>) -> String {
    match app.figures.get().map(|f| f.book().and_then(|b| b.id(bagholder_core::jiff::Timestamp::now()).map_err(|e| e.to_string()))) {
        Some(Ok(id)) => id,
        Some(Err(e)) => {
            crate::app::log(&format!("bagholder: the book's id could not be read: {e}"));
            String::new()
        }
        None => String::new(),
    }
}

/// One page's stream: what each of its subscriptions was last sent, and so what
/// to send it next.
pub struct Feed {
    app: Arc<App>,
    _watching: Watching,
    registered: Registered,
    wanted: Arc<Mutex<Wanted>>,
    /// The figures' version the views were last brought forward to.
    at: u64,
    /// The market's context the views were last built on.
    base: usize,
    /// The views of the figures this page shows, by key, with their parameters.
    views: std::collections::BTreeMap<String, (Value, Box<dyn crate::views::View>)>,
    /// The other documents it shows, as last sent, and the changes counted when they were read.
    sent_docs: std::collections::BTreeMap<String, (Value, crate::docs::Doc, Stamp)>,
    /// The header's status as last sent, for the filters it was asked with.
    status: Option<(Value, crate::status::Status)>,
    /// Subscriptions refused, with the parameters they were refused for: said once.
    refused: std::collections::BTreeMap<String, Value>,
}

/// One message on the stream: its event name and its data.
pub type Message = (&'static str, Value);

/// A version as the page keeps it: text, since a page's numbers stop being exact
/// above 2^53.
fn v_text(v: u64) -> String {
    format!("{v:016x}")
}

impl Feed {
    pub fn open(app: Arc<App>) -> Feed {
        let bus = app.events.clone();
        let (registered, wanted) = Registered::new(&bus);
        Feed { app, _watching: Watching::new(&bus), registered, wanted, at: 0, base: 0, views: Default::default(), sent_docs: Default::default(), status: None, refused: Default::default() }
    }

    /// The header's status as this page shows it: its badge counts the Orders panel's
    /// Pending cards, and the panel follows the page's account filter (`SPEC.md` §4,
    /// Orders), so the count is of the accounts in this page's scope.
    pub(crate) fn status_for(&self, status: &dyn Fn(&Arc<App>) -> crate::status::Status, params: &Value) -> crate::status::Status {
        let mut now = status(&self.app);
        let filters = params.get("filters").cloned().map(serde_json::from_value::<crate::wire::filters::Filters>).and_then(Result::ok).and_then(|f| f.to_engine().ok()).unwrap_or_default();
        if filters.accounts.is_empty() {
            return now;
        }
        // the figures name each account by the broker's id, which an order names
        let names = match self.app.figures.get().map(|f| f.names()) {
            Some(Ok(n)) => n,
            Some(Err(e)) => {
                crate::app::log(&format!("bagholder orders: the accounts in scope could not be named for the badge: {e}"));
                return now;
            }
            None => return now,
        };
        let scope: std::collections::HashSet<String> = filters.accounts.iter().filter_map(|a| names.account.get(a).cloned()).collect();
        now.open_orders = crate::orders::open_orders_count(&self.app, Some(&scope));
        now
    }

    /// This stream's id.
    pub fn id(&self) -> u64 {
        self.registered.0
    }

    /// The first message: the id the page names in `POST /api/events/watch`.
    pub fn hello(&self, book: &str) -> Message {
        ("hello", serde_json::json!({"id": self.id(), "book": book}))
    }

    /// A state first sent to the page: its version alone when the page holds it
    /// already, the whole of it otherwise.
    fn first(out: &mut Vec<Message>, key: &str, have: &Option<String>, v: u64, data: impl FnOnce() -> Value) {
        let v = v_text(v);
        if have.as_deref() == Some(v.as_str()) {
            out.push(("same", serde_json::json!({"doc": key, "v": v})));
        } else {
            out.push(("snapshot", serde_json::json!({"doc": key, "data": data(), "v": v})));
        }
    }

    /// What differs now from what this page was last sent: nothing when nothing
    /// does. Reads the store and the engine, so it runs off the runtime's own threads.
    pub fn step(&mut self, status: &dyn Fn(&Arc<App>) -> crate::status::Status) -> Vec<Message> {
        let mut out: Vec<Message> = Vec::new();
        if self.app.events.take_resync(self.id()) {
            // the page missed a message: everything it shows is sent whole again
            self.views.clear();
            self.sent_docs.clear();
            self.status = None;
            self.refused.clear();
            let mut w = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
            for want in w.values_mut() {
                want.have = None;
            }
        }
        let want: Wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.views.retain(|k, (params, _)| want.get(k).is_some_and(|w| w.params == *params));
        self.sent_docs.retain(|k, (params, _, _)| want.get(k).is_some_and(|w| w.params == *params));
        let stamp = self.app.events.stamp();
        self.refused.retain(|k, params| want.get(k).is_some_and(|w| w.params == *params));
        self.figures(&want, &mut out);
        for (key, w) in &want {
            if self.refused.contains_key(key) {
                continue;
            }
            if key == "status" {
                let now = self.status_for(status, &w.params);
                match &self.status {
                    Some((p, was)) if *p == w.params => {
                        let ops = bagholder_diff::typed(was, &now);
                        if !ops.is_empty() {
                            out.push(("patch", serde_json::json!({"doc": key, "ops": ops, "v": v_text(crate::views::version_of(&now))})));
                        }
                    }
                    _ => Self::first(&mut out, key, &w.have, crate::views::version_of(&now), || serde_json::to_value(&now).unwrap_or(Value::Null)),
                }
                self.status = Some((w.params.clone(), now));
                continue;
            }
            if crate::views::open(key, &Value::Null).is_some() {
                continue; // a view of the figures: above
            }
            // read again only when what it reads moved
            if let Some((_, _, at)) = self.sent_docs.get(key) {
                if crate::docs::sources(key).iter().all(|s| at[*s as usize] == stamp[*s as usize]) {
                    continue;
                }
            }
            let app = &self.app;
            let key2 = key.clone();
            let Ok(Some(now)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::docs::read(app, &key2))) else { continue };
            match self.sent_docs.get(key) {
                None => Self::first(&mut out, key, &w.have, crate::views::version_of(&now), || serde_json::to_value(&now).unwrap_or(Value::Null)),
                Some((_, was, _)) => {
                    let ops = bagholder_diff::typed(was, &now);
                    if !ops.is_empty() {
                        out.push(("patch", serde_json::json!({"doc": key, "ops": ops, "v": v_text(crate::views::version_of(&now))})));
                    }
                }
            }
            self.sent_docs.insert(key.clone(), (w.params.clone(), now, stamp));
        }
        out
    }

    /// The views of the figures this page shows: each opened is built whole, and
    /// each already open is brought forward by what moved since it was last.
    fn figures(&mut self, want: &Wanted, out: &mut Vec<Message>) {
        // each subscription asked for anew is opened, its parameters read strictly:
        // one they do not fit is said to the page once
        let mut fresh: Vec<String> = Vec::new();
        for (key, w) in want {
            if self.views.contains_key(key) || self.refused.contains_key(key) {
                continue;
            }
            match crate::views::open(key, &w.params) {
                None => {}
                Some(Err(why)) => {
                    self.refused.insert(key.clone(), w.params.clone());
                    out.push(("refused", serde_json::json!({"doc": key, "error": why})));
                }
                Some(Ok(view)) => {
                    self.views.insert(key.clone(), (w.params.clone(), view));
                    fresh.push(key.clone());
                }
            }
        }
        if self.views.is_empty() {
            return;
        }
        let Some(f) = self.app.figures.get() else { return };
        let names = match f.names() {
            Ok(n) => n,
            Err(e) => {
                crate::app::log(&format!("bagholder: the broker's names could not be read: {e}"));
                return;
            }
        };
        // the market's context: what the person follows, and the tables the
        // earlier store keeps (rebuilt only when one of them moved)
        let context = match self.app.market_context() {
            Ok(c) => c,
            Err(e) => {
                for key in self.views.keys() {
                    out.push(("refused", serde_json::json!({"doc": key, "error": format!("the market's context: {e}")})));
                }
                self.views.clear();
                return;
            }
        };
        let base_ptr = Arc::as_ptr(&context) as usize;
        let base_moved = self.base != base_ptr;
        let at = self.at;
        let views = &mut self.views;
        let done = f.read(|engine| {
            let since = f.moved_since(at);
            let now_at = f.version();
            let door = crate::wire::context::Door { built: &context, app: &self.app };
            let cx = crate::views::Cx { engine, names: &names, tables: &door, following: &context.following };
            let none = bagholder_engine::engine::Moved::default();
            for (key, (_, view)) in views.iter_mut() {
                if fresh.contains(key) {
                    let data = view.snapshot(&cx);
                    Self::first(out, key, &want[key].have, view.version(), || data);
                    continue;
                }
                let moved = match &since {
                    crate::figures::Since::Everything => {
                        let data = view.snapshot(&cx);
                        out.push(("snapshot", serde_json::json!({"doc": key, "data": data, "v": v_text(view.version())})));
                        continue;
                    }
                    crate::figures::Since::Moved(m) => m,
                    crate::figures::Since::Nothing => &none,
                };
                if !view.reads(moved, base_moved) {
                    continue;
                }
                let ops = view.update(&cx, moved, base_moved);
                if !ops.is_empty() {
                    out.push(("patch", serde_json::json!({"doc": key, "ops": ops, "v": v_text(view.version())})));
                }
            }
            now_at
        });
        // a past day's holdings worked out for a screen: their closes are read
        if f.read(|e| e.past_built()) == Some(true) {
            f.wake();
        }
        match done {
            Some(now_at) => {
                self.at = now_at;
                self.base = base_ptr;
            }
            // nothing built yet (no page has stated a zone): opened again when it is
            None => self.views.retain(|k, _| !fresh.contains(k)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Signals that never stop (a long import telling its progress) still reach the
    /// page: the gathering ends a window after the first signal, never pushed back.
    #[test]
    fn signals_that_never_stop_still_reach_the_page_once_a_window() {
        let bus = Arc::new(Bus::new());
        let stop = Arc::new(AtomicBool::new(false));
        let (b2, s2) = (bus.clone(), stop.clone());
        let ringing = std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                b2.signal();
                std::thread::yield_now();
            }
        });
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let mut rx = bus.subscribe();
        let (tx, told) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            rt.block_on(async {
                for _ in 0..3 {
                    changed(&mut rx).await;
                    rx.borrow_and_update();
                }
            });
            let _ = tx.send(());
        });
        // under a gathering pushed back by each signal this never comes
        let came = told.recv_timeout(Duration::from_secs(10));
        stop.store(true, Ordering::SeqCst);
        ringing.join().unwrap();
        assert!(came.is_ok(), "the page was never told while the signals went on");
    }

    /// Work that waits for someone to look does nothing until a page connects, and
    /// starts the moment one does.
    #[test]
    fn test_parked_work_starts_when_a_page_connects_and_not_before() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let ran = Arc::new(AtomicBool::new(false));
        let t = {
            let ran = ran.clone();
            let app = app.clone();
            std::thread::spawn(move || {
                if app.events.park_until(&app, || app.events.watchers() > 0) {
                    ran.store(true, Ordering::SeqCst);
                }
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        app.events.signal(); // a change with nobody watching wakes it to look, and it parks again
        std::thread::sleep(Duration::from_millis(50));
        assert!(!ran.load(Ordering::SeqCst), "nobody is looking: nothing runs");
        let page = Watching::new(&app.events);
        t.join().unwrap();
        assert!(ran.load(Ordering::SeqCst));
        drop(page);
        assert_eq!(app.events.watchers(), 0);
    }

    #[test]
    fn test_a_bounded_park_gives_up_at_its_deadline() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let started = std::time::Instant::now();
        assert!(!app.events.park_until_or(&app, Duration::from_millis(80), || false));
        assert!(started.elapsed() >= Duration::from_millis(80));
    }

    #[test]
    fn test_a_page_that_missed_a_message_is_sent_its_whole_state_again() {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let now = bagholder_core::jiff::Timestamp::now();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let mut feed = Feed::open(app.clone());
        let want = |params: Value| Want { params, have: None };
        assert!(app.events.watch(&app, feed.id(), [("trades".to_string(), want(serde_json::json!({}))), ("status".to_string(), want(serde_json::json!({})))].into_iter().collect()));
        let first = feed.step(&crate::status::status);
        assert!(first.iter().any(|(name, m)| *name == "snapshot" && m["doc"] == "trades"), "the first step sends the whole state: {first:?}");
        assert!(feed.step(&crate::status::status).is_empty(), "nothing moved, nothing sent");
        assert!(app.events.resync(feed.id()));
        let again = feed.step(&crate::status::status);
        assert!(again.iter().any(|(name, m)| *name == "snapshot" && m["doc"] == "trades" && m["data"]["trades"].is_array()), "{again:?}");
        // a stream that is not open is not one to resync
        assert!(!app.events.resync(u64::MAX));
    }

    #[test]
    fn test_filters_the_engine_does_not_know_are_said_to_the_page() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let mut feed = Feed::open(app.clone());
        let filters: Value = serde_json::from_str(r#"{"filters":{"lists":{"account":["TFSA"]}}}"#).unwrap();
        assert!(app.events.watch(&app, feed.id(), [("dashboard".to_string(), Want { params: filters, have: None })].into_iter().collect()));
        let said = feed.step(&crate::status::status);
        assert!(said.iter().any(|(name, m)| *name == "refused" && m["error"].as_str().is_some_and(|e| e.contains("TFSA"))), "{said:?}");
        assert!(feed.step(&crate::status::status).is_empty(), "said once");
    }

    /// A document is read again only when something it reads moved: a commit to
    /// the market cache leaves the notifications unread; one to the store reads them.
    #[test]
    fn test_a_document_is_read_again_only_when_what_it_reads_moved() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let mut feed = Feed::open(app.clone());
        assert!(app.events.watch(&app, feed.id(), [("notifications".to_string(), Want { params: serde_json::json!({}), have: None })].into_iter().collect()));
        let reads = || crate::docs::READS.lock().unwrap().get("notifications").copied().unwrap_or(0);
        feed.step(&crate::status::status);
        let first = reads();
        assert!(first > 0);
        app.events.signal_from(Source::Cache);
        app.events.signal();
        feed.step(&crate::status::status);
        assert_eq!(reads(), first, "a quote stored, the app's own state written: the notifications read nothing new");
        app.events.signal_from(Source::Store);
        feed.step(&crate::status::status);
        assert_eq!(reads(), first + 1, "a commit to the store they live in reads them again");
    }

    /// What is held is quoted only while a page shows a holding's price: a page on
    /// the Dashboard or the trades asks for no quote; one showing the holdings does.
    #[test]
    fn test_quotes_are_owed_only_to_a_page_showing_a_price() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let feed = Feed::open(app.clone());
        let show = |keys: &[&str]| {
            assert!(app.events.watch(&app, feed.id(), keys.iter().map(|k| (k.to_string(), Want { params: serde_json::json!({}), have: None })).collect()));
            app.events.showing(&crate::due::PRICED)
        };
        assert!(!show(&["status", "book", "dashboard", "notifications"]));
        assert!(!show(&["status", "book", "trades"]));
        for priced in ["positions", "exposure", "markets", "cashflow", "trade:x"] {
            assert!(show(&["status", "book", priced]), "{priced}");
        }
        assert!(!show(&[]));
    }
}
