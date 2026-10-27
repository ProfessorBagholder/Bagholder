//! Agents elsewhere read the trade history through one agents-only address
//! (`docs/plans/agents-from-anywhere.md`): a listener of its own on loopback,
//! reached through the tunnel the person chose, off until they switch it on.
//!
//! Nothing here can write the person's data or reach the page's routes. The
//! tools read the server through `ReadOnly`, whose routes can only be `GET`s; the
//! only store this module is handed is `AgentsBook`, the book's agent tables
//! alone (who may read, and what each read). Approval happens on the person's own
//! page, on the main listener, which no tunnel reaches (`approve`, called from
//! `http::agents`). A source scan holds all of it (`tests`).

pub mod api;
pub mod approve;
pub mod fetch;
pub mod listener;
pub mod oauth;
pub mod secret;
pub mod tools;
pub mod tunnel;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};

use bagholder_book::agents::AgentsBook;
use bagholder_core::jiff::{SignedDuration, Timestamp};

use crate::mcp::ReadOnly;

// ---- the bounds (brief 73, 1; §4.1), each held by a test ----

/// Sign-ins waiting on the person at once: a sixth is refused until one ends.
pub const PENDING_KEPT: usize = 5;
/// Registered clients never approved: registering another drops the oldest.
pub const UNAPPROVED_KEPT: usize = 20;
/// Calls before sign-in, every caller together, in a minute.
pub const BEFORE_SIGN_IN_PER_MINUTE: u32 = 60;
/// The largest body `/oauth/register` and `/oauth/token` take.
pub const BODY_LIMIT: usize = 16 * 1024;
/// Reads one agent may make in a minute, and at once.
pub const READS_PER_MINUTE: u32 = 60;
pub const READS_BURST: u32 = 10;
/// How long a sign-in waits on the person.
pub const REQUEST_LIFE: SignedDuration = SignedDuration::from_mins(10);
/// How long an authorization code handed to the agent's page may wait to be exchanged.
pub const CODE_LIFE: SignedDuration = SignedDuration::from_mins(5);
/// How long an access token is good for.
pub const ACCESS_LIFE: SignedDuration = SignedDuration::from_hours(1);
/// A refresh token is good until it is used, or for this long.
pub const REFRESH_LIFE: SignedDuration = SignedDuration::from_hours(24 * 90);

/// The settings the person's choices are kept under, in the book.
pub const ON: &str = "agents.on";
pub const PORT: &str = "agents.port";
pub const TUNNEL: &str = "agents.tunnel";
pub const HOST: &str = "agents.host";

/// What the listener is given: the public name it answers for, the agent tables,
/// the server to read, the clock, and what it fetches from elsewhere with.
pub struct Ctx {
    /// The public name, as `Host` must name it: lower case, with its port when
    /// that is not 443. Empty until a tunnel's address is known: then nothing passes.
    pub host: std::sync::RwLock<String>,
    /// The `Host` the listener receives, where the person's own proxy sends one
    /// other than the public name; empty when it is the public name.
    pub receives: std::sync::RwLock<String>,
    pub store: Arc<dyn Fn() -> Result<AgentsBook, String> + Send + Sync>,
    pub read: Arc<dyn Fn() -> Result<ReadOnly, String> + Send + Sync>,
    pub now: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    pub outside: fetch::Outside,
    /// The buckets: before sign-in, and each agent's.
    pub limits: Mutex<listener::Limits>,
    /// The one-time reachability value while a check is open (§4.4).
    pub check: Mutex<Option<tunnel::Check>>,
}

impl Ctx {
    /// The public name the gate requires.
    pub fn host(&self) -> String {
        self.host.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The `Host` the gate takes: the public name, or what the person's proxy sends.
    pub fn received(&self) -> String {
        let r = self.receives.read().unwrap_or_else(|e| e.into_inner()).clone();
        if r.is_empty() {
            self.host()
        } else {
            r
        }
    }

    /// The address an agent is given: `https://{host}`.
    pub fn base(&self) -> String {
        format!("https://{}", self.host())
    }

    /// The one resource a token is for (RFC 8707).
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.base())
    }

    /// The agent tables, on a blocking thread's own connection.
    pub fn store(&self) -> Result<AgentsBook, String> {
        (self.store)()
    }

    pub fn now(&self) -> Timestamp {
        (self.now)()
    }
}

/// The listener while it runs.
struct Listening {
    port: u16,
    ctx: Arc<Ctx>,
    stop: tokio::sync::watch::Sender<bool>,
}

/// The agents' address in this app: off, or on with its listener and, once
/// started, its tunnel.
#[derive(Default)]
pub struct Agents {
    listening: Mutex<Option<Listening>>,
    tunnel: Mutex<tunnel::Tunnels>,
    /// The last check of the address, until the address or the tunnel changes.
    ready: Mutex<Option<tunnel::Ready>>,
    /// What Tailscale last said of this machine, when it was asked.
    tailscale: Mutex<Option<tunnel::TailscaleSays>>,
}

impl Agents {
    /// Switch on: the listener bound on `127.0.0.1:{port}` (any free port when 0)
    /// and served on `rt`; on already, it stays as it is. The port it answers on.
    pub fn switch_on(&self, rt: &tokio::runtime::Handle, port: u16, ctx: Ctx) -> Result<u16, String> {
        let mut held = self.listening.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = held.as_ref() {
            return Ok(l.port);
        }
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("the agents' address could not take 127.0.0.1:{port}: {e}"))?;
        std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = std_listener.local_addr().map_err(|e| e.to_string())?.port();
        let ctx = Arc::new(ctx);
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let serving = ctx.clone();
        rt.spawn(async move {
            match tokio::net::TcpListener::from_std(std_listener) {
                Ok(l) => listener::serve(l, serving, stopped).await,
                Err(e) => crate::app::log(&format!("bagholder: the agents' address could not be served: {e}")),
            }
        });
        *held = Some(Listening { port, ctx, stop });
        Ok(port)
    }

    /// Switch off: the listener takes nothing new and lets what it holds finish,
    /// and the tunnel stops. Every grant and key is refused from then on, because
    /// nothing answers.
    pub fn switch_off(&self) {
        if let Some(l) = self.listening.lock().unwrap_or_else(|e| e.into_inner()).take() {
            #[expect(clippy::let_underscore_must_use, reason = "it fails only when the listener has ended already, which is the stop asked for")]
            let _ = l.stop.send(true);
        }
        self.tunnel.lock().unwrap_or_else(|e| e.into_inner()).stop();
        self.set_ready(None);
    }

    /// What the last check found, if one ran since the address or tunnel changed.
    pub fn ready(&self) -> Option<tunnel::Ready> {
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_ready(&self, r: Option<tunnel::Ready>) {
        *self.ready.lock().unwrap_or_else(|e| e.into_inner()) = r;
    }

    /// Tailscale asked again: what it says now is what the panel shows.
    pub fn look_at_tailscale(&self) -> tunnel::TailscaleSays {
        let says = tunnel::tailscale_says(tunnel::find(tunnel::Provider::Tailscale).as_deref());
        *self.tailscale.lock().unwrap_or_else(|e| e.into_inner()) = Some(says.clone());
        says
    }

    /// What Tailscale last said, if it was asked in this run.
    pub fn tailscale(&self) -> Option<tunnel::TailscaleSays> {
        self.tailscale.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The port the listener answers on, while it is on.
    pub fn port(&self) -> Option<u16> {
        self.listening.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|l| l.port)
    }

    /// The public name the listener answers for, from now on.
    pub fn set_host(&self, host: &str, receives: Option<&str>) {
        if let Some(c) = self.ctx() {
            *c.host.write().unwrap_or_else(|e| e.into_inner()) = host.to_ascii_lowercase();
            *c.receives.write().unwrap_or_else(|e| e.into_inner()) = receives.unwrap_or("").to_ascii_lowercase();
        }
        self.set_ready(None);
    }

    /// What the running listener was given.
    pub fn ctx(&self) -> Option<Arc<Ctx>> {
        self.listening.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|l| l.ctx.clone())
    }

    /// The tunnels: the one running, and what the last one said.
    pub fn tunnels(&self) -> std::sync::MutexGuard<'_, tunnel::Tunnels> {
        self.tunnel.lock().unwrap_or_else(|e| e.into_inner())
    }
}
