//! Connecting Wealthsimple: a Chromium window the app opens on Wealthsimple's
//! own login page, watched over the DevTools protocol until the session's
//! cookies appear. In a container the window lives on a virtual display and
//! is streamed into the page, which forwards clicks and keys back to it.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use std::sync::Arc;

use crate::app::{f, log, spawn, App};

pub const CAPTURE_WAIT: Duration = Duration::from_secs(180);
pub const DEBUG_PORT: u16 = 18765;
pub const OAUTH_COOKIE: &str = "_oauth2_access_v2";
pub const DEVICE_COOKIE: &str = "wssdi";
pub const LOGIN_URL: &str = "https://my.wealthsimple.com/app/login";
/// Each DevTools call while capturing: short, so a closed window is noticed.
pub const CAPTURE_CALL: Duration = Duration::from_secs(2);
pub const WINDOW_CHECK: Duration = Duration::from_millis(500);
pub const CAPTURE_EVERY: Duration = Duration::from_millis(1500);
pub const LOGIN_VIEW_SIZE: (u32, u32) = (960, 1000);
const NO_BROWSER: &str = "Install Chrome, Brave, Edge, or another Chromium browser. Passkey login has to happen on Wealthsimple’s site.";

pub fn login_view() -> bool {
    !std::env::var("BAGHOLDER_LOGIN_VIEW").unwrap_or_default().trim().is_empty()
}

fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

fn is_file(p: &str) -> bool {
    !p.is_empty() && std::path::Path::new(p).is_file()
}

/// A Chromium-family browser capable of the DevTools
/// login flow.
pub fn find_chrome() -> String {
    let explicit = std::env::var("BAGHOLDER_CHROME").unwrap_or_default().trim().to_string();
    if is_file(&explicit) {
        return explicit;
    }
    let mac = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    if cfg!(target_os = "macos") {
        for p in mac {
            if is_file(p) {
                return p.into();
            }
        }
    }
    for n in ["google-chrome", "google-chrome-stable", "brave-browser", "brave-browser-stable", "brave", "chromium", "chromium-browser", "microsoft-edge", "msedge", "chrome"] {
        if let Some(p) = which(n) {
            return p;
        }
    }
    let pf = std::env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".into());
    let pf86 = std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let join = |a: &str, parts: &[&str]| -> String {
        let mut p = std::path::PathBuf::from(a);
        for x in parts {
            p.push(x);
        }
        p.to_string_lossy().into_owned()
    };
    let mut extras: Vec<String> = mac.iter().map(|s| s.to_string()).collect();
    for base in [&pf, &pf86, &local] {
        extras.push(join(base, &["Google", "Chrome", "Application", "chrome.exe"]));
    }
    for base in [&pf, &pf86, &local] {
        extras.push(join(base, &["BraveSoftware", "Brave-Browser", "Application", "brave.exe"]));
    }
    for base in [&pf, &pf86] {
        extras.push(join(base, &["Microsoft", "Edge", "Application", "msedge.exe"]));
    }
    extras.into_iter().find(|p| is_file(p)).unwrap_or_default()
}

fn http_get_local(port: u16, path: &str, timeout: Duration) -> Option<String> {
    let url = format!("http://127.0.0.1:{}{}", port, path);
    bagholder_market::client::request("GET", &url, &[], None, timeout).ok().map(|r| r.text())
}

/// The DevTools targets.
fn cdp_list(port: u16, timeout: Duration) -> Vec<Value> {
    for path in ["/json/list", "/json"] {
        if let Some(raw) = http_get_local(port, path, timeout) {
            if raw.is_empty() {
                continue;
            }
            if let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&raw) {
                return a;
            }
        }
    }
    vec![]
}

/// The login Chrome's open windows and tabs.
fn cdp_pages(port: u16) -> Vec<Value> {
    cdp_list(port, WINDOW_CHECK).into_iter().filter(|t| t.is_object() && f(t, "type") == "page" && !f(t, "id").is_empty()).collect()
}

// --- a WebSocket client, enough for DevTools --------------------------------------

pub struct Ws {
    sock: TcpStream,
    buf: Vec<u8>,
    next_id: i64,
    fragments: Option<(u8, Vec<u8>)>,
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1);
        for (i, x) in b.iter_mut().enumerate() {
            *x = (seed >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(31);
        }
    }
    b
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = ((c[0] as u32) << 16) | ((*c.get(1).unwrap_or(&0) as u32) << 8) | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn unb64(text: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a' + 26) as u32),
        b'0'..=b'9' => Some((c - b'0' + 52) as u32),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let bytes: Vec<u32> = text.bytes().filter_map(val).collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for c in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, v) in c.iter().enumerate() {
            n |= v << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if c.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

impl Ws {
    /// Always to 127.0.0.1, the Origin DevTools allows.
    pub fn connect(ws_url: &str, timeout: Duration) -> std::io::Result<Ws> {
        let rest = ws_url.split("://").nth(1).unwrap_or(ws_url);
        let (hostport, path) = match rest.find('/') { Some(i) => (&rest[..i], &rest[i..]), None => (rest, "/") };
        let port: u16 = hostport.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(80);
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().map_err(std::io::Error::other)?;
        let mut sock = TcpStream::connect_timeout(&addr, timeout)?;
        sock.set_read_timeout(Some(timeout))?;
        sock.set_write_timeout(Some(timeout))?;
        let key = b64(&random_bytes(16));
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\nOrigin: http://127.0.0.1\r\n\r\n",
            path, port, key
        );
        sock.write_all(req.as_bytes())?;
        let deadline = Instant::now() + timeout;
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            if Instant::now() > deadline {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "ws handshake timeout"));
            }
            let n = sock.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::other("ws handshake closed"));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let status = String::from_utf8_lossy(&buf[..buf[..end].iter().position(|c| *c == b'\r').unwrap_or(end)]).into_owned();
        if !status.contains("101") {
            return Err(std::io::Error::other(format!("ws handshake failed: {}", status)));
        }
        Ok(Ws { sock, buf: buf[end..].to_vec(), next_id: 1, fragments: None })
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
        let mask = random_bytes(4);
        let mut out: Vec<u8> = vec![0x80 | (opcode & 0x0f)];
        let n = payload.len();
        if n < 126 {
            out.push(0x80 | n as u8);
        } else if n < 65536 {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.sock.write_all(&out)
    }

    pub fn send_text(&mut self, text: &str) -> std::io::Result<()> {
        self.send_frame(0x1, text.as_bytes())
    }

    /// Reads more bytes into the buffer; false when the deadline passed first.
    fn fill(&mut self, deadline: Instant) -> std::io::Result<bool> {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            return Ok(false);
        }
        self.sock.set_read_timeout(Some(remain.max(Duration::from_millis(50))))?;
        let mut chunk = [0u8; 65536];
        match self.sock.read(&mut chunk) {
            Ok(0) => Err(std::io::Error::other("ws closed")),
            Ok(got) => {
                self.buf.extend_from_slice(&chunk[..got]);
                Ok(true)
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// One whole frame taken off the buffer once all of it has arrived: a read
    /// that runs out of time leaves a partly received frame where it is, so the
    /// next read carries on from its first byte.
    fn take_frame(&mut self) -> Option<(bool, u8, Vec<u8>)> {
        let b = &self.buf;
        if b.len() < 2 {
            return None;
        }
        let fin = b[0] & 0x80 != 0;
        let opcode = b[0] & 0x0f;
        let masked = b[1] & 0x80 != 0;
        let mut at = 2usize;
        let mut len = (b[1] & 0x7f) as usize;
        if len == 126 {
            if b.len() < at + 2 {
                return None;
            }
            len = u16::from_be_bytes([b[2], b[3]]) as usize;
            at += 2;
        } else if len == 127 {
            if b.len() < at + 8 {
                return None;
            }
            len = u64::from_be_bytes(b[2..10].try_into().unwrap()) as usize;
            at += 8;
        }
        let mask = if masked {
            if b.len() < at + 4 {
                return None;
            }
            let m = [b[at], b[at + 1], b[at + 2], b[at + 3]];
            at += 4;
            Some(m)
        } else {
            None
        };
        if b.len() < at + len {
            return None;
        }
        let mut payload = b[at..at + len].to_vec();
        if let Some(m) = mask {
            for (i, x) in payload.iter_mut().enumerate() {
                *x ^= m[i % 4];
            }
        }
        self.buf.drain(..at + len);
        Some((fin, opcode, payload))
    }

    /// One whole message: (opcode, payload); pings answered, pongs passed by.
    /// A timeout loses nothing: what arrived so far stays buffered.
    pub fn recv_message(&mut self, timeout: Duration) -> std::io::Result<(u8, Vec<u8>)> {
        let deadline = Instant::now() + timeout;
        loop {
            let (fin, opcode, payload) = loop {
                if let Some(f) = self.take_frame() {
                    break f;
                }
                if !self.fill(deadline)? {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "ws read timeout"));
                }
            };
            match opcode {
                0x8 => return Err(std::io::Error::other("ws closed")),
                0x9 => self.send_frame(0xA, &payload)?,
                0xA => {}
                0x1 | 0x2 => {
                    if fin {
                        return Ok((opcode, payload));
                    }
                    self.fragments = Some((opcode, payload));
                }
                0x0 => {
                    if let Some((op, mut acc)) = self.fragments.take() {
                        acc.extend(payload);
                        if fin {
                            return Ok((op, acc));
                        }
                        self.fragments = Some((op, acc));
                    }
                }
                _ => {}
            }
        }
    }

    /// One method call, its answer by id.
    pub fn call(&mut self, method: &str, params: Option<Value>, timeout: Duration) -> Option<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut payload = json!({"id": id, "method": method});
        if let Some(p) = params {
            payload["params"] = p;
        }
        self.send_text(&payload.to_string()).ok()?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let remain = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(200));
            let (op, data) = match self.recv_message(remain) { Ok(m) => m, Err(_) => continue };
            if op != 0x1 && op != 0x2 {
                continue;
            }
            if let Ok(msg) = serde_json::from_slice::<Value>(&data) {
                if msg.get("id").and_then(|v| v.as_i64()) == Some(id) {
                    return Some(msg);
                }
            }
        }
        None
    }

    /// Send a call without waiting for its answer.
    /// Send a call without waiting for its answer; false only when the socket is gone.
    pub fn fire(&mut self, method: &str, params: Value) -> bool {
        let id = self.next_id;
        self.next_id += 1;
        self.send_text(&json!({"id": id, "method": method, "params": params}).to_string()).is_ok()
    }

    pub fn close(mut self) {
        let _ = self.send_frame(0x8, &[]);
    }
}

// --- the session in the cookies -----------------------------------------------------

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A cookie value that is, or URL-decodes
/// to, a JSON object carrying an access token.
pub fn json_with_access_token(raw: &str) -> Option<Value> {
    let mut cur = raw.trim().to_string();
    if cur.is_empty() {
        return None;
    }
    for _ in 0..3 {
        if cur.contains("access_token") {
            if let Ok(obj) = serde_json::from_str::<Value>(&cur) {
                if obj.is_object() && crate::app::truthy(obj.get("access_token")) {
                    return Some(obj);
                }
            }
        }
        let next = unquote(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    None
}

fn cookies_from_document_cookie(text: &str) -> Vec<Value> {
    text.split(';')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty() && p.contains('='))
        .map(|p| {
            let (n, v) = p.split_once('=').unwrap();
            json!({"name": n.trim(), "value": v})
        })
        .collect()
}

pub fn tokens_from_cookie_list(_app: &Arc<App>, cookies: &[Value]) -> Option<Value> {
    let mut oauth: Option<Value> = None;
    let mut wssdi = String::new();
    for c in cookies {
        if !c.is_object() {
            continue;
        }
        let (name, value) = (f(c, "name"), f(c, "value"));
        if name == DEVICE_COOKIE && !value.is_empty() {
            wssdi = value.clone();
        }
        if let Some(parsed) = json_with_access_token(&value) {
            if name == OAUTH_COOKIE || oauth.is_none() {
                oauth = Some(parsed);
            }
        }
    }
    let oauth = oauth.filter(|o| crate::app::truthy(o.get("access_token")))?;
    let mut body = serde_json::Map::new();
    for k in ["access_token", "refresh_token", "identity_canonical_id", "client_id", "session_id"] {
        if crate::app::truthy(oauth.get(k)) {
            body.insert(k.into(), oauth[k].clone());
        }
    }
    let ident = {
        let ids: bagholder_ws::session::IdentityKeys = serde_json::from_value(oauth.clone()).unwrap_or_default();
        ids.identity()
    };
    if !ident.is_empty() {
        body.insert("identity_canonical_id".into(), json!(ident));
    }
    if let Some(e) = oauth.get("expires_at").filter(|v| !v.is_null()) {
        body.insert("expires_at".into(), e.clone());
    }
    if !wssdi.is_empty() {
        body.insert("wssdi".into(), json!(wssdi));
    }
    Some(Value::Object(body))
}

fn cookie_list(msg: Option<Value>) -> Vec<Value> {
    msg.and_then(|m| m.get("result").filter(|r| r.is_object()).and_then(|r| r.get("cookies")).and_then(|c| c.as_array()).cloned()).unwrap_or_default()
}

fn cookies_from_target(app: &Arc<App>, ws_url: &str) -> Option<Value> {
    let mut ws = Ws::connect(ws_url, CAPTURE_CALL).ok()?;
    let with_ua = |mut body: Value, ua: &str| {
        if !ua.is_empty() {
            body["user_agent"] = json!(ua);
        }
        body
    };
    let ua = ws.call("Browser.getVersion", None, CAPTURE_CALL).and_then(|v| v.get("result").map(|r| f(r, "userAgent").trim().to_string())).unwrap_or_default();
    if !ua.is_empty() {
        app.ws_home().save_user_agent(&ua);
    }
    ws.call("Network.enable", None, CAPTURE_CALL);
    let mut cookies = cookie_list(ws.call("Network.getAllCookies", None, CAPTURE_CALL));
    if let Some(b) = tokens_from_cookie_list(app, &cookies) {
        ws.close();
        return Some(with_ua(b, &ua));
    }
    let extra = cookie_list(ws.call("Storage.getCookies", None, CAPTURE_CALL));
    cookies.extend(extra);
    if let Some(b) = tokens_from_cookie_list(app, &cookies) {
        ws.close();
        return Some(with_ua(b, &ua));
    }
    let ev = ws.call("Runtime.evaluate", Some(json!({"expression": "document.cookie", "returnByValue": true})), CAPTURE_CALL);
    let val = ev.and_then(|e| e.get("result").and_then(|r| r.get("result")).map(|r| f(r, "value"))).unwrap_or_default();
    ws.close();
    tokens_from_cookie_list(app, &cookies_from_document_cookie(&val)).map(|b| with_ua(b, &ua))
}

fn try_capture(app: &Arc<App>, port: u16) -> Option<Value> {
    let targets = cdp_list(port, Duration::from_secs(1));
    let (pages, others): (Vec<Value>, Vec<Value>) = targets.into_iter().filter(|t| !f(t, "webSocketDebuggerUrl").is_empty()).partition(|t| f(t, "type") == "page");
    for t in pages.into_iter().chain(others) {
        if let Some(body) = cookies_from_target(app, &f(&t, "webSocketDebuggerUrl")) {
            if crate::app::truthy(body.get("access_token")) {
                return Some(body);
            }
        }
    }
    None
}

fn attempt_is(app: &Arc<App>, attempt: i64) -> bool {
    app.state.lock().unwrap().login_attempt == attempt
}

fn capturing(app: &Arc<App>) -> bool {
    app.state.lock().unwrap().capturing
}

fn capture_loop(app: &Arc<App>, pid: u32, attempt: i64) {
    let mut refused: Option<String> = None;
    while attempt_is(app, attempt) {
        if !capturing(app) {
            return;
        }
        let body = if !cdp_pages(DEBUG_PORT).is_empty() { try_capture(app, DEBUG_PORT) } else { None };
        if let Some(b) = body {
            let rt = f(&b, "refresh_token");
            if attempt_is(app, attempt) && Some(rt.clone()) != refused {
                if !capturing(app) {
                    return;
                }
                if crate::session::capture_tokens(app, &b).get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                    log("bagholder captured Wealthsimple session");
                    close_login_browser(app, Some(pid));
                    return;
                }
                refused = Some(rt);
                log("bagholder login: Wealthsimple refused the captured session on refresh; still watching the window");
            }
        }
        std::thread::sleep(CAPTURE_EVERY);
    }
}

fn proc_alive(app: &Arc<App>, pid: u32) -> bool {
    let mut st = app.state.lock().unwrap();
    if st.chrome_pid != pid {
        return false;
    }
    match st.chrome_proc.as_mut() {
        Some(c) => matches!(c.try_wait(), Ok(None)),
        None => false,
    }
}

fn poll_session(app: &Arc<App>, pid: u32, attempt: i64) {
    let deadline = Instant::now() + CAPTURE_WAIT;
    let start = Instant::now();
    let mut seen_page = false;
    app.spawn_with("bagholder-cdp-capture", move |app| capture_loop(&app, pid, attempt));
    while Instant::now() < deadline {
        if !attempt_is(app, attempt) || !capturing(app) {
            return;
        }
        let alive = proc_alive(app, pid);
        let pages = if alive { cdp_pages(DEBUG_PORT) } else { vec![] };
        seen_page = seen_page || !pages.is_empty();
        let gone = !alive || (pages.is_empty() && (seen_page || start.elapsed() > Duration::from_secs(10)));
        if gone {
            if !attempt_is(app, attempt) {
                return;
            }
            {
                let mut st = app.state.lock().unwrap();
                if st.capturing {
                    st.error = "The Chrome window closed before a session showed up. Choose Connect Wealthsimple to try again.".into();
                    st.capturing = false;
                }
            }
            log("bagholder login: window closed, waiting stopped");
            close_login_browser(app, Some(pid));
            return;
        }
        std::thread::sleep(WINDOW_CHECK);
    }
    if !attempt_is(app, attempt) {
        return;
    }
    {
        let mut st = app.state.lock().unwrap();
        if st.capturing {
            st.error = "No session yet. Finish login in the Chrome window, then wait a few seconds.".into();
            st.capturing = false;
        }
    }
    close_login_browser(app, Some(pid));
}

fn browser_ws() -> Option<String> {
    let raw = http_get_local(DEBUG_PORT, "/json/version", Duration::from_secs(2))?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let u = f(&v, "webSocketDebuggerUrl");
    if u.is_empty() { None } else { Some(u) }
}

fn wait_child(child: &mut Child, d: Duration) -> bool {
    let until = Instant::now() + d;
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Gracefully through DevTools, then by
/// ending the process. Only ever the app's own instance.
pub fn close_login_browser(app: &Arc<App>, only: Option<u32>) {
    let (mut child, current) = {
        let mut st = app.state.lock().unwrap();
        match only {
            Some(pid) if st.chrome_pid != pid => (None, false),
            _ => {
                st.chrome_pid = 0;
                (st.chrome_proc.take(), true)
            }
        }
    };
    if !current {
        return;
    }
    let mut child = match child.take() { Some(c) => c, None => return };
    if let Some(u) = browser_ws() {
        if let Ok(mut ws) = Ws::connect(&u, Duration::from_secs(5)) {
            ws.call("Browser.close", None, Duration::from_secs(8));
            ws.close();
        }
    }
    if !wait_child(&mut child, Duration::from_secs(5)) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn browser_alive(app: &Arc<App>) -> bool {
    let alive = {
        let mut st = app.state.lock().unwrap();
        match st.chrome_proc.as_mut() { Some(c) => matches!(c.try_wait(), Ok(None)), None => false }
    };
    alive && browser_ws().is_some() && !cdp_pages(DEBUG_PORT).is_empty()
}

// --- the streamed window ----------------------------------------------------------

#[derive(Default)]
struct View {
    ws: Option<Ws>,
    target: String,
}

#[derive(Default)]
struct Cast {
    frame: Option<Vec<u8>>,
    seq: u64,
    at: Option<Instant>,
}

/// The streamed Wealthsimple login window: the socket to it, and the frames cast
/// from it for whoever is watching (`login_stream`).
#[derive(Default)]
pub struct LoginState {
    view: Mutex<View>,
    cast: (Mutex<Cast>, Condvar),
}

fn with_view<T>(app: &App, f_: impl FnOnce(&mut Ws) -> Option<T>) -> Option<T> {
    let pages = cdp_pages(DEBUG_PORT);
    let mut v = app.login.view.lock().unwrap();
    let page = match pages.first() {
        Some(p) => p.clone(),
        None => {
            v.ws = None;
            v.target.clear();
            return None;
        }
    };
    if v.ws.is_none() || v.target != f(&page, "id") {
        v.ws = Ws::connect(&f(&page, "webSocketDebuggerUrl"), CAPTURE_CALL).ok();
        v.target = f(&page, "id");
    }
    let ws = v.ws.as_mut()?;
    let out = f_(ws);
    if out.is_none() {
        v.ws = None;
        v.target.clear();
    }
    out
}

/// The login window as a JPEG.
pub fn login_frame(app: &Arc<App>) -> Option<Vec<u8>> {
    if !capturing(app) {
        return None;
    }
    with_view(app, |ws| {
        let r = ws.call("Page.captureScreenshot", Some(json!({"format": "jpeg", "quality": 60})), CAPTURE_CALL)?;
        let data = r.get("result").map(|x| f(x, "data")).unwrap_or_default();
        if data.is_empty() { None } else { Some(unb64(&data)) }
    })
}

fn screencast_params() -> Value {
    json!({"format": "jpeg", "quality": 60, "maxWidth": LOGIN_VIEW_SIZE.0, "maxHeight": LOGIN_VIEW_SIZE.1, "everyNthFrame": 1})
}

/// A passkey sign-in refused the moment it is asked for, as Cancel in the
/// browser's dialog would; the page's autofill request and everything else
/// pass through untouched.
const NO_PASSKEYS: &str = r#"(() => {
  const c = navigator.credentials;
  if (!c || c.__bagholderNoPasskeys) return;
  const get = c.get.bind(c);
  c.get = (o) => (o && o.publicKey && o.mediation !== "conditional"
    ? Promise.reject(new DOMException("The operation either timed out or was not allowed.", "NotAllowedError"))
    : get(o));
  Object.defineProperty(c, "__bagholderNoPasskeys", { value: true });
})();"#;

fn publish_frame(app: &App, frame: Vec<u8>) {
    let (m, c) = &app.login.cast;
    let mut g = m.lock().unwrap();
    g.frame = Some(frame);
    g.seq += 1;
    g.at = Some(Instant::now());
    c.notify_all();
}

/// The screencast sends a frame only when the page changes, and in a container
/// it can send none at all: whenever no frame has come for a moment, a
/// screenshot taken over a socket of its own stands in for one.
fn screenshot_loop(app: &Arc<App>, attempt: i64) {
    let mut ws: Option<Ws> = None;
    while attempt_is(app, attempt) && capturing(app) {
        std::thread::sleep(Duration::from_millis(250));
        let stale = { let (m, _) = &app.login.cast; m.lock().unwrap().at.map(|t| t.elapsed() > Duration::from_millis(700)).unwrap_or(true) };
        if !stale {
            continue;
        }
        if ws.is_none() {
            ws = cdp_pages(DEBUG_PORT).first().and_then(|p| Ws::connect(&f(p, "webSocketDebuggerUrl"), CAPTURE_CALL).ok());
        }
        let shot = ws.as_mut().and_then(|w| w.call("Page.captureScreenshot", Some(json!({"format": "jpeg", "quality": 60})), CAPTURE_CALL));
        match shot.as_ref().and_then(|r| r.pointer("/result/data")).and_then(|d| d.as_str()) {
            Some(data) => {
                let frame = unb64(data);
                if !frame.is_empty() {
                    publish_frame(app, frame);
                }
            }
            None => ws = None,
        }
    }
}

fn screencast_loop(app: &Arc<App>, attempt: i64) {
    while attempt_is(app, attempt) {
        if !capturing(app) {
            return;
        }
        let pages = cdp_pages(DEBUG_PORT);
        let page = match pages.first() { Some(p) => p.clone(), None => { std::thread::sleep(Duration::from_millis(500)); continue } };
        let mut ws = match Ws::connect(&f(&page, "webSocketDebuggerUrl"), CAPTURE_CALL) { Ok(w) => w, Err(_) => { std::thread::sleep(Duration::from_millis(500)); continue } };
        // A passkey sign-in opens Chromium's own passkey dialog, which is drawn
        // outside the page: the stream never shows it and the page's clicks
        // never reach it, so the page would wait on it for good. In every
        // document the tab shows while this socket is open, such a request is
        // refused at once instead, and the page offers another way to sign in.
        ws.call("Page.enable", None, CAPTURE_CALL);
        ws.call("Page.addScriptToEvaluateOnNewDocument", Some(json!({"source": NO_PASSKEYS, "runImmediately": true})), CAPTURE_CALL);
        ws.call("Runtime.evaluate", Some(json!({"expression": NO_PASSKEYS})), CAPTURE_CALL);
        ws.call("Page.startScreencast", Some(screencast_params()), CAPTURE_CALL);
        loop {
            if !attempt_is(app, attempt) || !capturing(app) {
                ws.close();
                return;
            }
            let (op, data) = match ws.recv_message(Duration::from_secs(2)) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => break,
            };
            if op != 0x1 && op != 0x2 {
                continue;
            }
            let msg: Value = match serde_json::from_slice(&data) { Ok(m) => m, Err(_) => break };
            if f(&msg, "method") != "Page.screencastFrame" {
                continue;
            }
            let p = msg.get("params").cloned().unwrap_or(json!({}));
            let frame = unb64(&f(&p, "data"));
            if !frame.is_empty() {
                publish_frame(app, frame);
            }
            // acknowledged without waiting for the answer
            ws.fire("Page.screencastFrameAck", json!({"sessionId": p.get("sessionId").cloned().unwrap_or(Value::Null)}));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The window as a multipart JPEG stream, each frame
/// as Chromium pushes it.
pub fn login_stream<W: FnMut(&[u8]) -> bool>(app: &Arc<App>, mut write: W) {
    // The latest frame goes out at once, then every new one, and the latest
    // again after a second without one: a page that is redrawn opens a new
    // stream and drops the old one, and only a write finds out that the reader
    // is gone. Without it every dropped stream stays open, and the browser,
    // which keeps six connections to a host, stops loading anything from here.
    // Only the newest reader is served: the page opens a new stream on every
    // redraw and the image it replaced may keep reading, so each older stream
    // is ended here and its connection freed.
    static READER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let me = READER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    app.login.cast.1.notify_all();
    let mut last = u64::MAX;
    let mut last_sent: Option<()> = None;
    loop {
        if !capturing(app) || READER.load(std::sync::atomic::Ordering::SeqCst) != me {
            return;
        }
        let frame = {
            let (m, c) = &app.login.cast;
            let mut g = m.lock().unwrap();
            if g.seq == last {
                g = c.wait_timeout(g, Duration::from_secs(1)).unwrap().0;
            }
            if READER.load(std::sync::atomic::Ordering::SeqCst) != me {
                return;
            }
            match g.frame.clone() {
                Some(f) => {
                    last = g.seq;
                    f
                }
                None => continue,
            }
        };
        // each part is closed by the next boundary as soon as it is sent: a
        // browser shows a part only once that boundary has arrived, so a part
        // left open would stay blank until the page next changed
        let mut chunk = Vec::new();
        if last_sent.is_none() {
            chunk.extend_from_slice(b"--frame\r\n");
        }
        chunk.extend_from_slice(format!("Content-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", frame.len()).as_bytes());
        chunk.extend_from_slice(&frame);
        chunk.extend_from_slice(b"\r\n--frame\r\n");
        last_sent = Some(());
        if !write(&chunk) {
            return;
        }
    }
}

const VIEW_KEYS: [(&str, i64); 11] = [("Enter", 13), ("Tab", 9), ("Backspace", 8), ("Delete", 46), ("Escape", 27), ("ArrowLeft", 37), ("ArrowUp", 38), ("ArrowRight", 39), ("ArrowDown", 40), ("Home", 36), ("End", 35)];

fn key_event(ch: char, typ: &str) -> Value {
    let up = ch.to_uppercase().collect::<String>();
    let (code, vk) = if bagholder_model::unichars::is_digit(ch) {
        (format!("Digit{}", ch), ch as i64)
    } else if ch.is_ascii() && up.len() == 1 && ("A"..="Z").contains(&up.as_str()) {
        (format!("Key{}", up), up.chars().next().unwrap() as i64)
    } else if ch == ' ' {
        ("Space".to_string(), 32)
    } else {
        (String::new(), 0)
    };
    let mut ev = json!({"key": ch.to_string(), "text": ch.to_string(), "unmodifiedText": ch.to_string(), "code": code});
    if vk != 0 {
        ev["windowsVirtualKeyCode"] = json!(vk);
        ev["nativeVirtualKeyCode"] = json!(vk);
    }
    ev["type"] = json!(typ);
    ev
}

/// One click, text, key or scroll from the page.
pub fn login_input(app: &App, ev: &Value) -> Value {
    let kind = f(ev, "kind");
    let x = crate::app::num(ev.get("x"), Some(0.0)).unwrap_or(0.0);
    let y = crate::app::num(ev.get("y"), Some(0.0)).unwrap_or(0.0);
    let mut unknown: Option<&str> = None;
    let r = with_view(app, |ws| {
        // sent without waiting on the answers: a slow reply is not a failure,
        // only a socket that is gone is
        let mut sent = true;
        let mut call = |m: &str, p: Value| sent &= ws.fire(m, p);
        match kind.as_str() {
            "click" => {
                call("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y}));
                for typ in ["mousePressed", "mouseReleased"] {
                    call("Input.dispatchMouseEvent", json!({"type": typ, "x": x, "y": y, "button": "left", "clickCount": 1}));
                }
            }
            "text" => {
                let text = f(ev, "text");
                let n = text.chars().count();
                if n == 1 || (n > 0 && n <= 8 && text.chars().all(|c| c.is_alphanumeric())) {
                    // a keystroke, or a pasted code: one key per character, since a
                    // one-time-code field listens for keys
                    for ch in text.chars() {
                        call("Input.dispatchKeyEvent", key_event(ch, "keyDown"));
                        call("Input.dispatchKeyEvent", key_event(ch, "keyUp"));
                    }
                } else if !text.is_empty() {
                    call("Input.insertText", json!({"text": text}));
                }
            }
            "key" => {
                let key = f(ev, "key");
                let vk = match VIEW_KEYS.iter().find(|(k, _)| *k == key) { Some((_, v)) => *v, None => { unknown = Some("unknown key"); return Some(()) } };
                let mut base = json!({"key": key, "code": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk});
                if key == "Enter" {
                    base["text"] = json!("\r");
                }
                let mut down = base.clone();
                down["type"] = json!("keyDown");
                call("Input.dispatchKeyEvent", down);
                let mut up = base;
                up["type"] = json!("keyUp");
                call("Input.dispatchKeyEvent", up);
            }
            "wheel" => {
                let dy = crate::app::num(ev.get("deltaY"), Some(0.0)).unwrap_or(0.0);
                call("Input.dispatchMouseEvent", json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": dy}));
            }
            _ => {
                unknown = Some("unknown input");
            }
        }
        // the answers are read and dropped, so they never pile up on the socket
        while ws.recv_message(Duration::from_millis(1)).is_ok() {}
        if sent { Some(()) } else { None }
    });
    if let Some(u) = unknown {
        return json!({"ok": false, "error": u});
    }
    match r {
        Some(()) => json!({"ok": true}),
        None if cdp_pages(DEBUG_PORT).is_empty() => json!({"ok": false, "error": "No login window."}),
        None => json!({"ok": false, "error": "The login window did not take that."}),
    }
}

pub fn cancel_login(app: &Arc<App>) -> Value {
    let was = {
        let mut st = app.state.lock().unwrap();
        let was = st.capturing;
        st.capturing = false;
        st.error.clear();
        was
    };
    log("bagholder login: cancelled");
    close_login_browser(app, None);
    json!({"ok": true, "cancelled": was})
}

/// Open the login window, or bring forward the
/// one the app already has up.
pub fn start_login_browser(app: &Arc<App>) -> Value {
    if let Err(e) = bagholder_store::guard_home(&app.home) {
        return json!({"ok": false, "error": e});
    }
    log("bagholder login: connect requested");
    if browser_alive(app) {
        if let Some(u) = browser_ws() {
            if let Ok(mut ws) = Ws::connect(&u, Duration::from_secs(5)) {
                if let Some(p) = cdp_pages(DEBUG_PORT).first() {
                    ws.call("Target.activateTarget", Some(json!({"targetId": f(p, "id")})), Duration::from_secs(8));
                }
                ws.close();
            }
        }
        let (already, pid, attempt) = {
            let mut st = app.state.lock().unwrap();
            let already = st.capturing;
            st.capturing = true;
            st.error.clear();
            if !already {
                st.login_attempt += 1;
            }
            (already, st.chrome_pid, st.login_attempt)
        };
        if !already {
            let a = app.clone();
            spawn("bagholder-cdp-capture", move || poll_session(&a, pid, attempt));
        }
        log("bagholder login: window already up, brought forward");
        return json!({"ok": true, "reused": true});
    }
    close_login_browser(app, None);
    let chrome = find_chrome();
    if chrome.is_empty() {
        return json!({"ok": false, "error": NO_BROWSER});
    }
    let profile = app.home.join("chrome");
    let _ = std::fs::create_dir_all(&profile);
    if login_view() {
        // a container's profile outlives the container: the lock a previous one
        // left names a host that no longer exists, and Chromium would refuse the
        // profile as in use elsewhere
        for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
            let _ = std::fs::remove_file(profile.join(name));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700));
    }
    let mut args: Vec<String> = vec![
        format!("--user-data-dir={}", profile.to_string_lossy()),
        format!("--remote-debugging-port={}", DEBUG_PORT),
        "--remote-debugging-address=127.0.0.1".into(),
        "--remote-allow-origins=http://127.0.0.1".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--new-window".into(),
    ];
    if login_view() {
        // a container: a real window on its virtual display, sized for the page
        args.extend(["--no-sandbox".into(), "--disable-gpu".into(), "--disable-dev-shm-usage".into(), "--window-position=0,0".into(), "--hide-crash-restore-bubble".into(),
                     format!("--window-size={},{}", LOGIN_VIEW_SIZE.0, LOGIN_VIEW_SIZE.1)]);
    }
    args.push(LOGIN_URL.into());
    let mut cmd = Command::new(&chrome);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = match cmd.spawn() { Ok(c) => c, Err(_) => return json!({"ok": false, "error": NO_BROWSER}) };
    let pid = child.id();
    log(&format!("bagholder login: chrome launched (pid {})", pid));
    let attempt = {
        let mut st = app.state.lock().unwrap();
        st.chrome_proc = Some(child);
        st.chrome_pid = pid;
        st.capturing = true;
        st.error.clear();
        st.login_attempt += 1;
        st.login_attempt
    };
    app.spawn_with("bagholder-cdp-capture", move |app| poll_session(&app, pid, attempt));
    if login_view() {
        {
            let (m, _) = &app.login.cast;
            let mut g = m.lock().unwrap();
            g.frame = None;
            g.seq = 0;
            g.at = None;
        }
        let a1 = app.clone(); spawn("bagholder-screencast", move || screencast_loop(&a1, attempt));
        let a2 = app.clone(); spawn("bagholder-screenshots", move || screenshot_loop(&a2, attempt));
    }
    json!({"ok": true})
}

