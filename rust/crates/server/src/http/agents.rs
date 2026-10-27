//! The Agents panel's routes, on the page's own listener behind its gate
//! (`docs/plans/agents-from-anywhere.md`, §3, §4): switching the agents' address
//! on and off, the tunnel chosen, started, stopped and checked, a sign-in allowed
//! or denied, a key made, an agent cut off, and an agent's log. No tunnel reaches
//! this listener, so nothing here can be asked from the public address.

use std::sync::{Arc, Weak};

use axum::extract::State;
use axum::Json;
use bagholder_diff_derive::Diff;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{api_routes, blocking, Api, ApiError, AppState, OkOr, Params, Required, Routed};
use crate::agents::approve::{self, Connected, KeyAsked, LogLine, MadeKey, Waiting};
use crate::agents::tunnel::{self, Provider, Ready, Setup, TailscaleSays, TunnelState};
use crate::agents::{self, Ctx};
use crate::app::App;

pub fn routes() -> Routed {
    api_routes! {
        post "/api/agents/switch" => switch;
        post "/api/agents/tunnel" => choose;
        post "/api/agents/tunnel/start" => start_tunnel;
        post "/api/agents/tunnel/stop" => stop_tunnel;
        post "/api/agents/tunnel/look" => look;
        post "/api/agents/check" => check;
        post "/api/agents/requests/allow" => allow;
        post "/api/agents/requests/deny" => deny;
        post "/api/agents/keys" => make_key;
        post "/api/agents/cut-off" => cut_off;
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

/// The Agents panel, as the page's status carries it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct AgentsStatus {
    pub on: bool,
    pub port: Option<u32>,
    /// The tunnel the person chose, and what they entered for it.
    pub setup: Option<Setup>,
    /// The address an agent is given, once its name is known.
    pub address: String,
    pub programs: Vec<ProgramFound>,
    /// What Tailscale said of this machine when last asked: asked when Tailscale is
    /// chosen, when its tunnel starts, and when the person asks again.
    pub tailscale: Option<TailscaleSays>,
    pub tunnel: TunnelState,
    pub ready: Option<Ready>,
    pub waiting: Vec<Waiting>,
    /// Every agent that may read: each grant and each key.
    pub allowed: Vec<Connected>,
    /// What could not be read, in words.
    pub error: String,
}

/// The agent tables, on a connection whose commits reach the page.
fn store(app: &App) -> Result<bagholder_book::agents::AgentsBook, String> {
    let f = app.figures.get().ok_or("The book is not open yet.")?;
    let a = f.book()?.agents();
    a.on_commit(app.store_signal.clone()).map_err(|e| e.to_string())?;
    Ok(a)
}

fn setup(a: &bagholder_book::agents::AgentsBook) -> Result<Option<Setup>, String> {
    a.setting(agents::TUNNEL).map_err(|e| e.to_string())?.map(|s| serde_json::from_str(&s).map_err(|e| format!("the tunnel's settings could not be read: {e}"))).transpose()
}

/// The panel's state, for the page's status.
pub fn status(app: &Arc<App>) -> AgentsStatus {
    let mut out = AgentsStatus { on: app.agents.port().is_some(), port: app.agents.port().map(u32::from), ..AgentsStatus::default() };
    {
        let mut t = app.agents.tunnels();
        t.reap();
        out.tunnel = t.state();
    }
    out.ready = app.agents.ready();
    out.tailscale = app.agents.tailscale();
    out.address = app.agents.ctx().map(|c| c.host()).filter(|h| !h.is_empty()).map(|h| format!("https://{h}")).unwrap_or_default();
    out.programs = Provider::ALL.iter().filter(|p| p.program().is_some()).map(|p| ProgramFound { provider: *p, path: tunnel::find(*p).map(|x| x.display().to_string()) }).collect();
    let mut read = || -> Result<(), String> {
        let a = store(app)?;
        out.setup = setup(&a)?;
        if out.address.is_empty() {
            out.address = a.setting(agents::HOST).map_err(|e| e.to_string())?.map(|h| format!("https://{h}")).unwrap_or_default();
        }
        out.waiting = approve::waiting(&a, bagholder_core::jiff::Timestamp::now())?;
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

/// What the listener is given in this app: the agent tables, this server's own
/// address to read, the clock, and the network. Held weakly, so the listener
/// never keeps the app alive.
fn ctx(app: &Arc<App>, host: &str, receives: Option<&str>) -> Ctx {
    let (w1, w2): (Weak<App>, Weak<App>) = (Arc::downgrade(app), Arc::downgrade(app));
    Ctx {
        host: std::sync::RwLock::new(host.to_ascii_lowercase()),
        receives: std::sync::RwLock::new(receives.unwrap_or("").to_ascii_lowercase()),
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
    }
}

/// Switch the address on, on the port it was given before when it is free.
pub fn switch_on(app: &Arc<App>, rt: &tokio::runtime::Handle) -> Result<u16, String> {
    let a = store(app)?;
    let host = a.setting(agents::HOST).map_err(|e| e.to_string())?.unwrap_or_default();
    let receives = setup(&a)?.map(|s| tunnel::received_host(&s)).transpose()?.flatten();
    let kept: u16 = a.setting(agents::PORT).map_err(|e| e.to_string())?.and_then(|p| p.parse().ok()).unwrap_or(0);
    // never the page's own ports
    let wanted = if (8765..=8767).contains(&kept) { 0 } else { kept };
    let port = app.agents.switch_on(rt, wanted, ctx(app, &host, receives.as_deref())).or_else(|_| app.agents.switch_on(rt, 0, ctx(app, &host, receives.as_deref())))?;
    let now = bagholder_core::jiff::Timestamp::now();
    a.set_setting(agents::ON, Some("1"), now).map_err(|e| e.to_string())?;
    a.set_setting(agents::PORT, Some(&port.to_string()), now).map_err(|e| e.to_string())?;
    Ok(port)
}

/// At start: the address on again when the person left it on.
pub fn open(app: &Arc<App>, rt: &tokio::runtime::Handle) {
    let on = store(app).and_then(|a| a.setting(agents::ON).map_err(|e| e.to_string()));
    if let Ok(Some(v)) = on {
        if v == "1" {
            if let Err(e) = switch_on(app, rt) {
                crate::app::log(&format!("bagholder: the agents' address could not open again: {e}"));
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
            store(&app)?.set_setting(agents::ON, Some("0"), bagholder_core::jiff::Timestamp::now()).map_err(|e| e.to_string())?;
        }
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// The tunnel chosen, with what the person entered for it; its public name
/// worked out where it can be now (Tailscale's is asked of Tailscale).
async fn choose(State(state): State<AppState>, Required(asked): Required<Setup>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let mut s = asked;
        if s.provider != Provider::Tailscale {
            s.name = if s.provider == Provider::Zrok { s.name.trim().to_ascii_lowercase() } else { tunnel::clean_name(&s.name)? };
        }
        if !matches!(s.funnel_port, None | Some(443) | Some(8443)) {
            return Err("Funnel serves on 443 or 8443 here.".into());
        }
        let tailscale = if s.provider == Provider::Tailscale {
            let says = app.agents.look_at_tailscale();
            // 443 held by another program here: Funnel serves on 8443 unless the person chose
            if says.https_port_taken && s.funnel_port.is_none() {
                s.funnel_port = Some(8443);
            }
            says.name
        } else {
            None
        };
        let host = tunnel::host_of(&s, tailscale.as_deref())?;
        let receives = tunnel::received_host(&s)?;
        let a = store(&app)?;
        let now = bagholder_core::jiff::Timestamp::now();
        a.set_setting(agents::TUNNEL, Some(&serde_json::to_string(&s).map_err(|e| e.to_string())?), now).map_err(|e| e.to_string())?;
        a.set_setting(agents::HOST, Some(&host), now).map_err(|e| e.to_string())?;
        app.agents.set_host(&host, receives.as_deref());
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

async fn start_tunnel(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let port = app.agents.port().ok_or("Switch Agents on first.")?;
        let s = setup(&store(&app)?)?.ok_or("Choose a tunnel first.")?;
        let program = s.provider.program().ok_or("Your own proxy is yours to run: point it at the address shown.")?;
        let path = tunnel::find(s.provider).ok_or_else(|| format!("{program} is not installed on this computer: follow its first step."))?;
        if s.provider == Provider::Tailscale {
            app.agents.look_at_tailscale();
        }
        let command = tunnel::command(&s, &path.display().to_string(), port).ok_or("This tunnel has no command.")?;
        let (signal, weak, provider) = (app.events.clone(), Arc::downgrade(&app), s.provider);
        let checking = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // each line the tunnel writes is told to the page, and, until a check has
        // found the address answering, checks it again: the tunnel writes as it
        // comes up, so the address is checked as soon as it can answer
        let on_line = move || {
            signal.signal();
            if let Some(app) = weak.upgrade() {
                check_while_coming_up(&app, provider, &checking);
            }
        };
        app.agents.tunnels().start(s.provider, tunnel::guarded(&command)?, Arc::new(on_line))?;
        app.agents.set_ready(None);
        app.events.signal();
        Ok(())
    })
    .await?;
    Ok(Json(done.map_or_else(OkOr::err, |()| OkOr::ok())))
}

/// The address checked on a thread of its own, one check at a time, until one finds
/// it answering (Ready, or confirmed from the agent where the public path is closed
/// to this computer).
fn check_while_coming_up(app: &Arc<App>, provider: Provider, checking: &Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    if matches!(app.agents.ready(), Some(Ready::Ready | Ready::Unconfirmed { .. })) || checking.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(ctx) = app.agents.ctx() else {
        checking.store(false, Ordering::SeqCst);
        return;
    };
    let (app, done) = (app.clone(), checking.clone());
    let started = std::thread::Builder::new().name("agents-check".into()).spawn(move || {
        app.agents.set_ready(Some(tunnel::reach(&ctx, provider)));
        app.events.signal();
        done.store(false, Ordering::SeqCst);
    });
    if let Err(e) = started {
        checking.store(false, Ordering::SeqCst);
        crate::app::log(&format!("bagholder: the agents' address could not be checked: {e}"));
    }
}

async fn stop_tunnel(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    blocking(move || {
        app.agents.tunnels().stop();
        app.agents.set_ready(None);
        app.events.signal();
    })
    .await?;
    Ok(Json(OkOr::ok()))
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

/// Whether the address answers from outside (§4.4): only a fetch through the
/// public path shows Ready.
async fn check(State(state): State<AppState>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || -> Result<(), String> {
        let ctx = app.agents.ctx().ok_or("Switch Agents on first.")?;
        let provider = setup(&store(&app)?)?.map_or(Provider::Own, |s| s.provider);
        let ready = tunnel::reach(&ctx, provider);
        app.agents.set_ready(Some(ready));
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

/// Allow a sign-in the person started: its agent's page goes on with its code.
async fn allow(State(state): State<AppState>, Required(asked): Required<AgentId>) -> Api<OkOr> {
    answer(state, asked.id, true).await
}

async fn deny(State(state): State<AppState>, Required(asked): Required<AgentId>) -> Api<OkOr> {
    answer(state, asked.id, false).await
}

/// A key made for an agent: shown this once.
async fn make_key(State(state): State<AppState>, Required(asked): Required<KeyAsked>) -> Api<MadeKey> {
    let app = state.app.clone();
    let made = blocking(move || store(&app).and_then(|a| approve::make_key(&a, &asked, bagholder_core::jiff::Timestamp::now()))).await?;
    made.map(Json).map_err(ApiError::BadRequest)
}

async fn cut_off(State(state): State<AppState>, Required(asked): Required<AgentId>) -> Api<OkOr> {
    let app = state.app.clone();
    let done = blocking(move || store(&app).and_then(|a| approve::cut_off(&a, &asked.id, bagholder_core::jiff::Timestamp::now()))).await?;
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
        app.agents.set_host("", None);
        let checking = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // no address known yet: the check runs, and says so
        check_while_coming_up(&app, Provider::Own, &checking);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.agents.ready().is_none() && std::time::Instant::now() < until {
            std::thread::yield_now();
        }
        assert_eq!(app.agents.ready(), Some(Ready::Failed { said: "The address is not known yet.".into() }));
        // one already running: no second
        checking.store(true, std::sync::atomic::Ordering::SeqCst);
        app.agents.set_ready(None);
        check_while_coming_up(&app, Provider::Own, &checking);
        assert_eq!(app.agents.ready(), None, "a check in flight is not started again");
        // found answering: never checked again
        checking.store(false, std::sync::atomic::Ordering::SeqCst);
        app.agents.set_ready(Some(Ready::Ready));
        check_while_coming_up(&app, Provider::Own, &checking);
        assert!(!checking.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(app.agents.ready(), Some(Ready::Ready));
        app.agents.switch_off();
        crate::clear::clear(&app, app.figures.get().unwrap(), &[crate::clear::Kind::Agents], bagholder_core::jiff::Timestamp::now()).unwrap();
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
        let r = NewRequest { id: "0123456789abcdef0123456789abcdef", code: "ABCDEF", client_id: "c-status", who: "agent.example", redirect_uri: "https://agent.example/cb", state: None, challenge: "x", resource: "https://agents.example/mcp" };
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
}
