//! A session that presents a browser's TLS handshake, for the hosts that gate
//! on it, as `curl_cffi`'s `Session(impersonate="chrome")` is on the Python
//! side. The handshake lives in the `bagholder-browser` helper beside this
//! binary; a session is one helper process, and its cookies are that
//! process's.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

pub struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

pub struct Answer {
    pub status: u16,
    pub body: String,
}

/// The helper beside the running binary, or `BAGHOLDER_BROWSER` when set.
fn helper() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("BAGHOLDER_BROWSER") {
        if !p.trim().is_empty() {
            return p.into();
        }
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().map(|d| d.join("bagholder-browser")).unwrap_or_else(|| "bagholder-browser".into())
}

impl Session {
    /// None where the helper is not there to start, and the source that needs
    /// it is then simply unknown, as it is in Python without `curl_cffi`.
    pub fn new() -> Option<Session> {
        let mut child = Command::new(helper())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let input = child.stdin.take()?;
        let output = BufReader::new(child.stdout.take()?);
        Some(Session { child, input, output })
    }

    /// A GET that answers with its status whatever it is, as a `requests`
    /// session does.
    pub fn get(&mut self, url: &str, timeout: Duration) -> Result<Answer, String> {
        self.send(json!({"method": "GET", "url": url, "timeout": timeout.as_secs_f64()}))
    }

    fn send(&mut self, req: Value) -> Result<Answer, String> {
        writeln!(self.input, "{}", req).map_err(|e| e.to_string())?;
        self.input.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if self.output.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("the browser helper exited".into());
        }
        let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            return Err(e.to_string());
        }
        Ok(Answer {
            status: v.get("status").and_then(|s| s.as_u64()).unwrap_or(0) as u16,
            body: v.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string(),
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
