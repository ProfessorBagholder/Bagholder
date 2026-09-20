//! Order tickets and the brackets that watch them.
//!
//! Every ticket is written before anything is sent, so a crash between the
//! write and the broker's answer leaves a record rather than a silence. A
//! ticket is never deleted; it gains a status.
//!
//! `types` says what an order and a bracket are; `typed` reads and writes them;
//! `rows` is the same store spoken to in JSON, for the callers not yet moved
//! (docs/architecture.md, stage 5) -- it is made of `typed`, not beside it.

mod rows;
pub mod typed;
pub mod types;

pub use rows::*;
pub use types::*;
