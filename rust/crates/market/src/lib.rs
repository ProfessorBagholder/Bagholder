//! The public market sources: what each one sends, how it is read, and the
//! top-ups that keep the stored series current.

pub mod http;
pub mod interlisted;
pub mod parse;
pub mod search;
pub mod sedar;
pub mod shorts;
pub mod stored;
pub mod disclosures;
pub mod edgar;
pub mod enrich;
pub mod exposure;
pub mod formnames;
pub mod forms;
pub mod localmodel;
pub mod pdftext;
pub mod fear;
pub mod news;
pub mod history;
pub mod quotes;
pub mod tmx;
pub mod universes;
pub mod xls;

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

/// The wall clock the readers age what they keep by: the system's, which goes on
/// through a machine's sleep, where a monotonic clock stops.
pub fn wall_now() -> std::time::SystemTime {
    #[cfg(feature = "test-clock")]
    return std::time::SystemTime::now() + std::time::Duration::from_millis(SLEPT_MS.load(std::sync::atomic::Ordering::SeqCst));
    #[cfg(not(feature = "test-clock"))]
    std::time::SystemTime::now()
}

#[cfg(feature = "test-clock")]
static SLEPT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A machine that slept for `d`, as the wall clock sees it: tests only.
#[cfg(feature = "test-clock")]
pub fn sleep_through(d: std::time::Duration) {
    SLEPT_MS.fetch_add(d.as_millis() as u64, std::sync::atomic::Ordering::SeqCst);
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
