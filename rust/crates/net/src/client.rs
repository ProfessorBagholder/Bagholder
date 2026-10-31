//! A small HTTP/1.1 client over OpenSSL.
//!
//! Not a choice of convenience. Some of these hosts gate on the TLS
//! handshake: FRED answers an OpenSSL client and stonewalls both rustls and
//! macOS SecureTransport, so the client speaks through OpenSSL.
//!
//! One connection per host is kept open between requests, because for a
//! request this small the handshake is the whole cost.

use openssl::ssl::{SslConnector, SslMethod, SslOptions, SslStream};
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
    /// Refused by `BAGHOLDER_OFFLINE`: nothing was asked of anyone.
    Offline,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Status(c) => write!(f, "HTTP {}", c),
            Error::Transport(m) => write!(f, "{}", m),
            Error::Offline => f.write_str(OFFLINE),
        }
    }
}

/// What failed, in words a person reads; or a request the process was told not to
/// make (`BAGHOLDER_OFFLINE`), which asked nobody and is no failure of anyone's,
/// known by its type wherever it is carried, never by its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Offline,
    Said(String),
}

impl Failure {
    /// The words changed by `f`; an offline refusal stays one.
    pub fn map(self, f: impl FnOnce(String) -> String) -> Failure {
        match self {
            Failure::Offline => Failure::Offline,
            Failure::Said(s) => Failure::Said(f(s)),
        }
    }

    /// Whether nothing was asked because the process is offline.
    pub fn is_offline(&self) -> bool {
        matches!(self, Failure::Offline)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Offline => f.write_str(OFFLINE),
            Failure::Said(s) => f.write_str(s),
        }
    }
}

impl From<String> for Failure {
    fn from(s: String) -> Failure {
        Failure::Said(s)
    }
}

impl From<Error> for Failure {
    /// A failed request: an offline refusal by its kind, any other in its words.
    fn from(e: Error) -> Failure {
        match e {
            Error::Offline => Failure::Offline,
            e => Failure::Said(e.to_string()),
        }
    }
}

impl From<&str> for Failure {
    fn from(s: &str) -> Failure {
        Failure::Said(s.to_string())
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

/// The one TLS connector, or why it could not be made: a CA bundle found but
/// not loadable leaves nothing to trust, and every TLS request says so.
fn connector() -> Result<&'static SslConnector, Error> {
    static C: OnceLock<Result<SslConnector, String>> = OnceLock::new();
    C.get_or_init(|| connector_settings().map(|b| b.build())).as_ref().map_err(|e| Error::Transport(e.clone()))
}

/// OpenSSL's `SSL_OP_TLSEXT_PADDING`, which the safe crate names no constant for.
const TLSEXT_PADDING: SslOptions = SslOptions::from_bits_retain(0x10);

/// The connector's settings, before it is built.
fn connector_settings() -> Result<openssl::ssl::SslConnectorBuilder, String> {
    let mut b = SslConnector::builder(SslMethod::tls_client()).expect("openssl");
    if let Some(ca) = ca_file() {
        b.set_ca_file(&ca).map_err(|e| format!("cannot load the CA bundle {ca}: {e}"))?;
    }
    // no padding extension in the hello: the connector's `SSL_OP_ALL` turns it on,
    // and OpenSSL 3.0 adds it to a hello of 256 to 511 bytes (a workaround for old
    // load balancers) where a newer OpenSSL's larger hello never takes it, so the
    // hello differed by the system's OpenSSL. Evolve's site refuses the padded one
    // (403, 2026-10-30), so a build on Debian 12 or Ubuntu 24.04 could not read it
    // where the macOS build could. Every build sends the hello without it.
    b.clear_options(TLSEXT_PADDING);
    // HTTP/1.1 only: this client speaks nothing else, and a host that
    // negotiated h2 would then answer in a framing it cannot read.
    b.set_alpn_protos(b"\x08http/1.1").expect("a well-formed protocol list is accepted");
    Ok(b)
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
    /// TLS over a socket whose reads end at a deadline (`get_at`).
    TlsUntil(Box<SslStream<Until>>),
    Plain(TcpStream),
}

/// A socket whose reads end at a deadline on the clock it is handed: each waits
/// only what is left of it, so a peer sending a byte at a time cannot hold the
/// read past it.
struct Until {
    tcp: TcpStream,
    clock: std::sync::Arc<dyn crate::Clock>,
    at: jiff::Timestamp,
}

impl std::fmt::Debug for Until {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a socket read until {}", self.at)
    }
}

impl Read for Until {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = Duration::try_from(self.at.duration_since(self.clock.now())).ok().filter(|d| !d.is_zero());
        let Some(left) = left else {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the answer took longer than it is given"));
        };
        self.tcp.set_read_timeout(Some(left))?;
        self.tcp.read(buf)
    }
}

impl Write for Until {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tcp.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.tcp.flush()
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tls(s) => s.read(buf),
            Conn::TlsUntil(s) => s.read(buf),
            Conn::Plain(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tls(s) => s.write(buf),
            Conn::TlsUntil(s) => s.write(buf),
            Conn::Plain(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Tls(s) => s.flush(),
            Conn::TlsUntil(s) => s.flush(),
            Conn::Plain(s) => s.flush(),
        }
    }
}

fn pool() -> &'static Mutex<HashMap<String, Vec<Conn>>> {
    static P: OnceLock<Mutex<HashMap<String, Vec<Conn>>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn tcp_to(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, Error> {
    use std::net::ToSocketAddrs;
    let addrs: Vec<std::net::SocketAddr> =
        (host, port).to_socket_addrs().map_err(|e| Error::Transport(format!("cannot resolve {host}: {e}")))?.collect();
    if addrs.is_empty() {
        return Err(Error::Transport(format!("cannot resolve {host}")));
    }
    // each address the name resolves to, in turn: a host listening on one family
    // only (a local proxy on 127.0.0.1 while `localhost` is ::1 first) is reached
    let mut last = None;
    let mut tcp = None;
    for a in &addrs {
        match TcpStream::connect_timeout(a, timeout) {
            Ok(t) => {
                tcp = Some(t);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let tcp = match tcp {
        Some(t) => t,
        None => return Err(transport(last.expect("an address was tried"))),
    };
    tcp.set_read_timeout(Some(timeout)).map_err(transport)?;
    tcp.set_write_timeout(Some(timeout)).map_err(transport)?;
    tcp.set_nodelay(true).map_err(transport)?;
    Ok(tcp)
}

fn open(u: &Url, timeout: Duration) -> Result<Conn, Error> {
    if !u.tls {
        return Ok(Conn::Plain(tcp_to(&u.host, u.port, timeout)?));
    }
    let env = |k: &str| std::env::var(k).ok().or_else(|| std::env::var(k.to_ascii_lowercase()).ok()).filter(|v| !v.trim().is_empty());
    let tcp = match proxy::for_host(env("HTTPS_PROXY").as_deref(), env("NO_PROXY").as_deref(), &u.host).map_err(Error::Transport)? {
        Some(p) => {
            let mut tcp = tcp_to(&p.host, p.port, timeout)?;
            proxy::tunnel(&mut tcp, &p, &u.host, u.port)?;
            tcp
        }
        None => tcp_to(&u.host, u.port, timeout)?,
    };
    let s = connector()?.connect(&u.host, tcp).map_err(|e| Error::Transport(e.to_string()))?;
    Ok(Conn::Tls(Box::new(s)))
}

/// An HTTPS request through the proxy the environment names (`HTTPS_PROXY`,
/// with `NO_PROXY`'s exceptions), the way curl and every other client reads
/// them: a `CONNECT` tunnel to the host, and TLS with the host through it, so
/// the handshake a host gates on is still this client's own.
mod proxy {
    use std::io::{Read, Write};
    use std::net::{IpAddr, TcpStream};

    use super::Error;

    #[derive(Debug, PartialEq, Eq)]
    pub struct Proxy {
        pub host: String,
        pub port: u16,
        /// `user:password` from the proxy's URL, sent as basic authorization.
        pub auth: Option<String>,
    }

    /// The proxy a URL names; an error for one this client cannot speak to (any
    /// scheme but http), never a silent direct connection.
    fn parse(url: &str) -> Result<Proxy, String> {
        let unusable = || format!("HTTPS_PROXY names {url:?}, a proxy this client cannot use (it speaks to http:// proxies)");
        let rest = match url.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
            Some(_) => return Err(unusable()),
            None => url,
        };
        parse_authority(rest).ok_or_else(unusable)
    }

    fn parse_authority(rest: &str) -> Option<Proxy> {
        let authority = rest.split('/').next()?;
        let (auth, hostport) = match authority.rsplit_once('@') {
            Some((a, h)) => (Some(a.to_string()), h),
            None => (None, authority),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (h, p.parse().ok()?),
            _ => (hostport, 80),
        };
        if host.is_empty() {
            return None;
        }
        Some(Proxy { host: host.trim_matches(['[', ']']).to_string(), port, auth })
    }

    /// Whether `NO_PROXY` exempts `host`: `*`, the host itself, a domain it is
    /// under (`example.com` and `.example.com` both), or an address range.
    fn exempt(no_proxy: &str, host: &str) -> bool {
        let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
        let ip: Option<IpAddr> = host.parse().ok();
        no_proxy.split(',').map(str::trim).filter(|e| !e.is_empty()).any(|e| {
            let e = e.to_ascii_lowercase();
            if e == "*" {
                return true;
            }
            if let (Some(ip), Some((net, bits))) = (ip, e.split_once('/')) {
                return match (ip, net.parse::<IpAddr>(), bits.parse::<u32>()) {
                    (IpAddr::V4(a), Ok(IpAddr::V4(n)), Ok(b)) if b <= 32 => {
                        let mask = if b == 0 { 0 } else { u32::MAX << (32 - b) };
                        u32::from(a) & mask == u32::from(n) & mask
                    }
                    (IpAddr::V6(a), Ok(IpAddr::V6(n)), Ok(b)) if b <= 128 => {
                        let mask = if b == 0 { 0 } else { u128::MAX << (128 - b) };
                        u128::from(a) & mask == u128::from(n) & mask
                    }
                    _ => false,
                };
            }
            let d = e.trim_start_matches('.');
            host == d || host.ends_with(&format!(".{d}"))
        })
    }

    pub fn for_host(https_proxy: Option<&str>, no_proxy: Option<&str>, host: &str) -> Result<Option<Proxy>, String> {
        match https_proxy {
            Some(p) if !no_proxy.is_some_and(|n| exempt(n, host)) => parse(p.trim()).map(Some),
            _ => Ok(None),
        }
    }

    /// Ask the proxy for a tunnel to `host:port`; anything but a 2xx is the
    /// proxy's refusal, named with its status line.
    pub fn tunnel(tcp: &mut TcpStream, p: &Proxy, host: &str, port: u16) -> Result<(), Error> {
        let mut req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
        if let Some(a) = &p.auth {
            req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", openssl::base64::encode_block(a.as_bytes())));
        }
        req.push_str("\r\n");
        tcp.write_all(req.as_bytes()).map_err(super::transport)?;
        // the proxy's reply ends with its blank line; nothing of the tunnel follows
        // until the handshake is sent, so it is read a byte at a time
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if tcp.read(&mut byte).map_err(super::transport)? == 0 {
                return Err(Error::Transport(format!("the proxy {}:{} closed before answering", p.host, p.port)));
            }
            head.push(byte[0]);
            if head.len() > 16 * 1024 {
                return Err(Error::Transport("the proxy's answer has no end".into()));
            }
        }
        let status = String::from_utf8_lossy(&head);
        let line = status.lines().next().unwrap_or("");
        match line.split_whitespace().nth(1) {
            Some(code) if code.starts_with('2') => Ok(()),
            _ => Err(Error::Transport(format!("the proxy {}:{} refused a tunnel to {host}: {line}", p.host, p.port))),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_proxy_is_read_as_curl_reads_it() {
            let p = for_host(Some("http://user:pa55@localhost:60265"), Some("localhost,127.0.0.1"), "www.bankofcanada.ca").unwrap().unwrap();
            assert_eq!(p, Proxy { host: "localhost".into(), port: 60265, auth: Some("user:pa55".into()) });
            assert_eq!(for_host(Some("proxy.example:3128"), None, "a.b").unwrap().unwrap().port, 3128);
            assert_eq!(for_host(Some("http://proxy.example"), None, "a.b").unwrap().unwrap().port, 80);
            assert_eq!(for_host(None, None, "a.b"), Ok(None));
            // a proxy that is not http is one this client cannot speak to: a
            // failure naming it, never a request sent around it
            assert!(for_host(Some("socks5://proxy.example:1080"), None, "a.b").unwrap_err().contains("cannot use"));
            assert!(for_host(Some("https://proxy.example:443"), None, "a.b").unwrap_err().contains("cannot use"));
            // unless the host is one NO_PROXY exempts
            assert_eq!(for_host(Some("socks5://proxy.example:1080"), Some("a.b"), "a.b"), Ok(None));
        }

        #[test]
        fn no_proxy_exempts_hosts_domains_and_ranges() {
            let n = "localhost, .internal.example, example.org, 10.0.0.0/8, ::1/128";
            for host in ["localhost", "a.internal.example", "internal.example", "www.example.org", "10.1.2.3", "[::1]"] {
                assert_eq!(for_host(Some("http://p:1"), Some(n), host), Ok(None), "{host}");
            }
            for host in ["notexample.org", "11.0.0.1", "www.bankofcanada.ca"] {
                assert!(for_host(Some("http://p:1"), Some(n), host).unwrap().is_some(), "{host}");
            }
            assert_eq!(for_host(Some("http://p:1"), Some("*"), "anything"), Ok(None));
        }
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

/// Where a body goes as it is read, counted: past its ceiling, where it has one,
/// the read stops, at the ceiling plus one chunk at most. An answer read into memory
/// has none: no answer is cut at a size of the app's choosing. A file whose size its
/// source states comes through `download`, to disk, held to that size.
struct Capped<'a> {
    out: &'a mut dyn Write,
    left: u64,
    ceiling: u64,
    written: u64,
}

impl<'a> Capped<'a> {
    fn new(out: &'a mut dyn Write, ceiling: u64) -> Capped<'a> {
        Capped { out, left: ceiling, ceiling, written: 0 }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() as u64 > self.left {
            return Err(Error::Transport(format!("an answer larger than the {} bytes it may be", self.ceiling)));
        }
        self.out.write_all(bytes).map_err(|e| Error::Transport(format!("the answer could not be kept: {e}")))?;
        self.left -= bytes.len() as u64;
        self.written += bytes.len() as u64;
        Ok(())
    }
}

fn read_exact_n(conn: &mut Conn, n: usize, out: &mut Capped<'_>) -> Result<(), Error> {
    if n as u64 > out.left {
        // announced larger than it may be: nothing of it is read
        return Err(Error::Transport(format!("an answer of {n} bytes, larger than the {} it may be", out.ceiling)));
    }
    let mut left = n;
    let mut chunk = [0u8; 16 * 1024];
    while left > 0 {
        let want = left.min(chunk.len());
        let got = conn.read(&mut chunk[..want]).map_err(transport)?;
        if got == 0 {
            // the host closed with the body short of what it announced: a reply cut
            // off, never a whole one
            return Err(Error::Transport(format!("connection closed {} bytes before the end of the body", left)));
        }
        out.put(&chunk[..got])?;
        left -= got;
    }
    Ok(())
}

fn read_to_close(conn: &mut Conn, out: &mut Capped<'_>) -> Result<(), Error> {
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => out.put(&chunk[..n])?,
            // a host that closes without a clean TLS shutdown has still sent
            // its body, and that body is kept
            Err(_) => return Ok(()),
        }
    }
}

/// One line of a chunked body, its CRLF included; the host closing first is a
/// reply cut off.
fn read_line(conn: &mut Conn) -> Result<Vec<u8>, Error> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = conn.read(&mut byte).map_err(transport)?;
        if n == 0 {
            return Err(Error::Transport("connection closed before the end of the chunked body".into()));
        }
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return Ok(line);
        }
        if line.len() > 64 * 1024 {
            return Err(Error::Transport("chunk line too long".into()));
        }
    }
}

/// A chunked body (RFC 9112 §7.1), read to its end: the chunks, the last one of
/// size zero, then any trailer fields and the empty line that closes the body.
/// That line has to be read too, or it stays on a kept connection and the next
/// reply read there starts with it: an empty status line.
fn read_chunked(conn: &mut Conn, out: &mut Capped<'_>) -> Result<(), Error> {
    loop {
        let line = read_line(conn)?;
        let text = String::from_utf8_lossy(&line);
        let size_hex = text.trim().split(';').next().unwrap_or("").trim().to_string();
        let size = usize::from_str_radix(&size_hex, 16).map_err(|_| Error::Transport(format!("bad chunk size {size_hex:?}")))?;
        if size == 0 {
            while read_line(conn)? != b"\r\n" {}
            return Ok(());
        }
        read_exact_n(conn, size, out)?;
        if read_line(conn)? != b"\r\n" {
            return Err(Error::Transport("a chunk longer than its stated size".into()));
        }
    }
}

/// A body read to its end, however the reply frames it, into `out`; whether the
/// connection can be kept.
fn read_body(conn: &mut Conn, head: &Head, out: &mut Capped<'_>) -> Result<bool, Error> {
    let chunked = head.get("transfer-encoding").map(|v| v.to_lowercase().contains("chunked")).unwrap_or(false);
    let length: Option<usize> = head.get("content-length").and_then(|v| v.trim().parse().ok());
    if chunked {
        read_chunked(conn, out)?;
    } else if let Some(n) = length {
        read_exact_n(conn, n, out)?;
    } else {
        read_to_close(conn, out)?;
    }
    Ok(head.get("connection").map(|v| !v.to_lowercase().contains("close")).unwrap_or(true) && (chunked || length.is_some()))
}

/// The body as the server meant it, undoing the content coding its
/// `Content-Encoding` header names (RFC 9110 §8.4). The body is never judged by
/// its own bytes: a file that is itself gzip (a `.tar.gz` release archive) is
/// returned as it came. A coding that cannot be undone is the error, never the
/// raw bytes passed off as the body.
/// The body as sent, its content coding undone, at most `ceiling` bytes once
/// undone: a small coded body can inflate far past what it may be, and is stopped
/// as it is undone, never after.
fn decoded(raw: Vec<u8>, headers: &[(String, String)], ceiling: u64) -> Result<Vec<u8>, Error> {
    let coding = headers.iter().find(|(k, _)| k == "content-encoding").map(|(_, v)| v.trim().to_ascii_lowercase()).unwrap_or_default();
    // Ok(None): it does not decode as `name`
    let undo = |reader: &mut dyn Read| -> Result<Option<Vec<u8>>, Error> {
        let mut out = Vec::new();
        if reader.take(ceiling.saturating_add(1)).read_to_end(&mut out).is_err() {
            return Ok(None);
        }
        if out.len() as u64 > ceiling {
            return Err(Error::Transport(format!("an answer larger than the {ceiling} bytes it may be once decoded")));
        }
        Ok(Some(out))
    };
    let not = |name: &str| Error::Transport(format!("a {name} body that does not decode"));
    match coding.as_str() {
        "" | "identity" => Ok(raw),
        "gzip" | "x-gzip" => undo(&mut flate2::read::GzDecoder::new(&raw[..]))?.ok_or_else(|| not("gzip")),
        // HTTP's deflate is zlib-wrapped (RFC 9110 §8.4.1.2); some servers send the bare stream
        "deflate" => match undo(&mut flate2::read::ZlibDecoder::new(&raw[..]))? {
            Some(out) => Ok(out),
            None => undo(&mut flate2::read::DeflateDecoder::new(&raw[..]))?.ok_or_else(|| not("deflate")),
        },
        other => Err(Error::Transport(format!("a body in a coding not asked for: {other}"))),
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

/// Requests this process has tried to send off the machine (to any host but
/// loopback), whether or not they got through: what a test that must never touch
/// the network asserts is zero.
static OUTBOUND: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether every request is named as it leaves (`BAGHOLDER_LOG_REQUESTS=1`): a
/// client whose requests share one path names what each asks for beside it.
pub fn logging_requests() -> bool {
    static LOGGED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOGGED.get_or_init(|| crate::switch::switch_on("BAGHOLDER_LOG_REQUESTS"))
}

/// Names a call a client makes to a service a test stands in for (Wealthsimple),
/// as it is made, when every request is named (`BAGHOLDER_LOG_REQUESTS=1`),
/// beside the `outbound` line its request makes: what a test checks the stand-in
/// answered, so a call sent anywhere else is seen (briefs 39, 41).
pub fn named_call(service: &str, what: &str) {
    if logging_requests() {
        eprintln!("{service} {what}");
    }
}

/// How many requests this process has tried to send off the machine.
pub fn outbound_requests() -> usize {
    OUTBOUND.load(std::sync::atomic::Ordering::SeqCst)
}

/// What a request refused by `BAGHOLDER_OFFLINE` says: nothing was asked of
/// anyone, so it tells nothing of the source it was meant for.
pub const OFFLINE: &str = "offline: BAGHOLDER_OFFLINE is set";

/// Whether the process was told to stay off the network (`BAGHOLDER_OFFLINE`).
pub fn offline() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| crate::switch::switch_on("BAGHOLDER_OFFLINE"))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Whether a request to `host` is one `BAGHOLDER_OFFLINE` refuses.
pub fn refused_offline(host: &str) -> bool {
    offline() && !is_loopback(host)
}

/// The one door every request goes through as it is made, whichever way it is sent
/// (this client, or the browser helper through `Net`): it is counted when it is for
/// another machine, named when `BAGHOLDER_LOG_REQUESTS=1`, and refused when the
/// process is offline, named first. How "nothing is asked for while nobody is
/// looking", and "nothing goes to a host but the stand-in", are checked on a running
/// app. The query is left out, so nothing a URL carries reaches a log.
pub fn leave(method: &str, url: &str) -> Result<(), Error> {
    leaving(method, &parse_url(url)?)
}

thread_local! {
    /// What runs on this thread as a request actually leaves (`while_leaving`).
    static ON_LEAVING: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// `work`, with `sent` run each time one of its requests on this thread actually
/// leaves through the door: never for one the door refuses (offline), nor for one
/// refused before it reaches the door (a host resting, nothing to send it with).
/// How "under way" is said only of what was asked of someone (#534).
pub fn while_leaving<T>(sent: impl Fn() + 'static, work: impl FnOnce() -> T) -> T {
    // the hook before is put back however `work` ends, a panic too, so a pooled
    // thread never carries this one into the next work it runs
    struct Restore(Option<Box<dyn Fn()>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let was = self.0.take();
            ON_LEAVING.with(|h| *h.borrow_mut() = was);
        }
    }
    let _restore = Restore(ON_LEAVING.with(|h| h.borrow_mut().replace(Box::new(sent))));
    work()
}

fn leaving(method: &str, u: &Url) -> Result<(), Error> {
    if !is_loopback(&u.host) {
        OUTBOUND.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    let refused = refused_offline(&u.host);
    if logging_requests() {
        // the port where it is not the scheme's own, so two servers on one host are told apart
        let port = if u.port == if u.tls { 443 } else { 80 } { String::new() } else { format!(":{}", u.port) };
        eprintln!("outbound {} {}{}{}{}", method, u.host, port, u.path.split('?').next().unwrap_or(""), if refused { " (refused: offline)" } else { "" });
    }
    if refused {
        return Err(Error::Offline);
    }
    ON_LEAVING.with(|h| {
        if let Some(sent) = h.borrow().as_ref() {
            sent();
        }
    });
    Ok(())
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
    send(method, url, headers, body, timeout, false, false)
}

/// The same request, answering an HTTP error with its response rather than
/// failing -- for a caller that reads what an error page sets, such as its
/// cookies.
pub fn request_any(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, timeout: Duration) -> Result<Response, Error> {
    send(method, url, headers, body, timeout, true, false)
}

/// A request that must reach its host at most once: one that places, cancels or
/// changes an order. It goes out on a fresh connection, is never sent a second time
/// (a pooled connection's failed read may still have been received, RFC 9110
/// §9.2.2), and follows no redirect; an answer it does not get is an error the
/// caller settles by asking what became of it, never by sending again. An HTTP
/// error answers with its response, as `request_any`.
pub fn request_once(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, timeout: Duration) -> Result<Response, Error> {
    send(method, url, headers, body, timeout, true, true)
}

fn send(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
    lenient: bool,
    once: bool,
) -> Result<Response, Error> {
    send_into(method, url, headers, body, timeout, lenient, once, None).map(|(r, _)| r)
}

/// One `GET` of an https `url` from the address `to` alone: the name is never
/// resolved again, so a second answer for it cannot move the request elsewhere; it
/// is checked for TLS and sent as `Host`. Nothing is pooled, no proxy is used, and
/// no redirect is followed (the redirect is the answer). The body is at most
/// `ceiling` bytes, and the whole of it, from connecting to the last byte, takes at
/// most `timeout` on `clock`. For a fetch whose address was checked before it was made: an
/// agent's metadata document, and the agents' address asked from outside
/// (`docs/plans/agents-from-anywhere.md`, §2.1 and §4.4).
pub fn get_at(clock: &std::sync::Arc<dyn crate::Clock>, to: std::net::SocketAddr, url: &str, headers: &[(&str, &str)], timeout: Duration, ceiling: u64) -> Result<Response, Error> {
    let u = parse_url(url)?;
    if !u.tls {
        return Err(Error::Transport(format!("{url} is not https")));
    }
    leaving("GET", &u)?;
    let at = jiff::SignedDuration::try_from(timeout).ok().and_then(|d| clock.now().checked_add(d).ok()).ok_or_else(|| Error::Transport(format!("{timeout:?} is no deadline")))?;
    let tcp = TcpStream::connect_timeout(&to, timeout).map_err(transport)?;
    tcp.set_write_timeout(Some(timeout)).map_err(transport)?;
    #[cfg(test)]
    let plain = PLAIN.with(|p| p.get());
    #[cfg(not(test))]
    let plain = false;
    let mut c = if plain {
        Conn::Plain(tcp)
    } else {
        Conn::TlsUntil(Box::new(connector()?.connect(&u.host, Until { tcp, clock: clock.clone(), at }).map_err(|e| Error::Transport(e.to_string()))?))
    };
    let host_header = if u.port == 443 { u.host.clone() } else { format!("{}:{}", u.host, u.port) };
    let mut req = format!("GET {} HTTP/1.1\r\nHost: {}\r\nAccept-Encoding: identity\r\nConnection: close\r\n", u.path, host_header);
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    c.write_all(req.as_bytes()).and_then(|_| c.flush()).map_err(transport)?;
    answer_of(&mut c, ceiling)
}

#[cfg(test)]
thread_local! {
    /// In a test, `get_at` speaks plain HTTP to its address, so the answer it reads
    /// is the one under test and not a handshake no test peer can make.
    static PLAIN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The answer on `c` once its request is sent (`get_at`): its head, and its body
/// held to `ceiling` as it arrives and again once decoded.
fn answer_of(c: &mut Conn, ceiling: u64) -> Result<Response, Error> {
    let head = read_head(c).and_then(|(raw, _)| parse_head(&raw))?;
    let mut raw: Vec<u8> = Vec::new();
    read_body(c, &head, &mut Capped::new(&mut raw, ceiling))?;
    Ok(Response { status: head.status, body: decoded(raw, &head.headers, ceiling)?, headers: head.headers.clone() })
}

/// Where a download's body goes: the file, how large it may be, and the hosts it
/// may come from, asked of every hop of a redirect.
struct ToFile<'a> {
    file: &'a mut std::fs::File,
    ceiling: u64,
    allow: &'a dyn Fn(&str) -> bool,
    progress: &'a mut dyn FnMut(u64) -> bool,
}

/// A download's file, telling `progress` the bytes written after each piece; a
/// `false` from it stops the download there.
struct Told<'a> {
    file: &'a mut std::fs::File,
    written: u64,
    progress: &'a mut dyn FnMut(u64) -> bool,
}

impl Write for Told<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        self.written += n as u64;
        if !(self.progress)(self.written) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "stopped"));
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// A file fetched to disk as it arrives, never held in memory: at most `ceiling`
/// bytes, from hosts `allow` takes on every hop of a redirect, with no content
/// coding to undo. `progress` is told the bytes written as they arrive, and a
/// `false` from it stops the download. The bytes written, or why it stopped (a part
/// written is the caller's to remove).
pub fn download(url: &str, headers: &[(&str, &str)], timeout: Duration, file: &mut std::fs::File, ceiling: u64, allow: &dyn Fn(&str) -> bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<u64, Error> {
    let mut to = ToFile { file, ceiling, allow, progress };
    let (_, written) = send_into("GET", url, headers, None, timeout, false, false, Some(&mut to))?;
    Ok(written.unwrap_or(0))
}

#[allow(clippy::too_many_arguments)]
fn send_into(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
    lenient: bool,
    once: bool,
    mut to_file: Option<&mut ToFile<'_>>,
) -> Result<(Response, Option<u64>), Error> {
    let mut url = url.to_string();
    for _ in 0..=REDIRECT_MAX {
        let u = parse_url(&url)?;
        let key = format!("{}:{}", u.host, u.port);
        // BAGHOLDER_OFFLINE=1: nothing leaves this machine. The browser tests run the
        // real server on a made-up book this way, so they read the same on any
        // machine and ask nothing of anyone.
        if let Some(t) = to_file.as_ref() {
            if !(t.allow)(&u.host) {
                return Err(Error::Transport(format!("{} is not a host this file is fetched from", u.host)));
            }
        }
        leaving(method, &u)?;

        // the port is part of the Host whenever it is not the scheme's own: a
        // server that builds URLs from it (DevTools does) needs it
        let host_header = if u.port == if u.tls { 443 } else { 80 } { u.host.clone() } else { format!("{}:{}", u.host, u.port) };
        let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", method, u.path, host_header);
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
            // the second attempt is always on a fresh connection: another kept one
            // could have been closed by the host too
            let pooled = if once || attempt > 0 {
                None
            } else {
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

        let redirect = (301..=308).contains(&head.status) && !once && head.get("location").is_some();
        let mut raw: Vec<u8> = Vec::new();
        let mut written: Option<u64> = None;
        let keep = match to_file.as_mut() {
            // a download's own body goes to its file, as it is: a coding is not undone there
            Some(t) if !redirect && head.status < 400 => {
                let coding = head.get("content-encoding").map(|v| v.trim().to_ascii_lowercase()).unwrap_or_default();
                if !coding.is_empty() && coding != "identity" {
                    return Err(Error::Transport(format!("a file sent in a coding not asked for: {coding}")));
                }
                let mut told = Told { file: &mut *t.file, written: 0, progress: &mut *t.progress };
                let mut sink = Capped::new(&mut told, t.ceiling);
                let keep = read_body(&mut c, &head, &mut sink)?;
                written = Some(sink.written);
                keep
            }
            _ => read_body(&mut c, &head, &mut Capped::new(&mut raw, u64::MAX))?,
        };
        if keep {
            let mut p = pool().lock().unwrap();
            let v = p.entry(key).or_default();
            if v.len() < POOL_PER_HOST {
                v.push(c);
            }
        }

        if (301..=308).contains(&head.status) && !once {
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
        // a download's body is in its file: the answer carries the bytes written
        let body = if written.is_some() { Vec::new() } else { decoded(raw, &head.headers, u64::MAX)? };
        return Ok((Response { status: head.status, body, headers: head.headers.clone() }, written));
    }
    Err(Error::Transport("too many redirects".into()))
}

#[cfg(test)]
mod leaving_tests {
    use super::{leave, while_leaving};
    use std::{cell::Cell, rc::Rc};

    /// The hook `while_leaving` sets is put back however its work ends, a panic too,
    /// so a pooled thread that caught the panic and runs on never says a later
    /// request was sent for work that has gone (#534).
    #[test]
    fn a_panic_in_the_work_leaves_no_hook_behind() {
        let said = Rc::new(Cell::new(0));
        let s = said.clone();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            while_leaving(move || s.set(s.get() + 1), || panic!("the work failed"))
        }));
        assert!(caught.is_err());
        // a request on the same thread, after: no hook of the work that panicked runs
        assert!(leave("GET", "http://127.0.0.1:9/after").is_ok());
        assert_eq!(said.get(), 0, "the hook outlived its work");
        // and one set around another work's request runs as before, nested ones too
        let outer = Rc::new(Cell::new(0));
        let inner = Rc::new(Cell::new(0));
        let (o, i) = (outer.clone(), inner.clone());
        while_leaving(move || o.set(o.get() + 1), || {
            while_leaving(move || i.set(i.get() + 1), || leave("GET", "http://127.0.0.1:9/inner").unwrap());
            leave("GET", "http://127.0.0.1:9/outer").unwrap();
        });
        assert_eq!((outer.get(), inner.get()), (1, 1));
    }
}

#[cfg(test)]
mod coding_tests {
    use super::{answer_of, decoded, Conn};
    use std::io::Write;

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }
    fn h(coding: &str) -> Vec<(String, String)> {
        vec![("content-encoding".to_string(), coding.to_string())]
    }

    /// A body is decoded by the coding the server names, never by its own bytes:
    /// a file that is itself gzip (a release archive) comes back as it was sent.
    #[test]
    fn a_body_is_decoded_by_its_content_encoding_alone() {
        let file = gz(b"an archive's own bytes");
        assert_eq!(decoded(file.clone(), &[], u64::MAX).unwrap(), file, "a .tar.gz sent with no coding is the file itself");
        assert_eq!(decoded(file.clone(), &h("identity"), u64::MAX).unwrap(), file);
        assert_eq!(decoded(gz(b"text"), &h("gzip"), u64::MAX).unwrap(), b"text");
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(b"text").unwrap();
        assert_eq!(decoded(z.finish().unwrap(), &h("deflate"), u64::MAX).unwrap(), b"text");
        let mut d = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        d.write_all(b"text").unwrap();
        assert_eq!(decoded(d.finish().unwrap(), &h("deflate"), u64::MAX).unwrap(), b"text", "the bare stream some servers send");
        // what cannot be undone is the error, never the raw bytes
        assert!(decoded(b"not gzip".to_vec(), &h("gzip"), u64::MAX).is_err());
        assert!(decoded(b"x".to_vec(), &h("br"), u64::MAX).is_err());
    }

    /// A small coded body that inflates past its ceiling is refused as it is
    /// undone: a megabyte of zeros sent as about a kilobyte of gzip, held to 8 KiB.
    #[test]
    fn a_coded_body_is_held_to_its_ceiling_once_decoded() {
        // and in the answer `get_at` reads, over a socket (brief 80's re-review, 5)
        let bomb = gz(&vec![0u8; 1 << 20]);
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let to = l.local_addr().unwrap();
        let sent = bomb.clone();
        let peer = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let _ = std::io::Write::write_all(&mut s, format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n", sent.len()).as_bytes());
            let _ = std::io::Write::write_all(&mut s, &sent);
        });
        let mut c = Conn::Plain(std::net::TcpStream::connect(to).unwrap());
        let said = answer_of(&mut c, 8 * 1024).err().expect("refused once decoded");
        assert!(said.to_string().contains("larger than the 8192 bytes"), "{said}");
        peer.join().unwrap();
        let bomb = gz(&vec![0u8; 1 << 20]);
        assert!(bomb.len() < 8 * 1024, "{}", bomb.len());
        let said = decoded(bomb.clone(), &h("gzip"), 8 * 1024).err().expect("refused");
        assert!(said.to_string().contains("larger than the 8192 bytes"), "{said}");
        assert_eq!(decoded(gz(b"text"), &h("gzip"), 4).unwrap(), b"text", "at the ceiling exactly");
        assert!(decoded(gz(b"texts"), &h("gzip"), 4).is_err());
    }

    /// `get_at` holds what it reads to the ceiling it is given: a coded body that
    /// inflates past it is refused, and one within it is read whole (brief 80,
    /// round 3, 5).
    #[test]
    fn get_at_holds_its_answer_to_its_own_ceiling() {
        let peer = |body: Vec<u8>| {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let to = l.local_addr().unwrap();
            let t = std::thread::spawn(move || {
                let (mut s, _) = l.accept().unwrap();
                // the request read whole first: a socket closed with bytes unread
                // resets the connection, and macOS then drops the answer unread
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && std::io::Read::read(&mut s, &mut b).unwrap() == 1 {
                    head.push(b[0]);
                }
                let _ = std::io::Write::write_all(&mut s, format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes());
                let _ = std::io::Write::write_all(&mut s, &body);
            });
            (to, t)
        };
        let clock: std::sync::Arc<dyn crate::Clock> = std::sync::Arc::new(crate::SystemClock);
        super::PLAIN.with(|p| p.set(true));
        let (to, t) = peer(gz(&vec![0u8; 1 << 20]));
        let said = super::get_at(&clock, to, "https://localhost/doc", &[], std::time::Duration::from_secs(5), 8 * 1024).err().expect("refused once decoded");
        t.join().unwrap();
        let (to, t) = peer(gz(b"a document"));
        let read = super::get_at(&clock, to, "https://localhost/doc", &[], std::time::Duration::from_secs(5), 8 * 1024);
        t.join().unwrap();
        super::PLAIN.with(|p| p.set(false));
        assert!(said.to_string().contains("larger than the 8192 bytes"), "{said}");
        assert_eq!(read.unwrap().body, b"a document");
    }
}

#[cfg(test)]
mod ceiling_tests {
    use super::download;
    use std::io::{Read, Write};
    use std::time::Duration;

    /// A host on 127.0.0.1 that answers every request with `head` then `body_bytes`
    /// bytes of `x` (chunked when `head` says so), counting what it was able to send.
    fn host(head: &'static str, body_bytes: usize, chunked: bool) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let s2 = sent.clone();
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf);
                let _ = c.write_all(head.as_bytes());
                let chunk = vec![b'x'; 16 * 1024];
                let mut left = body_bytes;
                while left > 0 {
                    let n = left.min(chunk.len());
                    let ok = if chunked { c.write_all(format!("{n:x}\r\n").as_bytes()).and_then(|_| c.write_all(&chunk[..n])).and_then(|_| c.write_all(b"\r\n")) } else { c.write_all(&chunk[..n]) };
                    if ok.is_err() {
                        break;
                    }
                    s2.fetch_add(n, std::sync::atomic::Ordering::SeqCst);
                    left -= n;
                }
                if chunked {
                    let _ = c.write_all(b"0\r\n\r\n");
                }
            }
        });
        (port, sent)
    }

    #[test]
    fn a_download_past_its_ceiling_stops_while_it_is_read_and_never_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.part");
        // a reply that does not say its length, in chunks, far past the ceiling
        let (port, _) = host("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n", 4 * 1024 * 1024, true);
        let mut f = std::fs::File::create(&path).unwrap();
        let e = download(&format!("http://127.0.0.1:{port}/f"), &[], Duration::from_secs(10), &mut f, 100 * 1024, &|_| true, &mut |_| true).unwrap_err();
        assert!(e.to_string().contains("larger than"), "{e}");
        assert!(std::fs::metadata(&path).unwrap().len() <= 100 * 1024, "nothing past the ceiling is written");
        // one that announces a length past the ceiling: refused before any of it is read
        let (port, _) = host("HTTP/1.1 200 OK\r\nContent-Length: 4194304\r\nConnection: close\r\n\r\n", 4 * 1024 * 1024, false);
        let mut f = std::fs::File::create(&path).unwrap();
        assert!(download(&format!("http://127.0.0.1:{port}/f"), &[], Duration::from_secs(10), &mut f, 100 * 1024, &|_| true, &mut |_| true).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        // within it: written whole, counted
        let (port, _) = host("HTTP/1.1 200 OK\r\nContent-Length: 50000\r\nConnection: close\r\n\r\n", 50_000, false);
        let mut f = std::fs::File::create(&path).unwrap();
        assert_eq!(download(&format!("http://127.0.0.1:{port}/f"), &[], Duration::from_secs(10), &mut f, 100 * 1024, &|_| true, &mut |_| true).unwrap(), 50_000);
    }

    /// A peer that answers a byte at a time, each well within a socket's wait, is
    /// still cut off once the whole fetch has taken its time.
    #[test]
    fn a_fetch_from_a_checked_address_ends_at_its_deadline_however_slowly_the_peer_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let to = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                // a TLS handshake record announcing 1 KiB, then its bytes one by one
                let _ = s.write_all(&[0x16, 0x03, 0x03, 0x04, 0x00]);
                for _ in 0..1024 {
                    std::thread::sleep(Duration::from_millis(50));
                    if s.write_all(&[0]).is_err() {
                        break;
                    }
                }
            }
        });
        let clock: std::sync::Arc<dyn crate::Clock> = std::sync::Arc::new(crate::SystemClock);
        let started = clock.now();
        assert!(super::get_at(&clock, to, "https://localhost/doc", &[], Duration::from_millis(500), 1024).is_err());
        let took = clock.now().duration_since(started);
        assert!(took < jiff::SignedDuration::from_secs(3), "{took:?}");
    }

    #[test]
    fn a_download_redirected_to_a_host_it_does_not_take_goes_no_further() {
        let (port, _) = host("HTTP/1.1 302 Found\r\nLocation: http://elsewhere.example/f\r\nContent-Length: 0\r\n\r\n", 0, false);
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f")).unwrap();
        let e = download(&format!("http://127.0.0.1:{port}/f"), &[], Duration::from_secs(10), &mut f, 1024, &|h| h == "127.0.0.1", &mut |_| true).unwrap_err();
        assert!(e.to_string().contains("elsewhere.example is not a host"), "{e}");
    }
}

#[cfg(test)]
mod hello {
    use openssl::ssl::SslOptions;

    /// The hello is the same whatever OpenSSL the build links: no padding
    /// extension, which OpenSSL 3.0 adds under `SSL_OP_ALL` and Evolve's site
    /// refuses; the connector's other options stay as they are.
    #[test]
    fn the_hello_carries_no_padding_extension() {
        let options = super::connector_settings().unwrap().options();
        assert!(!options.contains(super::TLSEXT_PADDING));
        assert!(options.contains(SslOptions::NO_COMPRESSION));
        assert!(super::connector().is_ok());
    }
}
