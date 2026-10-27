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

/// Whether `ip` is one of this machine's own addresses: one a socket here can be
/// bound to, which an address on none of its interfaces cannot.
pub fn this_machine(ip: IpAddr) -> bool {
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
/// label that is not all digits; never `localhost` or a name under it. An address
/// written as a host (`127.0.0.1`, `127.1`, `0x7f.1`, `[::1]`) is none.
pub fn dns_name(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        && !labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
        && labels.last() != Some(&"localhost")
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
    let to = checked(url, resolve, within)?;
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
        Outside { resolve: std::sync::Arc::new(System), get: std::sync::Arc::new(Net), public_dns: std::sync::Arc::new(PublicDns) }
    }
}

/// Public DNS over HTTPS: Cloudflare's 1.1.1.1 and Google's 8.8.8.8, both asked,
/// each at its own address, and the answers of either taken.
pub struct PublicDns;

impl Resolve for PublicDns {
    fn resolve(&self, host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
        let mut all: Vec<IpAddr> = Vec::new();
        let mut said = Vec::new();
        for (at, server) in [("1.1.1.1", "cloudflare-dns.com"), ("8.8.8.8", "dns.google")] {
            let to = SocketAddr::new(at.parse().expect("an address"), 443);
            let mut got = Vec::new();
            for ty in ["A", "AAAA"] {
                let url = format!("https://{server}/{}?name={host}&type={ty}", if server == "dns.google" { "resolve" } else { "dns-query" });
                match bagholder_net::client::get_at(&machine_clock(), to, &url, &[("Accept", "application/dns-json")], FETCH_LIMIT, 64 * 1024) {
                    Ok(r) if r.status == 200 => {
                        let v: serde_json::Value = serde_json::from_slice(&r.body).map_err(|e| format!("{server} answered what is not DNS JSON: {e}"))?;
                        for a in v.get("Answer").and_then(|a| a.as_array()).into_iter().flatten() {
                            // record types 1 (A) and 28 (AAAA); a CNAME's own line is followed by its answer
                            if matches!(a.get("type").and_then(|t| t.as_u64()), Some(1) | Some(28)) {
                                if let Some(ip) = a.get("data").and_then(|d| d.as_str()).and_then(|d| d.parse::<IpAddr>().ok()) {
                                    got.push(ip);
                                }
                            }
                        }
                    }
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
            let said = metadata("https://agent.example/client.json", &answers(vec![ip]), &get).unwrap_err();
            assert!(said.contains("not a public address"), "{bad}: {said}");
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
        // an address in a public range that is this machine's own, as a server's can be
        let own: IpAddr = "93.184.215.14".parse().unwrap();
        assert!(!public_beside(own, |ip| ip == own));
        assert!(public_beside("93.184.215.15".parse().unwrap(), |ip| ip == own));
    }

    /// What a refusal says names no address the resolver gave and no reason a
    /// connection failed: a stranger asks, and those name this machine's network.
    #[test]
    fn a_refusal_names_no_address_resolved_and_no_connect_error() {
        let said = metadata("https://agent.example/client.json", &answers(vec!["10.0.0.5".parse().unwrap()]), &recorded(200, b"{}")).unwrap_err();
        assert!(!said.contains("10.0.0.5"), "{said}");
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
        assert!(said.contains("not resolved within"), "{said}");
        assert!(get.asked.lock().unwrap().is_empty());
    }
}
