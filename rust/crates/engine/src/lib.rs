//! Bagholder's engine (`docs/architecture.md` §8): every figure `SPEC.md`
//! defines, computed from the book, the facts the figures use, the market and the
//! clock, all handed in. A pure function of its inputs: the same inputs always
//! give the same figures, and nothing is read from a database, the network or a
//! clock (`crates/core/tests/boundaries.rs`).

pub mod fx;
pub mod gap;
pub mod input;
pub mod ledger;
pub mod families;
pub mod dust;
pub mod identity;
pub mod positions;
pub mod stat;
pub mod trades;
pub mod cashflow;
pub mod equity;
pub mod scope;
pub mod engine;
pub mod needs;

pub use engine::{Change, Engine, Entity, Moved};

/// Whether a probe's measured window is open on this thread, where the test
/// oracles stay out of what is measured (`bagholder_sqlite::work::measuring`
/// for the book; the server sets this one beside it).
#[cfg(feature = "oracles")]
pub fn oracles_off() -> bool {
    OFF.with(|o| o.get())
}

/// Turn the oracles off on this thread while a probe measures.
pub fn set_oracles_off(off: bool) {
    #[cfg(feature = "oracles")]
    OFF.with(|o| o.set(off));
    #[cfg(not(feature = "oracles"))]
    let _ = off;
}

#[cfg(feature = "oracles")]
thread_local! {
    static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
