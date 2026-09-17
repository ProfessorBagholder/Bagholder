//! The public market sources: what each one sends, how it is read, and the
//! top-ups that keep the stored series current.

pub mod client;
pub mod http;
pub mod parse;
pub mod refresh;
pub mod search;
pub mod sedar;
pub mod shorts;
pub mod browser;
pub mod clockzone;
pub mod disclosures;
pub mod edgar;
pub mod enrich;
pub mod exposure;
pub mod htmltables;
pub mod formnames;
pub mod forms;
pub mod localmodel;
pub mod pdftext;
pub mod fear;
pub mod entities;
pub mod news;
pub mod history;
pub mod quotes;
pub mod tmx;
pub mod universes;
pub mod xls;

/// When each quote was stored, re-exported so the quote loop can ask when each
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

/// (today in UTC, now as unix seconds, now stamped): the three forms of the
/// present the market readers are handed.
pub fn clock_now() -> (String, f64, String) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let secs = now as i64;
    let (y, m, d) = bagholder_model::dates::from_days(secs.div_euclid(86400));
    (bagholder_model::dates::fmt(y, m, d), now, now_stamp())
}
