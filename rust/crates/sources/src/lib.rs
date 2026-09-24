//! Where Bagholder's facts and market data come from (`docs/architecture.md` §9,
//! `docs/plans/stage-3a-sources.md`).
//!
//! Every reply is read exactly and strictly ([`reply`]): a number is the decimal
//! its digits spell, a field an adapter needs is there with the type it needs or
//! the whole reply is a failure of the source, and a reply whose shape differs
//! from the recorded ones says so. Every request's [`outcome`] is recorded in the
//! market [`cache`], and a source's [`health`] is read from those records.

pub mod adapters;
pub mod ask;
pub mod cache;
pub mod contract;
pub mod health;
pub mod outcome;
pub mod rates;
pub mod read;
pub mod reply;
pub mod venue;
