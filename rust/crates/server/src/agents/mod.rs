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
pub mod cloudflare;
pub mod install;
pub mod fetch;
pub mod listener;
pub mod ngrok;
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
/// Wrong entries of one request's code before the request ends (brief 81 §2).
pub const CODE_TRIES: u8 = 3;
/// Wrong code entries in all, on every request, before no code is taken until the
/// person resumes sign-in in their panel (brief 81 §2).
pub const WRONG_CODES_PAUSE: u32 = 5;
/// The wrong code entries counted since the person last resumed sign-in.
pub const WRONG_CODES: &str = "agents.wrong_codes";

/// The wrong code entries counted, kept in the book so a restart never forgets them.
pub fn wrong_codes(store: &AgentsBook) -> Result<u32, String> {
    Ok(store.setting(WRONG_CODES).map_err(|e| e.to_string())?.and_then(|n| n.parse().ok()).unwrap_or(0))
}
/// How long an authorization code handed to the agent's page may wait to be exchanged.
pub const CODE_LIFE: SignedDuration = SignedDuration::from_mins(5);
/// How often the waiting page may ask after one sign-in.
pub const POLL_EVERY: SignedDuration = SignedDuration::from_secs(1);

/// The sign-ins that have ended, gone: called on a write the person or an agent
/// makes (an answer, a new sign-in), never on a read.
pub fn end_expired(store: &AgentsBook, now: Timestamp) -> bagholder_book::Result<()> {
    store.end_expired(now - REQUEST_LIFE, now - CODE_LIFE)
}
/// How long an access token is good for.
pub const ACCESS_LIFE: SignedDuration = SignedDuration::from_hours(1);
/// A refresh token is good until it is used, or for this long.
pub const REFRESH_LIFE: SignedDuration = SignedDuration::from_hours(24 * 90);

/// The settings the person's choices are kept under, in the book.
pub const ON: &str = "agents.on";
pub const PORT: &str = "agents.port";
pub const TUNNEL: &str = "agents.tunnel";
pub const HOST: &str = "agents.host";
/// The tunnels the person last started, as a JSON list of providers: each is added
/// when they start it, taken out when they stop it, and all go when they switch
/// Agents off. A tunnel starts again with Bagholder only while it is listed, so one
/// the person stopped stays stopped, and one that ended on its own starts again. A
/// book written before several addresses holds `1` for its one tunnel.
pub const RUNNING: &str = "agents.running";
/// Every way set up, each giving one address (brief 81 §3), as a JSON list of
/// `Setup`: one of each provider. A book written before several addresses holds its
/// one in `TUNNEL`.
pub const TUNNELS: &str = "agents.tunnels";
/// Each address's public name, as a JSON object from provider to host. A book
/// written before several addresses holds its one in `HOST`.
pub const HOSTS: &str = "agents.hosts";

/// One address the listener answers at (brief 81 §3): the public name, as `Host`
/// must name it (lower case, with its port when that is not 443), and the `Host`
/// the person's own proxy sends for it when that is another (empty when not).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub host: String,
    pub receives: String,
}

impl Site {
    pub fn new(host: &str, receives: Option<&str>) -> Site {
        Site { host: host.to_ascii_lowercase(), receives: receives.unwrap_or("").to_ascii_lowercase() }
    }

    /// The `Host` the gate takes for it: the public name, or what the proxy sends.
    pub fn received(&self) -> &str {
        if self.receives.is_empty() { &self.host } else { &self.receives }
    }

    /// The address an agent is given: `https://{host}`.
    pub fn base(&self) -> String {
        format!("https://{}", self.host)
    }

    /// The one resource a token made at this address is for (RFC 8707).
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.base())
    }
}

/// What the listener is given: the public name it answers for, the agent tables,
/// the server to read, the clock, and what it fetches from elsewhere with.
pub struct Ctx {
    /// Every address it answers at: a request names one in `Host`, or is refused.
    /// None until a tunnel's address is known: then nothing passes.
    pub sites: std::sync::RwLock<Vec<Site>>,
    pub store: Arc<dyn Fn() -> Result<AgentsBook, String> + Send + Sync>,
    pub read: Arc<dyn Fn() -> Result<ReadOnly, String> + Send + Sync>,
    pub now: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    pub outside: fetch::Outside,
    /// The buckets: before sign-in, and each agent's.
    pub limits: Mutex<listener::Limits>,
    /// The one-time reachability value while a check is open (§4.4).
    pub check: Mutex<Option<tunnel::Check>>,
    /// Held by the check running: a second waits for it, so neither replaces the
    /// other's value (brief 80 §5.3).
    pub one_check: Mutex<()>,
}

impl Ctx {
    /// Every address it answers at.
    pub fn sites(&self) -> Vec<Site> {
        self.sites.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The address a request's `Host` names, if it names one.
    pub fn site_for(&self, received: &str) -> Option<Site> {
        self.sites().into_iter().find(|s| !s.host.is_empty() && s.received() == received)
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
    /// Each address's tunnel, by its provider.
    tunnel: Mutex<std::collections::BTreeMap<tunnel::Provider, tunnel::Tunnels>>,
    /// Each address's last check, until it or its tunnel changes.
    ready: Mutex<std::collections::BTreeMap<tunnel::Provider, tunnel::Ready>>,
    /// What Tailscale last said of this machine, when it was asked.
    tailscale: Mutex<Option<tunnel::TailscaleSays>>,
    /// Cloudflare's sign-in, while it runs, and what it said.
    cloudflare: Arc<Mutex<SignIn>>,
    /// Each program Bagholder installs (`install`): empty while it runs, why it
    /// failed once it did, gone once it worked.
    installing: Mutex<std::collections::BTreeMap<tunnel::Provider, String>>,
}

/// Cloudflare's `tunnel login` while it runs: the page it asks the person to open,
/// and its last line, said when it ends without a sign-in.
#[derive(Default)]
struct SignIn {
    child: Option<std::process::Child>,
    url: Option<String>,
    last: String,
}

impl Agents {
    /// An install of `provider`'s program begun, unless one runs already: false then.
    pub fn install_begun(&self, provider: tunnel::Provider) -> bool {
        let mut i = self.installing.lock().unwrap_or_else(|e| e.into_inner());
        if i.get(&provider).is_some_and(|said| said.is_empty()) {
            return false;
        }
        i.insert(provider, String::new());
        true
    }

    /// An install ended: gone once it worked, why it failed kept until it is tried again.
    pub fn install_ended(&self, provider: tunnel::Provider, result: Result<(), String>) {
        let mut i = self.installing.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => i.remove(&provider),
            Err(e) => i.insert(provider, e),
        };
    }

    /// Each install running, or failed, with why.
    pub fn installing(&self) -> std::collections::BTreeMap<tunnel::Provider, String> {
        self.installing.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

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
        for t in self.tunnels().values_mut() {
            t.stop();
        }
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// What the last check of `provider`'s address found, if one ran since it or its
    /// tunnel changed.
    pub fn ready(&self, provider: tunnel::Provider) -> Option<tunnel::Ready> {
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).get(&provider).cloned()
    }

    pub fn set_ready(&self, provider: tunnel::Provider, r: Option<tunnel::Ready>) {
        let mut held = self.ready.lock().unwrap_or_else(|e| e.into_inner());
        match r {
            Some(r) => held.insert(provider, r),
            None => held.remove(&provider),
        };
    }

    /// Tailscale asked again: what it says now is what the panel shows.
    pub fn look_at_tailscale(&self) -> tunnel::TailscaleSays {
        let mut says = tunnel::tailscale_says(tunnel::find(tunnel::Provider::Tailscale).as_deref());
        // only while Bagholder's own Funnel isn't running: it would answer there
        if self.tunnels().get(&tunnel::Provider::Tailscale).is_none_or(|t| t.state().running.is_none()) {
            says.https_port_taken = says.address.as_deref().and_then(|a| a.parse::<std::net::IpAddr>().ok()).is_some_and(|a| tunnel::answers(std::net::SocketAddr::new(a, 443)));
        }
        *self.tailscale.lock().unwrap_or_else(|e| e.into_inner()) = Some(says.clone());
        says
    }

    /// What Tailscale last said, if it was asked in this run.
    pub fn tailscale(&self) -> Option<tunnel::TailscaleSays> {
        self.tailscale.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Cloudflare's sign-in started (brief 81 §3), under the tunnel's guard so it
    /// ends with Bagholder: the page it asks for is told to the panel as it says it.
    pub fn cloudflare_sign_in(&self, home: &std::path::Path, signal: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        let files = cloudflare::Files::new(home);
        if files.signed_in() {
            return Ok(());
        }
        let mut held = self.cloudflare.lock().unwrap_or_else(|e| e.into_inner());
        if held.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))) {
            return Ok(());
        }
        let program = tunnel::find_here(home, tunnel::Provider::Cloudflare).ok_or("Cloudflare’s connector isn’t installed: install it first.")?;
        let (argv, mut env) = tunnel::guarded(&files.sign_in(&program.display().to_string())?)?;
        env.extend(files.env());
        let mut child = tunnel::spawn_piped(&argv, env).map_err(|e| format!("Cloudflare’s sign-in could not start: {e}"))?;
        *held = SignIn::default();
        for out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
            let (state, signal, home) = (self.cloudflare.clone(), signal.clone(), home.to_path_buf());
            std::thread::Builder::new()
                .name("cloudflare-sign-in".into())
                .spawn(move || {
                    tunnel::each_line(out, |line| {
                        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                        match cloudflare::sign_in_url(&line) {
                            Some(u) => s.url = Some(u),
                            None if !line.trim().is_empty() => s.last = line,
                            None => {}
                        }
                        drop(s);
                        signal();
                    });
                    // the sign-in written as the sign-in ends: readable by the person alone
                    if let Err(e) = cloudflare::Files::new(&home).secure() {
                        crate::app::log(&format!("bagholder: {e}"));
                    }
                    signal();
                })
                .map_err(|e| e.to_string())?;
        }
        held.child = Some(child);
        Ok(())
    }

    /// Cloudflare as the panel shows it: signed in, or the sign-in under way, or why
    /// the last one ended without one.
    pub fn cloudflare_says(&self, home: &std::path::Path) -> tunnel::CloudflareSays {
        let files = cloudflare::Files::new(home);
        let mut s = self.cloudflare.lock().unwrap_or_else(|e| e.into_inner());
        let running = s.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if !running && s.child.take().is_some() && !files.signed_in() && s.last.is_empty() {
            s.last = "Cloudflare’s sign-in ended before a domain was authorized.".into();
        }
        let signed_in = files.signed_in();
        tunnel::CloudflareSays {
            signed_in,
            signing_in: running && !signed_in,
            sign_in_url: if running { s.url.clone() } else { None },
            said: if signed_in || running { String::new() } else { s.last.clone() },
        }
    }

    /// The port the listener answers on, while it is on.
    pub fn port(&self) -> Option<u16> {
        self.listening.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|l| l.port)
    }

    /// The addresses the listener answers at, from now on.
    pub fn set_sites(&self, sites: Vec<Site>) {
        if let Some(c) = self.ctx() {
            *c.sites.write().unwrap_or_else(|e| e.into_inner()) = sites;
        }
    }

    /// What the running listener was given.
    pub fn ctx(&self) -> Option<Arc<Ctx>> {
        self.listening.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|l| l.ctx.clone())
    }

    /// Each address's tunnel: running or not, and what it last said.
    pub fn tunnels(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<tunnel::Provider, tunnel::Tunnels>> {
        self.tunnel.lock().unwrap_or_else(|e| e.into_inner())
    }
}
