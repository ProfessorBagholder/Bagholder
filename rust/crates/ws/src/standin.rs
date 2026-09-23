//! A stand-in Wealthsimple server on 127.0.0.1, and the fresh `Home` it is
//! pointed at through `BAGHOLDER_WS_BASE`.
//!
//! Every test using this fixture sets that env var, so callers run one at a
//! time under `ENV`'s lock. Built only with the `standin` feature, which the
//! client's own tests and the server's turn on.

use crate::session::Home;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub const FAKE_CLIENT_ID: &str = "abababababababababababababababababababababababababababababababab";

static ENV: Mutex<()> = Mutex::new(());
static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub body: Value,
}

pub type Handler = dyn Fn(&Req) -> (u16, Vec<(String, String)>, Vec<u8>) + Send + Sync;

pub struct Fixture {
    _guard: MutexGuard<'static, ()>,
    pub home: Home,
    reqs: Arc<Mutex<Vec<Req>>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::env::remove_var("BAGHOLDER_WS_BASE");
        let _ = std::fs::remove_dir_all(&self.home.dir);
    }
}

impl Fixture {
    pub fn client(&self) -> crate::session::Client<'_> {
        crate::session::Client { home: &self.home }
    }
    pub fn requests(&self) -> Vec<Req> {
        self.reqs.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Req> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).ok()?;
    let body = serde_json::from_slice(&buf).unwrap_or(Value::Null);
    Some(Req { method, path, body })
}

/// A fresh home and a server answering with `handler`, `BAGHOLDER_WS_BASE`
/// pointed at it.
pub fn fixture(handler: Box<Handler>) -> Fixture {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir: PathBuf = std::env::temp_dir().join(format!("bh-ws-http-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = Home::new(&dir);
    // clears a refused refresh token another test left behind
    home.delete_session();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reqs: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = reqs.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream { Ok(s) => s, Err(_) => continue };
            while let Some(req) = read_request(&mut stream) {
                seen.lock().unwrap().push(req.clone());
                let (status, headers, body) = handler(&req);
                let mut head = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", status, body.len());
                for (k, v) in headers {
                    head.push_str(&format!("{}: {}\r\n", k, v));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                break;
            }
        }
    });
    std::env::set_var("BAGHOLDER_WS_BASE", format!("http://127.0.0.1:{}", port));
    Fixture { _guard: guard, home, reqs }
}

pub fn ok_json(v: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    (200, vec![("Content-Type".into(), "application/json".into())], serde_json::to_vec(&v).unwrap())
}

pub fn graphql(data: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    ok_json(json!({"data": data}))
}

pub fn graphql_errors(errors: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    ok_json(json!({"errors": errors}))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// `gzip.compress`, with one stored deflate block.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let len = data.len() as u16;
    out.push(1);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

pub fn unused() -> Box<Handler> {
    Box::new(|_| ok_json(json!({})))
}
