//! One place every request waits its turn at a host.
//!
//! Each host has a [`Pace`]: the least gap between two requests to it, and how
//! long to leave it alone after it refuses one. A caller takes the host's next
//! free moment, moves it on by the gap, and waits for its own turn with no lock
//! held: callers side by side wait side by side, each for its own turn, and a
//! host is never asked faster than its gap however many ask. After a refusal the
//! host rests for as long as its `Retry-After` says, or, when it says nothing, for
//! its pace's rest, doubled for each refusal since it last answered, up to
//! [`MOST_REST`]; a request in that time is not sent at all.
//!
//! This replaced a pacing function per module and Yahoo's and SEDAR+'s own gates.

use crate::clock::Clock;
use jiff::{SignedDuration, Timestamp};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    /// The least time between two requests to the host.
    pub gap: Duration,
    /// How long the host is left alone after a refusal that names no time.
    pub rest: Duration,
}

impl Pace {
    /// A host with no pace of its own: asked as fast as its callers ask, and
    /// left a minute after a refusal.
    pub const DEFAULT: Pace = Pace { gap: Duration::ZERO, rest: Duration::from_secs(60) };
}

/// The longest a host is left after refusals that named no time: a host still
/// refusing is asked again half an hour on, never left for good.
pub const MOST_REST: Duration = Duration::from_secs(30 * 60);

/// A host resting after a refusal, until the time it may be asked again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resting {
    pub until: Timestamp,
}

#[derive(Default)]
struct Host {
    pace: Option<Pace>,
    next_free: Option<Timestamp>,
    resting_until: Option<Timestamp>,
    /// Refusals since it last answered.
    refusals: u32,
}

#[derive(Default)]
pub struct Limiter {
    hosts: Mutex<HashMap<String, Host>>,
}

fn plus(t: Timestamp, d: Duration) -> Timestamp {
    t.checked_add(SignedDuration::try_from(d).unwrap_or(SignedDuration::MAX)).unwrap_or(Timestamp::MAX)
}

impl Limiter {
    pub fn new() -> Limiter {
        Limiter::default()
    }

    /// Set `host`'s pace.
    pub fn configure(&self, host: &str, pace: Pace) {
        self.hosts.lock().unwrap_or_else(|e| e.into_inner()).entry(host.to_string()).or_default().pace = Some(pace);
    }

    pub fn pace(&self, host: &str) -> Pace {
        self.hosts.lock().unwrap_or_else(|e| e.into_inner()).get(host).and_then(|h| h.pace).unwrap_or(Pace::DEFAULT)
    }

    /// The moment this caller may ask `host`, with the host's next free moment
    /// moved a gap past it; or the host's rest, when it is resting.
    pub fn reserve(&self, host: &str, now: Timestamp) -> Result<Timestamp, Resting> {
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        let h = hosts.entry(host.to_string()).or_default();
        if let Some(until) = h.resting_until {
            if now < until {
                return Err(Resting { until });
            }
            h.resting_until = None;
        }
        let gap = h.pace.unwrap_or(Pace::DEFAULT).gap;
        let turn = h.next_free.map_or(now, |free| free.max(now));
        h.next_free = Some(plus(turn, gap));
        Ok(turn)
    }

    /// `host` refused a request at `now`: it rests for `retry_after` when it said
    /// how long, else for its pace's rest, doubled for each refusal since it last
    /// answered, up to `MOST_REST`. A later, longer rest is never cut short by an
    /// earlier refusal's.
    pub fn refused(&self, host: &str, now: Timestamp, retry_after: Option<Duration>) {
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        let h = hosts.entry(host.to_string()).or_default();
        let grown = h.pace.unwrap_or(Pace::DEFAULT).rest.saturating_mul(1 << h.refusals.min(16)).min(MOST_REST);
        h.refusals = h.refusals.saturating_add(1);
        let until = plus(now, retry_after.unwrap_or(grown));
        h.resting_until = Some(h.resting_until.map_or(until, |u| u.max(until)));
    }

    /// `host` answered a request: its next refusal rests it for its pace's rest
    /// again.
    pub fn answered(&self, host: &str) {
        if let Some(h) = self.hosts.lock().unwrap_or_else(|e| e.into_inner()).get_mut(host) {
            h.refusals = 0;
        }
    }

    /// Wait for this caller's turn at `host` on `clock`, or say the host is resting.
    pub fn turn(&self, host: &str, clock: &dyn Clock) -> Result<(), Resting> {
        let now = clock.now();
        let at = self.reserve(host, now)?;
        if at > now {
            let wait = now.duration_until(at);
            clock.sleep(Duration::try_from(wait).unwrap_or(Duration::ZERO));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;

    fn t0() -> Timestamp {
        "2026-09-24T14:00:00Z".parse().unwrap()
    }

    #[test]
    fn callers_side_by_side_are_given_turns_a_gap_apart() {
        let l = Limiter::new();
        let gap = Duration::from_millis(300);
        l.configure("turns.example", Pace { gap, rest: Duration::from_secs(60) });
        let now = t0();
        let turns: Vec<Timestamp> = (0..3).map(|_| l.reserve("turns.example", now).unwrap()).collect();
        assert_eq!(turns, [now, plus(now, gap), plus(now, gap * 2)]);
        assert_eq!(l.reserve("another.example", now).unwrap(), now, "a host's turns are its own");
        let later = plus(now, Duration::from_secs(5));
        assert_eq!(l.reserve("turns.example", later).unwrap(), later, "an idle host is asked at once");
    }

    #[test]
    fn a_turn_waits_on_the_clock_it_is_handed() {
        let l = Limiter::new();
        l.configure("h.example", Pace { gap: Duration::from_secs(2), rest: Duration::from_secs(600) });
        let clock = ManualClock::at(t0());
        l.turn("h.example", &clock).unwrap();
        assert_eq!(clock.now(), t0(), "the first ask goes at once");
        l.turn("h.example", &clock).unwrap();
        assert_eq!(clock.now(), plus(t0(), Duration::from_secs(2)), "the second waits the gap");
    }

    #[test]
    fn a_refusal_rests_the_host_for_its_retry_after_else_its_rest() {
        let l = Limiter::new();
        l.configure("y.example", Pace { gap: Duration::from_secs(2), rest: Duration::from_secs(600) });
        let now = t0();
        l.refused("y.example", now, None);
        let rest_end = plus(now, Duration::from_secs(600));
        assert_eq!(l.reserve("y.example", plus(now, Duration::from_secs(599))), Err(Resting { until: rest_end }));
        assert!(l.reserve("y.example", rest_end).is_ok(), "asked again when the rest is over");

        let l = Limiter::new();
        l.configure("y.example", Pace { gap: Duration::ZERO, rest: Duration::from_secs(600) });
        l.refused("y.example", now, Some(Duration::from_secs(30)));
        assert_eq!(l.reserve("y.example", plus(now, Duration::from_secs(29))), Err(Resting { until: plus(now, Duration::from_secs(30)) }));
        assert!(l.reserve("y.example", plus(now, Duration::from_secs(30))).is_ok(), "Retry-After is honoured, not the longer rest");

        // a shorter rest after a longer one does not cut it short
        let l = Limiter::new();
        l.refused("z.example", now, Some(Duration::from_secs(300)));
        l.refused("z.example", now, Some(Duration::from_secs(5)));
        assert!(l.reserve("z.example", plus(now, Duration::from_secs(100))).is_err());
    }

    /// Refusals that name no time rest the host twice as long each time, up to half
    /// an hour, and an answer brings it back to its pace's rest; one that names a
    /// time rests it for that time.
    #[test]
    fn a_host_refusing_again_rests_twice_as_long_each_time_up_to_half_an_hour() {
        let l = Limiter::new();
        l.configure("y.example", Pace { gap: Duration::from_secs(2), rest: Duration::from_secs(600) });
        let mut now = t0();
        let mut rests = Vec::new();
        for _ in 0..5 {
            l.refused("y.example", now, None);
            let Err(Resting { until }) = l.reserve("y.example", now) else { panic!("resting") };
            rests.push(now.duration_until(until).as_secs());
            now = until;
        }
        assert_eq!(rests, [600, 1200, 1800, 1800, 1800]);
        l.answered("y.example");
        l.refused("y.example", now, None);
        assert_eq!(l.reserve("y.example", now), Err(Resting { until: plus(now, Duration::from_secs(600)) }), "an answer, then a refusal: its rest again");
        let later = plus(now, Duration::from_secs(600));
        l.refused("y.example", later, Some(Duration::from_secs(45)));
        assert_eq!(l.reserve("y.example", later), Err(Resting { until: plus(later, Duration::from_secs(45)) }), "the time it names");
        // a host with no pace of its own: a minute, doubled up to half an hour
        let free = Limiter::new();
        let mut rests = Vec::new();
        for _ in 0..7 {
            free.refused("free.example", now, None);
            let Err(Resting { until }) = free.reserve("free.example", now) else { panic!("resting") };
            rests.push(now.duration_until(until).as_secs());
            now = until;
        }
        assert_eq!(rests, [60, 120, 240, 480, 960, 1800, 1800]);
    }

    #[test]
    fn a_host_with_no_pace_is_asked_at_once_and_rests_a_minute() {
        let l = Limiter::new();
        let now = t0();
        assert_eq!(l.reserve("free.example", now).unwrap(), now);
        assert_eq!(l.reserve("free.example", now).unwrap(), now);
        l.refused("free.example", now, None);
        assert_eq!(l.reserve("free.example", now), Err(Resting { until: plus(now, Duration::from_secs(60)) }));
    }
}
