//! The derived trading model: the same pipeline as `model.py`, which the
//! shared cases in `tests/cases` hold every implementation to.
//!
//!     activities -> normalize -> match_fifo -> apply_fx -> trades
//!                                                       -> positions
//!                                                       -> cashflow

pub mod value;
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
pub mod trades;
pub mod positions;
pub mod pytext;
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
