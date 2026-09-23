//! One browser session over a pipe: each line on stdin is a request,
//! `{"method": "GET"|"POST", "url": ..., "timeout": seconds, "headers":
//! [[name, value], ...], "body": ..., "binary": bool}`, and each line on
//! stdout its answer, `{"status": ..., "url": final url, "headers": [[name,
//! value], ...], "body": text}` -- or `"base64"` in place of `"body"` when the
//! request asked for bytes -- or `{"error": ...}`. Redirects are followed and
//! cookies kept for the life of the process.

use serde::Deserialize;
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

/// One request line: `{"method": "GET"|"POST", "url": ..., "timeout": seconds,
/// "headers": [[name, value], ...], "body": ..., "binary": bool}`.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Req {
    method: String,
    url: String,
    timeout: Option<f64>,
    headers: Vec<(String, String)>,
    body: Option<String>,
    binary: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_pads_by_the_remainder() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_a_request_line_reads_its_fields_leniently() {
        let r: Req = serde_json::from_str(r#"{"method": "post", "url": "http://x", "timeout": 5, "headers": [["A", "1"], ["B", "2"]], "body": "hi", "binary": true}"#).unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str(), r.timeout, r.body.as_deref(), r.binary), ("post", "http://x", Some(5.0), Some("hi"), true));
        assert_eq!(r.headers, vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]);
        let empty: Req = serde_json::from_str("{}").unwrap();
        assert_eq!((empty.method.as_str(), empty.url.as_str(), empty.timeout, empty.body, empty.binary), ("", "", None, None, false));
    }
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
        let req: Req = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"error": e.to_string()}));
                let _ = out.flush();
                continue;
            }
        };
        let answer = rt.block_on(async {
            let url = req.url.as_str();
            let method = req.method.to_uppercase();
            let secs = req.timeout.unwrap_or(30.0);
            let binary = req.binary;
            let mut b = if method == "POST" { client.post(url) } else { client.get(url) };
            b = b.timeout(Duration::from_secs_f64(secs));
            for (k, v) in &req.headers {
                if !k.is_empty() {
                    b = b.header(k, v);
                }
            }
            if let Some(body) = req.body {
                b = b.body(body);
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
