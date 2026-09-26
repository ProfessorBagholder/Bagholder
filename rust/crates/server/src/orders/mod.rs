//! Orders and brackets (`docs/architecture.md` §11, `docs/plans/stage-4-execution.md`):
//! the ticket, reading orders back from Wealthsimple, the bracket engine, changing an
//! order or a bracket, and what the Orders panel is sent.
//!
//! Orders and brackets live in the book, each with its log. Every request that places,
//! cancels or changes an order leaves through `gate`, which records it before it is
//! sent and refuses everything when orders are off (`BAGHOLDER_DRY_ORDERS=1`); nothing
//! else here can reach Wealthsimple's order mutations.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
#[cfg(test)]
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::app::App;

pub(crate) mod brackets;
mod doc;
mod edit;
pub mod gate;
pub mod preview;
mod readback;
mod ticket;
mod tools;

pub use brackets::*;
pub use doc::*;
pub use edit::*;
pub use readback::*;
pub use ticket::*;
pub use tools::*;

/// What Cancel, Edit and a bracket's Save, Remove and Cancel answer with orders off
/// (`SPEC.md` §4, Orders, Refresh): nothing of the kind is sent or changed.
pub const ORDERS_OFF: &str = "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple.";

/// One order or bracket action -- cancel, modify, adjust -- refused with a reason, or
/// accepted with what changed.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct OrderActionAnswer {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unchanged: Option<bool>,
}

impl OrderActionAnswer {
    pub fn err(e: impl Into<String>) -> OrderActionAnswer {
        OrderActionAnswer { ok: false, error: Some(e.into()), id: None, status: None, unchanged: None }
    }
    pub fn accepted(id: impl Into<String>) -> OrderActionAnswer {
        OrderActionAnswer { ok: true, error: None, id: Some(id.into()), status: None, unchanged: None }
    }
    pub fn with_status(id: impl Into<String>, status: &str) -> OrderActionAnswer {
        OrderActionAnswer { ok: true, error: None, id: Some(id.into()), status: Some(status.into()), unchanged: None }
    }
    pub fn already(id: impl Into<String>) -> OrderActionAnswer {
        OrderActionAnswer { ok: true, error: None, id: Some(id.into()), status: None, unchanged: Some(true) }
    }
}

/// What the order code keeps while the app runs: whether orders go out at all, the
/// gate's broker and locks, the orders Wealthsimple reports that were placed
/// elsewhere, and what was looked up once so it is not looked up again.
pub struct OrdersState {
    /// Orders reach Wealthsimple: off under `BAGHOLDER_DRY_ORDERS=1`, read once at start.
    pub(crate) live: AtomicBool,
    /// When orders were last read back from Wealthsimple.
    pub(crate) refreshed_at: Mutex<Option<jiff::Timestamp>>,
    /// A read-back is asked for now (an order was just sent, cancelled or changed).
    pub(crate) read_asked: AtomicBool,
    /// Whether Wealthsimple takes stop orders for a security, by its id.
    pub(crate) stop_allowed: Mutex<HashMap<String, bool>>,
    /// Shares per unit of a security the book does not hold, as Wealthsimple's quote
    /// stated it, by its id.
    pub(crate) units: Mutex<HashMap<String, bagholder_core::Dec>>,
    /// Listings the book does not hold, as Wealthsimple's search found them, by
    /// symbol: a ticket on one asks the search once while the app runs.
    pub(crate) found: Mutex<HashMap<String, bagholder_model::securities::Security>>,
    /// Orders Wealthsimple reports that were not placed here (its own app's): they are
    /// Wealthsimple's, not the app's, so they are held here and never in the book.
    pub(crate) elsewhere: Mutex<Vec<Elsewhere>>,
    /// Why the brackets' quote could not be acted on, until the next good read.
    pub(crate) quote_problem: Mutex<Option<String>>,
    /// The gate's broker and its per-bracket locks.
    pub(crate) gate: gate::GateState,
    /// Under test: the session and Wealthsimple's read answers, per app.
    #[cfg(test)]
    pub(crate) seam: seam::Seam,
}

impl OrdersState {
    pub fn from_env() -> OrdersState {
        let live = std::env::var("BAGHOLDER_DRY_ORDERS").map(|v| v.trim() != "1").unwrap_or(true);
        OrdersState {
            live: AtomicBool::new(live),
            refreshed_at: Mutex::new(None),
            read_asked: AtomicBool::new(false),
            stop_allowed: Mutex::new(HashMap::new()),
            units: Mutex::new(HashMap::new()),
            found: Mutex::new(HashMap::new()),
            elsewhere: Mutex::new(Vec::new()),
            quote_problem: Mutex::new(None),
            gate: gate::GateState::default(),
            #[cfg(test)]
            seam: seam::Seam::default(),
        }
    }
}

/// Test seams, one set per app: the session the order code sees, and Wealthsimple's
/// answers to the reads (quotes, the feed, market data). Order mutations never pass
/// here: the gate's broker is the test's fake.
#[cfg(test)]
pub mod seam {
    use super::*;
    pub type Gql = Arc<dyn Fn(&str, &serde_json::Value) -> Result<serde_json::Value, bagholder_ws::session::CallError> + Send + Sync>;
    #[derive(Default)]
    pub struct Seam {
        pub session: Mutex<Option<bagholder_ws::session::Session>>,
        pub gql: Mutex<Option<Gql>>,
        pub stop_allowed: Mutex<Option<bool>>,
    }
}

/// Orders reach Wealthsimple.
pub fn orders_live(app: &App) -> bool {
    app.orders.live.load(Ordering::SeqCst)
}

#[cfg(test)]
impl App {
    /// Turn orders on or off for this app (a test's).
    pub fn set_orders_live(&self, on: bool) {
        self.orders.live.store(on, Ordering::SeqCst);
    }
}

/// Ask the orders loop to read back now: something was just sent.
pub(crate) fn ask_read(app: &App) {
    app.orders.read_asked.store(true, Ordering::SeqCst);
    app.events.signal();
}
