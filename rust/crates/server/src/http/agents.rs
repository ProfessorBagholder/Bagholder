//! The Agents panel's routes, on the page's own listener behind its gate
//! (`docs/plans/agents-from-anywhere.md`, §3, §4): switching the agents' address
//! on and off, the tunnel chosen, started, stopped and checked, a sign-in allowed
//! or denied, a key made, an agent cut off, and an agent's log. No tunnel reaches
//! this listener, so nothing here can be asked from the public address.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

use axum::extract::State;
use axum::Json;
use bagholder_diff_derive::Diff;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{api_routes, blocking, Api, ApiError, AppState, OkOr, Params, Required, Routed};
use crate::agents::approve::{self, Connected, KeyAsked, LogLine, MadeKey, Renamed, Waiting};
use crate::agents::tunnel::{self, Provider, Ready, Setup, TailscaleSays, TunnelState};
use crate::agents::{self, Ctx, Site};
use crate::app::App;

pub fn routes() -> Routed {
    api_routes! {
        post "/api/agents/switch" => switch;
        post "/api/agents/tunnel" => choose;
        post "/api/agents/tunnel/start" => start_tunnel;
        post "/api/agents/tunnel/stop" => stop_tunnel;
        post "/api/agents/tunnel/remove" => remove_tunnel;
        post "/api/agents/tunnel/look" => look;
        post "/api/agents/check" => check;
        post "/api/agents/requests/deny" => deny;
        post "/api/agents/codes/resume" => resume_codes;
        post "/api/agents/keys" => make_key;
        post "/api/agents/cut-off" => cut_off;
        post "/api/agents/rename" => rename;
        post "/api/agents/cloudflare/sign-in" => cloudflare_sign_in;
        post "/api/agents/ngrok/token" => ngrok_token;
        post "/api/agents/install" => install;
        get "/api/agents/log" => log;
    }
}

/// A tunnel's program, as this machine has it.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct ProgramFound {
    pub provider: Provider,
    /// Where it was found; none when it is not installed.
    pub path: Option<String>,
}

/// A program Bagholder is installing where it runs, or failed to.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct Installing {
    pub provider: Provider,
    /// Empty while it runs; why it failed, once it did.
    pub failed: String,
}

/// The Agents panel, as the page's status carries it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct AgentsStatus {
    pub on: bool,
    pub port: Option<u32>,
    /// Every address agents reach Bagholder at, each with its tunnel (brief 81 §3).
    pub addresses: Vec<tunnel::Address>,
    pub programs: Vec<ProgramFound>,
    /// The computer Bagholder runs on, by its name: what a program missing is
    /// missing from (`tunnel::computer`).
    pub computer: String,
    /// Its system (`linux`, `macos` or `windows`): what Tailscale is installed
    /// with there.
    pub os: String,
    /// The programs Bagholder is installing there, or failed to (`agents::install`).
    pub installing: Vec<Installing>,
    /// What Tailscale said of this machine when last asked: asked when Tailscale is
    /// chosen, when its tunnel starts, and when the person asks again.
    pub tailscale: Option<TailscaleSays>,
    /// Cloudflare as Bagholder sets it up: signed in, or signing in.
    pub cloudflare: tunnel::CloudflareSays,
    pub waiting: Vec<Waiting>,
    /// Every agent that may read: each grant and each key.
    pub allowed: Vec<Connected>,
    /// The requests refused from callers no grant or key names since the address
    /// opened, counted by reason (none is written).
    pub refused: Vec<RefusedCount>,
    /// Whether sign-in is paused: codes were typed wrong too often, on any request,
    /// and no code is taken until the person resumes it (brief 81 §2).
    pub codes_paused: bool,
    /// What could not be read, in words.
    pub error: String,
}

/// A reason the address refused callers no grant or key names, and how many.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct RefusedCount {
    pub why: String,
    pub count: u32,
}

/// The agent tables, on a connection whose commits reach the page.
fn store(app: &App) -> Result<bagholder_book::agents::AgentsBook, String> {
    let f = app.figures.get().ok_or("The book is not open yet.")?;
    let a = f.book()?.agents();
    a.on_commit(app.store_signal.clone()).map_err(|e| e.to_string())?;
    Ok(a)
}

/// A setting read, in words when it can't be.
fn setting(a: &bagholder_book::agents::AgentsBook, key: &str) -> Result<Option<String>, String> {
    a.setting(key).map_err(|e| e.to_string())
}

/// Every way set up, one of each provider (brief 81 §3). A book written before
/// several addresses holds its one in `TUNNEL`: read as the one of a list.
fn setups(a: &bagholder_book::agents::AgentsBook) -> Result<Vec<Setup>, String> {
    if let Some(t) = setting(a, agents::TUNNELS)? {
        return serde_json::from_str(&t).map_err(|e| format!("the tunnels' settings could not be read: {e}"));
    }
    let one = setting(a, agents::TUNNEL)?.map(|s| serde_json::from_str::<Setup>(&s)).transpose().map_err(|e| format!("the tunnel's settings could not be read: {e}"))?;
    Ok(one.into_iter().collect())
}

fn setup_of(a: &bagholder_book::agents::AgentsBook, provider: Provider) -> Result<Option<Setup>, String> {
    Ok(setups(a)?.into_iter().find(|s| s.provider == provider))
}

/// Each address's public name, by provider. A book written before several addresses
/// holds its one in `HOST`, for its one way.
fn hosts(a: &bagholder_book::agents::AgentsBook) -> Result<BTreeMap<Provider, String>, String> {
    if let Some(h) = setting(a, agents::HOSTS)? {
        return serde_json::from_str(&h).map_err(|e| format!("the addresses' settings could not be read: {e}"));
    }
    let one = setups(a)?.first().map(|s| s.provider);
    Ok(setting(a, agents::HOST)?.zip(one).map(|(h, p)| BTreeMap::from([(p, h)])).unwrap_or_default())
}

/// The tunnels to start again with Bagholder. A book written before several
/// addresses holds `1` for its one tunnel.
fn running(a: &bagholder_book::agents::AgentsBook) -> Result<BTreeSet<Provider>, String> {
    match setting(a, agents::RUNNING)?.as_deref() {
        None => Ok(BTreeSet::new()),
        Some("1") => Ok(setups(a)?.first().map(|s| s.provider).into_iter().collect()),
        Some(r) => serde_json::from_str(r).map_err(|e| format!("the running tunnels could not be read: {e}")),
    }
}

/// The ways, their names and the tunnels to start again, kept: in the lists that
/// hold several, the single settings of before cleared.
fn keep(a: &bagholder_book::agents::AgentsBook, setups: &[Setup], hosts: &BTreeMap<Provider, String>, running: &BTreeSet<Provider>) -> Result<(), String> {
    let now = bagholder_core::jiff::Timestamp::now();
    a.atomically(|| {
        a.set_setting(agents::TUNNELS, Some(&json(&setups)), now)?;
        a.set_setting(agents::HOSTS, Some(&json(hosts)), now)?;
        a.set_setting(agents::RUNNING, (!running.is_empty()).then(|| json(running)).as_deref(), now)?;
        a.set_setting(agents::TUNNEL, None, now)?;
        a.set_setting(agents::HOST, None, now)
    })
    .map_err(|e| e.to_string())
}

fn json<T: Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// The addresses the listener answers at: each way whose name is known.
fn sites(a: &bagholder_book::agents::AgentsBook) -> Result<Vec<Site>, String> {
    let hosts = hosts(a)?;
    let mut out = Vec::new();
    for s in setups(a)? {
        if let Some(h) = hosts.get(&s.provider) {
            out.push(Site::new(h, tunnel::received_host(&s)?.as_deref()));
        }
    }
    Ok(out)
}

/// The panel's state, for the page's status.
pub fn status(app: &Arc<App>) -> AgentsStatus {
    let mut out = AgentsStatus { on: app.agents.port().is_some(), port: app.agents.port().map(u32::from), ..AgentsStatus::default() };
    let states: BTreeMap<Provider, TunnelState> = {
        let mut t = app.agents.tunnels();
        t.values_mut().for_each(|x| x.reap());
        t.iter().map(|(p, x)| (*p, x.state())).collect()
    };
    out.tailscale = app.agents.tailscale();
    out.cloudflare = app.agents.cloudflare_says(&app.home);
    out.refused = app.agents.ctx().map(|c| agents::listener::strangers(&c)).unwrap_or_default().into_iter().map(|(why, n)| RefusedCount { why, count: n.min(u32::MAX as u64) as u32 }).collect();
    out.computer = tunnel::computer();
    out.os = std::env::consts::OS.to_string();
    out.installing = app.agents.installing().into_iter().map(|(provider, failed)| Installing { provider, failed }).collect();
    out.programs = Provider::ALL.iter().filter(|p| p.program().is_some()).map(|p| ProgramFound { provider: *p, path: tunnel::find_here(&app.home, *p).map(|x| x.display().to_string()) }).collect();
    let mut read = || -> Result<(), String> {
        let a = store(app)?;
        let hosts = hosts(&a)?;
        // each address: its way, its name whether it runs or not, and, while its tunnel
        // runs, what the last check found (brief 80 §5.2: nothing ready when it doesn't)
        for s in setups(&a)? {
            let st = states.get(&s.provider).cloned().unwrap_or_default();
            let running = s.provider == Provider::Own || st.running.is_some();
            out.addresses.push(tunnel::Address {
                provider: s.provider,
                url: hosts.get(&s.provider).map(|h| format!("https://{h}")).unwrap_or_default(),
                port443: !(s.provider == Provider::Tailscale && s.funnel_port == Some(8443)),
                running,
                ready: if running { app.agents.ready(s.provider) } else { None },
                exited: st.exited,
                lines: st.lines,
            });
        }
        out.waiting = approve::waiting(&a, bagholder_core::jiff::Timestamp::now())?;
        out.codes_paused = agents::wrong_codes(&a)? >= agents::WRONG_CODES_PAUSE;
        out.allowed = approve::connected(&a)?;
        Ok(())
    };
    if app.figures.get().is_some() {
        if let Err(e) = read() {
            out.error = e;
        }
    }
    out
}

/// What the listener is given in this app: the addresses it answers at, the agent
/// tables, this server's own address to read, the clock, and the network. Held
/// weakly, so the listener never keeps the app alive.
fn ctx(app: &Arc<App>, sites: Vec<Site>) -> Ctx {
    let (w1, w2): (Weak<App>, Weak<App>) = (Arc::downgrade(app), Arc::downgrade(app));
    Ctx {
        sites: std::sync::RwLock::new(sites),
        store: Arc::new(move || {
            let app = w1.upgrade().ok_or_else(|| "Bagholder is stopping.".to_string())?;
            store(&app)
        }),
        read: Arc::new(move || {
            let app = w2.upgrade().ok_or("Bagholder is stopping.")?;
            let port = *app.port.lock().unwrap_or_else(|e| e.into_inner());
            Ok(crate::mcp::ReadOnly::own(&format!("http://127.0.0.1:{port}")))
        }),
        now: Arc::new(bagholder_core::jiff::Timestamp::now),
        outside: agents::fetch::Outside::network(),
        limits: Default::default(),
        check: Default::default(),
        one_check: Default::default(),
    }
}

/// The page's own ports, never the agents' address's.
const PAGE_PORTS: std::ops::RangeInclusive<u16> = 8765..=8767;

/// A port the address was given, as it is kept for the next start: never one of
/// the page's own, which the system can hand out when it chooses (brief 80 §4.3).
fn kept_port(port: u16) -> Option<u16> {
    (!PAGE_PORTS.contains(&port)).then_some(port)
}

/// Switch the address on, on the port it was given before when it is free.
pub fn switch_on(app: &Arc<App>, rt: &tokio::runtime::Handle) -> Result<u16, String> {
    let a = store(app)?;
    let sites = sites(&a)?;
    let kept: u16 = setting(&a, agents::PORT)?.and_then(|p| p.parse().ok()).unwrap_or(0);
    // never the page's own ports
    let wanted = kept_port(kept).unwrap_or(0);
    let port = app.agents.switch_on(rt, wanted, ctx(app, sites.clone())).or_else(|_| app.agents.switch_on(rt, 0, ctx(app, sites)))?;
    let now = bagholder_core::jiff::Timestamp::now();
    a.set_setting(agents::ON, Some("1"), now).map_err(|e| e.to_string())?;
    a.set_setting(agents::PORT, kept_port(port).map(|p| p.to_string()).as_deref(), now).map_err(|e| e.to_string())?;
    Ok(port)
}

/// At start: the address on again when the person left it on. A copy of the book
/// started for some work (`BAGHOLDER_EXIT_WITH`, every scratch copy's rule) opens
/// no agents address at all (brief 80, round 3, 1): on the person's kept port it
/// would answer their own proxy, or a cloudflared or ngrok they run as a service,
/// under their public name with their grants.
pub fn open(app: &Arc<App>, rt: &tokio::runtime::Handle) {
    if std::env::var_os(crate::exit_with::VAR).is_some() {
        return;
    }
    let on = store(app).and_then(|a| a.setting(agents::ON).map_err(|e| e.to_string()));
    if let Ok(Some(v)) = on {
        if v == "1" {
            if let Err(e) = switch_on(app, rt) {
                crate::app::log(&format!("bagholder: the agents' address could not open again: {e}"));
                return;
            }
            // and each tunnel the person started, so every agent can reach it again
            // after an update or a restart (brief 80 §5.1), where it was running when
            // Bagholder stopped; their own proxy is theirs to run
            match store(app).and_then(|a| running(&a)) {
                Ok(started) => {
                    for p in started.into_iter().filter(|p| *p != Provider::Own) {
                        if let Err(e) = tunnel_up(app, p) {
                            crate::app::log(&format!("bagholder: the agents' {p:?} tunnel could not start again: {e}"));
                        }
                    }
                }
                Err(e) => crate::app::log(&format!("bagholder: the agents' tunnels could not be read: {e}")),
            }
        }
    }
}

#[derive(Deserialize, TS)]
pub struct SwitchAsked {
    pub on: bool,
}

async fn switch(State(state): State<AppState>, Required(asked): Required<SwitchAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let rt = tokio::runtime::Handle::current();
    let done = blocking(move || -> Result<(), String> {
        if asked.on {
            switch_on(&app, &rt)?;
        } else {
            app.agents.switch_off();
            let (a, now) = (store(&app)?, bagholder_core::jiff::Timestamp::now());
            a.set_setting(agents::ON, Some("0"), now).map_err(|e| e.to_string())?;
            keep(&a, &setups(&a)?, &hosts(&a)?, &BTreeSet::new())?;
        }
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// A way set up, with what the person entered for it, beside any others: one of
/// each provider, so setting one up again replaces it. Its public name is worked
/// out where it can be now (Tailscale's is asked of Tailscale).
async fn choose(State(state): State<AppState>, Required(asked): Required<Setup>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let mut s = asked;
        // Cloudflare signed in through Bagholder: its tunnel and the address's DNS record
        // are made here, and the address is the one Cloudflare made (brief 81 §3)
        let cf = agents::cloudflare::Files::new(&app.home);
        if s.provider == Provider::Cloudflare && cf.signed_in() {
            let program = tunnel::find_here(&app.home, Provider::Cloudflare).ok_or("Cloudflare’s connector isn’t installed: install it first.")?;
            s.name = cf.set_up(&program.display().to_string(), s.name.trim())?;
        }
        // ngrok with Bagholder's authtoken: its address is the account's dev domain,
        // known once ngrok says it as it starts
        let learned = s.provider == Provider::Ngrok && s.name.trim().is_empty() && agents::ngrok::Files::new(&app.home).has_token();
        if s.provider != Provider::Tailscale && !learned {
            s.name = if s.provider == Provider::Zrok { s.name.trim().to_ascii_lowercase() } else { tunnel::clean_name(&s.name)? };
        }
        if !matches!(s.funnel_port, None | Some(443) | Some(8443)) {
            return Err("Funnel serves on 443 or 8443 here.".into());
        }
        let a = store(&app)?;
        let (mut all, mut names, started) = (setups(&a)?, hosts(&a)?, running(&a)?);
        if learned {
            names.remove(&s.provider);
        } else {
            let tailscale = if s.provider == Provider::Tailscale { app.agents.look_at_tailscale().name } else { None };
            names.insert(s.provider, tunnel::host_of(&s, tailscale.as_deref())?);
            tunnel::received_host(&s)?;
        }
        all.retain(|x| x.provider != s.provider);
        all.push(s.clone());
        keep(&a, &all, &names, &started)?;
        app.agents.set_sites(sites(&a)?);
        app.agents.set_ready(s.provider, None);
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// Which address an action is for.
#[derive(Deserialize, TS)]
pub struct ProviderAsked {
    pub provider: Provider,
}

/// An address given up: its tunnel stopped, and its way, name and start-again gone.
/// Agents signed in at it no longer reach Bagholder; the panel says so.
async fn remove_tunnel(State(state): State<AppState>, Required(asked): Required<ProviderAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let p = asked.provider;
        if let Some(t) = app.agents.tunnels().get_mut(&p) {
            t.stop();
        }
        // what Bagholder made at Cloudflare goes with it, first: a removal that fails
        // there keeps the server, so it can be removed again
        if p == Provider::Cloudflare {
            let address = hosts(&store(&app)?)?.get(&p).cloned();
            remove_cloudflare(&app.home, address.as_deref(), &delete_record)?;
        }
        let a = store(&app)?;
        let (mut all, mut names, mut started) = (setups(&a)?, hosts(&a)?, running(&a)?);
        all.retain(|x| x.provider != p);
        names.remove(&p);
        started.remove(&p);
        keep(&a, &all, &names, &started)?;
        app.agents.set_sites(sites(&a)?);
        app.agents.set_ready(p, None);
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// Cloudflare's server removed at Cloudflare too, as the person asked by removing
/// it: the address's DNS record while it still points at the tunnel Bagholder made
/// (one pointing anywhere else is the person's, and stays), the tunnel, and
/// Bagholder's sign-in there. `dns` deletes the record (`delete_record`).
fn remove_cloudflare(home: &std::path::Path, address: Option<&str>, dns: &dyn Fn(&agents::cloudflare::Access, &str, &str) -> Result<(), String>) -> Result<(), String> {
    let cf = agents::cloudflare::Files::new(home);
    if let (Some(address), Some(id), Some(access)) = (address, cf.tunnel_id(), cf.access()) {
        dns(&access, address, &format!("{id}.cfargotunnel.com")).map_err(|e| format!("Cloudflare’s address {address} couldn’t be deleted: {e}"))?;
    }
    if cf.tunnel().is_some() {
        let program = tunnel::find_here(home, Provider::Cloudflare).ok_or("Cloudflare’s connector isn’t installed, so the tunnel can’t be deleted: install it first.")?;
        cf.delete_tunnel(&program.display().to_string())?;
    }
    cf.forget()
}

/// The CNAME records named `address` that point at `target`: the ones Bagholder
/// made, from Cloudflare's list of records (`result`, each with `id` and `content`).
fn ours(listed: &serde_json::Value, target: &str) -> Vec<String> {
    listed.get("result").and_then(serde_json::Value::as_array).into_iter().flatten()
        .filter(|r| r.get("content").and_then(serde_json::Value::as_str).is_some_and(|c| c.eq_ignore_ascii_case(target)))
        .filter_map(|r| r.get("id").and_then(serde_json::Value::as_str).map(str::to_string))
        .collect()
}

/// The address's DNS record deleted through Cloudflare's API with the sign-in's
/// token, only where it points at `target`.
fn delete_record(access: &agents::cloudflare::Access, address: &str, target: &str) -> Result<(), String> {
    let auth = format!("Bearer {}", access.token);
    let headers = [("Authorization", auth.as_str()), ("Accept", "application/json")];
    let records = format!("https://api.cloudflare.com/client/v4/zones/{}/dns_records", access.zone);
    let wait = std::time::Duration::from_secs(30);
    let query: String = serde_urlencoded::to_string([("type", "CNAME"), ("name", address)]).map_err(|e| e.to_string())?;
    let listed = bagholder_net::client::request("GET", &format!("{records}?{query}"), &headers, None, wait).map_err(|e| e.to_string())?;
    let listed: serde_json::Value = serde_json::from_slice(&listed.body).map_err(|e| format!("Cloudflare’s answer could not be read: {e}"))?;
    for id in ours(&listed, target) {
        bagholder_net::client::request("DELETE", &format!("{records}/{id}"), &headers, None, wait).map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn start_tunnel(State(state): State<AppState>, Required(asked): Required<ProviderAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        tunnel_up(&app, asked.provider)?;
        // running when Bagholder stops: it starts again with it
        let a = store(&app)?;
        let mut started = running(&a)?;
        started.insert(asked.provider);
        keep(&a, &setups(&a)?, &hosts(&a)?, &started)
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// A way's tunnel, started for its address: from the panel, and again when
/// Bagholder starts with Agents on.
fn tunnel_up(app: &Arc<App>, provider: Provider) -> Result<(), String> {
    tunnel_up_trying(app, provider, CLASH_TRIES)
}

/// How many times a tunnel Tailscale refused for a clash in its settings is started
/// (`tunnel::clashed`): the other program's change is made in a moment.
const CLASH_TRIES: u32 = 3;

fn tunnel_up_trying(app: &Arc<App>, provider: Provider, tries: u32) -> Result<(), String> {
    let port = app.agents.port().ok_or("Switch Agents on first.")?;
    let s = setup_of(&store(app)?, provider)?.ok_or("Set this way up first.")?;
    let program = s.provider.program().ok_or("Your own proxy is yours to run: point it at the address shown.")?;
    let path = tunnel::find_here(&app.home, s.provider).ok_or_else(|| format!("{program} isn’t installed: install it first."))?;
    if s.provider == Provider::Tailscale {
        app.agents.look_at_tailscale();
    }
    let command = tunnel::command(&s, &path.display().to_string(), port, &app.home).ok_or("This tunnel has no command.")?;
    let (signal, weak) = (app.events.clone(), Arc::downgrade(app));
    let checking = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // each line the tunnel writes is told to the page, and, until a check has
    // found the address answering, checks it again: the tunnel writes as it
    // comes up, so the address is checked as soon as it can answer
    let on_line = move || {
        signal.signal();
        if let Some(app) = weak.upgrade() {
            if provider == Provider::Ngrok {
                learn_ngrok_address(&app);
            }
            check_while_coming_up(&app, provider, &checking);
        }
    };
    let (argv, mut env) = tunnel::guarded(&command)?;
    // a tunnel Bagholder made is run from its own folder, as its home
    let cf = agents::cloudflare::Files::new(&app.home);
    if s.provider == Provider::Cloudflare && cf.tunnel().is_some() {
        env.extend(cf.env());
    }
    // ended on its own after Tailscale refused it for a clash: started again, while
    // tries are left, never once the person stopped it (`Tunnels::ended`)
    let weak = Arc::downgrade(app);
    let ended = move |run: u64| {
        let Some(app) = weak.upgrade() else { return };
        let again = app.agents.tunnels().get_mut(&provider).is_some_and(|t| t.start_again(run));
        if again && tries > 1 {
            if let Err(e) = tunnel_up_trying(&app, provider, tries - 1) {
                crate::app::log(&format!("bagholder: the {provider:?} tunnel could not start again: {e}"));
            }
        }
        app.events.signal();
    };
    app.agents.tunnels().entry(provider).or_default().start(provider, (argv, env), Arc::new(on_line), Arc::new(ended))?;
    app.agents.set_ready(provider, None);
    app.events.signal();
    Ok(())
}

/// ngrok's address, as it says it starting: the listener answers for it from then,
/// and it is kept as the address agents are given.
fn learn_ngrok_address(app: &Arc<App>) {
    let lines = app.agents.tunnels().get(&Provider::Ngrok).map(|t| t.state().lines).unwrap_or_default();
    let Some(host) = lines.iter().rev().find_map(|l| agents::ngrok::address(l)) else { return };
    let kept = store(app).and_then(|a| {
        let mut names = hosts(&a)?;
        if names.get(&Provider::Ngrok) == Some(&host) {
            return Ok(());
        }
        names.insert(Provider::Ngrok, host);
        keep(&a, &setups(&a)?, &names, &running(&a)?)?;
        app.agents.set_sites(sites(&a)?);
        Ok(())
    });
    if let Err(e) = kept {
        crate::app::log(&format!("bagholder: ngrok's address could not be kept: {e}"));
    }
}

/// The public name of `provider`'s address, once known.
fn host_of_provider(app: &Arc<App>, provider: Provider) -> String {
    store(app).and_then(|a| hosts(&a)).ok().and_then(|h| h.get(&provider).cloned()).unwrap_or_default()
}

/// The address checked on a thread of its own, one check at a time, until one finds
/// it answering (Ready, or confirmed from the agent where the public path is closed
/// to this computer).
fn check_while_coming_up(app: &Arc<App>, provider: Provider, checking: &Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    if matches!(app.agents.ready(provider), Some(Ready::Ready | Ready::Unconfirmed { .. })) || checking.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(ctx) = app.agents.ctx() else {
        checking.store(false, Ordering::SeqCst);
        return;
    };
    let (app, done) = (app.clone(), checking.clone());
    let started = std::thread::Builder::new().name("agents-check".into()).spawn(move || {
        let host = host_of_provider(&app, provider);
        app.agents.set_ready(provider, Some(tunnel::reach(&ctx, &host, provider)));
        app.events.signal();
        done.store(false, Ordering::SeqCst);
    });
    if let Err(e) = started {
        checking.store(false, Ordering::SeqCst);
        crate::app::log(&format!("bagholder: the agents' address could not be checked: {e}"));
    }
}

async fn stop_tunnel(State(state): State<AppState>, Required(asked): Required<ProviderAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        if let Some(t) = app.agents.tunnels().get_mut(&asked.provider) {
            t.stop();
        }
        app.agents.set_ready(asked.provider, None);
        app.events.signal();
        // stopped by the person: it stays stopped when Bagholder starts again
        let a = store(&app)?;
        let mut started = running(&a)?;
        started.remove(&asked.provider);
        keep(&a, &setups(&a)?, &hosts(&a)?, &started)
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// Tailscale asked again, when the person has signed in or let Funnel on since.
async fn look(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    blocking(move || {
        app.agents.look_at_tailscale();
        app.events.signal();
    })
    .await?;
    Ok(Json(OkOr::ok()))
}

/// Whether an address answers from outside (§4.4): only a fetch through the
/// public path shows Ready.
async fn check(State(state): State<AppState>, Required(asked): Required<ProviderAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let ctx = app.agents.ctx().ok_or("Switch Agents on first.")?;
        let host = host_of_provider(&app, asked.provider);
        let ready = tunnel::reach(&ctx, &host, asked.provider);
        app.agents.set_ready(asked.provider, Some(ready));
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

#[derive(Deserialize, TS)]
pub struct AgentId {
    pub id: String,
}

async fn answer(state: AppState, id: String, allowed: bool) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || store(&app).and_then(|a| approve::answer(&a, &id, allowed, bagholder_core::jiff::Timestamp::now()))).await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// End a sign-in the person did not start: its agent is told it was refused. A
/// sign-in is allowed only by its code, typed on the agent's page (brief 81 §2).
async fn deny(State(state): State<AppState>, Required(asked): Required<AgentId>) -> Api<OkOr> {
    answer(state, asked.id, false).await
}

/// Resume sign-in after codes were typed wrong too often: the count starts again.
async fn resume_codes(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || store(&app).and_then(|a| a.set_setting(agents::WRONG_CODES, None, bagholder_core::jiff::Timestamp::now()).map_err(|e| e.to_string()))).await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// A key made for an agent: shown this once.
async fn make_key(State(state): State<AppState>, Required(asked): Required<KeyAsked>) -> Api<MadeKey> {
    let app = state.app.clone();
    let made = blocking(move || store(&app).and_then(|a| approve::make_key(&a, &asked, &sites(&a)?.iter().map(Site::base).collect::<Vec<_>>(), bagholder_core::jiff::Timestamp::now()))).await?;
    made.map(Json).map_err(ApiError::BadRequest)
}

async fn cut_off(State(state): State<AppState>, Required(asked): Required<AgentId>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || store(&app).and_then(|a| approve::cut_off(&a, &asked.id, bagholder_core::jiff::Timestamp::now()))).await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// Cloudflare's sign-in started: the page it asks for reaches the panel in the status.
async fn cloudflare_sign_in(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || {
        let events = app.events.clone();
        app.agents.cloudflare_sign_in(&app.home, Arc::new(move || events.signal()))
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

#[derive(Deserialize, TS)]
pub struct NgrokToken {
    pub token: String,
}

/// A way's program installed by Bagholder where it runs (`agents::install`), on a
/// thread of its own: the panel follows it in the status.
async fn install(State(state): State<AppState>, Required(asked): Required<ProviderAsked>) -> Api<OkOr> {
    let app = state.app.clone();
    let source = match agents::install::source(asked.provider, std::env::consts::OS, std::env::consts::ARCH) {
        Ok(s) => s,
        Err(e) => return Ok(Json(OkOr::err(e))),
    };
    if !app.agents.install_begun(asked.provider) {
        return Ok(Json(OkOr::ok()));
    }
    app.events.signal();
    let a = app.clone();
    let started = std::thread::Builder::new().name("agents-install".into()).spawn(move || {
        let done = agents::install::install(&a.home, asked.provider, &source, &agents::install::download).map(|_| ());
        a.agents.install_ended(asked.provider, done.map_err(|e| format!("It couldn’t be installed: {e}")));
        a.events.signal();
    });
    if let Err(e) = started {
        app.agents.install_ended(asked.provider, Err(e.to_string()));
    }
    Ok(Json(OkOr::ok()))
}

/// The person's ngrok authtoken, kept by ngrok in Bagholder's own config.
async fn ngrok_token(State(state): State<AppState>, Required(asked): Required<NgrokToken>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || {
        let program = tunnel::find_here(&app.home, Provider::Ngrok).ok_or("ngrok isn’t installed: install it first.")?;
        agents::ngrok::Files::new(&app.home).save_token(&program.display().to_string(), &asked.token)
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// A connected agent named again by the person.
async fn rename(State(state): State<AppState>, Required(asked): Required<Renamed>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || store(&app).and_then(|a| approve::rename(&a, &asked))).await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

#[derive(Deserialize, Serialize, TS)]
pub struct LogAsked {
    pub agent: Option<String>,
}

/// An agent's log, newest first: its reads (tool and arguments, never answers)
/// and refusals.
#[derive(Serialize, TS)]
pub struct AgentLog {
    pub lines: Vec<LogLine>,
}

async fn log(State(state): State<AppState>, Params(q): Params<LogAsked>) -> Api<AgentLog> {
    let app = state.app.clone();
    let agent = q.agent.ok_or_else(|| ApiError::BadRequest("Which agent's log?".into()))?;
    let lines = blocking(move || store(&app).and_then(|a| approve::log(&a, &agent))).await?;
    lines.map(|lines| Json(AgentLog { lines })).map_err(ApiError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As the tunnel comes up, the address is checked on a thread of its own, once at
    /// a time, and no more once a check has found it answering.
    #[test]
    fn the_address_is_checked_as_the_tunnel_comes_up_until_it_answers() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        switch_on(&app, rt.handle()).unwrap();
        app.agents.set_sites(vec![]);
        let checking = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let own = Provider::Own;
        // no address known yet: the check runs, and says so
        check_while_coming_up(&app, own, &checking);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.agents.ready(own).is_none() && std::time::Instant::now() < until {
            std::thread::yield_now();
        }
        assert_eq!(app.agents.ready(own), Some(Ready::Failed { said: "The address is not known yet.".into() }));
        // one already running: no second
        checking.store(true, std::sync::atomic::Ordering::SeqCst);
        app.agents.set_ready(own, None);
        check_while_coming_up(&app, own, &checking);
        assert_eq!(app.agents.ready(own), None, "a check in flight is not started again");
        // found answering: never checked again
        checking.store(false, std::sync::atomic::Ordering::SeqCst);
        app.agents.set_ready(own, Some(Ready::Ready));
        check_while_coming_up(&app, own, &checking);
        assert!(!checking.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(app.agents.ready(own), Some(Ready::Ready));
        app.agents.switch_off();
        crate::clear::clear(&app, app.figures.get().unwrap(), &[crate::clear::Kind::Agents], bagholder_core::jiff::Timestamp::now()).unwrap();
    }

    /// Ready lasts only while the tunnel Bagholder starts runs, and the address is
    /// given to agents only then: with it not running, nothing is ready, whatever a
    /// check found; the person's own proxy, which Bagholder does not run, keeps it
    /// (brief 80 §5.2). The address is named either way, for the panel to show.
    #[test]
    fn ready_and_the_address_last_only_while_the_tunnel_runs() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        switch_on(&app, rt.handle()).unwrap();
        let a = store(&app).unwrap();
        let now = bagholder_core::jiff::Timestamp::now();
        for (provider, kept) in [(Provider::Ngrok, false), (Provider::Own, true)] {
            let s = Setup { provider, name: "agents.example.com".into(), funnel_port: None, sends_host: None };
            keep(&a, &[s], &BTreeMap::from([(provider, "agents.example.com".to_string())]), &BTreeSet::new()).unwrap();
            app.agents.set_sites(sites(&a).unwrap());
            app.agents.set_ready(provider, Some(Ready::Ready));
            let st = status(&app);
            let at = &st.addresses[0];
            assert_eq!(at.url, "https://agents.example.com", "{provider:?}");
            assert_eq!((at.ready.is_some(), at.running), (kept, kept), "{provider:?}");
        }
        let _ = now;
        app.agents.switch_off();
        crate::clear::clear(&app, app.figures.get().unwrap(), &[crate::clear::Kind::Agents], bagholder_core::jiff::Timestamp::now()).unwrap();
    }

    /// Clear's Agents kind removes the five settings and stops the listener and the
    /// tunnel: nothing is left to answer an agent (the plan's test, brief 80 §6.5).
    #[test]
    fn clearing_the_agents_stops_the_listener_and_the_tunnel_and_removes_their_settings() {
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        switch_on(&app, rt.handle()).unwrap();
        let a = store(&app).unwrap();
        let now = bagholder_core::jiff::Timestamp::now();
        let s = Setup { provider: Provider::Own, name: "agents.example.com".into(), funnel_port: None, sends_host: None };
        keep(&a, &[s], &BTreeMap::from([(Provider::Own, "agents.example.com".to_string())]), &BTreeSet::from([Provider::Own])).unwrap();
        // every one of the agents' settings set, `agents.running` with them, so
        // Clear is shown to remove each (brief 80, round 3, 5)
        for key in bagholder_book::agents::SETTINGS {
            if a.setting(key).unwrap().is_none() {
                a.set_setting(key, Some("1"), now).unwrap();
            }
            assert!(a.setting(key).unwrap().is_some(), "{key} is set before Clear");
        }
        assert!(app.agents.port().is_some());
        crate::clear::clear(&app, app.figures.get().unwrap(), &[crate::clear::Kind::Agents], now).unwrap();
        assert!(app.agents.port().is_none(), "the listener stopped");
        assert!(app.agents.tunnels().values().all(|t| t.state().running.is_none()), "no tunnel runs");
        let a = store(&app).unwrap();
        for key in bagholder_book::agents::SETTINGS {
            assert_eq!(a.setting(key).unwrap(), None, "{key} is gone");
        }
    }

    /// A port in the page's own range is never kept for the agents' address, whoever
    /// chose it (brief 80 §4.3).
    #[test]
    fn a_port_of_the_page_s_own_is_never_kept() {
        for p in [8765, 8766, 8767] {
            assert_eq!(kept_port(p), None, "{p}");
        }
        assert_eq!(kept_port(8768), Some(8768));
        assert_eq!(kept_port(41234), Some(41234));
    }

    /// The page's status reads the agents' tables and never writes them, Agents off
    /// or on, with a sign-in kept past its life among them: the store's commits are
    /// where they were. It ran on every status step while holding the app's state,
    /// which a CSV import takes inside its own write.
    #[test]
    fn the_page_s_status_never_writes_the_book_agents_off_or_on() {
        use bagholder_book::agents::{Client, NewRequest};
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        // the header's own reads record their first answer once (`feeds::feed_answered`)
        crate::status::status(&app);
        let a = store(&app).unwrap();
        let long_ago = bagholder_core::jiff::Timestamp::now() - agents::REQUEST_LIFE - agents::REQUEST_LIFE;
        let client = Client { id: "c-status".into(), metadata: false, name: "An agent".into(), redirect_uris: vec!["https://agent.example/cb".into()], made_at: long_ago, approved: false };
        a.keep_client(&client, agents::UNAPPROVED_KEPT).unwrap();
        let r = NewRequest { id: "0123456789abcdef0123456789abcdef", code: "ABCDEF", client_id: "c-status", client_name: "An agent", who: "agent.example", redirect_uri: "https://agent.example/cb", state: None, challenge: "x", resource: "https://agents.example/mcp" };
        assert!(a.add_request(&r, long_ago, agents::PENDING_KEPT).unwrap());
        for on in [false, true] {
            if on {
                switch_on(&app, rt.handle()).unwrap();
            }
            let before = app.events.stamp()[crate::events::Source::Store as usize];
            let s = crate::status::status(&app);
            assert_eq!(s.agents.on, on);
            assert!(s.agents.waiting.is_empty(), "a sign-in past its life is not waiting");
            assert_eq!(app.events.stamp()[crate::events::Source::Store as usize], before, "Agents {}: nothing written", if on { "on" } else { "off" });
            assert!(a.request("0123456789abcdef0123456789abcdef").unwrap().is_some(), "and nothing removed");
        }
        app.agents.switch_off();
        crate::clear::clear(&app, app.figures.get().unwrap(), &[crate::clear::Kind::Agents], bagholder_core::jiff::Timestamp::now()).unwrap();
    }

    /// A made-up Cloudflare sign-in, as cloudflared writes one: its token block, base64 of JSON.
    fn sign_in_pem(zone: &str, token: &str) -> String {
        let json = serde_json::json!({ "zoneID": zone, "accountID": "acct", "apiToken": token }).to_string();
        let b64 = crate::agents::secret::b64url(json.as_bytes()).replace('-', "+").replace('_', "/");
        let padded = format!("{b64}{}", "=".repeat((4 - b64.len() % 4) % 4));
        format!("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n-----BEGIN ARGO TUNNEL TOKEN-----\n{}\n{}\n-----END ARGO TUNNEL TOKEN-----\n", &padded[..20], &padded[20..])
    }

    /// Removing Cloudflare's server deletes what Bagholder made there, as the
    /// person asked by removing it: the address's record (only one pointing at its
    /// tunnel), the tunnel, then its sign-in here. A deletion Cloudflare refuses
    /// keeps it all, so it can be removed again.
    #[cfg(unix)]
    #[test]
    fn removing_cloudflare_s_server_deletes_its_address_tunnel_and_sign_in_there() {
        use std::os::unix::fs::PermissionsExt;
        let made = || {
            let home = tempfile::tempdir().unwrap();
            let dir = home.path().join("agents-cloudflare");
            std::fs::create_dir_all(dir.join(".cloudflared")).unwrap();
            std::fs::write(dir.join(".cloudflared").join("cert.pem"), sign_in_pem("zone-1", "token-1")).unwrap();
            std::fs::write(dir.join("tunnel.json"), r#"{"AccountTag":"acct","TunnelSecret":"x","TunnelID":"t-1"}"#).unwrap();
            std::fs::write(dir.join("tunnel-name"), "bagholder-x").unwrap();
            // a stand-in for Cloudflare's program, as Bagholder's own copy: it writes what it was asked
            let program = crate::agents::install::path(home.path(), Provider::Cloudflare).unwrap();
            let log = home.path().join("asked.txt");
            std::fs::write(&program, format!("#!/bin/sh\necho \"$@\" >> {}\n", log.display())).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            (home, dir, log)
        };
        let (home, _, _) = made();
        assert_eq!(agents::cloudflare::Files::new(home.path()).access(), Some(agents::cloudflare::Access { zone: "zone-1".into(), token: "token-1".into() }));

        let (home, dir, log) = made();
        let asked = std::sync::Mutex::new(Vec::new());
        remove_cloudflare(home.path(), Some("agents.example.com"), &|a, address, target| {
            asked.lock().unwrap().push((a.zone.clone(), a.token.clone(), address.to_string(), target.to_string()));
            Ok(())
        })
        .unwrap();
        assert_eq!(asked.into_inner().unwrap(), vec![("zone-1".to_string(), "token-1".to_string(), "agents.example.com".to_string(), "t-1.cfargotunnel.com".to_string())]);
        let said = std::fs::read_to_string(&log).unwrap();
        assert!(said.contains("tunnel") && said.contains("delete -f bagholder-x"), "{said}");
        assert!(!dir.exists(), "the sign-in and the tunnel's credentials are gone");

        let (home, dir, log) = made();
        let e = remove_cloudflare(home.path(), Some("agents.example.com"), &|_, _, _| Err("offline".into())).unwrap_err();
        assert!(e.contains("agents.example.com") && e.contains("offline"), "{e}");
        assert!(!log.exists(), "no tunnel deleted");
        assert!(dir.join(".cloudflared").join("cert.pem").exists(), "kept, to remove again");
    }

    /// Only a record pointing at Bagholder's tunnel is deleted: one the person
    /// pointed elsewhere under the same name is theirs.
    #[test]
    fn only_the_record_pointing_at_bagholder_s_tunnel_is_deleted() {
        let listed = serde_json::json!({ "result": [
            { "id": "r1", "name": "agents.example.com", "content": "T-1.cfargotunnel.com" },
            { "id": "r2", "name": "agents.example.com", "content": "someone-else.example.net" }
        ]});
        assert_eq!(ours(&listed, "t-1.cfargotunnel.com"), vec!["r1".to_string()]);
        assert!(ours(&serde_json::json!({ "result": [] }), "t-1.cfargotunnel.com").is_empty());
    }
}
