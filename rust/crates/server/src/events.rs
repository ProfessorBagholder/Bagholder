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

use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use bagholder_model::patch;

use crate::app::app;

fn bell() -> &'static (Mutex<u64>, Condvar) {
    static B: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
    B.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// Something changed. Cheap, and safe to call from anywhere, SQLite's commit
/// hook included: it counts and wakes, no more.
pub fn signal() {
    let (m, c) = bell();
    *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    c.notify_all();
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
pub fn park_until(ready: impl Fn() -> bool) -> bool {
    let (m, c) = bell();
    loop {
        if app().stopping() {
            return false;
        }
        let seen = *m.lock().unwrap_or_else(|e| e.into_inner());
        if ready() {
            return true;
        }
        let g = m.lock().unwrap_or_else(|e| e.into_inner());
        drop(c.wait_while(g, |n| *n == seen && !app().stopping()));
    }
}

/// As `park_until`, but no longer than `most`: true when `ready`, false when the
/// time ran out or the app is stopping.
pub fn park_until_or(most: Duration, ready: impl Fn() -> bool) -> bool {
    let (m, c) = bell();
    let until = std::time::Instant::now() + most;
    loop {
        if app().stopping() {
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
        drop(c.wait_timeout_while(g, left, |n| *n == seen && !app().stopping()));
    }
}

/// How long signals are gathered before the page is told. A quote pass commits a
/// dozen rows one after another; they reach the page as one message.
const GATHER: Duration = Duration::from_millis(40);
const KEEPALIVE: Duration = Duration::from_secs(15);

/// Wait for a signal after `seen`; then let the signals that follow it close
/// behind settle. The count now, or `None` when the wait timed out.
fn wait_for_change(seen: u64) -> Option<u64> {
    let (m, c) = bell();
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    if *g == seen {
        g = c.wait_timeout_while(g, KEEPALIVE, |n| *n == seen).unwrap_or_else(|e| e.into_inner()).0;
        if *g == seen {
            return None;
        }
    }
    loop {
        let at = *g;
        g = c.wait_timeout_while(g, GATHER, |n| *n == at).unwrap_or_else(|e| e.into_inner()).0;
        if *g == at {
            return Some(at);
        }
    }
}

fn message(event: &str, data: &Value) -> String {
    format!("event: {}\ndata: {}\n\n", event, data)
}

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
pub fn watch(id: u64, docs: Wanted) -> bool {
    let Some(wanted) = streams().lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned() else { return false };
    let fresh: Vec<String> = {
        let mut w = wanted.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = docs.keys().filter(|k| !w.contains_key(*k)).cloned().collect();
        *w = docs;
        fresh
    };
    // a document just opened is worth reading fresh: once, now, in the background
    for key in &fresh {
        crate::docs::opened(key);
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

/// One page's stream. `write` sends text and says whether the page is still there.
pub fn stream<W: FnMut(&str) -> bool>(filters: Option<Value>, detail: Option<String>, status: &dyn Fn() -> Value, mut write: W) {
    let _watching = Watching::new();
    let (registered, wanted) = Registered::new();
    if !write(&message("hello", &serde_json::json!({"id": registered.0}))) {
        return;
    }
    let mut seen = *bell().0.lock().unwrap_or_else(|e| e.into_inner());
    let mut sent: Option<(Arc<Value>, Value)> = None;
    let mut sent_docs: std::collections::BTreeMap<String, Value> = Default::default();
    while !app().stopping() {
        let mut out = String::new();
        let view = match std::panic::catch_unwind(|| app().view(filters.as_ref(), detail.as_deref())) {
            Ok(Ok(v)) => Some(v),
            _ => None, // the store is busy or the model failed: say nothing, try at the next change
        };
        if let Some(view) = view {
            let mut now = status();
            // the two version strings are how a polling page learned that something
            // moved; this page is told what moved, so they would only be noise here
            if let Some(o) = now.as_object_mut() {
                o.remove("dataVersion");
                o.remove("coreVersion");
            }
            match &sent {
                None => {
                    let mut whole = (*view).clone();
                    whole["status"] = now.clone();
                    out.push_str(&message("snapshot", &serde_json::json!({"doc": "model", "data": whole})));
                }
                Some((was, was_status)) => {
                    // the same view object is the same data: only a different one is compared
                    let mut ops = if Arc::ptr_eq(was, &view) { vec![] } else { patch::diff(was, &view) };
                    ops.extend(patch::diff_under(&["status"], was_status, &now));
                    if !ops.is_empty() {
                        out.push_str(&message("patch", &serde_json::json!({"doc": "model", "ops": ops})));
                    }
                }
            }
            sent = Some((view, now));
        }
        // the documents this page is showing now
        let want: Wanted = wanted.lock().unwrap_or_else(|e| e.into_inner()).clone();
        sent_docs.retain(|k, _| want.contains_key(k));
        for (key, params) in &want {
            let Ok(Some(now)) = std::panic::catch_unwind(|| crate::docs::read(key, params)) else { continue };
            match sent_docs.get(key) {
                None => out.push_str(&message("snapshot", &serde_json::json!({"doc": key, "data": now}))),
                Some(was) => {
                    let ops = patch::diff(was, &now);
                    if !ops.is_empty() {
                        out.push_str(&message("patch", &serde_json::json!({"doc": key, "ops": ops})));
                    }
                }
            }
            sent_docs.insert(key.clone(), now);
        }
        if !out.is_empty() && !write(&out) {
            return;
        }
        // nothing is looked at again until something signals: a quiet stream only
        // carries its keepalive
        seen = loop {
            match wait_for_change(seen) {
                Some(n) => break n,
                None if app().stopping() || !write(": ping\n\n") => return,
                None => {}
            }
        };
    }
}

/// The day is an input of the model (an option expires, year-to-date rolls over).
/// It turns at a known moment, so that moment is waited for; nothing checks the
/// clock in between.
pub fn signal_at_each_midnight() {
    crate::app::spawn("bagholder-midnight", || loop {
        let secs = bagholder_model::clock::seconds_until_local_midnight().max(1) + 1;
        if app().wait(Duration::from_secs(secs)) {
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
        let ran = Arc::new(AtomicBool::new(false));
        let t = {
            let ran = ran.clone();
            std::thread::spawn(move || {
                if park_until(|| watchers() > 0) {
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
        let started = std::time::Instant::now();
        assert!(!park_until_or(Duration::from_millis(80), || false));
        assert!(started.elapsed() >= Duration::from_millis(80));
    }
}
