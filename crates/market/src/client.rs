//! A small HTTP/1.1 client over OpenSSL.
//!
//! Not a choice of convenience. Some of these hosts gate on the TLS
//! handshake: FRED answers an OpenSSL client and stonewalls both rustls and
//! macOS SecureTransport, so the client speaks through OpenSSL.
//!
//! One connection per host is kept open between requests, because for a
//! request this small the handshake is the whole cost.

use openssl::ssl::{SslConnector, SslMethod, SslStream};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const REDIRECT_MAX: usize = 5;
const POOL_PER_HOST: usize = 2;

#[derive(Debug, Clone)]
pub enum Error {
    Status(u16),
    Transport(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Status(c) => write!(f, "HTTP {}", c),
            Error::Transport(m) => write!(f, "{}", m),
        }
    }
}

impl Error {
    pub fn code(&self) -> Option<u16> {
        match self { Error::Status(c) => Some(*c), _ => None }
    }
}

fn transport<E: std::fmt::Display>(e: E) -> Error {
    Error::Transport(e.to_string())
}

/// The CA bundle, looked up in order:
/// certifi's if the environment names one, then the system's.
fn ca_file() -> Option<String> {
    let mut paths: Vec<String> = Vec::new();
    if let Ok(p) = std::env::var("SSL_CERT_FILE") {
        paths.push(p);
    }
    paths.extend(
        [
            "/etc/ssl/cert.pem",
            "/etc/ssl/certs/ca-certificates.crt",
            "/opt/homebrew/etc/openssl@3/cert.pem",
            "/usr/local/etc/openssl@3/cert.pem",
            "/opt/homebrew/etc/openssl@1.1/cert.pem",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    paths.into_iter().find(|p| std::path::Path::new(p).is_file())
}

fn connector() -> &'static SslConnector {
    static C: OnceLock<SslConnector> = OnceLock::new();
    C.get_or_init(|| {
        let mut b = SslConnector::builder(SslMethod::tls_client()).expect("openssl");
        if let Some(ca) = ca_file() {
            let _ = b.set_ca_file(ca);
        }
        // HTTP/1.1 only: this client speaks nothing else, and a host that
        // negotiated h2 would then answer in a framing it cannot read.
        let _ = b.set_alpn_protos(b"\x08http/1.1");
        b.build()
    })
}

struct Url {
    host: String,
    port: u16,
    path: String,
    tls: bool,
}

fn parse_url(url: &str) -> Result<Url, Error> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| Error::Transport(format!("bad url {url}")))?;
    let tls = scheme.eq_ignore_ascii_case("https");
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.bytes().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h.to_string(), p.parse().unwrap_or(if tls { 443 } else { 80 }))
        }
        _ => (authority.to_string(), if tls { 443 } else { 80 }),
    };
    Ok(Url { host, port, path: path.to_string(), tls })
}

enum Conn {
    Tls(Box<SslStream<TcpStream>>),
    Plain(TcpStream),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tls(s) => s.read(buf),
            Conn::Plain(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tls(s) => s.write(buf),
            Conn::Plain(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Tls(s) => s.flush(),
            Conn::Plain(s) => s.flush(),
        }
    }
}

fn pool() -> &'static Mutex<HashMap<String, Vec<Conn>>> {
    static P: OnceLock<Mutex<HashMap<String, Vec<Conn>>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open(u: &Url, timeout: Duration) -> Result<Conn, Error> {
    let addr = format!("{}:{}", u.host, u.port);
    let sock = addr
        .to_socket_addrs_first()
        .ok_or_else(|| Error::Transport(format!("cannot resolve {}", u.host)))?;
    let tcp = TcpStream::connect_timeout(&sock, timeout).map_err(transport)?;
    tcp.set_read_timeout(Some(timeout)).map_err(transport)?;
    tcp.set_write_timeout(Some(timeout)).map_err(transport)?;
    tcp.set_nodelay(true).ok();
    if !u.tls {
        return Ok(Conn::Plain(tcp));
    }
    let s = connector().connect(&u.host, tcp).map_err(|e| Error::Transport(e.to_string()))?;
    Ok(Conn::Tls(Box::new(s)))
}

trait FirstAddr {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr>;
}

impl FirstAddr for String {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        self.to_socket_addrs().ok()?.next()
    }
}

/// Reads bytes until the header block ends, leaving whatever of the body came
/// with it.
fn read_head(conn: &mut Conn) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut byte = [0u8; 1];
    loop {
        let n = conn.read(&mut byte).map_err(transport)?;
        if n == 0 {
            return Err(Error::Transport("connection closed before the headers".into()));
        }
        buf.push(byte[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            let head = buf[..buf.len() - 4].to_vec();
            return Ok((head, Vec::new()));
        }
        if buf.len() > 64 * 1024 {
            return Err(Error::Transport("header block too large".into()));
        }
    }
}

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

fn parse_head(raw: &[u8]) -> Result<Head, Error> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| Error::Transport(format!("bad status line {status_line:?}")))?;
    let headers = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())))
        .collect();
    Ok(Head { status, headers })
}

impl Head {
    fn get(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

fn read_exact_n(conn: &mut Conn, n: usize, out: &mut Vec<u8>) -> Result<(), Error> {
    let mut left = n;
    let mut chunk = [0u8; 16 * 1024];
    while left > 0 {
        let want = left.min(chunk.len());
        let got = conn.read(&mut chunk[..want]).map_err(transport)?;
        if got == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..got]);
        left -= got;
    }
    Ok(())
}

fn read_to_close(conn: &mut Conn, out: &mut Vec<u8>) {
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            // a host that closes without a clean TLS shutdown has still sent
            // its body, and that body is kept
            Err(_) => break,
        }
    }
}

fn read_chunked(conn: &mut Conn, out: &mut Vec<u8>) -> Result<(), Error> {
    loop {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = conn.read(&mut byte).map_err(transport)?;
            if n == 0 {
                return Ok(());
            }
            line.push(byte[0]);
            if line.ends_with(b"\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&line);
        let size_hex = text.trim().split(';').next().unwrap_or("").trim().to_string();
        let size = usize::from_str_radix(&size_hex, 16).unwrap_or(0);
        if size == 0 {
            return Ok(());
        }
        read_exact_n(conn, size, out)?;
        let mut crlf = [0u8; 2];
        let _ = conn.read(&mut crlf);
    }
}

fn gunzip(raw: &[u8]) -> Vec<u8> {
    // The gzip two-byte magic; anything else is returned as it came.
    if raw.len() < 2 || raw[0] != 0x1f || raw[1] != 0x8b {
        return raw.to_vec();
    }
    let mut out = Vec::new();
    let mut d = flate2::read::GzDecoder::new(raw);
    match d.read_to_end(&mut out) {
        Ok(_) => out,
        Err(_) => raw.to_vec(),
    }
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// The final answer's headers, names lower-cased.
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

/// One request, following redirects, with the connection given back when the
/// host is willing to keep it open.
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<Response, Error> {
    send(method, url, headers, body, timeout, false)
}

/// The same request, answering an HTTP error with its response rather than
/// failing -- for a caller that reads what an error page sets, such as its
/// cookies.
pub fn request_any(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, timeout: Duration) -> Result<Response, Error> {
    send(method, url, headers, body, timeout, true)
}

fn send(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
    lenient: bool,
) -> Result<Response, Error> {
    let mut url = url.to_string();
    for _ in 0..=REDIRECT_MAX {
        let u = parse_url(&url)?;
        let key = format!("{}:{}", u.host, u.port);

        let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", method, u.path, u.host);
        let mut seen: Vec<String> = Vec::new();
        for (k, v) in headers {
            req.push_str(&format!("{}: {}\r\n", k, v));
            seen.push(k.to_lowercase());
        }
        // `identity` unless the caller asks otherwise. It is not cosmetic: FRED answers a keep-alive
        // request that asks for identity and simply never replies to one that
        // asks for gzip.
        if !seen.iter().any(|k| k == "accept-encoding") {
            req.push_str("Accept-Encoding: identity\r\n");
        }
        if !seen.iter().any(|k| k == "connection") {
            req.push_str("Connection: keep-alive\r\n");
        }
        if let Some(b) = body {
            req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        req.push_str("\r\n");

        // Two attempts: a pooled connection the host has already closed shows
        // up as a failed read rather than a failed write, so the first try is
        // allowed to fail and the second always opens a fresh one.
        let mut c;
        let head;
        let mut attempt = 0;
        loop {
            let pooled = {
                let mut p = pool().lock().unwrap();
                p.get_mut(&key).and_then(|v| v.pop())
            };
            let from_pool = pooled.is_some();
            let mut conn = match pooled { Some(c) => c, None => open(&u, timeout)? };

            let sent = conn.write_all(req.as_bytes()).and_then(|_| {
                if let Some(b) = body { conn.write_all(b)?; }
                conn.flush()
            });
            let got = match sent {
                Ok(()) => read_head(&mut conn).and_then(|(raw, _)| parse_head(&raw)),
                Err(e) => Err(transport(e)),
            };
            match got {
                Ok(h) => { c = conn; head = h; break }
                Err(e) => {
                    attempt += 1;
                    if attempt > 1 || !from_pool {
                        return Err(e);
                    }
                }
            }
        }

        let mut raw: Vec<u8> = Vec::new();
        let chunked = head.get("transfer-encoding").map(|v| v.to_lowercase().contains("chunked")).unwrap_or(false);
        let length: Option<usize> = head.get("content-length").and_then(|v| v.trim().parse().ok());
        if chunked {
            read_chunked(&mut c, &mut raw)?;
        } else if let Some(n) = length {
            read_exact_n(&mut c, n, &mut raw)?;
        } else {
            read_to_close(&mut c, &mut raw);
        }

        let keep = head
            .get("connection")
            .map(|v| !v.to_lowercase().contains("close"))
            .unwrap_or(true)
            && (chunked || length.is_some());
        if keep {
            let mut p = pool().lock().unwrap();
            let v = p.entry(key).or_default();
            if v.len() < POOL_PER_HOST {
                v.push(c);
            }
        }

        if (301..=308).contains(&head.status) {
            if let Some(loc) = head.get("location") {
                url = if loc.starts_with("http") {
                    loc.to_string()
                } else {
                    format!("{}://{}{}", if u.tls { "https" } else { "http" }, u.host, loc)
                };
                continue;
            }
        }
        if head.status >= 400 && !lenient {
            return Err(Error::Status(head.status));
        }
        return Ok(Response { status: head.status, body: gunzip(&raw), headers: head.headers.clone() });
    }
    Err(Error::Transport("too many redirects".into()))
}
