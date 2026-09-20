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
        Watching
    }
}
impl Drop for Watching {
    fn drop(&mut self) {
        WATCHERS.fetch_sub(1, Ordering::SeqCst);
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

/// One page's stream. `write` sends text and says whether the page is still there.
pub fn stream<W: FnMut(&str) -> bool>(filters: Option<Value>, detail: Option<String>, status: &dyn Fn() -> Value, mut write: W) {
    let _watching = Watching::new();
    if !write(": bagholder\n\n") {
        return;
    }
    let mut seen = *bell().0.lock().unwrap_or_else(|e| e.into_inner());
    let mut sent: Option<(Arc<Value>, Value)> = None;
    while !app().stopping() {
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
            let text = match &sent {
                None => {
                    let mut whole = (*view).clone();
                    whole["status"] = now.clone();
                    Some(message("snapshot", &whole))
                }
                Some((was, was_status)) => {
                    // the same view object is the same data: only a different one is compared
                    let mut ops = if Arc::ptr_eq(was, &view) { vec![] } else { patch::diff(was, &view) };
                    ops.extend(patch::diff_under(&["status"], was_status, &now));
                    if ops.is_empty() { None } else { Some(message("patch", &Value::Array(ops))) }
                }
            };
            if let Some(t) = text {
                if !write(&t) {
                    return;
                }
            }
            sent = Some((view, now));
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
