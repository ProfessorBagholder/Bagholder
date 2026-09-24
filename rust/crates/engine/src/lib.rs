//! Bagholder's engine (`docs/architecture.md` §8): every figure `SPEC.md`
//! defines, computed from the book, the facts the figures use, the market and the
//! clock, all handed in. A pure function of its inputs: the same inputs always
//! give the same figures, and nothing is read from a database, the network or a
//! clock (`crates/core/tests/boundaries.rs`).

pub mod fx;
pub mod gap;
pub mod input;
pub mod ledger;
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
