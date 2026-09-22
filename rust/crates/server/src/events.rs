//! The page's one connection to the server: its data once, then only what
//! changes in it (docs/architecture.md, rules 0 and 2).
//!
//! Nothing polls. A change reaches the page because it happened:
//!
//! - every database commit, on any connection, signals here (`store::on_commit`);
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use bagholder_model::patch;

use crate::app::App;

fn bell() -> &'static (Mutex<u64>, Condvar) {
    static B: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
    B.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// The same bell for whoever waits without a thread of its own (a page's stream
/// is a task on the runtime): a channel that holds the count.
fn ticker() -> &'static tokio::sync::watch::Sender<u64> {
    static T: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();
    T.get_or_init(|| tokio::sync::watch::channel(0).0)
}

/// Something changed. Cheap, and safe to call from anywhere, SQLite's commit
/// hook included: it counts and wakes, no more.
pub fn signal() {
    let (m, c) = bell();
    *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    c.notify_all();
    ticker().send_modify(|n| *n = n.wrapping_add(1));
}

/// A receiver that is told of every signal from now on.
pub fn subscribe() -> tokio::sync::watch::Receiver<u64> {
    ticker().subscribe()
}

/// Wait for the next signal; then let the signals that follow it close behind
/// settle (`GATHER`), so a burst reaches the page as one message.
pub async fn changed(rx: &mut tokio::sync::watch::Receiver<u64>) {
    if rx.changed().await.is_err() {
        return std::future::pending().await; // the sender is a static: this cannot happen
    }
    while let Ok(Ok(())) = tokio::time::timeout(GATHER, rx.changed()).await {}
}

/// Pages connected now. What a periodic read of an outside source consults
/// before it runs: nobody watching, nothing to fetch.
pub fn watchers() -> usize {
    WATCHERS.load(Ordering::SeqCst)
}

static WATCHERS: AtomicUsize = AtomicUsize::new(0);

struct Watching;
impl Watching {
    fn new() -> Watching {
        WATCHERS.fetch_add(1, Ordering::SeqCst);
        signal(); // work that waits for someone to look can start
        Watching
    }
}
impl Drop for Watching {
    fn drop(&mut self) {
        WATCHERS.fetch_sub(1, Ordering::SeqCst);
        signal();
    }
}

/// Sleep until `ready` says so, looking again only when something signals: no
/// clock is consulted while it waits. False when the app is stopping instead. What
/// background work that has nothing to do parks on -- a loop with no one watching
/// its data, an engine with nothing armed.
pub fn park_until(app: &App, ready: impl Fn() -> bool) -> bool {
    let (m, c) = bell();
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
pub fn park_until_or(app: &App, most: Duration, ready: impl Fn() -> bool) -> bool {
    let (m, c) = bell();
    let until = std::time::Instant::now() + most;
    loop {
        if app.stopping() {
            return false;
        }
        let seen = *m.lock().unwrap_or_else(|e| e.into_inner());
        if ready() {
            return true;
        }
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        let g = m.lock().unwrap_or_else(|e| e.into_inner());
        drop(c.wait_timeout_while(g, left, |n| *n == seen && !app.stopping()));
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

type Wanted = std::collections::BTreeMap<String, Value>;

fn streams() -> &'static Mutex<std::collections::HashMap<u64, Arc<Mutex<Wanted>>>> {
    static S: OnceLock<Mutex<std::collections::HashMap<u64, Arc<Mutex<Wanted>>>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// What the page on stream `id` is showing now, beyond the model. Replaces what it
/// said before. False when there is no such stream (it closed; the page will
/// connect again and say so again).
pub fn watch(app: &Arc<App>, id: u64, docs: Wanted) -> bool {
    let Some(wanted) = streams().lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned() else { return false };
    let fresh: Vec<String> = {
        let mut w = wanted.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = docs.keys().filter(|k| !w.contains_key(*k)).cloned().collect();
        *w = docs;
        fresh
    };
    // a document just opened is worth reading fresh: once, now, in the background
    for key in &fresh {
        crate::docs::opened(app, key);
    }
    signal();
    true
}

/// Whether any page is showing the document `key` now.
pub fn watched(key: &str) -> bool {
    streams().lock().unwrap_or_else(|e| e.into_inner()).values().any(|w| w.lock().unwrap_or_else(|e| e.into_inner()).contains_key(key))
}

struct Registered(u64);
impl Registered {
    fn new() -> (Registered, Arc<Mutex<Wanted>>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let wanted = Arc::new(Mutex::new(Wanted::new()));
        streams().lock().unwrap_or_else(|e| e.into_inner()).insert(id, wanted.clone());
        (Registered(id), wanted)
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        streams().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// One page's stream: what it was last sent, and so what to send it next.
pub struct Feed {
    app: Arc<App>,
    _watching: Watching,
    registered: Registered,
    wanted: Arc<Mutex<Wanted>>,
    filters: Option<Value>,
    detail: Option<String>,
    sent: Option<(Arc<bagholder_model::wire::View>, Value)>,
    sent_docs: std::collections::BTreeMap<String, Value>,
}

/// One message on the stream: its event name and its data.
pub type Message = (&'static str, Value);

impl Feed {
    pub fn open(app: Arc<App>, filters: Option<Value>, detail: Option<String>) -> Feed {
        let (registered, wanted) = Registered::new();
        Feed { app, _watching: Watching::new(), registered, wanted, filters, detail, sent: None, sent_docs: Default::default() }
    }

    /// This stream's id.
    pub fn id(&self) -> u64 {
        self.registered.0
    }

    /// The first message: the id the page names in `POST /api/events/watch`.
    pub fn hello(&self) -> Message {
        ("hello", serde_json::json!({"id": self.id()}))
    }

    /// What differs now from what this page was last sent: nothing when nothing
    /// does. Reads the store and may rebuild a layer of the model, so it runs off
    /// the runtime's own threads.
    pub fn step(&mut self, status: &dyn Fn(&Arc<App>) -> Value) -> Vec<Message> {
        let mut out: Vec<Message> = Vec::new();
        let (filters, detail) = (self.filters.clone(), self.detail.clone());
        let app = self.app.clone();
        let view = match std::panic::catch_unwind(move || app.view(filters.as_ref(), detail.as_deref())) {
            Ok(Ok(v)) => Some(v),
            _ => None, // the store is busy or the model failed: say nothing, try at the next change
        };
        if let Some(view) = view {
            let mut now = status(&self.app);
            // the two version strings are how a polling page learned that something
            // moved; this page is told what moved, so they would only be noise here
            if let Some(o) = now.as_object_mut() {
                o.remove("dataVersion");
                o.remove("coreVersion");
            }
            match &self.sent {
                None => {
                    let mut whole = view.to_value();
                    whole["status"] = now.clone();
                    out.push(("snapshot", serde_json::json!({"doc": "model", "data": whole})));
                }
                Some((was, was_status)) => {
                    // compared as the model's own values, row by row by each row's id; the
                    // same view object, or a part both views share, is not compared at all
                    let mut ops = patch::typed(was, &view);
                    ops.extend(patch::diff_under(&["status"], was_status, &now));
                    if !ops.is_empty() {
                        out.push(("patch", serde_json::json!({"doc": "model", "ops": ops})));
                    }
                }
            }
            self.sent = Some((view, now));
        }
        // the documents this page is showing now
        let want: Wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.sent_docs.retain(|k, _| want.contains_key(k));
        for (key, params) in &want {
            let app = &self.app;
            let Ok(Some(now)) = std::panic::catch_unwind(|| crate::docs::read(app, key, params)) else { continue };
            match self.sent_docs.get(key) {
                None => out.push(("snapshot", serde_json::json!({"doc": key, "data": now}))),
                Some(was) => {
                    let ops = patch::diff(was, &now);
                    if !ops.is_empty() {
                        out.push(("patch", serde_json::json!({"doc": key, "ops": ops})));
                    }
                }
            }
            self.sent_docs.insert(key.clone(), now);
        }
        out
    }
}

/// The day is an input of the model (an option expires, year-to-date rolls over).
/// It turns at a known moment, so that moment is waited for; nothing checks the
/// clock in between.
pub fn signal_at_each_midnight(app: Arc<App>) {
    crate::app::spawn("bagholder-midnight", move || loop {
        let secs = bagholder_model::clock::seconds_until_local_midnight().max(1) + 1;
        if app.wait(Duration::from_secs(secs)) {
            return;
        }
        signal();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Work that waits for someone to look does nothing until a page connects, and
    /// starts the moment one does.
    #[test]
    fn test_parked_work_starts_when_a_page_connects_and_not_before() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let ran = Arc::new(AtomicBool::new(false));
        let t = {
            let ran = ran.clone();
            std::thread::spawn(move || {
                if park_until(&app, || watchers() > 0) {
                    ran.store(true, Ordering::SeqCst);
                }
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        signal(); // a change with nobody watching wakes it to look, and it parks again
        std::thread::sleep(Duration::from_millis(50));
        assert!(!ran.load(Ordering::SeqCst), "nobody is looking: nothing runs");
        let page = Watching::new();
        t.join().unwrap();
        assert!(ran.load(Ordering::SeqCst));
        drop(page);
        assert_eq!(watchers(), 0);
    }

    #[test]
    fn test_a_bounded_park_gives_up_at_its_deadline() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let started = std::time::Instant::now();
        assert!(!park_until_or(&app, Duration::from_millis(80), || false));
        assert!(started.elapsed() >= Duration::from_millis(80));
    }
}
