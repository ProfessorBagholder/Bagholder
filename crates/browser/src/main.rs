//! One browser session over a pipe: each line on stdin is a request,
//! `{"method": "GET", "url": ..., "timeout": seconds, "headers": {...},
//! "body": ...}`, and each line on stdout its answer, `{"status": ...,
//! "body": ...}` or `{"error": ...}`. Cookies are kept for the life of the
//! process, as a `curl_cffi` session keeps them.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let client = wreq::Client::builder()
        .emulation(wreq_util::Emulation::Chrome137)
        .cookie_store(true)
        .build()
        .expect("client");
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"error": e.to_string()}));
                let _ = out.flush();
                continue;
            }
        };
        let answer = rt.block_on(async {
            let url = req.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_uppercase();
            let secs = req.get("timeout").and_then(|v| v.as_f64()).unwrap_or(30.0);
            let mut b = if method == "POST" { client.post(url) } else { client.get(url) };
            b = b.timeout(Duration::from_secs_f64(secs));
            if let Some(h) = req.get("headers").and_then(|v| v.as_object()) {
                for (k, v) in h {
                    b = b.header(k.as_str(), v.as_str().unwrap_or(""));
                }
            }
            if let Some(body) = req.get("body").and_then(|v| v.as_str()) {
                b = b.body(body.to_string());
            }
            match b.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    match resp.text().await {
                        Ok(body) => json!({"status": status, "body": body}),
                        Err(e) => json!({"error": e.to_string()}),
                    }
                }
                Err(e) => json!({"error": e.to_string()}),
            }
        });
        if writeln!(out, "{}", answer).is_err() || out.flush().is_err() {
            break;
        }
    }
}
