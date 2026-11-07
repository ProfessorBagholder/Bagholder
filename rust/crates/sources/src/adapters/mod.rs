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

/// Every source's pace, set on the process's limiter when the app starts, before
/// anything asks them: a pace set only by the reader that ran first left every
/// other request to that host unpaced, and Yahoo's quotes went out with no gap
/// at all until a chart was opened.
pub const PACES: [(&str, bagholder_net::Pace); 2] = [(yahoo::HOST, yahoo::PACE), (tmx::HOST, tmx::PACE)];

/// Set `PACES` on `limiter`.
pub fn pace_every_source(limiter: &bagholder_net::Limiter) {
    for (host, pace) in PACES {
        limiter.configure(host, pace);
    }
}
