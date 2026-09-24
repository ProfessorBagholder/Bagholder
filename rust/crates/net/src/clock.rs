//! The clock the network is handed. Everything in this crate asks the clock it
//! was given, so pacing and rests are tested on a [`ManualClock`]; the machine's
//! clock is `machine.rs`'s, the one file that reads it.

use jiff::{SignedDuration, Timestamp};
use std::sync::Mutex;
use std::time::Duration;

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
    /// Wait `d`.
    fn sleep(&self, d: Duration);
}

/// A clock that moves only when told to, or when slept on: sleeping moves it by
/// the time slept, at once.
pub struct ManualClock(Mutex<Timestamp>);

impl ManualClock {
    pub fn at(t: Timestamp) -> ManualClock {
        ManualClock(Mutex::new(t))
    }

    pub fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        *t = t.checked_add(SignedDuration::try_from(d).unwrap()).unwrap();
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }

    fn sleep(&self, d: Duration) {
        self.advance(d)
    }
}
