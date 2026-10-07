//! The app's log, kept on disk in the data folder on every device (`bagholder.log`),
//! so what the app did and when (a read, a failure, a start) can be read back after
//! the fact instead of guessed at; the terminal it was started from, if any, sees
//! every line too. It is rotated as logrotate's own default configuration rotates
//! logs: once a week, the four weeks before kept (`bagholder.log.1` to `.4`), so it
//! never grows without bound on any machine.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use bagholder_core::jiff::Timestamp;

/// The file the log is written to, in the data folder.
pub const LOG_FILE: &str = "bagholder.log";
/// The weeks kept beside this week's: logrotate's default `rotate 4`, `weekly`.
pub const KEPT_WEEKS: u32 = 4;

struct Log {
    dir: PathBuf,
    /// The ISO week the open file is for.
    week: (i16, i8),
    file: std::fs::File,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

fn week_of(t: Timestamp) -> (i16, i8) {
    let w = t.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).date().iso_week_date();
    (w.year(), w.week())
}

fn path(dir: &Path, n: u32) -> PathBuf {
    if n == 0 { dir.join(LOG_FILE) } else { dir.join(format!("{LOG_FILE}.{n}")) }
}

/// This week's file moved to `.1`, each older one up a place, the oldest kept dropped.
fn rotate(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path(dir, KEPT_WEEKS)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    for n in (0..KEPT_WEEKS).rev() {
        match std::fs::rename(path(dir, n), path(dir, n + 1)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

fn open(dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().create(true).append(true).open(path(dir, 0))
}

/// The log kept in `dir` from now on. A file left from an earlier week is rotated
/// first. Called once, when the app starts.
pub fn start(dir: &Path, now: Timestamp) -> std::io::Result<()> {
    let week = week_of(now);
    let written = std::fs::metadata(path(dir, 0)).and_then(|m| m.modified()).ok().and_then(|t| Timestamp::try_from(t).ok());
    if written.is_some_and(|t| week_of(t) != week) {
        rotate(dir)?;
    }
    let file = open(dir)?;
    let _ = LOG.set(Mutex::new(Log { dir: dir.to_path_buf(), week, file }));
    Ok(())
}

/// One line written to the log at `now`, the week's file rotated when the week has
/// turned. A log that cannot be written is said on stderr, which the line goes to
/// anyway: a failure to log never stops what was being logged.
pub fn write(line: &str, now: Timestamp) {
    let Some(log) = LOG.get() else { return };
    let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
    let week = week_of(now);
    if week != log.week {
        let dir = log.dir.clone();
        match rotate(&dir).and_then(|_| open(&dir)) {
            Ok(f) => {
                log.file = f;
                log.week = week;
            }
            Err(e) => eprintln!("bagholder: the log could not be rotated: {e}"),
        }
    }
    if let Err(e) = writeln!(log.file, "{now} {line}") {
        eprintln!("bagholder: the log could not be written: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line is kept with its moment; the week turning moves the file aside, and
    /// four weeks before this one are kept, no more, as logrotate keeps them.
    #[test]
    fn the_log_keeps_each_line_and_rotates_weekly_keeping_four_weeks() {
        let dir = tempfile::tempdir().unwrap();
        let day = |d: &str| -> Timestamp { format!("{d}T12:00:00Z").parse().unwrap() };
        start(dir.path(), day("2026-01-05")).unwrap();
        write("first week", day("2026-01-05"));
        write("same week", day("2026-01-09"));
        for (n, d) in ["2026-01-12", "2026-01-19", "2026-01-26", "2026-02-02", "2026-02-09", "2026-02-16"].iter().enumerate() {
            write(&format!("week {}", n + 2), day(d));
        }
        let read = |n: u32| std::fs::read_to_string(path(dir.path(), n)).unwrap_or_default();
        assert_eq!(read(0), "2026-02-16T12:00:00Z week 7\n");
        assert_eq!(read(1), "2026-02-09T12:00:00Z week 6\n");
        assert_eq!(read(4), "2026-01-19T12:00:00Z week 3\n");
        assert!(!path(dir.path(), 5).exists(), "four weeks kept before this one, no more");
        assert!(!(0..=4).any(|n| read(n).contains("first week")), "the oldest weeks dropped");
    }
}
