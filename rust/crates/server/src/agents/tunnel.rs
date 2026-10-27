//! The tunnels (`docs/plans/agents-from-anywhere.md`, §4.2-§4.4): the one the
//! person chose, started as Bagholder's child with fixed arguments (never through
//! a shell), its life tied to the server's through `bagholder tunnel-guard`, its
//! output read as it arrives; and the check that the address answers from outside
//! before it shows as Ready.
//!
//! The only module of the agents' code that starts a process.

use std::collections::VecDeque;
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use bagholder_diff_derive::Diff;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{fetch, secret, Ctx};

/// The tunnels Bagholder supports, side by side; Tailscale Funnel recommended,
/// none chosen for the person (brief 72, §2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Diff)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Tailscale,
    Cloudflare,
    Ngrok,
    Zrok,
    Own,
}

impl Provider {
    pub const ALL: [Provider; 5] = [Provider::Tailscale, Provider::Cloudflare, Provider::Ngrok, Provider::Zrok, Provider::Own];

    /// The program Bagholder starts, by its own name: none for the person's own proxy.
    pub fn program(self) -> Option<&'static str> {
        match self {
            Provider::Tailscale => Some("tailscale"),
            Provider::Cloudflare => Some("cloudflared"),
            Provider::Ngrok => Some("ngrok"),
            // zrok v2 names its command `zrok2` (pinned at v2.0.8)
            Provider::Zrok => Some("zrok2"),
            Provider::Own => None,
        }
    }

    /// Where each maker's installer puts its program, beside `PATH`.
    fn installed(self) -> &'static [&'static str] {
        match self {
            Provider::Tailscale => &["/Applications/Tailscale.app/Contents/MacOS/Tailscale", "/usr/local/bin/tailscale", "/opt/homebrew/bin/tailscale", "/usr/bin/tailscale", "C:\\Program Files\\Tailscale\\tailscale.exe"],
            Provider::Cloudflare => &["/usr/local/bin/cloudflared", "/opt/homebrew/bin/cloudflared", "/usr/bin/cloudflared", "C:\\Program Files (x86)\\cloudflared\\cloudflared.exe", "C:\\Program Files\\cloudflared\\cloudflared.exe"],
            Provider::Ngrok => &["/usr/local/bin/ngrok", "/opt/homebrew/bin/ngrok", "/usr/bin/ngrok", "/snap/bin/ngrok"],
            Provider::Zrok => &["/usr/local/bin/zrok2", "/opt/homebrew/bin/zrok2", "/usr/bin/zrok2"],
            Provider::Own => &[],
        }
    }
}

/// What the person set up for the tunnel they chose, as the book keeps it
/// (`agents.tunnel`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub provider: Provider,
    /// Cloudflare: the name on their domain the tunnel's DNS route gives; ngrok: the
    /// account's dev domain; zrok: the reserved name; their own proxy: the public
    /// name. Tailscale's is read from Tailscale itself.
    #[serde(default)]
    pub name: String,
    /// Tailscale only: 443, or 8443 when their Funnel already serves something on 443.
    #[serde(default)]
    pub funnel_port: Option<u32>,
    /// Their own proxy only: the `Host` it sends, when not the public name.
    #[serde(default)]
    pub sends_host: Option<String>,
}

/// A name a person typed for a host: lower case, no scheme, no path, no spaces.
pub fn clean_name(s: &str) -> Result<String, String> {
    let t = s.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_ascii_lowercase();
    if t.is_empty() || t.contains(['/', ' ', '@', '?', '#']) || !t.contains('.') {
        return Err(format!("{s:?} is not a host name, such as agents.example.com"));
    }
    Ok(t)
}

/// The command Bagholder starts for `setup`, with fixed arguments: the program's
/// path first. None for the person's own proxy, which is theirs to run.
pub fn command(setup: &Setup, program: &str, port: u16) -> Option<Vec<String>> {
    let target = format!("127.0.0.1:{port}");
    let args: Vec<String> = match setup.provider {
        // in the foreground: `--bg` would keep it serving after Bagholder ends (§4.3a)
        Provider::Tailscale => vec!["funnel".into(), format!("--https={}", setup.funnel_port.unwrap_or(443)), target],
        Provider::Cloudflare => vec!["tunnel".into(), "run".into(), "--url".into(), format!("http://{target}"), "bagholder-agents".into()],
        Provider::Ngrok => vec!["http".into(), format!("--url={}", setup.name), target],
        // zrok v2.0.8: the reserved name in the public namespace, no terminal UI
        Provider::Zrok => vec!["share".into(), "public".into(), target, "--headless".into(), "-n".into(), format!("public:{}", setup.name)],
        Provider::Own => return None,
    };
    Some(std::iter::once(program.to_string()).chain(args).collect())
}

/// The public name the address answers for, as the gate requires it.
pub fn host_of(setup: &Setup, tailscale_name: Option<&str>) -> Result<String, String> {
    let name = match setup.provider {
        Provider::Tailscale => tailscale_name.ok_or("Tailscale has not said this machine's name yet: is it signed in?")?.trim_end_matches('.').to_ascii_lowercase(),
        // zrok's public namespace is under share.zrok.io
        Provider::Zrok => format!("{}.share.zrok.io", clean_name(&format!("{}.x", setup.name))?.trim_end_matches(".x")),
        _ => clean_name(&setup.name)?,
    };
    Ok(match setup.provider {
        Provider::Tailscale if setup.funnel_port == Some(8443) => format!("{name}:8443"),
        _ => name,
    })
}

/// The `Host` the listener receives where it is not the public name: the person's
/// own proxy may send its own (§4.3a). Every tunnel Bagholder starts sends the
/// public name.
pub fn received_host(setup: &Setup) -> Result<Option<String>, String> {
    match setup.provider {
        Provider::Own => setup.sends_host.as_deref().filter(|h| !h.trim().is_empty()).map(clean_name).transpose(),
        _ => Ok(None),
    }
}

/// The program's path: the maker's install location, else on `PATH`.
pub fn find(provider: Provider) -> Option<PathBuf> {
    let program = provider.program()?;
    for p in provider.installed() {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) { format!("{program}.exe") } else { program.to_string() };
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(&exe)).find(|p| p.is_file()))
}

/// What Tailscale states of this machine, by which the panel shows each of its
/// steps done: installed, signed in, its tailnet letting it take HTTPS
/// certificates and use Funnel, and its name on the tailnet.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleSays {
    pub installed: bool,
    pub signed_in: bool,
    pub https: bool,
    pub funnel: bool,
    pub name: Option<String>,
    /// This machine's first address on its tailnet.
    pub address: Option<String>,
    /// Whether another program answers on 443 at that address: Funnel's traffic on
    /// 443 would reach it, not Bagholder, so Funnel serves on 8443.
    pub https_port_taken: bool,
    /// Why Tailscale could not be asked, in its words (its app not running, say).
    pub said: String,
}

/// Tailscale asked through its own program (`tailscale status --json`).
pub fn tailscale_says(program: Option<&std::path::Path>) -> TailscaleSays {
    let Some(program) = program else { return TailscaleSays::default() };
    let installed = TailscaleSays { installed: true, ..TailscaleSays::default() };
    let out = match Command::new(program).args(["status", "--json"]).stdin(Stdio::null()).output() {
        Ok(o) => o,
        Err(e) => return TailscaleSays { said: format!("Tailscale could not be asked: {e}"), ..installed },
    };
    match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
        Ok(v) => read_tailscale_status(&v),
        Err(_) => TailscaleSays { said: format!("Tailscale said: {}", String::from_utf8_lossy(&out.stderr).trim()), ..installed },
    }
}

/// `tailscale status --json`, read: signed in when its backend runs; HTTPS and
/// Funnel by the capabilities its tailnet gave this machine (`Self.Capabilities`,
/// and `Self.CapMap` in later versions).
pub fn read_tailscale_status(v: &serde_json::Value) -> TailscaleSays {
    let me = v.get("Self");
    let listed = me.and_then(|s| s.get("Capabilities")).and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| c.as_str());
    let mapped = me.and_then(|s| s.get("CapMap")).and_then(|c| c.as_object()).into_iter().flat_map(|m| m.keys().map(String::as_str));
    let caps: Vec<&str> = listed.chain(mapped).collect();
    TailscaleSays {
        installed: true,
        signed_in: v.get("BackendState").and_then(|s| s.as_str()) == Some("Running"),
        https: caps.contains(&"https"),
        funnel: caps.contains(&"funnel"),
        name: me.and_then(|s| s.get("DNSName")).and_then(|n| n.as_str()).map(|n| n.trim_end_matches('.').to_ascii_lowercase()).filter(|n| !n.is_empty()),
        address: me.and_then(|s| s.get("TailscaleIPs")).and_then(|a| a.as_array()).into_iter().flatten().filter_map(|a| a.as_str()).find(|a| a.parse::<std::net::IpAddr>().is_ok()).map(str::to_string),
        https_port_taken: false,
        said: String::new(),
    }
}

/// Whether a program answers at `to`: 443 at Tailscale's address for this machine,
/// asked while no tunnel of Bagholder's runs.
pub fn answers(to: SocketAddr) -> bool {
    std::net::TcpStream::connect_timeout(&to, std::time::Duration::from_millis(500)).is_ok()
}

/// How many of a tunnel's last lines are kept to show.
const LINES_KEPT: usize = 20;

/// What the tunnel is doing, as the panel shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct TunnelState {
    /// The provider of the tunnel running, if one is.
    pub running: Option<Provider>,
    /// The command it was started with.
    pub command: Vec<String>,
    /// Its last lines, as it wrote them.
    pub lines: Vec<String>,
    /// How the last one ended, when it ended on its own.
    pub exited: Option<String>,
}

/// The one tunnel child, and what the last one said.
#[derive(Default)]
pub struct Tunnels {
    child: Option<Child>,
    state: Arc<Mutex<TunnelState>>,
}

impl Tunnels {
    pub fn state(&self) -> TunnelState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Start `argv` with `env` (`guarded`: the tunnel through `bagholder
    /// tunnel-guard`, ending with this process). Refused while one runs, naming it.
    pub fn start(&mut self, provider: Provider, (argv, env): Guarded, signal: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        if let Some(running) = self.state.lock().unwrap_or_else(|e| e.into_inner()).running {
            return Err(format!("A tunnel is running already ({running:?}): stop it first."));
        }
        let (program, args) = argv.split_first().ok_or("no command")?;
        let mut cmd = Command::new(program);
        cmd.args(args).envs(env);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("The tunnel could not start: {e}"))?;
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            *s = TunnelState { running: Some(provider), command: argv.clone(), lines: Vec::new(), exited: None };
        }
        let lines: Arc<Mutex<VecDeque<String>>> = Arc::default();
        for out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
            let (state, signal, lines) = (self.state.clone(), signal.clone(), lines.clone());
            std::thread::Builder::new()
                .name("tunnel-output".into())
                .spawn(move || {
                    for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                        let mut l = lines.lock().unwrap_or_else(|e| e.into_inner());
                        l.push_back(line);
                        while l.len() > LINES_KEPT {
                            l.pop_front();
                        }
                        state.lock().unwrap_or_else(|e| e.into_inner()).lines = l.iter().cloned().collect();
                        drop(l);
                        signal();
                    }
                    // the stream closed: the tunnel ended, and the panel reads how
                    signal();
                })
                .map_err(|e| e.to_string())?;
        }
        self.child = Some(child);
        Ok(())
    }

    /// Whether the child ended on its own: said, with its last lines kept.
    pub fn reap(&mut self) {
        let Some(c) = self.child.as_mut() else { return };
        let status = match c.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return,
            Err(e) => {
                // its state cannot be read: said, and asked again at the next look
                self.state.lock().unwrap_or_else(|e| e.into_inner()).exited = Some(format!("The tunnel's state could not be read: {e}"));
                return;
            }
        };
        self.child = None;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.running = None;
        s.exited = Some(format!("The tunnel ended: {status}"));
    }

    /// Stop the tunnel: the guard ends its group.
    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            #[cfg(unix)]
            // SAFETY: a signal to the guard this process started
            unsafe {
                libc::kill(c.id() as libc::pid_t, libc::SIGTERM);
            }
            #[cfg(not(unix))]
            #[expect(clippy::let_underscore_must_use, reason = "the guard may have ended already; the wait below reaps it either way")]
            let _ = c.kill();
            #[expect(clippy::let_underscore_must_use, reason = "reaping the guard; how it ended is read from its output")]
            let _ = c.wait();
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.running = None;
    }
}

/// A command line and the environment it is started with.
pub type Guarded = (Vec<String>, Vec<(String, String)>);

/// `command` as this server starts it: through `bagholder tunnel-guard`, which
/// ends the tunnel when the process `BAGHOLDER_EXIT_WITH` names (this server) ends.
pub fn guarded(command: &[String]) -> Result<Guarded, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let argv = [exe.display().to_string(), "tunnel-guard".into(), "--".into()].into_iter().chain(command.iter().cloned()).collect();
    Ok((argv, vec![(crate::exit_with::VAR.to_string(), std::process::id().to_string())]))
}

// ---- tunnel-guard ----

/// `bagholder tunnel-guard -- {command}`: the tunnel started as this process's
/// child, in a process group of its own, and the group stopped when either side
/// ends (§4.4): when the server named by `BAGHOLDER_EXIT_WITH` ends, when this
/// guard is told to stop, or (Linux, `PR_SET_PDEATHSIG`; Windows, a job object
/// that kills on close) when this guard ends however it ends.
pub fn guard(args: &[String]) -> i32 {
    let command: Vec<&String> = args.iter().skip_while(|a| *a != "--").skip(1).collect();
    let Some((program, rest)) = command.split_first() else {
        eprintln!("usage: bagholder tunnel-guard -- <program> [args]");
        return 2;
    };
    let server = match crate::exit_with::pid() {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!("bagholder tunnel-guard: {} names no process to end with", crate::exit_with::VAR);
            return 2;
        }
        Err(e) => {
            eprintln!("bagholder tunnel-guard: {e}");
            return 2;
        }
    };
    let mut cmd = Command::new(program);
    cmd.args(rest.iter().map(|s| s.as_str()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        // SAFETY: in the child before exec, async-signal-safe calls only. A signal mask
        // is inherited across exec, so the stop signals the guard blocks for itself
        // (below) are let through again: a tunnel that blocked them could not be
        // stopped. On Linux the child is ended with the guard however the guard ends.
        unsafe {
            cmd.pre_exec(|| {
                let mut none: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut none);
                libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
                #[cfg(target_os = "linux")]
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    // the stop signals are taken by a thread of the guard's own (below), never by
    // the default action, which would leave the tunnel behind
    #[cfg(unix)]
    let stops = block_stops();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bagholder tunnel-guard: {program} could not start: {e}");
            return 1;
        }
    };
    let pid = child.id();
    #[cfg(windows)]
    let _job = match windows_job::hold(&child) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("bagholder tunnel-guard: {e}");
            let _ = child.kill();
            return 1;
        }
    };
    if let Err(e) = crate::exit_with::watch(server, move || {
        end_group(pid);
        std::process::exit(0);
    }) {
        eprintln!("bagholder tunnel-guard: {e}");
        end_group(pid);
        return 1;
    }
    #[cfg(unix)]
    {
        let watching = std::thread::Builder::new().name("tunnel-stop".into()).spawn(move || {
            let mut sig: libc::c_int = 0;
            // SAFETY: the set built by block_stops, and one int to write the signal into
            unsafe { libc::sigwait(&stops, &mut sig) };
            end_group(pid);
            std::process::exit(0);
        });
        // a guard that cannot hear a stop would leave the tunnel when told to stop
        if let Err(e) = watching {
            eprintln!("bagholder tunnel-guard: {e}");
            end_group(pid);
            return 1;
        }
    }
    match child.wait() {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// TERM, INT and HUP blocked on this thread (and every thread it starts), to be
/// taken by `sigwait`.
#[cfg(unix)]
fn block_stops() -> libc::sigset_t {
    // SAFETY: a set initialised empty, then filled with the three signals, and set as this thread's mask
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, s);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

/// The tunnel's process group stopped.
fn end_group(pid: u32) {
    #[cfg(unix)]
    // SAFETY: a signal to the group of the child this guard started, whose id leads it
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        // the job's handle closes as the guard exits, and the job's processes end with it
        let _ = pid;
    }
}

#[cfg(windows)]
mod windows_job {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};

    /// A job that ends every process in it when its last handle closes: this guard's.
    pub struct Job(HANDLE);

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle CreateJobObjectW gave, closed once
            unsafe { CloseHandle(self.0) };
        }
    }

    /// `child` held in a job that kills it when this guard ends.
    pub fn hold(child: &std::process::Child) -> Result<Job, String> {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: an unnamed job with default security
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(format!("the tunnel's job could not be made: {}", std::io::Error::last_os_error()));
        }
        let job = Job(job);
        // SAFETY: an all-zero limit structure is valid; one flag set
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the job's handle, the structure and its size
        let set = unsafe { SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &info as *const _ as *const core::ffi::c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) };
        // SAFETY: the job's handle and the child's process handle, both open
        if set == 0 || unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0 {
            return Err(format!("the tunnel could not be tied to its guard: {}", std::io::Error::last_os_error()));
        }
        Ok(job)
    }
}

// ---- the check (§4.4) ----

/// The one-time value the listener answers while a check is open.
pub struct Check {
    pub value: String,
    /// It was asked for, through the listener.
    pub seen: bool,
    /// It came with `Tailscale-Funnel-Request: ?1`, which only the Funnel path sets.
    pub funnel: bool,
}

/// Whether the address answers, as the panel says it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, Diff)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum Ready {
    /// Through the public path: an agent can reach it.
    Ready,
    /// Only from this computer, or the public path could not be taken.
    Unconfirmed { said: String },
    Failed { said: String },
}

/// What the panel says where the public path could not be taken.
pub const UNCONFIRMED: &str = "Reachable from this computer; confirm from your agent.";

/// Whether the address answers through the public path (brief 73, 7): the name
/// resolved through public DNS, the answer a public address, the value fetched
/// from that address with the public name for TLS and `Host`; for Tailscale, the
/// request arriving the Funnel way.
pub fn reach(ctx: &Ctx, provider: Provider) -> Ready {
    let value = secret::secret("bhcheck_");
    *ctx.check.lock().unwrap_or_else(|e| e.into_inner()) = Some(Check { value: value.clone(), seen: false, funnel: false });
    let ready = reach_with(ctx, provider, &value);
    *ctx.check.lock().unwrap_or_else(|e| e.into_inner()) = None;
    ready
}

fn reach_with(ctx: &Ctx, provider: Provider, value: &str) -> Ready {
    let host = ctx.host();
    if host.is_empty() {
        return Ready::Failed { said: "The address is not known yet.".into() };
    }
    let url = format!("https://{host}/agents/check/{value}");
    let (name, port) = match fetch::https_host(&url) {
        Ok(h) => h,
        Err(e) => return Ready::Failed { said: e },
    };
    let fetched = |to: SocketAddr| -> Result<(), String> {
        let r = ctx.outside.get.get(to, &url, &[])?;
        if r.status == 200 && r.body == value.as_bytes() {
            Ok(())
        } else {
            Err(format!("{url} answered {} with what this check did not send", r.status))
        }
    };
    let local = || match ctx.outside.resolve.resolve(&name, port).ok().and_then(|a| a.first().copied()) {
        Some(ip) => match fetched(SocketAddr::new(ip, port)) {
            Ok(()) => Ready::Unconfirmed { said: UNCONFIRMED.into() },
            Err(e) => Ready::Failed { said: e },
        },
        None => Ready::Failed { said: format!("{name} does not resolve from this computer") },
    };
    // the public path: public DNS, a public answer, that address alone
    let answers = match ctx.outside.public_dns.resolve(&name, port) {
        Ok(a) => a,
        Err(_) => return local(),
    };
    let Some(ip) = answers.iter().copied().find(|a| fetch::public(*a)) else { return local() };
    if let Err(e) = fetched(SocketAddr::new(ip, port)) {
        // a machine may not reach its own address the public way (Tailscale's relay
        // does not carry a node's own traffic back to it): where this computer
        // reaches it, it is asked from here, and never Ready
        return match local() {
            unconfirmed @ Ready::Unconfirmed { .. } => unconfirmed,
            _ => Ready::Failed { said: e },
        };
    }
    let (seen, funnel) = ctx.check.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or((false, false), |c| (c.seen, c.funnel));
    if !seen {
        return Ready::Failed { said: format!("{url} answered, but not from this Bagholder") };
    }
    if provider == Provider::Tailscale && !funnel {
        // it went the tailnet way: no Tailscale page says a node reaches its own Funnel publicly
        return Ready::Unconfirmed { said: UNCONFIRMED.into() };
    }
    Ready::Ready
}
