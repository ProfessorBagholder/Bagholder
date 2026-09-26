//! The derived trading model: one pipeline, which the
//! shared cases in `tests/cases` hold every implementation to.
//!
//! ```text
//! activities -> normalize -> match_fifo -> apply_fx -> trades
//!                                                   -> positions
//!                                                   -> cashflow
//! ```

// the derives name this crate by its own name, inside it as outside
extern crate self as bagholder_model;

pub mod value;
pub mod lenient;
pub mod activity;
pub mod input;
pub mod wire;
pub mod symbols;
pub mod normalize;
pub mod dates;
pub mod fifo;
pub mod fold;
pub mod fx;
pub mod synth;
pub mod securities;
pub mod book;
pub mod clock;
pub mod context;
pub mod trades;
pub mod positions;
pub mod unichars;
pub mod symbols_of;
pub mod textrules;
pub mod cashflow;
pub mod nav;
pub mod filters;
pub mod stats;
pub mod exposure;
pub mod base;
pub mod venues;
pub mod instruments;
pub mod markets;
pub mod view;
/// The diff the model's wire types derive, re-exported where the earlier code finds it.
pub use bagholder_diff as patch;
pub mod cases;
pub mod testing;
