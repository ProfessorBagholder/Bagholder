//! A session that presents a browser's TLS handshake, for the hosts that gate
//! on it (Chrome's). The handshake lives in the `bagholder-browser` helper beside this
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
    /// Where the request ended up after redirects.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
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

fn unbase64(text: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let bytes: Vec<u8> = text.bytes().filter(|c| *c != b'=').collect();
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        let mut k: usize = 0;
        for c in chunk {
            if let Some(v) = val(*c) {
                n = (n << 6) | v;
                k += 1;
            }
        }
        n <<= 6 * (4 - k);
        let got = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&got[..k.saturating_sub(1)]);
    }
    out
}

impl Session {
    /// None where the helper is not there to start, and the source that needs
    /// it is then simply unknown.
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

    /// Whether the helper is installed beside this binary at all.
    pub fn available() -> bool {
        helper().is_file()
    }

    /// A GET that answers with its status whatever it is, as a `requests`
    /// session does.
    pub fn get(&mut self, url: &str, timeout: Duration) -> Result<Answer, String> {
        self.request("GET", url, &[], None, timeout, false)
    }

    pub fn request(
        &mut self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
        timeout: Duration,
        binary: bool,
    ) -> Result<Answer, String> {
        let hdrs: Vec<Value> = headers.iter().map(|(k, v)| json!([k, v])).collect();
        let mut req = json!({"method": method, "url": url, "timeout": timeout.as_secs_f64(), "headers": hdrs, "binary": binary});
        if let Some(b) = body {
            req["body"] = json!(b);
        }
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
        let body = match v.get("base64").and_then(|b| b.as_str()) {
            Some(b) => unbase64(b),
            None => v.get("body").and_then(|b| b.as_str()).unwrap_or("").as_bytes().to_vec(),
        };
        Ok(Answer {
            status: v.get("status").and_then(|s| s.as_u64()).unwrap_or(0) as u16,
            url: v.get("url").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            headers: v
                .get("headers")
                .and_then(|h| h.as_array())
                .map(|a| {
                    a.iter()
                        .map(|p| (p[0].as_str().unwrap_or("").to_string(), p[1].as_str().unwrap_or("").to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            body,
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
