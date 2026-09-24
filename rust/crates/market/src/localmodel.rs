//! A local language model the app runs itself, for filing summaries.
//!
//! Resolution order, cheapest first:
//! 1. An endpoint the user already runs -- `BAGHOLDER_LLM_URL`, or Ollama on
//!    its default port. If one answers, it is used and nothing is downloaded.
//! 2. Otherwise the app provisions its own: it downloads a single
//!    self-contained model executable (a llamafile) into the app's home,
//!    verifies it against a pinned SHA-256, and runs it as a background
//!    server. This happens once, lazily, the first time a summary is asked
//!    for; later starts reuse the downloaded file.
//!
//! Everything is best-effort and answers "" rather than failing. The file is
//! fetched over HTTPS from a pinned Hugging Face URL and refused unless its
//! SHA-256 matches the pin, so a tampered or truncated download is never run.

use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Stand-ins, per thread, for what reaches outside the process: the running
/// endpoint probe, provisioning, the chat request and the wait's sleep.
pub mod hooks {
    use std::cell::RefCell;
    pub type Detect = Box<dyn Fn() -> Option<(String, String)>>;
    pub type Ensure = Box<dyn Fn()>;
    pub type Endpoint = Box<dyn Fn() -> String>;
    pub type Chat = Box<dyn Fn(&str, i64) -> String>;
    /// (url, body) -> response text, or Err for a backend error
    pub type Post = Box<dyn Fn(&str, &str) -> Result<String, String>>;
    pub type Available = Box<dyn Fn() -> bool>;
    pub type Status = Box<dyn Fn() -> &'static str>;
    thread_local! {
        pub static DETECT: RefCell<Option<Detect>> = RefCell::new(None);
        pub static ENSURE: RefCell<Option<Ensure>> = RefCell::new(None);
        pub static ENDPOINT: RefCell<Option<Endpoint>> = RefCell::new(None);
        pub static CHAT: RefCell<Option<Chat>> = RefCell::new(None);
        pub static POST: RefCell<Option<Post>> = RefCell::new(None);
        pub static AVAILABLE: RefCell<Option<Available>> = RefCell::new(None);
        pub static STATUS: RefCell<Option<Status>> = RefCell::new(None);
        pub static NO_SLEEP: RefCell<bool> = RefCell::new(false);
    }
    pub fn clear() {
        DETECT.with(|h| *h.borrow_mut() = None);
        ENSURE.with(|h| *h.borrow_mut() = None);
        ENDPOINT.with(|h| *h.borrow_mut() = None);
        CHAT.with(|h| *h.borrow_mut() = None);
        POST.with(|h| *h.borrow_mut() = None);
        AVAILABLE.with(|h| *h.borrow_mut() = None);
        STATUS.with(|h| *h.borrow_mut() = None);
        NO_SLEEP.with(|h| *h.borrow_mut() = false);
    }
}

/// Back to the state a fresh process starts in (nothing detected, off).
pub fn reset_state() {
    let mut st = state().lock().unwrap();
    st.phase = "off";
    st.detail.clear();
    st.proc = None;
    st.endpoint.clear();
    st.model.clear();
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

pub fn llamafile_url() -> String {
    env("BAGHOLDER_LLAMAFILE_URL", "https://huggingface.co/mozilla-ai/gemma-2-2b-it-llamafile/resolve/main/gemma-2-2b-it.Q4_K_M.llamafile")
}

pub fn llamafile_sha256() -> String {
    env("BAGHOLDER_LLAMAFILE_SHA256", "5eae1115b231c9115b260cc2442263db9e84dec99be8610cfe19fac137284217")
}

fn user_llm_url() -> String {
    env("BAGHOLDER_LLM_URL", "").trim_end_matches('/').to_string()
}

fn ollama_url() -> String {
    env("BAGHOLDER_OLLAMA_URL", "http://127.0.0.1:11434").trim_end_matches('/').to_string()
}

fn ollama_model() -> String {
    env("BAGHOLDER_OLLAMA_MODEL", "llama3.2")
}

pub const MANAGED_HOST: &str = "127.0.0.1";

fn managed_port() -> u16 {
    env("BAGHOLDER_LLM_PORT", "8121").parse().unwrap_or(8121)
}

/// A big file over a slow link.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 30);
/// The server loading the model.
pub const START_TIMEOUT: Duration = Duration::from_secs(120);

fn chat_timeout() -> Duration {
    Duration::from_secs_f64(env("BAGHOLDER_LLM_CHAT_TIMEOUT", "40").parse().unwrap_or(40.0))
}

const ALLOWED_HOSTS: [&str; 3] = ["huggingface.co", "cdn-lfs.huggingface.co", "cdn-lfs-us-1.huggingface.co"];

/// Phases that finish in seconds, unlike a download.
pub const COMING_UP: [&str; 2] = ["detecting", "starting"];

struct State {
    phase: &'static str,
    detail: String,
    proc: Option<Child>,
    endpoint: String,
    model: String,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { phase: "off", detail: String::new(), proc: None, endpoint: String::new(), model: String::new() }))
}

fn home() -> PathBuf {
    match std::env::var("BAGHOLDER_HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".bagholder-rust"),
    }
}

fn llamafile_path() -> PathBuf {
    // a test never makes or fills the person's model folder
    let d = bagholder_store::guard_home(&home()).expect("a test reached the real data folder").join("models");
    let _ = std::fs::create_dir_all(&d);
    d.join("summarizer.llamafile")
}

fn get_ok(url: &str, timeout: Duration) -> bool {
    bagholder_net::client::request("GET", url, &[("User-Agent", "Bagholder")], None, timeout).is_ok()
}

/// A user-run endpoint, if one answers now.
fn detect_running() -> Option<(String, String)> {
    if let Some(r) = hooks::DETECT.with(|h| h.borrow().as_ref().map(|f| f())) {
        return r;
    }
    let user = user_llm_url();
    if !user.is_empty() && get_ok(&format!("{}/v1/models", user), Duration::from_secs(2)) {
        return Some((user, env("BAGHOLDER_LLM_MODEL", "local")));
    }
    let ollama = ollama_url();
    if get_ok(&format!("{}/api/tags", ollama), Duration::from_secs(2)) {
        return Some((ollama, ollama_model()));
    }
    None
}

/// Off, detecting, downloading, starting, ready, failed.
pub fn status() -> &'static str {
    if let Some(r) = hooks::STATUS.with(|h| h.borrow().as_ref().map(|f| f())) {
        return r;
    }
    let st = state().lock().unwrap();
    if !st.endpoint.is_empty() { "ready" } else { st.phase }
}

pub fn available() -> bool {
    if let Some(r) = hooks::AVAILABLE.with(|h| h.borrow().as_ref().map(|f| f())) {
        return r;
    }
    !endpoint().is_empty()
}

/// Whether a model is up now, as already known: asks nothing and starts nothing,
/// so it can be consulted as often as anyone likes.
pub fn is_ready() -> bool {
    if let Some(r) = hooks::AVAILABLE.with(|h| h.borrow().as_ref().map(|f| f())) {
        return r;
    }
    !state().lock().unwrap().endpoint.is_empty()
}

/// Wait at most `seconds` for a model that is coming
/// up right now. A download is never waited for.
pub fn wait_ready(seconds: f64) -> bool {
    endpoint();
    let deadline = Instant::now() + Duration::from_secs_f64(seconds.max(0.0));
    while Instant::now() < deadline {
        if available() {
            return true;
        }
        if !COMING_UP.contains(&status()) {
            return false;
        }
        if !hooks::NO_SLEEP.with(|h| *h.borrow()) {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    available()
}

/// The base URL of a working local model, or "" if none
/// is up yet. Never blocks on a download.
pub fn endpoint() -> String {
    if let Some(r) = hooks::ENDPOINT.with(|h| h.borrow().as_ref().map(|f| f())) {
        return r;
    }
    {
        let st = state().lock().unwrap();
        if !st.endpoint.is_empty() {
            return st.endpoint.clone();
        }
    }
    if let Some((url, model)) = detect_running() {
        let mut st = state().lock().unwrap();
        st.endpoint = url.clone();
        st.model = model;
        st.phase = "ready";
        drop(st);
        changed();
        return url;
    }
    ensure();
    String::new()
}

/// Start provisioning if it is not already under way.
pub fn ensure() {
    if hooks::ENSURE.with(|h| h.borrow().as_ref().map(|f| f()).is_some()) {
        return;
    }
    {
        let mut st = state().lock().unwrap();
        if ["detecting", "downloading", "starting"].contains(&st.phase) || !st.endpoint.is_empty() {
            return;
        }
        st.phase = "detecting";
    }
    let _ = std::thread::Builder::new().name("bagholder-localmodel".into()).spawn(provision);
}

static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Called whenever the model's phase changes (coming up, ready, failed): how whoever
/// waits for a model learns of it without asking again and again. Set once, at start.
pub fn on_change(f: impl Fn() + Send + Sync + 'static) {
    let _ = ON_CHANGE.set(Box::new(f));
}

fn changed() {
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

fn set(phase: &'static str, detail: &str) {
    {
        let mut st = state().lock().unwrap();
        st.phase = phase;
        st.detail = detail.to_string();
    }
    changed();
}

fn provision() {
    if let Some((url, model)) = detect_running() {
        let mut st = state().lock().unwrap();
        st.endpoint = url;
        st.model = model;
        st.phase = "ready";
        drop(st);
        changed();
        return;
    }
    let path = llamafile_path();
    if !verified(&path) {
        set("downloading", "");
        if !download(&path) {
            set("failed", "download failed");
            return;
        }
    }
    if !verified(&path) {
        set("failed", "checksum mismatch");
        let _ = std::fs::remove_file(&path);
        return;
    }
    set("starting", "");
    if spawn(&path) && wait_started() {
        let mut st = state().lock().unwrap();
        st.endpoint = format!("http://{}:{}", MANAGED_HOST, managed_port());
        st.model = "local".into();
        st.phase = "ready";
        drop(st);
        changed();
    } else {
        set("failed", "server did not start");
    }
}

pub fn verified(path: &PathBuf) -> bool {
    let pin = llamafile_sha256();
    if !path.exists() || pin.is_empty() {
        return false;
    }
    let mut f = match std::fs::File::open(path) { Ok(f) => f, Err(_) => return false };
    let mut h = openssl::sha::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    let hex: String = h.finish().iter().map(|b| format!("{:02x}", b)).collect();
    hex == pin.to_lowercase()
}

pub fn download(path: &PathBuf) -> bool {
    let url = llamafile_url();
    let host = url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").split(':').next().unwrap_or("").to_string();
    if !ALLOWED_HOSTS.contains(&host.as_str()) {
        return false;
    }
    let tmp = path.with_extension("part");
    let got = bagholder_net::client::request("GET", &url, &[("User-Agent", "Bagholder")], None, DOWNLOAD_TIMEOUT)
        .map_err(|e| e.to_string())
        .and_then(|r| std::fs::write(&tmp, &r.body).map_err(|e| e.to_string()))
        .and_then(|_| std::fs::rename(&tmp, path).map_err(|e| e.to_string()));
    if got.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perm = meta.permissions();
            perm.set_mode(perm.mode() | 0o110);
            let _ = std::fs::set_permissions(path, perm);
        }
    }
    // a downloaded executable is quarantined on macOS; clear it so it can run
    if cfg!(target_os = "macos") {
        let _ = Command::new("xattr").args(["-d", "com.apple.quarantine"]).arg(path).output();
    }
    true
}

fn spawn(path: &PathBuf) -> bool {
    let port = managed_port().to_string();
    let direct = Command::new(path)
        .args(["--server", "--nobrowser", "--host", MANAGED_HOST, "--port", &port, "-ngl", "0", "--log-disable"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let child = match direct {
        Ok(c) => c,
        // some hosts must run the APE via a shell
        Err(_) => match Command::new("sh")
            .arg(path)
            .args(["--server", "--nobrowser", "--host", MANAGED_HOST, "--port", &port, "--log-disable"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        },
    };
    state().lock().unwrap().proc = Some(child);
    true
}

fn wait_started() -> bool {
    let base = format!("http://{}:{}", MANAGED_HOST, managed_port());
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        {
            let mut st = state().lock().unwrap();
            if let Some(p) = st.proc.as_mut() {
                if let Ok(Some(_)) = p.try_wait() {
                    return false;
                }
            }
        }
        if get_ok(&format!("{}/health", base), Duration::from_secs(2)) || get_ok(&format!("{}/v1/models", base), Duration::from_secs(2)) {
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    false
}

/// Stop the model server this app started.
pub fn shutdown() {
    let proc = state().lock().unwrap().proc.take();
    if let Some(mut p) = proc {
        if let Ok(None) = p.try_wait() {
            let _ = p.kill();
            let _ = p.wait();
        }
    }
}

/// One completion from the local model over the
/// OpenAI-compatible API both Ollama and llamafile speak, or "".
pub fn chat(prompt: &str, max_tokens: i64) -> String {
    if let Some(r) = hooks::CHAT.with(|h| h.borrow().as_ref().map(|f| f(prompt, max_tokens))) {
        return r;
    }
    let base = endpoint();
    if base.is_empty() {
        return String::new();
    }
    let model = { let m = state().lock().unwrap().model.clone(); if m.is_empty() { "local".to_string() } else { m } };
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.1,
        "max_tokens": max_tokens,
        // a model that ends its turn with a marker of its own: cut there, so no
        // marker reaches a title, which has no sentence for the trim to find
        "stop": ["<end_of_turn>", "<|eot_id|>", "</s>"],
        "stream": false,
    });
    let text = serde_json::to_string(&body).unwrap_or_default();
    let url = format!("{}/v1/chat/completions", base);
    let got = match hooks::POST.with(|h| h.borrow().as_ref().map(|f| f(&url, &text))) {
        Some(r) => r,
        None => bagholder_net::client::request("POST", &url, &[("Content-Type", "application/json")], Some(text.as_bytes()), chat_timeout())
            .map(|r| r.text())
            .map_err(|e| e.to_string()),
    };
    let body = match got {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    let v: Value = match serde_json::from_str(&body) { Ok(v) => v, Err(_) => return String::new() };
    let content = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"));
    match content {
        Some(Value::String(s)) => bagholder_model::textrules::trim_space(s).to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => bagholder_model::textrules::trim_space(&bagholder_model::value::s(Some(other))).to_string(),
    }
}
