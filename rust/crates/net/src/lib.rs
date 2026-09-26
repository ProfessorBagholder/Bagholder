//! The network (`docs/plans/stage-3a-sources.md`, "Two new crates"): the HTTP
//! client, the browser-handshake session, the one limiter every host is paced by,
//! and [`Net`], which asks through them with the clock it is handed.

pub mod browser;
pub mod client;
pub mod clock;
pub mod limiter;
pub mod machine;
mod net;

pub use clock::{Clock, ManualClock};
pub use machine::SystemClock;
pub use limiter::{Limiter, Pace, Resting};
pub use net::{host_of, retry_after, Answer, Ask, Net, NetError, Reply, Transport, Via};
