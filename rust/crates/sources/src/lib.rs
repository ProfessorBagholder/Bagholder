//! Where Bagholder's facts and market data come from (`docs/architecture.md` §9).
//!
//! Every reply is read exactly and strictly ([`reply`]): a number is the decimal
//! its digits spell, a field an adapter needs is there with the type it needs or
//! the whole reply is a failure of the source, and a reply whose shape differs
//! from the recorded ones says so.

pub mod reply;
