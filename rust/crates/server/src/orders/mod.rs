//! Manual activity, the order ticket, reading orders back from Wealthsimple,
//! and the bracket engine.
//!
//! Without live orders (`BAGHOLDER_DRY_ORDERS=1`) nothing that places, cancels
//! or modifies an order is ever sent: the ticket is recorded as `dry`, and the
//! GraphQL wrapper here refuses the three order mutations outright as a second
//! line of defence.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Map, Value};

use bagholder_store::orders as so;
use bagholder_store::orders::{Bracket, BracketPatch, BracketStatus, Order, OrderPatch, OrderStatus, OrderType, Role, Side, SlKind, Source, SlMode, StopLoss, TakeProfit, TrailUnit};
use serde::Deserialize;
use bagholder_ws::session::{identity_from, CallError};
// the one real caller is compiled out of test builds, where no call reaches the network
#[cfg(not(test))]
use bagholder_ws::session::Client;

use crate::app::{f, log, now_iso, now_unix, num, qty_text, s, truthy, uuid4, App};

/// Test seams: a fake Wealthsimple, the live switch, the session, and threads.
/// Under `cfg(test)` nothing reaches the network: without a fake every call fails.
#[cfg(test)]
pub mod seam {
    use super::*;
    use std::sync::Arc;
    pub type Gql = Arc<dyn Fn(&str, &Value) -> Result<Value, CallError> + Send + Sync>;
    pub static GQL: Mutex<Option<Gql>> = Mutex::new(None);
    pub static LIVE: Mutex<Option<bool>> = Mutex::new(None);
    pub static SESSION: Mutex<Option<Option<Value>>> = Mutex::new(None);
    /// 0: a spawned thread never runs; 1: it runs inline.
    pub static SPAWN_INLINE: AtomicBool = AtomicBool::new(false);
    pub fn reset() {
        *GQL.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = None;
        SPAWN_INLINE.store(false, Ordering::SeqCst);
    }
}

fn spawn<F: FnOnce() + Send + 'static>(name: &str, f: F) {
    #[cfg(test)]
    {
        let _ = name;
        if seam::SPAWN_INLINE.load(Ordering::SeqCst) {
            f();
        }
        return;
    }
    #[cfg(not(test))]
    crate::app::spawn(name, f)
}
use crate::notify;
use crate::session::{ensure_fresh_token, load_session};

mod brackets;
mod edit;
mod manual;
mod readback;
mod ticket;
mod tools;

pub use brackets::*;
pub use edit::*;
pub use manual::*;
pub use readback::*;
pub use ticket::*;
pub use tools::*;

/// The order loops' own state: brackets in flight, the last readback, and what
/// has already been said or looked up once so it is not said or looked up again.
#[derive(Default)]
pub struct OrdersState {
    /// When orders were last read back from Wealthsimple.
    pub(crate) refreshed_at: Mutex<String>,
    pub(crate) refreshing: AtomicBool,
    /// One bracket tick at a time.
    pub(crate) bracket_lock: AtomicBool,
    /// What a bracket has already logged once, so it is not logged again.
    pub(crate) bracket_said: Mutex<HashSet<String>>,
    pub(crate) stop_allowed_cache: Mutex<HashMap<String, bool>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_g(5.0), "5");
        assert_eq!(fmt_g(1.5), "1.5");
        assert_eq!(fmt_g(1234567.0), "1.23457e+06");
        assert_eq!(fmt_g(0.00001), "1e-05");
        assert_eq!(price_words(Some(0.625)), "0.625");
        assert_eq!(price_words(Some(0.54)), "0.54");
        assert_eq!(order_tick(Some(1.005)), Some(1.0));
        assert_eq!(parse_z("2026-09-10T12:00:00Z"), Some(bagholder_model::dates::to_days(2026, 9, 10) * 86400 + 43200));
        assert_eq!(app_status("POSTED").as_str(), "filled");
        assert_eq!(app_status("WHATEVER").as_str(), "pending");
    }
}
