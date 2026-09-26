//! The machine's clock, and the process's one limiter on it: the one file of
//! this crate that reads the machine's time.

use crate::clock::Clock;
use crate::limiter::{Limiter, Pace};
use crate::net::Net;
use jiff::Timestamp;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The machine's clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d)
    }
}

/// The process's one limiter, which every request to the network goes through.
pub fn global() -> &'static Limiter {
    shared_ref()
}

fn shared_ref() -> &'static Arc<Limiter> {
    static L: OnceLock<Arc<Limiter>> = OnceLock::new();
    L.get_or_init(|| Arc::new(Limiter::new()))
}

/// The same limiter, for a [`Net`] built by a caller.
pub fn shared() -> Arc<Limiter> {
    shared_ref().clone()
}

/// The process's network: the one limiter, on the machine's clock.
pub fn net() -> &'static Net {
    static N: OnceLock<Net> = OnceLock::new();
    N.get_or_init(|| Net::new(Arc::new(SystemClock), shared()))
}

/// For the readers not yet behind the source contract (stage 5): wait for this
/// caller's turn at `host`, `gap` after the last, on the process's limiter and
/// the machine's clock.
pub fn turn(host: &str, gap: Duration) {
    let l = global();
    if l.pace(host).gap < gap {
        l.configure(host, Pace { gap, ..l.pace(host) });
    }
    // a host these callers pace is never marked resting by them
    let _ = l.turn(host, &SystemClock);
}

/// For the same readers: `host` refused a request just now.
pub fn refused_now(host: &str, retry_after: Option<Duration>) {
    global().refused(host, SystemClock.now(), retry_after)
}
