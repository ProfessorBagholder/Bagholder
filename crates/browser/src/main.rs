//! One browser session over a pipe: each line on stdin is a request,
//! `{"method": "GET"|"POST", "url": ..., "timeout": seconds, "headers":
//! [[name, value], ...], "body": ..., "binary": bool}`, and each line on
//! stdout its answer, `{"status": ..., "url": final url, "headers": [[name,
//! value], ...], "body": text}` -- or `"base64"` in place of `"body"` when the
//! request asked for bytes -- or `{"error": ...}`. Redirects are followed and
//! cookies kept for the life of the process.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Duration;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let client = wreq::Client::builder()
        .emulation(wreq_util::Emulation::Chrome149)
        .cookie_store(true)
        .redirect(wreq::redirect::Policy::limited(10))
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
            let binary = req.get("binary").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut b = if method == "POST" { client.post(url) } else { client.get(url) };
            b = b.timeout(Duration::from_secs_f64(secs));
            if let Some(h) = req.get("headers").and_then(|v| v.as_array()) {
                for pair in h {
                    let k = pair.get(0).and_then(|v| v.as_str()).unwrap_or("");
                    let v = pair.get(1).and_then(|v| v.as_str()).unwrap_or("");
                    if !k.is_empty() {
                        b = b.header(k, v);
                    }
                }
            }
            if let Some(body) = req.get("body").and_then(|v| v.as_str()) {
                b = b.body(body.to_string());
            }
            match b.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let final_url = resp.uri().to_string();
                    let headers: Vec<Value> = resp
                        .headers()
                        .iter()
                        .map(|(k, v)| json!([k.as_str(), String::from_utf8_lossy(v.as_bytes())]))
                        .collect();
                    match resp.bytes().await {
                        Ok(bytes) => {
                            if binary {
                                json!({"status": status, "url": final_url, "headers": headers, "base64": base64(&bytes)})
                            } else {
                                json!({"status": status, "url": final_url, "headers": headers, "body": String::from_utf8_lossy(&bytes)})
                            }
                        }
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
