//! What the agents' address fetches from elsewhere, each guarded against being
//! turned on this machine or its network (`docs/plans/agents-from-anywhere.md`,
//! §2.1, brief 73, 2; §4.4, brief 73, 7): an agent's metadata document, whose URL
//! a stranger supplies, and the address itself, asked from outside. A name is
//! resolved once; every answer must be a public unicast address; the request goes
//! to the address checked and no other, its name sent for TLS and `Host`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The most a metadata document may be.
pub const DOCUMENT_LIMIT: u64 = 8 * 1024;
/// How long a fetch may take, from resolving its name to its last byte.
pub const FETCH_LIMIT: Duration = Duration::from_secs(5);

/// Whether `ip` is an address on the public internet: never one of this machine's
/// own, a private network, a link, CGNAT (which holds Tailscale's addresses), a
/// unique- or site-local range, multicast or the unspecified address, nor an IPv4
/// address written as IPv6 that is any of those.
pub fn public(ip: IpAddr) -> bool {
    public_beside(ip, this_machine)
}

#[cfg(test)]
thread_local! {
    /// In a test, the addresses this machine is to have, in place of its own.
    pub(crate) static OWN: std::cell::RefCell<Option<Vec<IpAddr>>> = const { std::cell::RefCell::new(None) };
}

/// The same, with `own` saying which addresses are this machine's.
fn public_beside(ip: IpAddr, own: impl Fn(IpAddr) -> bool) -> bool {
    let range = match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => public_v4(v4),
            None => public_v6(v6),
        },
    };
    range && !own(ip)
}

/// Whether `ip` is one of this machine's own addresses.
pub fn this_machine(ip: IpAddr) -> bool {
    #[cfg(test)]
    if let Some(own) = OWN.with(|o| o.borrow().clone()) {
        return own.contains(&ip);
    }
    own_address(ip)
}

/// On Linux and macOS, an address one of the interfaces has, as the system lists
/// them: a bind tells nothing, as Linux binds any address under `ip_nonlocal_bind`.
#[cfg(unix)]
fn own_address(ip: IpAddr) -> bool {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a list the system allocates, freed below
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return false;
    }
    let mut found = false;
    let mut at = list;
    while !at.is_null() && !found {
        // SAFETY: a node of the list getifaddrs returned, read before freeifaddrs
        let ifa = unsafe { &*at };
        if !ifa.ifa_addr.is_null() {
            // SAFETY: a non-null address of the list, read by the family it states
            let family = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
            let addr = if family == libc::AF_INET {
                // SAFETY: an AF_INET address is a sockaddr_in
                let sin = unsafe { &*ifa.ifa_addr.cast::<libc::sockaddr_in>() };
                Some(IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))))
            } else if family == libc::AF_INET6 {
                // SAFETY: an AF_INET6 address is a sockaddr_in6
                let sin6 = unsafe { &*ifa.ifa_addr.cast::<libc::sockaddr_in6>() };
                Some(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr)))
            } else {
                None
            };
            found = addr == Some(ip);
        }
        at = ifa.ifa_next;
    }
    // SAFETY: the list getifaddrs allocated, freed once and not read after
    unsafe { libc::freeifaddrs(list) };
    found
}

/// On Windows, an address a socket here can be bound to, which an address on none
/// of its interfaces cannot (Windows has no `ip_nonlocal_bind`).
#[cfg(windows)]
fn own_address(ip: IpAddr) -> bool {
    std::net::UdpSocket::bind(SocketAddr::new(ip, 0)).is_ok()
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        // 0.0.0.0/8, "this network"
        || a == 0
        // 192.0.0.0/24, IETF protocol assignments
        || (a == 192 && b == 0 && c == 0)
        // 100.64.0.0/10, CGNAT
        || (a == 100 && (64..128).contains(&b))
        // 198.18.0.0/15, benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4, reserved
        || a >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // fc00::/7, unique-local
        || (first & 0xfe00) == 0xfc00
        // fe80::/10, link-local
        || (first & 0xffc0) == 0xfe80
        // fec0::/10, site-local
        || (first & 0xffc0) == 0xfec0
        // ::ffff:0:0/96 is handled as IPv4; ::/96 (IPv4-compatible) and 64:ff9b::/96 (NAT64) carry IPv4 too
        || ip.segments()[..6] == [0, 0, 0, 0, 0, 0]
        || first == 0x64
        // 2001:db8::/32, documentation
        || (first == 0x2001 && ip.segments()[1] == 0xdb8))
}

/// Whether `host` is a DNS name a browser reads as that name alone: labels of
/// letters, digits and hyphens (none at either end), at least one dot, and a last
/// label a browser does not read as a number; never `localhost` or a name under
/// it. An address written as a host (`127.0.0.1`, `127.1`, `0x7f.1`, `0x7f.0x1`,
/// `127.0x`, `[::1]`) is none.
pub fn dns_name(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        && !labels.last().is_some_and(|l| numeric(l))
        && labels.last() != Some(&"localhost")
}

/// Whether a browser reads a host's last label as a number, and so the host as an
/// address (the WHATWG URL standard's "ends in a number"): all digits, or `0x`
/// followed by hex digits or by nothing.
fn numeric(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    (!l.is_empty() && l.bytes().all(|b| b.is_ascii_digit())) || l.strip_prefix("0x").is_some_and(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// What turns a name into addresses: the system's resolver in use, a stand-in in tests.
pub trait Resolve: Send + Sync {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, String>;
}

/// The system's resolver.
pub struct System;

impl Resolve for System {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, String> {
        use std::net::ToSocketAddrs;
        Ok((host, port).to_socket_addrs().map_err(|e| format!("{host} could not be resolved: {e}"))?.map(|a| a.ip()).collect())
    }
}

/// What makes the one request, within `within` from connecting to its last byte:
/// the network in use, a stand-in in tests (which records what it was asked, and is
/// never asked for a refused address).
pub trait Get: Send + Sync {
    fn get(&self, to: SocketAddr, url: &str, headers: &[(&str, &str)], within: Duration) -> Result<bagholder_net::client::Response, String>;
}

/// The machine's clock, which a guarded fetch's deadline is kept on.
fn machine_clock() -> std::sync::Arc<dyn bagholder_net::Clock> {
    std::sync::Arc::new(bagholder_net::SystemClock)
}

/// The network.
pub struct Net;

impl Get for Net {
    fn get(&self, to: SocketAddr, url: &str, headers: &[(&str, &str)], within: Duration) -> Result<bagholder_net::client::Response, String> {
        bagholder_net::client::get_at(&machine_clock(), to, url, headers, within, DOCUMENT_LIMIT + 1).map_err(|e| e.to_string())
    }
}

/// An https URL's host and port, refused when it is anything else: no user part,
/// no other scheme.
pub fn https_host(url: &str) -> Result<(String, u16), String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("{url} is not an https address"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(format!("{url} names no host"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => match p.parse::<u16>() {
            Ok(p) if !h.contains(':') || h.starts_with('[') => (h, p),
            _ => (authority, 443),
        },
        _ => (authority, 443),
    };
    Ok((host.trim_matches(['[', ']']).to_ascii_lowercase(), port))
}

/// The address to fetch `url` from: its host resolved once, within `within`, every
/// answer public. What is said never names an address the resolver gave, nor why
/// it failed: a stranger asks this, and the answers can be this machine's network.
pub fn checked(url: &str, resolve: &Arc<dyn Resolve>, within: Duration) -> Result<SocketAddr, String> {
    let (host, port) = https_host(url)?;
    let addrs = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => {
            let (tx, rx) = std::sync::mpsc::channel();
            let (r, h) = (resolve.clone(), host.clone());
            // the system's resolver keeps its own time: past ours, its answer is not waited for
            std::thread::Builder::new().name("agents-resolve".into()).spawn(move || tx.send(r.resolve(&h, port))).map_err(|_| format!("{host} could not be resolved"))?;
            match rx.recv_timeout(within) {
                Ok(Ok(a)) => a,
                Ok(Err(_)) => return Err(format!("{host} could not be resolved")),
                Err(_) => return Err(format!("{host} was not resolved within {} seconds", within.as_secs())),
            }
        }
    };
    let first = *addrs.first().ok_or_else(|| format!("{host} resolves to no address"))?;
    if addrs.iter().any(|a| !public(*a)) {
        return Err(format!("{host} resolves to an address that is not a public address"));
    }
    Ok(SocketAddr::new(first, port))
}

/// An agent's metadata document (the Client ID Metadata Document its `client_id`
/// names): https only, from its checked address alone, no redirect followed, at
/// most `DOCUMENT_LIMIT` bytes as decoded, all within `FETCH_LIMIT`.
pub fn metadata(url: &str, resolve: &Arc<dyn Resolve>, get: &dyn Get) -> Result<serde_json::Value, String> {
    metadata_within(url, resolve, get, FETCH_LIMIT)
}

/// The same, within `within` from resolving the name to the document's last byte.
pub fn metadata_within(url: &str, resolve: &Arc<dyn Resolve>, get: &dyn Get, within: Duration) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    // what the name resolved to, or that it did not, is never said: a stranger asks,
    // and it describes this machine's network (brief 80's re-review, 11)
    let to = checked(url, resolve, within).map_err(|_| format!("{url} was not fetched"))?;
    let left = within.checked_sub(started.elapsed()).filter(|d| !d.is_zero()).ok_or_else(|| format!("{url} was not fetched within {} seconds", within.as_secs()))?;
    // why a connection failed is never said: it can name this machine's network
    let r = get.get(to, url, &[("Accept", "application/json")], left).map_err(|_| format!("{url} could not be fetched"))?;
    if (300..400).contains(&r.status) {
        return Err(format!("{url} redirected, and a metadata document is never followed elsewhere"));
    }
    if r.status != 200 {
        return Err(format!("{url} answered {}", r.status));
    }
    if r.body.len() as u64 > DOCUMENT_LIMIT {
        return Err(format!("{url} is larger than the {DOCUMENT_LIMIT} bytes a metadata document may be"));
    }
    serde_json::from_slice(&r.body).map_err(|e| format!("{url} is not a JSON document: {e}"))
}

/// Everything the address's fetches are made with: in use, the system's resolver
/// and the network; in a test, stand-ins.
#[derive(Clone)]
pub struct Outside {
    pub resolve: std::sync::Arc<dyn Resolve>,
    pub get: std::sync::Arc<dyn Get>,
    /// Public DNS, asked over HTTPS (§4.4): never the system's resolver, which on a
    /// tailnet answers a Funnel name with its tailnet address.
    pub public_dns: std::sync::Arc<dyn Resolve>,
}

impl Outside {
    pub fn network() -> Outside {
        Outside { resolve: std::sync::Arc::new(System), get: std::sync::Arc::new(Net), public_dns: std::sync::Arc::new(PublicDns::network()) }
    }
}

/// The addresses a DNS-over-HTTPS JSON answer (RFC 8427's shape, as 1.1.1.1 and
/// 8.8.8.8 write it) gives: its A (1) and AAAA (28) records; a CNAME's own line is
/// followed by its answer, and is not an address.
pub fn dns_json(body: &[u8]) -> Result<Vec<IpAddr>, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("answered what is not DNS JSON: {e}"))?;
    let mut got = Vec::new();
    for a in v.get("Answer").and_then(|a| a.as_array()).into_iter().flatten() {
        if matches!(a.get("type").and_then(|t| t.as_u64()), Some(1) | Some(28)) {
            if let Some(ip) = a.get("data").and_then(|d| d.as_str()).and_then(|d| d.parse::<IpAddr>().ok()) {
                got.push(ip);
            }
        }
    }
    Ok(got)
}

/// What asks a public DNS server one question: at an address, a URL.
pub type AskDns = Arc<dyn Fn(SocketAddr, &str) -> Result<bagholder_net::client::Response, String> + Send + Sync>;

/// Public DNS over HTTPS: Cloudflare's 1.1.1.1 and Google's 8.8.8.8, both asked,
/// each at its own address, and the answers of either taken.
pub struct PublicDns {
    /// The network in use; recorded answers in tests.
    pub ask: AskDns,
}

impl PublicDns {
    /// Over the network.
    pub fn network() -> PublicDns {
        PublicDns { ask: Arc::new(|to, url| bagholder_net::client::get_at(&machine_clock(), to, url, &[("Accept", "application/dns-json")], FETCH_LIMIT, 64 * 1024).map_err(|e| e.to_string())) }
    }
}

impl Resolve for PublicDns {
    fn resolve(&self, host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
        let mut all: Vec<IpAddr> = Vec::new();
        let mut said = Vec::new();
        for (at, server) in [("1.1.1.1", "cloudflare-dns.com"), ("8.8.8.8", "dns.google")] {
            let to = SocketAddr::new(at.parse().expect("an address"), 443);
            let mut got = Vec::new();
            for ty in ["A", "AAAA"] {
                let url = format!("https://{server}/{}?name={host}&type={ty}", if server == "dns.google" { "resolve" } else { "dns-query" });
                match (self.ask)(to, &url) {
                    Ok(r) if r.status == 200 => got.extend(dns_json(&r.body).map_err(|e| format!("{server} {e}"))?),
                    Ok(r) => said.push(format!("{server} answered {}", r.status)),
                    Err(e) => said.push(format!("{server}: {e}")),
                }
            }
            all.extend(got);
        }
        all.sort();
        all.dedup();
        if all.is_empty() && said.len() == 4 {
            return Err(format!("public DNS could not be asked: {}", said.join("; ")));
        }
        if all.is_empty() {
            return Err(format!("public DNS has no address for {host}"));
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Answers(Vec<IpAddr>);
    impl Resolve for Answers {
        fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
            Ok(self.0.clone())
        }
    }

    fn answers(a: Vec<IpAddr>) -> Arc<dyn Resolve> {
        Arc::new(Answers(a))
    }

    /// Records each address it is asked to reach, and answers as told.
    struct Recorded {
        asked: Mutex<Vec<SocketAddr>>,
        status: u16,
        body: Vec<u8>,
    }
    impl Get for Recorded {
        fn get(&self, to: SocketAddr, _url: &str, _headers: &[(&str, &str)], _within: Duration) -> Result<bagholder_net::client::Response, String> {
            self.asked.lock().unwrap().push(to);
            Ok(bagholder_net::client::Response { status: self.status, body: self.body.clone(), headers: vec![] })
        }
    }

    fn recorded(status: u16, body: &[u8]) -> Recorded {
        Recorded { asked: Mutex::new(vec![]), status, body: body.to_vec() }
    }

    /// A `client_id` resolving into each refused range is refused, and nothing is
    /// fetched: no address is ever connected to (brief 73, 2).
    #[test]
    fn a_client_id_resolving_into_a_refused_range_fetches_nothing() {
        for bad in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "100.101.102.103", "fc00::1", "fd12:3456::1", "::ffff:127.0.0.1", "::1", "0.0.0.0", "224.0.0.1", "fe80::1", "192.0.0.8", "fec0::1"] {
            let ip: IpAddr = bad.parse().unwrap();
            let get = recorded(200, b"{}");
            let why = checked("https://agent.example/client.json", &answers(vec![ip]), FETCH_LIMIT).unwrap_err();
            assert!(why.contains("not a public address"), "{bad}: {why}");
            // and the refusal says only that it was not fetched
            assert_eq!(metadata("https://agent.example/client.json", &answers(vec![ip]), &get).unwrap_err(), "https://agent.example/client.json was not fetched", "{bad}");
            assert!(get.asked.lock().unwrap().is_empty(), "{bad}: something was fetched");
            // and one bad answer among good ones refuses the whole name
            let get = recorded(200, b"{}");
            assert!(metadata("https://agent.example/client.json", &answers(vec!["93.184.215.14".parse().unwrap(), ip]), &get).is_err(), "{bad} beside a public one");
            assert!(get.asked.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn only_a_public_https_document_under_the_limit_is_read() {
        let public: IpAddr = "93.184.215.14".parse().unwrap();
        // not https
        let get = recorded(200, b"{}");
        assert!(metadata("http://agent.example/client.json", &answers(vec![public]), &get).is_err());
        assert!(get.asked.lock().unwrap().is_empty());
        // a redirect is refused, not followed
        let get = recorded(302, b"");
        assert!(metadata("https://agent.example/client.json", &answers(vec![public]), &get).unwrap_err().contains("redirected"));
        assert_eq!(get.asked.lock().unwrap().len(), 1, "one request, to the checked address");
        // over 8 KiB
        let big = format!("{{\"a\":\"{}\"}}", "x".repeat(9000));
        assert!(metadata("https://agent.example/client.json", &answers(vec![public]), &recorded(200, big.as_bytes())).unwrap_err().contains("larger"));
        // a good one, read from the address checked
        let get = recorded(200, br#"{"client_id":"https://agent.example/client.json"}"#);
        assert!(metadata("https://agent.example/client.json", &answers(vec![public]), &get).is_ok());
        assert_eq!(get.asked.lock().unwrap()[0], SocketAddr::new(public, 443));
    }

    #[test]
    fn an_address_named_in_the_url_is_held_to_the_same_rule() {
        let get = recorded(200, b"{}");
        assert!(metadata("https://127.0.0.1/client.json", &answers(vec![]), &get).is_err());
        assert!(metadata("https://[::1]/client.json", &answers(vec![]), &get).is_err());
        assert!(metadata("https://user@agent.example/client.json", &answers(vec![]), &get).is_err());
        assert!(get.asked.lock().unwrap().is_empty());
    }

    /// One of this machine's own addresses is never public, whatever its range:
    /// the address this machine sends from is one.
    #[test]
    fn this_machine_s_own_addresses_are_never_public() {
        let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        // connecting a UDP socket sends nothing: it only picks the interface
        if probe.connect("93.184.215.14:443").is_ok() {
            let own = probe.local_addr().unwrap().ip();
            assert!(this_machine(own), "{own}");
            assert!(!public(own), "{own}");
        }
        assert!(!this_machine("93.184.215.14".parse().unwrap()));
        assert!(public("93.184.215.14".parse().unwrap()));
        // an address in a public range that is this machine's own, as a server's can
        // be: `public` asks, not resting on this machine's address being private
        // (brief 80's re-review, 6)
        let own: IpAddr = "93.184.215.14".parse().unwrap();
        OWN.with(|o| *o.borrow_mut() = Some(vec![own]));
        assert!(!public(own));
        assert!(public("93.184.215.15".parse().unwrap()));
        OWN.with(|o| *o.borrow_mut() = None);
    }

    /// What a refusal says names no address the resolver gave and no reason a
    /// connection failed: a stranger asks, and those name this machine's network.
    #[test]
    fn a_refusal_names_no_address_resolved_and_no_connect_error() {
        // a name resolving to a private address, to none, or not at all: the same words
        let doc = "https://agent.example/client.json";
        for resolved in [answers(vec!["10.0.0.5".parse().unwrap()]), answers(vec![]), answers(vec!["93.184.215.14".parse().unwrap(), "127.0.0.1".parse().unwrap()])] {
            assert_eq!(metadata(doc, &resolved, &recorded(200, b"{}")).unwrap_err(), format!("{doc} was not fetched"));
        }
        struct Refused;
        impl Get for Refused {
            fn get(&self, _to: SocketAddr, _url: &str, _h: &[(&str, &str)], _within: Duration) -> Result<bagholder_net::client::Response, String> {
                Err("connect 93.184.215.14:443: Connection refused (os error 111)".into())
            }
        }
        let said = metadata("https://agent.example/client.json", &answers(vec!["93.184.215.14".parse().unwrap()]), &Refused).unwrap_err();
        assert!(!said.contains("93.184.215.14") && !said.contains("os error"), "{said}");
    }

    /// The deadline covers resolving the name: a resolver that does not answer in
    /// time ends the fetch at the deadline, and nothing is fetched.
    #[test]
    fn the_deadline_covers_resolving_the_name() {
        struct Slow;
        impl Resolve for Slow {
            fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
                std::thread::sleep(Duration::from_millis(1500));
                Ok(vec!["93.184.215.14".parse().unwrap()])
            }
        }
        let get = recorded(200, b"{}");
        let started = Instant::now();
        let said = metadata_within("https://agent.example/client.json", &(Arc::new(Slow) as Arc<dyn Resolve>), &get, Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(1000), "waited {:?}", started.elapsed());
        assert_eq!(said, "https://agent.example/client.json was not fetched");
        assert!(get.asked.lock().unwrap().is_empty());
    }

    /// Public DNS's answers as 1.1.1.1 and 8.8.8.8 wrote them, recorded
    /// 2026-10-27 (`tests/replies/public-dns`): A and AAAA records read, a CNAME's
    /// line passed over, an answer with none giving none (brief 80 §7.2).
    #[test]
    fn public_dns_answers_are_read_as_recorded() {
        let read = |f: &str| dns_json(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/public-dns").join(f)).unwrap()).unwrap();
        let ips = |l: &[&str]| l.iter().map(|a| a.parse::<IpAddr>().unwrap()).collect::<Vec<_>>();
        assert_eq!(read("cloudflare-one.one.one.one-A.json"), ips(&["1.1.1.1", "1.0.0.1"]));
        assert_eq!(read("cloudflare-one.one.one.one-AAAA.json"), ips(&["2606:4700:4700::1111", "2606:4700:4700::1001"]));
        assert_eq!(read("cloudflare-www.github.com-A.json"), ips(&["20.29.134.23"]));
        assert_eq!(read("google-www.github.com-A.json"), ips(&["140.82.114.4"]));
        assert!(read("cloudflare-www.github.com-AAAA.json").is_empty(), "a CNAME alone is no address");
        assert!(read("google-www.github.com-AAAA.json").is_empty());
        assert!(dns_json(b"<html>blocked</html>").is_err());
    }

    /// Public DNS is asked as it is run: each server at its own address and path,
    /// A and AAAA, the answers of both merged, over recorded answers through the
    /// seam for its fetch (brief 80's re-review, 7). Where every question fails it
    /// says so; where they answer with no address it says that.
    #[test]
    fn public_dns_asks_both_servers_both_kinds_and_merges_what_they_answer() {
        let replies = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/public-dns");
        let asked: Arc<std::sync::Mutex<Vec<(SocketAddr, String)>>> = Arc::default();
        let recorded = |answering: bool| -> PublicDns {
            let (asked, replies) = (asked.clone(), replies.clone());
            PublicDns {
                ask: Arc::new(move |to, url| {
                    asked.lock().unwrap().push((to, url.to_string()));
                    if !answering {
                        return Err("unreachable".into());
                    }
                    let server = if url.starts_with("https://dns.google/") { "google" } else { "cloudflare" };
                    let (name, ty) = url.split_once("?name=").and_then(|(_, q)| q.split_once("&type=")).unwrap();
                    let body = std::fs::read(replies.join(format!("{server}-{name}-{ty}.json"))).map_err(|e| e.to_string())?;
                    Ok(bagholder_net::client::Response { status: 200, body, headers: vec![] })
                }),
            }
        };
        let ips = |l: &[&str]| l.iter().map(|a| a.parse::<IpAddr>().unwrap()).collect::<Vec<_>>();
        assert_eq!(recorded(true).resolve("one.one.one.one", 443).unwrap(), ips(&["1.0.0.1", "1.1.1.1", "2606:4700:4700::1001", "2606:4700:4700::1111"]));
        let one: SocketAddr = "1.1.1.1:443".parse().unwrap();
        let eight: SocketAddr = "8.8.8.8:443".parse().unwrap();
        assert_eq!(
            std::mem::take(&mut *asked.lock().unwrap()),
            vec![
                (one, "https://cloudflare-dns.com/dns-query?name=one.one.one.one&type=A".to_string()),
                (one, "https://cloudflare-dns.com/dns-query?name=one.one.one.one&type=AAAA".to_string()),
                (eight, "https://dns.google/resolve?name=one.one.one.one&type=A".to_string()),
                (eight, "https://dns.google/resolve?name=one.one.one.one&type=AAAA".to_string()),
            ]
        );
        // a name whose AAAA answers are a CNAME alone: the A answers of both
        assert_eq!(recorded(true).resolve("www.github.com", 443).unwrap(), ips(&["20.29.134.23", "140.82.114.4"]));
        let said = recorded(false).resolve("one.one.one.one", 443).unwrap_err();
        assert!(said.starts_with("public DNS could not be asked"), "{said}");
    }
}
