//! One place a request waits its turn at a host.
//!
//! Each host that asks to be read slowly (the SEC's published rate, a fund
//! company's site, a news feed) has a next free moment. A caller takes that
//! moment, moves it on by the host's gap, and waits for its own turn with no
//! lock held: callers reading side by side wait side by side, each for its own
//! turn, and a host is never asked faster than its gap however many ask.
//!
//! This replaced a pacing function per module, two of which slept holding the
//! lock the others were waiting for.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

fn next_free() -> &'static Mutex<HashMap<String, Instant>> {
    static NEXT: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    NEXT.get_or_init(Default::default)
}

/// The moment this caller may ask `host`, with the host's next free moment moved
/// `gap` past it.
pub fn reserve(host: &str, gap: Duration, now: Instant) -> Instant {
    let mut next = next_free().lock().unwrap_or_else(|e| e.into_inner());
    let turn = next.get(host).map_or(now, |free| (*free).max(now));
    next.insert(host.to_string(), turn + gap);
    turn
}

/// Wait for this caller's turn at `host`.
pub fn turn(host: &str, gap: Duration) {
    let now = Instant::now();
    let at = reserve(host, gap, now);
    if at > now {
        std::thread::sleep(at - now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_callers_side_by_side_are_given_turns_a_gap_apart() {
        let gap = Duration::from_millis(300);
        let now = Instant::now();
        let turns: Vec<Instant> = (0..3).map(|_| reserve("turns.example", gap, now)).collect();
        assert_eq!(turns, [now, now + gap, now + gap * 2]);
        assert_eq!(reserve("another.example", gap, now), now, "a host's turns are its own");
        let later = now + Duration::from_secs(5);
        assert_eq!(reserve("turns.example", gap, later), later, "an idle host is asked at once");
    }
}
