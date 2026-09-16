//! The public market sources: what each one sends, how it is read, and the
//! top-ups that keep the stored series current.

pub mod client;
pub mod http;
pub mod parse;
pub mod refresh;
pub mod sedar;
pub mod shorts;
pub mod browser;
pub mod clockzone;
pub mod disclosures;
pub mod edgar;
pub mod fear;
pub mod entities;
pub mod news;
pub mod history;
pub mod quotes;
pub mod tmx;
pub mod universes;
pub mod xls;

/// `store.quote_fetched_at`, re-exported so the quote loop can ask when each
/// symbol was last priced.
pub fn market_fetched(conn: &rusqlite::Connection) -> rusqlite::Result<serde_json::Map<String, serde_json::Value>> {
    bagholder_store::market::quote_fetched_at(conn)
}

/// `store`'s market writers, under the name this crate uses for them.
pub mod market {
    pub use bagholder_store::market::{distributions_fetched_at, quotes, upsert_quote};
}

/// The stamp the sources' health and the attempt marker are written with.
pub fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}
