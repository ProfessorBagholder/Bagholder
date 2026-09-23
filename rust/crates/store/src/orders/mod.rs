//! Order tickets and the brackets that watch them.
//!
//! Every ticket is written before anything is sent, so a crash between the
//! write and the broker's answer leaves a record rather than a silence. A
//! ticket is never deleted; it gains a status.
//!
//! `types` says what an order and a bracket are; `typed` reads and writes them.

pub mod typed;
pub mod types;

pub use types::*;
pub use typed::{mark_order_fill_booked, symbol_for_security};
