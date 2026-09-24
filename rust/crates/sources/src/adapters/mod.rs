//! One module per source (`docs/plans/stage-3a-sources.md`, "The contract"). Each
//! reads its replies strictly into an [`Outcome`](crate::outcome::Outcome) and
//! notes a reply whose shape differs from its recorded ones (`shapes/`, the union
//! of `tests/replies/<source>/`).

pub mod boc;
pub mod cboe_ca;
pub mod cboe_options;
pub mod coinbase;
pub mod holidays;
pub mod statcan;
pub mod tmx;
pub mod yahoo;
