//! Who the page answers (`docs/decisions.md`, 2026-10-30): its own machine, the
//! local network, and a name the person lists for a proxy with a login of its own.
//! Never the internet, unless through the page's address from anywhere, whose gate
//! is `remote`'s.
//!
//! Every request carries the name it asks for (`Host`) and comes from somewhere: the
//! peer that connected, or, through a reverse proxy on the local network, the
//! visitor the proxy names last in `X-Forwarded-For` (the address it saw itself; a
//! browser page can't add that header without the preflight the page refuses). A
//! visitor not on a private network is refused. So is one Bagholder can't tell:
//! Docker's own gateway passing a visit on without the visitor's address.
//!
//! The name asked for is held against DNS rebinding, where a website points its own
//! name at an address on the visitor's network: directly, only this machine's own
//! `127.0.0.1`, a private address, or a name that can't be in public DNS (one word,
//! or `.local`, `.lan`, `.home`, `.internal`, `.home.arpa`, `.localdomain`) is
//! answered. Through a proxy, any name it routes is: an attacker's name has no route.

use std::net::IpAddr;

use axum::http::StatusCode;

/// Why a request was refused, said to whoever sent it in one sentence (`said`).
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a method the page takes.
    Method(StatusCode),
    /// The agents' public name: never the page's.
    Agents,
    /// A name the page doesn't answer directly.
    Name(String),
    /// A visitor on the internet.
    Internet(IpAddr),
    /// Another computer, at an app that answers only its own.
    NotThisComputer(IpAddr),
    /// Where the visit came from can't be told.
    CannotTell(&'static str),
    /// A write that doesn't come from the page itself.
    Write,
}

impl Refused {
    pub fn status(&self) -> StatusCode {
        match self {
            Refused::Method(code) => *code,
            _ => StatusCode::FORBIDDEN,
        }
    }

    /// What the person is told: what Bagholder saw, and what to change.
    pub fn said(&self, port: u16) -> String {
        match self {
            Refused::Method(_) => "Bagholder’s page doesn’t take this kind of request.".into(),
            Refused::Agents => "This is the agents’ address: Bagholder’s page isn’t opened here.".into(),
            Refused::Name(name) => format!("Bagholder doesn’t answer “{name}” directly. Open it at this server’s address on your network, or a local name such as server.local. A reverse proxy can route any name to it if it passes the visitor’s address (X-Forwarded-For)."),
            Refused::Internet(ip) => format!("Bagholder answers only your own network, and this visit came from {ip}, on the internet. To open it from anywhere, turn on Open from anywhere in Bagholder’s menu."),
            Refused::NotThisComputer(ip) => format!("This Bagholder answers only the computer it runs on, and this visit came from {ip}. To open it from another device, turn on Open from anywhere in Bagholder’s menu."),
            Refused::CannotTell(how) => format!("Bagholder can’t tell where this visit came from: {how}. Open it through a reverse proxy that passes the visitor’s address (X-Forwarded-For), or at http://127.0.0.1:{port} on the server itself."),
            Refused::Write => "This change didn’t come from Bagholder’s own page.".into(),
        }
    }
}

/// One request, as the gate reads it.
pub struct Asked<'a> {
    pub write: bool,
    /// The `Host` header, lower case, trimmed.
    pub host: &'a str,
    /// The address that connected.
    pub peer: Option<IpAddr>,
    /// Every `X-Forwarded-For` value, joined; empty when there is none.
    pub forwarded: &'a str,
    /// `sec-fetch-site: same-origin`, or the page's own `X-Bagholder` header.
    pub from_the_page: bool,
    /// Sent in HTTP/1.0, which no browser sends: a proxy passed it on (nginx's
    /// default), so a visitor it doesn't name can't be told from a local one.
    pub old_http: bool,
}

/// What the gate holds a request against.
pub struct Server<'a> {
    /// What the listener is bound to: `127.0.0.1` for an app on the computer, else
    /// (a container) every interface.
    pub bind_host: &'a str,
    pub port: u16,
    /// The names listed for a proxy with a login of its own (`BAGHOLDER_HOSTS`).
    pub listed: &'a [String],
    /// Whether a name is one of the agents' public names.
    pub agents_name: &'a dyn Fn(&str) -> bool,
    /// Docker's gateway inside a container: a visit it passes on shows its address,
    /// not the visitor's.
    pub gateway: Option<IpAddr>,
}

/// Whether `ip` is on this machine or a private network: loopback, the private
/// ranges, Tailscale's and carrier-grade NAT's 100.64/10, link-local, and IPv6's
/// unique-local and link-local (an IPv4 address inside IPv6 judged as itself).
pub fn local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local() || (v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64),
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => local_ip(IpAddr::V4(v4)),
            None => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
        },
    }
}

/// The name in a `Host` value, without its port: `[::1]:8765` gives `::1`.
fn name_of(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("");
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
}

/// The names that can't be in public DNS, so no website can point one at this
/// network: the reserved and the customary private suffixes.
const LOCAL_SUFFIXES: [&str; 6] = [".local", ".lan", ".home", ".internal", ".home.arpa", ".localdomain"];

/// Whether `host` asks for this network directly: a private address, or a name that
/// can't be in public DNS. `localhost` isn't one: the page is opened at `127.0.0.1`.
pub fn local_name(host: &str) -> bool {
    let name = name_of(host).trim_end_matches('.');
    if let Ok(ip) = name.parse::<IpAddr>() {
        return local_ip(ip);
    }
    let label = |l: &str| !l.is_empty() && l.len() <= 63 && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') && !l.starts_with('-') && !l.ends_with('-');
    if name.is_empty() || name.len() > 253 || !name.split('.').all(label) || name == "localhost" || name.ends_with(".localhost") {
        return false;
    }
    // one word, a numeric one aside: no website's name is that
    (!name.contains('.') && !name.bytes().all(|c| c.is_ascii_digit())) || LOCAL_SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// The visitor a proxy names: the last address in `X-Forwarded-For`, the one the
/// nearest proxy added. None when it can't be read.
pub fn forwarded_for(forwarded: &str) -> Option<IpAddr> {
    let last = forwarded.split(',').map(str::trim).rfind(|s| !s.is_empty())?;
    let bare = last.strip_prefix('[').map_or(last, |r| r.split(']').next().unwrap_or(""));
    bare.parse().ok().or_else(|| bare.rsplit_once(':').and_then(|(ip, port)| port.bytes().all(|c| c.is_ascii_digit()).then(|| ip.parse().ok()).flatten()))
}

/// The page's own `127.0.0.1` and its port: in a container, any port, which Docker
/// may publish under another number.
fn own(server: &Server, host: &str) -> bool {
    match host.split_once(':') {
        Some((name, p)) => name == "127.0.0.1" && !p.is_empty() && p.len() <= 5 && p.bytes().all(|c| c.is_ascii_digit()) && (server.bind_host != "127.0.0.1" || p == server.port.to_string()),
        None => false,
    }
}

/// Whether the page answers `asked`, or why not.
pub fn admit(server: &Server, asked: &Asked) -> Result<(), Refused> {
    let host = asked.host;
    if host.is_empty() || host.contains(',') {
        return Err(Refused::Name(host.to_string()));
    }
    // the agents' public name is never the page's, listed or not: Allow and Deny are
    // not reached through the agents' tunnel
    if (server.agents_name)(host) {
        return Err(Refused::Agents);
    }
    let write_ok = || if asked.write && !asked.from_the_page { Err(Refused::Write) } else { Ok(()) };
    // a name listed for a proxy in front with a login of its own, exactly as listed:
    // its visitors are anywhere, and that proxy is on this machine (an app's
    // loopback) or the local network (a container's)
    if server.listed.iter().any(|l| l == host) {
        let loopback = |p: IpAddr| p.is_loopback() || matches!(p, IpAddr::V6(v) if v.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()));
        let peer_here = asked.peer.is_some_and(|p| if server.bind_host == "127.0.0.1" { loopback(p) } else { local_ip(p) });
        return if peer_here { write_ok() } else { Err(Refused::Internet(asked.peer.unwrap_or(IpAddr::from([0, 0, 0, 0])))) };
    }
    // everything else: from this machine or the local network alone; an app on its
    // computer, from that computer alone (a proxy there may bring the local network)
    let peer = asked.peer.ok_or(Refused::CannotTell("no address connected"))?;
    if !local_ip(peer) {
        return Err(Refused::Internet(peer));
    }
    if server.bind_host == "127.0.0.1" && !(peer.is_loopback() || matches!(peer, IpAddr::V6(v) if v.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()))) {
        return Err(Refused::NotThisComputer(peer));
    }
    let proxied = !asked.forwarded.trim().is_empty();
    if !proxied && asked.old_http {
        return Err(Refused::CannotTell("a proxy passed it on without the visitor’s address"));
    }
    let visitor = if proxied {
        forwarded_for(asked.forwarded).ok_or(Refused::CannotTell("the proxy’s X-Forwarded-For names no address"))?
    } else {
        peer
    };
    if !local_ip(visitor) {
        return Err(Refused::Internet(visitor));
    }
    if own(server, host) {
        return write_ok();
    }
    // Docker's gateway, with no visitor named: a visit Docker passed on without its
    // address, from this network or from the internet alike
    if !proxied && server.gateway == Some(peer) {
        return Err(Refused::CannotTell("Docker passed it on without the visitor’s address"));
    }
    if !proxied && !local_name(host) {
        return Err(Refused::Name(name_of(host).to_string()));
    }
    write_ok()
}

/// Docker's gateway, read once: a container's alone (an app on its computer has none
/// that matters, bound to loopback).
pub fn docker_gateway() -> Option<IpAddr> {
    static HELD: std::sync::OnceLock<Option<IpAddr>> = std::sync::OnceLock::new();
    *HELD.get_or_init(gateway)
}

/// Docker's gateway, as the container's routing table names it (`/proc/net/route`'s
/// default route, its gateway in little-endian hex). None outside Linux, or with no
/// default route.
pub fn gateway() -> Option<IpAddr> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    gateway_of(&table)
}

pub fn gateway_of(table: &str) -> Option<IpAddr> {
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() > 2 && f[1] == "00000000").then(|| u32::from_str_radix(f[2], 16).ok()).flatten().filter(|g| *g != 0).map(|g| IpAddr::from(g.to_le_bytes()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: &[String] = &[];

    fn server<'a>(bind_host: &'a str, listed: &'a [String], gateway: Option<IpAddr>) -> Server<'a> {
        Server { bind_host, port: 8765, listed, agents_name: &|h| h == "agents.example.com", gateway }
    }

    fn get<'a>(host: &'a str, peer: [u8; 4], forwarded: &'a str) -> Asked<'a> {
        Asked { write: false, host, peer: Some(IpAddr::from(peer)), forwarded, from_the_page: false, old_http: false }
    }

    const LAN: [u8; 4] = [192, 168, 1, 20];
    const NET: [u8; 4] = [203, 0, 113, 9];
    const PROXY: [u8; 4] = [172, 18, 0, 5];
    const GATEWAY: [u8; 4] = [172, 17, 0, 1];

    /// The addresses and names of this machine and the local network are told from
    /// the internet's, and from those a website could point at this network.
    #[test]
    fn local_addresses_and_names_are_told_from_the_internet_s() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.1", "192.168.1.8", "100.101.102.103", "169.254.1.1", "::1", "fd12::1", "fe80::1", "::ffff:192.168.1.8"] {
            assert!(local_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "203.0.113.9", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!local_ip(ip.parse().unwrap()), "{ip}");
        }
        for host in ["192.168.1.8", "192.168.1.8:8765", "[fd12::1]:8765", "server", "server:8765", "server.local", "nas.lan:443", "box.home", "bagholder.internal", "x.home.arpa", "srv.localdomain"] {
            assert!(local_name(host), "{host}");
        }
        for host in ["8.8.8.8:8765", "evil.example", "evil.example:8765", "bagholder.example.com", "localhost", "localhost:8765", "app.localhost", "1234", "evil.example.local.com", "-bad.lan", ""] {
            assert!(!local_name(host), "{host}");
        }
    }

    /// The visitor a proxy names is the address it added last; one that can't be
    /// read is none.
    #[test]
    fn the_visitor_a_proxy_names_is_the_last_address() {
        assert_eq!(forwarded_for("192.168.1.20"), Some(IpAddr::from(LAN)));
        assert_eq!(forwarded_for("8.8.8.8, 192.168.1.20"), Some(IpAddr::from(LAN)), "a client's own claim first, the proxy's last");
        assert_eq!(forwarded_for("192.168.1.20, 203.0.113.9"), Some(IpAddr::from(NET)));
        assert_eq!(forwarded_for("[fd12::1]:4433"), Some("fd12::1".parse().unwrap()));
        assert_eq!(forwarded_for("192.168.1.20:5555"), Some(IpAddr::from(LAN)));
        assert_eq!(forwarded_for("unknown"), None);
        assert_eq!(forwarded_for(" , "), None);
    }

    /// Directly from the local network: its own addresses and local names; never a
    /// public name (DNS rebinding), never a visitor from the internet.
    #[test]
    fn directly_the_local_network_is_answered_and_the_internet_is_not() {
        let s = server("0.0.0.0", NONE, Some(IpAddr::from(GATEWAY)));
        assert_eq!(admit(&s, &get("192.168.1.8:8765", LAN, "")), Ok(()));
        assert_eq!(admit(&s, &get("server.local:8765", LAN, "")), Ok(()));
        assert_eq!(admit(&s, &get("server:8765", LAN, "")), Ok(()));
        assert_eq!(admit(&s, &get("evil.example:8765", LAN, "")), Err(Refused::Name("evil.example".into())), "a website's own name pointed here");
        assert_eq!(admit(&s, &get("192.168.1.8:8765", NET, "")), Err(Refused::Internet(IpAddr::from(NET))), "a port reached from the internet");
        assert_eq!(admit(&s, &get("127.0.0.1:8765", NET, "")), Err(Refused::Internet(IpAddr::from(NET))), "this machine's own name, sent from the internet");
        assert_eq!(admit(&s, &get("server.local:8765", GATEWAY, "")), Err(Refused::CannotTell("Docker passed it on without the visitor’s address")));
        assert_eq!(admit(&s, &get("127.0.0.1:9000", GATEWAY, "")), Ok(()), "published on the host's own loopback");
    }

    /// Through a proxy on the local network: any name it routes, for a visitor on
    /// the local network; a visitor on the internet refused, routed or not.
    #[test]
    fn through_a_proxy_any_name_it_routes_for_a_local_visitor() {
        let s = server("0.0.0.0", NONE, Some(IpAddr::from(GATEWAY)));
        assert_eq!(admit(&s, &get("bagholder.example.com", PROXY, "192.168.1.20")), Ok(()), "a public domain the proxy routes at home");
        assert_eq!(admit(&s, &get("bagholder.example.com", PROXY, "203.0.113.9")), Err(Refused::Internet(IpAddr::from(NET))), "a route left open to the internet");
        assert_eq!(admit(&s, &get("bagholder.example.com", PROXY, "192.168.1.20, 203.0.113.9")), Err(Refused::Internet(IpAddr::from(NET))), "a claim before the proxy's own entry is not taken");
        assert_eq!(admit(&s, &get("bagholder.lan", GATEWAY, "192.168.1.20")), Ok(()), "a proxy on the host, through Docker's gateway");
        assert_eq!(admit(&s, &get("bagholder.lan", PROXY, "garbage")), Err(Refused::CannotTell("the proxy’s X-Forwarded-For names no address")));
        assert_eq!(admit(&s, &get("bagholder.lan", NET, "192.168.1.20")), Err(Refused::Internet(IpAddr::from(NET))), "a header claimed by a visitor on the internet");
        // nginx's default: HTTP/1.0, the upstream's name, and no visitor named
        let mut nginx = get("bh-app:8765", PROXY, "");
        nginx.old_http = true;
        assert_eq!(admit(&s, &nginx), Err(Refused::CannotTell("a proxy passed it on without the visitor’s address")));
        nginx.forwarded = "192.168.1.20";
        assert_eq!(admit(&s, &nginx), Ok(()), "the same proxy, naming its visitor");
    }

    /// An app on its computer, bound to loopback: its own 127.0.0.1 and port, a proxy
    /// on the computer for a local visitor, and a listed name; nothing else.
    #[test]
    fn an_app_on_its_computer_answers_itself_and_a_proxy_there() {
        let listed = ["home.example.com".to_string()];
        let s = server("127.0.0.1", &listed, None);
        let here = [127, 0, 0, 1];
        assert_eq!(admit(&s, &get("127.0.0.1:8765", here, "")), Ok(()));
        assert_eq!(admit(&s, &get("127.0.0.1:9000", here, "")), Ok(()), "this machine under another port, as a proxy here may name it");
        assert_eq!(admit(&s, &get("localhost:8765", here, "")), Err(Refused::Name("localhost".into())), "opened at 127.0.0.1, as the README says");
        assert_eq!(admit(&s, &get("evil.example:8765", here, "")), Err(Refused::Name("evil.example".into())), "DNS rebinding in the computer's own browser");
        assert_eq!(admit(&s, &get("bagholder.example.com", here, "192.168.1.20")), Ok(()), "a proxy on this computer");
        assert_eq!(admit(&s, &get("home.example.com", here, "203.0.113.9")), Ok(()), "a listed name: the proxy's own login, its visitors anywhere");
        assert_eq!(admit(&s, &get("home.example.com", NET, "")), Err(Refused::Internet(IpAddr::from(NET))), "a listed name, reached past the proxy");
        assert_eq!(admit(&s, &get("127.0.0.1:8765", LAN, "")), Err(Refused::NotThisComputer(IpAddr::from(LAN))), "another computer at an app that answers only its own");
        // in a container: the proxy on the local network, never the internet straight to its port
        let c = server("0.0.0.0", &listed, None);
        assert_eq!(admit(&c, &get("home.example.com", PROXY, "203.0.113.9")), Ok(()));
        assert_eq!(admit(&c, &get("home.example.com", NET, "")), Err(Refused::Internet(IpAddr::from(NET))), "a published port reached from the internet with the listed name");
    }

    /// The rules as before: methods, the agents' name, and writes from the page alone.
    #[test]
    fn a_write_comes_from_the_page_and_the_agents_name_is_never_the_page_s() {
        let s = server("0.0.0.0", NONE, None);
        assert_eq!(admit(&s, &get("agents.example.com", PROXY, "192.168.1.20")), Err(Refused::Agents));
        let mut w = get("192.168.1.8:8765", LAN, "");
        w.write = true;
        assert_eq!(admit(&s, &w), Err(Refused::Write));
        w.from_the_page = true;
        assert_eq!(admit(&s, &w), Ok(()));
        assert_eq!(admit(&s, &get("a,b", LAN, "")), Err(Refused::Name("a,b".into())));
    }

    #[test]
    fn docker_s_gateway_is_read_from_the_routing_table() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\neth0\t00000000\t010011AC\t0003\t0\t0\t0\t00000000\t0\t0\t0\neth0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0\n";
        assert_eq!(gateway_of(table), Some(IpAddr::from([172, 17, 0, 1])));
        assert_eq!(gateway_of("Iface\tDestination\tGateway\n"), None);
    }
}
