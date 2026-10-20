//! What the market readers keep, and the SQL they keep it with: their tables in
//! the market cache (news, filings, exposures, gauges, short interest, universes,
//! the charts' series, the `meta` keys and the counters that move with them,
//! made by `bagholder_sources::cache::MIGRATIONS`), and the notifications and
//! what was told, in the book (`bagholder_book::notices`).

pub mod bars;
pub mod charts;
pub mod feeds;
pub mod gens;
pub mod meta;
pub mod rows;
