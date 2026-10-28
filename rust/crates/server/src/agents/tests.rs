//! The agents' address held to its plan (`docs/plans/agents-from-anywhere.md`,
//! §6): read-only by construction, signed in as the MCP authorization
//! specification says, bounded before sign-in, approved only on the person's own
//! computer, and gone when it is switched off.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode};
use bagholder_core::jiff::{SignedDuration, Timestamp};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::fetch::{Get, Outside, Resolve};
use super::listener::{router, ROUTES};
use super::*;

const HOST: &str = "bagholder.example.ts.net";
const CIMD: &str = "https://agent.example/client.json";
const CALLBACK: &str = "https://agent.example/callback";
// a verifier and its S256 challenge, worked out apart from this code
const VERIFIER: &str = "dBjftJeZ4CVP-mJ0z4ExTCJrd2Pvg5ZjMk86qGm-eJ8";
const CHALLENGE: &str = "W15NQRG56e508qdyi6FrpLycmzoIRN-fBvEBGJkYnvE";

/// Every name resolves to one public address, made up.
struct PublicAnswer(IpAddr);
impl Resolve for PublicAnswer {
    fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
        Ok(vec![self.0])
    }
}

/// Public DNS that cannot be reached.
struct Unreachable;
impl Resolve for Unreachable {
    fn resolve(&self, host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
        Err(format!("public DNS could not be asked for {host}"))
    }
}

/// The agent's metadata document, served for `CIMD`; and every request it is
/// asked to make, recorded.
struct Documents {
    doc: Value,
    asked: Mutex<Vec<(SocketAddr, String)>>,
}
impl Get for Documents {
    fn get(&self, to: SocketAddr, url: &str, _h: &[(&str, &str)], _within: std::time::Duration) -> Result<bagholder_net::client::Response, String> {
        self.asked.lock().unwrap().push((to, url.to_string()));
        Ok(bagholder_net::client::Response { status: 200, body: self.doc.to_string().into_bytes(), headers: vec![] })
    }
}

struct Harness {
    ctx: Arc<Ctx>,
    /// Built once, as the listener builds it: an MCP session lives in its services.
    router: axum::Router,
    clock: Arc<Mutex<Timestamp>>,
    home: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
}

fn harness_with(outside: Outside, read: Arc<dyn Fn() -> Result<crate::mcp::ReadOnly, String> + Send + Sync>) -> Harness {
    crate::tests_common::home();
    let home = tempfile::tempdir().unwrap();
    let clock = Arc::new(Mutex::new("2026-10-20T12:00:00Z".parse::<Timestamp>().unwrap()));
    let path = home.path().to_path_buf();
    let c = clock.clone();
    let ctx = Ctx {
        host: std::sync::RwLock::new(HOST.into()),
        receives: Default::default(),
        store: Arc::new(move || bagholder_book::Book::open_in(&path, crate::app::APP_VERSION, Timestamp::now()).map(|(b, _)| b.agents()).map_err(|e| e.to_string())),
        read,
        now: Arc::new(move || *c.lock().unwrap()),
        outside,
        limits: Default::default(),
        check: Default::default(),
        one_check: Default::default(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let ctx = Arc::new(ctx);
    Harness { router: router(ctx.clone()), ctx, clock, home, rt }
}

fn documents() -> Arc<Documents> {
    Arc::new(Documents { doc: json!({ "client_id": CIMD, "client_name": "Agent Example", "redirect_uris": [CALLBACK] }), asked: Mutex::new(vec![]) })
}

fn harness() -> Harness {
    let docs = documents();
    harness_with(
        Outside { resolve: Arc::new(PublicAnswer("93.184.215.14".parse().unwrap())), get: docs, public_dns: Arc::new(Unreachable) },
        Arc::new(|| Err("no server in this test".to_string())),
    )
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
    fn header(&self, name: &str) -> String {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    }
}

impl Harness {
    fn advance(&self, by: SignedDuration) {
        let mut c = self.clock.lock().unwrap();
        *c = *c + by;
    }

    fn store(&self) -> bagholder_book::agents::AgentsBook {
        self.ctx.store().unwrap()
    }

    /// One request through the listener's router, from `peer`.
    fn ask_from(&self, peer: &str, method: &str, uri: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Answer {
        let mut req = Request::builder().method(method).uri(uri);
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            req = req.header("host", HOST);
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        if !body.is_empty() {
            req = req.header("content-length", body.len().to_string());
        }
        let mut req = req.body(Body::from(body)).unwrap();
        req.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let r = self.router.clone();
        self.rt.block_on(async move {
            let res = r.oneshot(req).await.unwrap();
            let (parts, body) = res.into_parts();
            let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap().to_vec();
            Answer { status: parts.status, headers: parts.headers, body: bytes }
        })
    }

    fn ask(&self, method: &str, uri: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Answer {
        self.ask_from("127.0.0.1:50000", method, uri, headers, body)
    }

    fn get(&self, uri: &str) -> Answer {
        self.ask("GET", uri, &[], vec![])
    }

    fn form(&self, uri: &str, form: &[(&str, &str)]) -> Answer {
        self.ask("POST", uri, &[("content-type", "application/x-www-form-urlencoded")], serde_urlencoded::to_string(form).unwrap().into_bytes())
    }

    fn register(&self, redirect: &str) -> Answer {
        self.ask("POST", "/oauth/register", &[("content-type", "application/json")], json!({ "redirect_uris": [redirect], "client_name": "Totally Claude" }).to_string().into_bytes())
    }

    fn authorize(&self, client: &str, redirect: &str, extra: &[(&str, &str)]) -> Answer {
        let resource = format!("https://{HOST}/mcp");
        let mut q: Vec<(&str, &str)> = vec![("response_type", "code"), ("client_id", client), ("redirect_uri", redirect), ("state", "s1"), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256"), ("resource", &resource)];
        for (k, v) in extra {
            q.retain(|(n, _)| n != k);
            if !v.is_empty() {
                q.push((k, v));
            }
        }
        self.get(&format!("/oauth/authorize?{}", serde_urlencoded::to_string(&q).unwrap()))
    }

    /// The request id the waiting page asks with, and the code it shows.
    fn waiting(&self, page: &Answer) -> (String, String) {
        assert_eq!(page.status, StatusCode::OK, "{}", page.text());
        let text = page.text();
        let id = text.split("var id=\"").nth(1).and_then(|r| r.split('"').next()).expect("the request's id").to_string();
        let code = self.store().waiting(bagholder_core::jiff::Timestamp::MIN).unwrap().into_iter().find(|r| r.id == id).expect("a waiting request").code;
        (id, code)
    }

    /// The whole sign-in, allowed on the computer: the tokens, and the grant.
    fn sign_in(&self) -> (String, String, String) {
        let (id, _) = self.waiting(&self.authorize(CIMD, CALLBACK, &[]));
        approve::answer(&self.store(), &id, true, self.ctx.now()).unwrap();
        let s = self.get(&format!("/oauth/authorize/status?request={id}")).json();
        assert_eq!(s["state"], "allowed", "{s}");
        let redirect = s["redirect"].as_str().unwrap().to_string();
        let code = redirect.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
        let t = self.form("/oauth/token", &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", CALLBACK), ("client_id", CIMD), ("code_verifier", VERIFIER), ("resource", &format!("https://{HOST}/mcp"))]);
        assert_eq!(t.status, StatusCode::OK, "{}", t.text());
        let t = t.json();
        let grant = self.store().grants().unwrap().pop().unwrap().id;
        (t["access_token"].as_str().unwrap().to_string(), t["refresh_token"].as_str().unwrap().to_string(), grant)
    }

    fn mcp(&self, path: &str, bearer: Option<&str>) -> Answer {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } });
        let auth = bearer.map(|b| format!("Bearer {b}"));
        let mut h = vec![("content-type", "application/json"), ("accept", "application/json, text/event-stream")];
        if let Some(a) = auth.as_deref() {
            h.push(("authorization", a));
        }
        self.ask("POST", path, &h, body.to_string().into_bytes())
    }

    /// An MCP session at `path`: initialized, its id.
    fn mcp_session(&self, path: &str, bearer: &str) -> String {
        let a = self.mcp(path, Some(bearer));
        assert_eq!(a.status, StatusCode::OK, "{}", a.text());
        let id = a.header("mcp-session-id");
        assert!(!id.is_empty(), "a session id");
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let r = self.mcp_post(path, bearer, &id, note);
        assert!(r.status.is_success(), "{}", r.status);
        id
    }

    /// One JSON-RPC message in an MCP session.
    fn mcp_post(&self, path: &str, bearer: &str, session: &str, body: Value) -> Answer {
        let auth = format!("Bearer {bearer}");
        let h = [("content-type", "application/json"), ("accept", "application/json, text/event-stream"), ("authorization", auth.as_str()), ("mcp-session-id", session), ("mcp-protocol-version", "2025-06-18")];
        self.ask("POST", path, &h, body.to_string().into_bytes())
    }

    /// A request's status alone, its body never read: a stream that does not end.
    fn status_only(&self, method: &str, uri: &str, headers: &[(&str, &str)]) -> StatusCode {
        let mut req = Request::builder().method(method).uri(uri).header("host", HOST);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
        let r = self.router.clone();
        self.rt.block_on(async move { r.oneshot(req).await.unwrap().status() })
    }

    fn key(&self, way: &str) -> String {
        approve::make_key(&self.store(), &approve::KeyAsked { name: "Grok Bot".into(), way: way.into() }, self.ctx.now()).unwrap().key
    }
}

// ---- read-only ----

/// The router answers exactly the table's paths and methods: every other method
/// on a path is 405, and a path off the table is 404.
#[test]
fn the_route_table_is_exactly_the_planned_one() {
    let h = harness();
    for (path, methods) in ROUTES {
        let path = path.replace("{value}", "x");
        for m in ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
            let a = h.ask(m, &path, &[], vec![]);
            if methods.iter().any(|x| x.as_str() == m) {
                assert!(a.status != StatusCode::NOT_FOUND || path.starts_with("/agents/check") || path.starts_with("/oauth/authorize/status"), "{m} {path} is on the table: {}", a.status);
                assert_ne!(a.status, StatusCode::METHOD_NOT_ALLOWED, "{m} {path}");
            } else {
                assert_eq!(a.status, StatusCode::METHOD_NOT_ALLOWED, "{m} {path} is not on the table");
            }
        }
    }
    assert_eq!(h.get("/anything-else").status, StatusCode::NOT_FOUND);
    // and the router names no path the table does not
    let named = routes_named(include_str!("listener.rs"));
    assert!(!named.is_empty());
    for p in named {
        assert!(ROUTES.iter().any(|(r, _)| *r == p), "the router names {p}, which the table does not");
    }
}

/// Every path a router source names, each `.route(` and `.route_service(` on a line.
fn routes_named(src: &str) -> Vec<&str> {
    src.lines().flat_map(|l| l.split(".route(\"").skip(1).chain(l.split(".route_service(\"").skip(1))).map(|r| r.split('"').next().unwrap()).collect()
}

/// One reachability check at a time: a second waits for the one running, so
/// neither replaces the other's value (brief 80 §5.3).
#[test]
fn a_second_reachability_check_waits_for_the_first() {
    let h = harness();
    let held = h.ctx.one_check.lock().unwrap();
    let ctx = h.ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let t = std::thread::spawn(move || tx.send(tunnel::reach(&ctx, tunnel::Provider::Own)).unwrap());
    assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "it ran beside the one running");
    drop(held);
    assert!(rx.recv_timeout(std::time::Duration::from_secs(30)).is_ok(), "it ran once the first was done");
    t.join().unwrap();
}

/// A tunnel's output that is not UTF-8 is read lossily and never stops the reading
/// (brief 80 §5.4).
#[test]
fn a_tunnel_line_that_is_not_utf8_is_read_and_the_reading_goes_on() {
    let mut lines = vec![];
    tunnel::each_line(&b"first\n\xff\xfe bytes\r\nlast"[..], |l| lines.push(l));
    assert_eq!(lines, vec!["first".to_string(), "\u{FFFD}\u{FFFD} bytes".to_string(), "last".to_string()]);
}

/// The Linux step Funnel needs from a user who is not root is shown until known
/// done, never done before a start is refused; elsewhere there is none (brief 80 §5.6).
#[test]
fn the_operator_step_is_linux_s_alone_and_done_only_for_root() {
    assert_eq!(tunnel::operator_step(false, false), None);
    assert_eq!(tunnel::operator_step(false, true), None);
    assert_eq!(tunnel::operator_step(true, false), Some(false));
    assert_eq!(tunnel::operator_step(true, true), Some(true));
}

/// The tunnel is started through the guard with the variable naming this process
/// passed to it, not only named in a comment (brief 80 §5.7).
#[test]
fn the_tunnel_s_guard_is_given_this_process_to_end_with() {
    let (argv, env) = tunnel::guarded(&["ngrok".into(), "http".into()]).unwrap();
    assert_eq!(&argv[1..], &["tunnel-guard".to_string(), "--".into(), "ngrok".into(), "http".into()]);
    assert_eq!(env, vec![(crate::exit_with::VAR.to_string(), std::process::id().to_string())]);
}

/// The route scan reads every route a line names: a second chained on one line is
/// held to the table too (brief 80 §4.2).
#[test]
fn the_route_scan_reads_every_route_on_a_line() {
    let line = r#"Router::new().route("/a", get(x)).route("/b", get(y)).route_service("/c", z)"#;
    assert_eq!(routes_named(line), vec!["/a", "/b", "/c"]);
}

/// Each provider's gate name is derived from its setup, Tailscale on 8443 with its
/// port (brief 80 §4.2).
#[test]
fn each_provider_s_gate_name_comes_from_its_setup() {
    use super::tunnel::{host_of, Provider, Setup};
    let setup = |provider, name: &str, funnel_port| Setup { provider, name: name.into(), funnel_port, sends_host: None };
    let ts = Some("box.tail1234.ts.net.");
    assert_eq!(host_of(&setup(Provider::Tailscale, "", None), ts).unwrap(), "box.tail1234.ts.net");
    assert_eq!(host_of(&setup(Provider::Tailscale, "", Some(443)), ts).unwrap(), "box.tail1234.ts.net");
    assert_eq!(host_of(&setup(Provider::Tailscale, "", Some(8443)), ts).unwrap(), "box.tail1234.ts.net:8443");
    assert!(host_of(&setup(Provider::Tailscale, "", None), None).is_err(), "no name from Tailscale yet");
    assert_eq!(host_of(&setup(Provider::Cloudflare, "Agents.Example.com", None), None).unwrap(), "agents.example.com");
    assert_eq!(host_of(&setup(Provider::Ngrok, "https://agents.ngrok.app/", None), None).unwrap(), "agents.ngrok.app");
    assert_eq!(host_of(&setup(Provider::Zrok, "mybook", None), None).unwrap(), "mybook.share.zrok.io");
    assert_eq!(host_of(&setup(Provider::Own, "agents.example.com", None), None).unwrap(), "agents.example.com");
}

/// Every page route asked on the agents' listener is 404, GET and POST alike.
#[test]
fn no_agents_route_reaches_the_page() {
    let h = harness();
    let mut paths: Vec<String> = crate::http::route_table().iter().map(|e| e.path.to_string()).collect();
    paths.extend(["/api/events", "/api/events/watch", "/api/notifications/stream", "/api/login/stream", "/", "/index.html"].map(String::from));
    for p in paths {
        assert!(!ROUTES.iter().any(|(r, _)| *r == p), "{p} is a page route and an agents route");
        for m in ["GET", "POST"] {
            assert_eq!(h.ask(m, &p, &[], vec![]).status, StatusCode::NOT_FOUND, "{m} {p} answered on the agents' listener");
        }
    }
}

/// The agents' module holds nothing that writes: no book opened, no app state, no
/// figures, no request but the read client's `GET`, no process outside the tunnel.
#[test]
fn the_agents_module_holds_no_write_handle() {
    let files = [
        ("mod.rs", include_str!("mod.rs")),
        ("listener.rs", include_str!("listener.rs")),
        ("oauth.rs", include_str!("oauth.rs")),
        ("api.rs", include_str!("api.rs")),
        ("approve.rs", include_str!("approve.rs")),
        ("fetch.rs", include_str!("fetch.rs")),
        ("secret.rs", include_str!("secret.rs")),
        ("tools.rs", include_str!("tools.rs")),
        ("tunnel.rs", include_str!("tunnel.rs")),
    ];
    // every file of the module is scanned
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agents")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n != "tests.rs").collect();
    assert_eq!(on_disk, files.iter().map(|(n, _)| n.to_string()).collect(), "a file of the agents' module is not scanned");
    for (name, src) in files {
        let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for word in ["Book::open", "AppState", "Figures", "client::send", "client::request(", "\"POST\"", "\"PUT\"", "\"DELETE\"", "hold_home", "reqwest", "hyper::"] {
            assert!(!code.contains(word), "agents/{name} names {word}");
        }
        if name != "tunnel.rs" {
            assert!(!code.contains("Command"), "agents/{name} starts a process");
        }
    }
}

/// The tools' module makes no request of its own: it reads through `ReadOnly`
/// alone (`tests_mcp`'s scan, over the shared tools).
#[test]
fn the_tools_read_only_through_the_read_client() {
    let code: String = include_str!("tools.rs").lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert_eq!(code.matches("client::").count(), 0);
    for word in ["TcpStream", "browser::Session", "Book::open", "hold_home", "\"POST\""] {
        assert!(!code.contains(word), "tools.rs names {word}");
    }
}

// ---- sign-in ----

#[test]
fn with_no_token_mcp_says_where_to_sign_in() {
    let h = harness();
    let a = h.mcp("/mcp", None);
    assert_eq!(a.status, StatusCode::UNAUTHORIZED);
    assert_eq!(a.header("www-authenticate"), format!("Bearer resource_metadata=\"https://{HOST}/.well-known/oauth-protected-resource\""));
    let pr = h.get("/.well-known/oauth-protected-resource").json();
    assert_eq!(pr["resource"], format!("https://{HOST}/mcp"));
    let asm = h.get("/.well-known/oauth-authorization-server").json();
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(asm["grant_types_supported"], json!(["authorization_code", "refresh_token"]));
    assert_eq!(asm["client_id_metadata_document_supported"], json!(true));
    assert_eq!(asm["registration_endpoint"], format!("https://{HOST}/oauth/register"));
}

/// A sign-in allowed on the computer completes: the token reaches `/mcp`.
#[test]
fn a_request_allowed_on_the_computer_completes() {
    let h = harness();
    let (access, _, _) = h.sign_in();
    let a = h.mcp("/mcp", Some(&access));
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert!(a.text().contains("bagholder"), "{}", a.text());
}

#[test]
fn a_denied_request_does_not() {
    let h = harness();
    let (id, code) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &id, false, h.ctx.now()).unwrap();
    let s = h.get(&format!("/oauth/authorize/status?request={id}")).json();
    assert_eq!(s["state"], "denied");
    assert!(s["redirect"].as_str().unwrap().contains("error=access_denied"));
    for c in [code.as_str(), "anything"] {
        let t = h.form("/oauth/token", &[("grant_type", "authorization_code"), ("code", c), ("redirect_uri", CALLBACK), ("code_verifier", VERIFIER)]);
        assert_eq!(t.status, StatusCode::BAD_REQUEST);
    }
    assert!(h.store().grants().unwrap().is_empty());
}

#[test]
fn an_expired_request_does_not() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    h.advance(SignedDuration::from_mins(11));
    assert!(approve::answer(&h.store(), &id, true, h.ctx.now()).is_err(), "an expired request cannot be allowed");
    assert_eq!(h.get(&format!("/oauth/authorize/status?request={id}")).status, StatusCode::NOT_FOUND);
}

/// The status route answers for its 128-bit id alone (brief 73, 5).
#[test]
fn a_waiting_page_s_polls_never_use_up_the_allowance_before_sign_in() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    for n in 0..3 * BEFORE_SIGN_IN_PER_MINUTE {
        assert_eq!(h.get(&format!("/oauth/authorize/status?request={id}")).status, StatusCode::OK, "poll {n}");
        h.advance(POLL_EVERY);
    }
    assert_eq!(h.get("/.well-known/oauth-protected-resource").status, StatusCode::OK, "the allowance is whole");
    // a poll naming no request is a stranger's call, and spends it
    let mut refused = false;
    for _ in 0..=BEFORE_SIGN_IN_PER_MINUTE {
        refused |= h.get(&format!("/oauth/authorize/status?request={}", secret::request_id())).status == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(refused, "unknown ids are bounded");
}

/// A waiting page asks after its sign-in at most once a `POLL_EVERY`: one that asks
/// sooner is told to wait before the book is opened (brief 80's re-review, 10: with
/// the book moved away, it is still told to wait, not that there is no such
/// sign-in), and the sign-in still answers after.
#[test]
fn a_waiting_page_asking_again_within_the_second_is_told_to_wait() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    let poll = || h.get(&format!("/oauth/authorize/status?request={id}"));
    assert_eq!(poll().status, StatusCode::OK);
    let (home, away) = (h.home.path().to_path_buf(), h.home.path().with_extension("away"));
    std::fs::rename(&home, &away).unwrap();
    let asked = poll().status;
    if home.exists() {
        std::fs::remove_dir_all(&home).unwrap();
    }
    std::fs::rename(&away, &home).unwrap();
    assert_eq!(asked, StatusCode::TOO_MANY_REQUESTS);
    h.advance(POLL_EVERY);
    assert_eq!(poll().json()["state"], "waiting");
}

/// A denial ends its sign-in once its page has read it: asked again, there is none.
#[test]
fn a_denied_sign_in_ends_once_its_page_has_read_the_denial() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &id, false, h.ctx.now()).unwrap();
    let poll = || h.get(&format!("/oauth/authorize/status?request={id}"));
    assert_eq!(poll().json()["state"], "denied");
    h.advance(POLL_EVERY);
    assert_eq!(poll().status, StatusCode::NOT_FOUND);
    assert!(h.store().request(&id).unwrap().is_none(), "nothing kept of it");
}

/// An allowed sign-in's code is minted once: the page asking again is told it is
/// done and is handed no other, and the one handed still exchanges.
#[test]
fn an_allowed_sign_in_s_code_is_minted_once() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &id, true, h.ctx.now()).unwrap();
    let poll = || h.get(&format!("/oauth/authorize/status?request={id}")).json();
    let first = poll();
    assert_eq!(first["state"], "allowed");
    for _ in 0..3 {
        h.advance(POLL_EVERY);
        assert_eq!(poll(), json!({ "state": "done" }), "no second code");
    }
    let code = first["redirect"].as_str().unwrap().split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    let t = h.form("/oauth/token", &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", CALLBACK), ("client_id", CIMD), ("code_verifier", VERIFIER), ("resource", &format!("https://{HOST}/mcp"))]);
    assert_eq!(t.status, StatusCode::OK, "{}", t.text());
    assert!(h.store().request(&id).unwrap().is_none(), "exchanged: it ends");
}

/// A code never exchanged ends `CODE_LIFE` after it was handed: the page asking
/// after it finds none, the code no longer exchanges, and the next sign-in made
/// ends what is left of it.
#[test]
fn an_allowed_sign_in_whose_code_is_never_exchanged_ends() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &id, true, h.ctx.now()).unwrap();
    let first = h.get(&format!("/oauth/authorize/status?request={id}")).json();
    let code = first["redirect"].as_str().unwrap().split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    h.advance(CODE_LIFE + SignedDuration::from_secs(1));
    assert_eq!(h.get(&format!("/oauth/authorize/status?request={id}")).status, StatusCode::NOT_FOUND);
    let t = h.form("/oauth/token", &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", CALLBACK), ("client_id", CIMD), ("code_verifier", VERIFIER), ("resource", &format!("https://{HOST}/mcp"))]);
    assert_eq!(t.status, StatusCode::BAD_REQUEST, "{}", t.text());
    h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    assert!(h.store().request(&id).unwrap().is_none(), "gone at the next sign-in made");
}

#[test]
fn the_status_route_answers_only_for_its_request_id() {
    let h = harness();
    let (id, code) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    let (other, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    assert_ne!(id, other);
    assert_eq!(h.get(&format!("/oauth/authorize/status?request={id}")).json()["state"], "waiting");
    for asked in [code.clone(), code.to_lowercase(), secret::request_id(), "0".repeat(32), String::new()] {
        assert_eq!(h.get(&format!("/oauth/authorize/status?request={asked}")).status, StatusCode::NOT_FOUND, "{asked}");
    }
}

/// Nothing on the listener, given the request's code, yields an authorization code.
#[test]
fn the_public_page_alone_grants_nothing() {
    let h = harness();
    let (_, code) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    for (path, methods) in ROUTES {
        let path = path.replace("{value}", &code);
        for m in methods.iter().map(|m| m.as_str()) {
            for q in [format!("?request={code}"), format!("?code={code}"), format!("?id={code}")] {
                let a = h.ask(m, &format!("{path}{q}"), &[("content-type", "application/x-www-form-urlencoded")], format!("grant_type=authorization_code&code={code}&code_verifier={VERIFIER}&redirect_uri={CALLBACK}").into_bytes());
                let t = a.text();
                assert!(!t.contains("bhcode_") && !t.contains("access_token\":\"bha_"), "{m} {path}{q} gave a code or a token");
            }
        }
    }
    assert!(h.store().grants().unwrap().is_empty());
}

/// Allow and Deny are the main listener's routes: not on the agents' listener.
#[test]
fn allow_and_deny_are_unreachable_through_the_agents_listener() {
    let h = harness();
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    for p in ["/api/agents/requests/allow", "/api/agents/requests/deny", "/api/agents/keys", "/api/agents/cut-off", "/api/agents/switch"] {
        assert_eq!(h.ask("POST", p, &[("content-type", "application/json")], json!({ "id": id }).to_string().into_bytes()).status, StatusCode::NOT_FOUND, "{p}");
        assert!(crate::http::route_table().iter().any(|e| e.path == p && e.method == "post"), "{p} is the page's");
    }
    assert_eq!(h.store().waiting(bagholder_core::jiff::Timestamp::MIN).unwrap().len(), 2 - 1, "still waiting");
}

#[test]
fn tokens_are_refused_for_another_resource_when_expired_and_once_cut_off() {
    let h = harness();
    let (access, _, grant) = h.sign_in();
    assert_eq!(h.mcp("/mcp", Some(&access)).status, StatusCode::OK);
    // a token bound to another resource is refused
    {
        let s = h.store();
        s.add_grant("bhg_other", &s.grants().unwrap()[0].client_id, "x", "x", "https://other.example/mcp", h.ctx.now()).unwrap();
        s.add_token(&secret::hash("bha_other"), false, "bhg_other", h.ctx.now() + ACCESS_LIFE).unwrap();
    }
    let a = h.mcp("/mcp", Some("bha_other"));
    assert_eq!(a.status, StatusCode::UNAUTHORIZED);
    assert!(a.header("www-authenticate").contains("invalid_token"));
    // expired
    h.advance(SignedDuration::from_mins(61));
    assert_eq!(h.mcp("/mcp", Some(&access)).status, StatusCode::UNAUTHORIZED);
    // cut off
    let (access2, _, grant2) = h.sign_in();
    assert_eq!(h.mcp("/mcp", Some(&access2)).status, StatusCode::OK);
    approve::cut_off(&h.store(), &grant2, h.ctx.now()).unwrap();
    assert_eq!(h.mcp("/mcp", Some(&access2)).status, StatusCode::UNAUTHORIZED);
    let _ = grant;
}

/// The token endpoint takes `client_id` and holds it to the client the code or the
/// refresh token was issued to (RFC 6749 §4.1.3): left out, or another's, it is
/// refused (brief 80 §3.3).
#[test]
fn the_token_endpoint_requires_the_client_the_grant_was_issued_to() {
    let h = harness();
    let code_for = |h: &Harness| {
        let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
        approve::answer(&h.store(), &id, true, h.ctx.now()).unwrap();
        let s = h.get(&format!("/oauth/authorize/status?request={id}")).json();
        s["redirect"].as_str().unwrap().split("code=").nth(1).unwrap().split('&').next().unwrap().to_string()
    };
    let resource = format!("https://{HOST}/mcp");
    let exchange = |code: &str, client: Option<&str>| {
        let mut f = vec![("grant_type", "authorization_code"), ("code", code), ("redirect_uri", CALLBACK), ("code_verifier", VERIFIER), ("resource", resource.as_str())];
        if let Some(c) = client {
            f.push(("client_id", c));
        }
        h.form("/oauth/token", &f)
    };
    for client in [None, Some("https://other.example/client.json")] {
        let code = code_for(&h);
        let t = exchange(&code, client);
        assert_eq!((t.status, t.json()["error"].clone()), (StatusCode::BAD_REQUEST, json!("invalid_grant")), "{client:?}");
    }
    let (_, refresh, _) = h.sign_in();
    for client in [None, Some("https://other.example/client.json")] {
        let mut f = vec![("grant_type", "refresh_token"), ("refresh_token", refresh.as_str())];
        if let Some(c) = client {
            f.push(("client_id", c));
        }
        assert_eq!(h.form("/oauth/token", &f).status, StatusCode::BAD_REQUEST, "{client:?}");
    }
    assert_eq!(h.form("/oauth/token", &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", CIMD)]).status, StatusCode::OK);
}

/// A metadata document's client is kept only once the person allows it (brief 80
/// §2.2): refused authorizations, a sign-in waiting, and one denied keep nothing of
/// it; allowed, it is kept under its document's name.
#[test]
fn a_metadata_client_is_kept_only_once_allowed() {
    let h = harness();
    let kept = |h: &Harness| h.store().client(CIMD).unwrap();
    // refused before any request is made: a redirect not its own, no PKCE, another resource
    assert_eq!(h.authorize(CIMD, "https://agent.example/elsewhere", &[]).status, StatusCode::BAD_REQUEST);
    assert_eq!(h.authorize(CIMD, CALLBACK, &[("code_challenge_method", "plain")]).status, StatusCode::BAD_REQUEST);
    assert_eq!(h.authorize(CIMD, CALLBACK, &[("resource", "https://elsewhere.example/mcp")]).status, StatusCode::BAD_REQUEST);
    assert!(kept(&h).is_none());
    // waiting, then denied
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    assert!(kept(&h).is_none(), "a sign-in waiting keeps nothing of its client");
    approve::answer(&h.store(), &id, false, h.ctx.now()).unwrap();
    assert!(kept(&h).is_none(), "nor one denied");
    // allowed
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &id, true, h.ctx.now()).unwrap();
    let c = kept(&h).expect("kept once allowed");
    assert!(c.metadata);
    assert_eq!(c.name, "Agent Example");
}

#[test]
fn a_refresh_token_used_twice_ends_the_grant() {
    let h = harness();
    let (_, refresh, _) = h.sign_in();
    let first = h.form("/oauth/token", &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", CIMD)]);
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let fresh = first.json()["access_token"].as_str().unwrap().to_string();
    assert_eq!(h.mcp("/mcp", Some(&fresh)).status, StatusCode::OK);
    // the old one again: someone else holds it
    let again = h.form("/oauth/token", &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", CIMD)]);
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.json()["error"], "invalid_grant");
    assert_eq!(h.mcp("/mcp", Some(&fresh)).status, StatusCode::UNAUTHORIZED, "the grant ended with it");
    assert!(h.store().grants().unwrap()[0].withdrawn_at.is_some());
}

#[test]
fn pkce_is_s256_and_required() {
    let h = harness();
    for extra in [vec![("code_challenge_method", "plain")], vec![("code_challenge", "")], vec![("code_challenge_method", "")]] {
        let a = h.authorize(CIMD, CALLBACK, &extra);
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{extra:?}");
        assert!(a.text().contains("invalid_request"), "{extra:?}: {}", a.text());
    }
    // and a wrong verifier exchanges nothing
    let (id, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &id, true, h.ctx.now()).unwrap();
    let redirect = h.get(&format!("/oauth/authorize/status?request={id}")).json()["redirect"].as_str().unwrap().to_string();
    let code = redirect.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    let t = h.form("/oauth/token", &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", CALLBACK), ("code_verifier", &"a".repeat(43))]);
    assert_eq!(t.json()["error"], "invalid_grant");
}

/// A redirect URI is matched exactly, and one not registered is never redirected to.
#[test]
fn a_redirect_not_exactly_registered_is_refused_never_redirected() {
    let h = harness();
    let client = h.register("https://agent.example/cb").json()["client_id"].as_str().unwrap().to_string();
    for wrong in ["https://agent.example/cb/", "https://agent.example:8443/cb", "https://agent.example/other", "https://agent.example/c", "https://agent.example/cb?x=1", "https://evil.example/cb"] {
        let a = h.authorize(&client, wrong, &[]);
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{wrong}");
        assert!(a.header("location").is_empty(), "{wrong} was redirected to");
    }
    assert_eq!(h.authorize(&client, "https://agent.example/cb", &[]).status, StatusCode::OK);
}

/// A redirect to this machine, or to a host a browser may read as it, is refused at
/// registration and at authorization (brief 73, 3; brief 80 §3.1). The metadata
/// document lists every one of them, so authorization reaches the redirect check
/// itself rather than the exact match (brief 80 §3.2).
#[test]
fn a_redirect_that_is_not_a_public_host_is_refused_everywhere() {
    let refused = [
        "http://localhost/cb", "http://localhost:33418/cb", "http://foo.localhost/cb", "http://127.0.0.2/cb", "http://[::1]/cb",
        "https://localhost/cb", "https://127.0.0.1/cb", "https://[::1]/cb", "https://app.localhost/cb",
        "https://2130706433/cb", "https://127.1/cb", "https://0x7f.1/cb", "https://localhost./cb", "https://0.0.0.0/cb",
        "https://0x7f.0x1/cb", "https://0x7f.0.0.0x1/cb", "https://0X7F.0X1/cb", "https://127.0x/cb",
        "https://%6Cocalhost/cb", "https://localhost\\x/cb", "https://10.0.0.1/cb", "https://[::ffff:127.0.0.1]/cb", "https://[0:0:0:0:0:0:0:1]/cb",
    ];
    let docs = Arc::new(Documents { doc: json!({ "client_id": CIMD, "client_name": "Agent Example", "redirect_uris": refused }), asked: Mutex::new(vec![]) });
    let h = harness_with(Outside { resolve: Arc::new(PublicAnswer("93.184.215.14".parse().unwrap())), get: docs.clone(), public_dns: Arc::new(Unreachable) }, Arc::new(|| Err("none".into())));
    for r in refused {
        assert_eq!(h.register(r).status, StatusCode::BAD_REQUEST, "registered {r}");
        assert_eq!(h.authorize(CIMD, r, &[]).status, StatusCode::BAD_REQUEST, "authorized {r}");
    }
    assert!(docs.asked.lock().unwrap().is_empty(), "refused before the document is fetched");
    // a public name, and a public address as written canonically, are taken
    for ok in ["https://agent.example/cb", "https://agent-1.example.co.uk:8443/cb", "https://93.184.215.14/cb"] {
        assert!(oauth::redirect_ok(ok).is_ok(), "{ok}");
    }
}

/// An address as a redirect's host is taken only as written canonically, and
/// public: another spelling of an address is refused, so the host shown is the
/// one a browser goes to (brief 80's re-review, 14).
#[test]
fn a_redirect_address_is_taken_only_in_its_canonical_form() {
    for ok in ["https://93.184.215.14/cb", "https://[2606:4700:4700::1111]/cb"] {
        assert!(oauth::redirect_ok(ok).is_ok(), "{ok}");
    }
    for refused in ["https://093.184.215.14/cb", "https://[2606:4700:4700:0:0:0:0:1111]/cb", "https://[2606:4700:4700:0::1111]/cb"] {
        assert!(oauth::redirect_ok(refused).is_err(), "{refused}");
    }
}

/// A metadata document whose `client_id` is not its own URL is refused.
#[test]
fn a_metadata_document_naming_another_client_is_refused() {
    let docs = Arc::new(Documents { doc: json!({ "client_id": "https://someone.else/client.json", "redirect_uris": [CALLBACK] }), asked: Mutex::new(vec![]) });
    let h = harness_with(Outside { resolve: Arc::new(PublicAnswer("93.184.215.14".parse().unwrap())), get: docs.clone(), public_dns: Arc::new(Unreachable) }, Arc::new(|| Err("none".into())));
    let a = h.authorize(CIMD, CALLBACK, &[]);
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert!(a.text().contains("another client_id"), "{}", a.text());
    assert!(h.store().waiting(bagholder_core::jiff::Timestamp::MIN).unwrap().is_empty());
}

/// A metadata document is fetched from the checked address, and one whose name
/// resolves to a private address is never fetched (brief 73, 2).
#[test]
fn a_metadata_document_on_a_private_address_is_never_fetched() {
    let docs = documents();
    let h = harness_with(Outside { resolve: Arc::new(PublicAnswer("10.0.0.5".parse().unwrap())), get: docs.clone(), public_dns: Arc::new(Unreachable) }, Arc::new(|| Err("none".into())));
    assert_eq!(h.authorize(CIMD, CALLBACK, &[]).status, StatusCode::BAD_REQUEST);
    assert!(docs.asked.lock().unwrap().is_empty(), "nothing fetched");
}

/// Who is asking, as the pages show it (brief 73, 4).
#[test]
fn the_pages_say_who_is_really_asking() {
    let h = harness();
    let page = h.authorize(CIMD, CALLBACK, &[]).text();
    assert!(page.contains("<b>agent.example</b>") && page.contains("Agent Example"), "{page}");
    let client = h.register("https://claude.example/cb").json()["client_id"].as_str().unwrap().to_string();
    let page = h.authorize(&client, "https://claude.example/cb", &[]).text();
    assert!(page.contains("<b>claude.example</b>") && page.contains("Totally Claude") && page.contains("name given by the agent, not checked"), "{page}");
    let waiting = approve::waiting(&h.store(), h.ctx.now()).unwrap();
    assert_eq!(waiting.iter().map(|w| (w.who.as_str(), w.checked)).collect::<Vec<_>>(), vec![("agent.example", true), ("claude.example", false)]);
}

/// Who is asking is the metadata document's host, never the callback's, where the
/// two differ; both pages name the host the agent is sent back to as well (plan §3,
/// brief 80 §3.4, §3.5).
#[test]
fn who_is_asking_is_the_document_s_host_and_the_callback_is_named_beside_it() {
    let docs = Arc::new(Documents { doc: json!({ "client_id": CIMD, "client_name": "Agent Example", "redirect_uris": ["https://callback.example/cb"] }), asked: Mutex::new(vec![]) });
    let h = harness_with(Outside { resolve: Arc::new(PublicAnswer("93.184.215.14".parse().unwrap())), get: docs, public_dns: Arc::new(Unreachable) }, Arc::new(|| Err("none".into())));
    let page = h.authorize(CIMD, "https://callback.example/cb", &[]).text();
    assert!(page.contains("<b>agent.example</b>") && page.contains("Agent Example"), "{page}");
    assert!(page.contains("It sends you back to <b>callback.example</b>"), "{page}");
    let w = approve::waiting(&h.store(), h.ctx.now()).unwrap();
    assert_eq!(w.iter().map(|w| (w.who.as_str(), w.redirect_host.as_str(), w.checked)).collect::<Vec<_>>(), vec![("agent.example", "callback.example", true)]);
}

/// The status route answers for the id it is asked with alone: asked with one
/// sign-in's id while another is allowed, it says that one still waits, and hands
/// no code (plan §3, brief 73, 5).
#[test]
fn the_status_route_answers_for_its_own_id_alone() {
    let h = harness();
    let (first, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    let (second, _) = h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    approve::answer(&h.store(), &second, true, h.ctx.now()).unwrap();
    assert_eq!(h.get(&format!("/oauth/authorize/status?request={first}")).json(), json!({ "state": "waiting" }));
    assert_eq!(h.get(&format!("/oauth/authorize/status?request={second}")).json()["state"], "allowed");
}

// ---- bounds before sign-in (brief 73, 1) ----

/// A caller no grant or key names is refused and counted, never written (brief 80
/// §2.1): a Host of 64 KiB is refused at the gate and kept nowhere, and a thousand
/// wrong keys leave the log as it was, each counted by its reason in memory.
#[test]
fn strangers_refused_are_counted_by_reason_and_never_written() {
    let h = harness();
    // the book's stamp: SQLite's count of commits made by other connections, read
    // on one held open throughout (brief 80 §2.1; its re-review, 14)
    let watching = rusqlite::Connection::open(h.home.path().join(bagholder_book::BOOK_FILE)).unwrap();
    let stamp = || watching.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0)).unwrap();
    // the harness's book opened once by this build, as the app's is before any caller
    drop(h.store());
    let before = stamp();
    let huge = "a".repeat(64 * 1024);
    assert_eq!(h.ask("GET", "/.well-known/oauth-protected-resource", &[("host", &huge)], vec![]).status, StatusCode::MISDIRECTED_REQUEST);
    for _ in 0..1000 {
        assert_eq!(h.ask("GET", "/api/agents/trades", &[("authorization", "Bearer bhk_wrong")], vec![]).status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(stamp(), before, "the book's stamp moved");
    assert!(h.store().reads("", usize::MAX).unwrap().is_empty(), "nothing written");
    assert_eq!(listener::strangers(&h.ctx), vec![("for another host".to_string(), 1), ("with a wrong key".to_string(), 1000)]);
}

/// A read through the JSON API keeps its arguments in the agent's log cut to
/// `LOGGED_TEXT`, as a read over MCP does: a 60 KB `sort` is never stored whole
/// (brief 80's re-review, 3).
#[test]
fn a_json_api_read_logs_its_arguments_cut() {
    let h = harness();
    let key = h.key("api");
    let agent = h.store().keys().unwrap()[0].id.clone();
    let sort = "x".repeat(60 * 1024);
    let a = h.ask("GET", &format!("/api/agents/trades?sort={sort}"), &[("authorization", &format!("Bearer {key}"))], vec![]);
    assert_ne!(a.status, StatusCode::UNAUTHORIZED);
    let reads = h.store().reads(&agent, 10).unwrap();
    let kept = reads.iter().find_map(|r| r.arguments.clone()).expect("the read is logged");
    assert!(kept.chars().count() <= listener::LOGGED_TEXT, "{} characters kept", kept.chars().count());
}

/// An agent holds at most `MCP_SESSIONS_KEPT` MCP sessions open at once on a
/// path: one more is refused until one ends, and another agent's are its own
/// (brief 80's re-review, 12).
#[test]
fn an_agent_s_open_mcp_sessions_are_bounded() {
    let h = harness();
    let (one, two) = (h.key("mcp"), h.key("mcp"));
    for _ in 0..MCP_SESSIONS_KEPT {
        h.mcp_session("/mcp/key", &one);
    }
    assert_eq!(h.mcp("/mcp/key", Some(&one)).status, StatusCode::TOO_MANY_REQUESTS, "one more than the bound");
    h.mcp_session("/mcp/key", &two);
}

/// What an agent's log keeps of a caller's own text is cut to `LOGGED_TEXT`.
#[test]
fn a_caller_s_text_is_kept_cut_to_its_named_length() {
    assert_eq!(listener::cut(&"é".repeat(5 * listener::LOGGED_TEXT)).chars().count(), listener::LOGGED_TEXT);
    assert_eq!(listener::cut("short"), "short");
}


#[test]
fn a_sixth_pending_request_is_refused() {
    let h = harness();
    for _ in 0..PENDING_KEPT {
        h.waiting(&h.authorize(CIMD, CALLBACK, &[]));
    }
    let a = h.authorize(CIMD, CALLBACK, &[]);
    assert_eq!(a.status, StatusCode::TOO_MANY_REQUESTS);
    // one ends, and another may wait
    let id = h.store().waiting(bagholder_core::jiff::Timestamp::MIN).unwrap()[0].id.clone();
    approve::answer(&h.store(), &id, false, h.ctx.now()).unwrap();
    assert_eq!(h.authorize(CIMD, CALLBACK, &[]).status, StatusCode::OK);
}

#[test]
fn a_21st_unapproved_registration_drops_the_oldest() {
    let h = harness();
    let mut ids = Vec::new();
    for _ in 0..=UNAPPROVED_KEPT {
        ids.push(h.register("https://agent.example/cb").json()["client_id"].as_str().unwrap().to_string());
        h.advance(SignedDuration::from_secs(1));
    }
    assert_eq!(h.store().unapproved().unwrap(), UNAPPROVED_KEPT);
    assert!(h.store().client(&ids[0]).unwrap().is_none(), "the oldest went");
    assert!(h.store().client(&ids[UNAPPROVED_KEPT]).unwrap().is_some());
}

#[test]
fn the_61st_call_before_sign_in_in_a_minute_is_refused() {
    let h = harness();
    for n in 0..BEFORE_SIGN_IN_PER_MINUTE {
        assert_eq!(h.get("/.well-known/oauth-authorization-server").status, StatusCode::OK, "call {n}");
    }
    let a = h.get("/.well-known/oauth-protected-resource");
    assert_eq!(a.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(a.header("retry-after").parse::<u64>().unwrap() >= 1);
    // computed when touched: a minute later, calls go through again
    h.advance(SignedDuration::from_secs(60));
    assert_eq!(h.get("/.well-known/oauth-protected-resource").status, StatusCode::OK);
}

/// The allowance before sign-in is every caller's together, on every route before
/// sign-in: spent on one, the next call on each of the others is refused too (brief
/// 80 §9).
#[test]
fn the_allowance_before_sign_in_holds_on_every_route_before_it() {
    let routes: [(&str, &str); 7] = [
        ("GET", "/.well-known/oauth-protected-resource"),
        ("GET", "/.well-known/oauth-authorization-server"),
        ("POST", "/oauth/register"),
        ("GET", "/oauth/authorize"),
        ("POST", "/oauth/token"),
        ("GET", "/agents/check/abc"),
        ("GET", "/api/agents/openapi.json"),
    ];
    for (method, path) in routes {
        let h = harness();
        for _ in 0..BEFORE_SIGN_IN_PER_MINUTE {
            h.get("/.well-known/oauth-authorization-server");
        }
        let a = h.ask(method, path, &[("content-type", "application/json")], if method == "POST" { b"{}".to_vec() } else { vec![] });
        assert_eq!(a.status, StatusCode::TOO_MANY_REQUESTS, "{method} {path}");
    }
}

/// The bounds are the plan's numbers (brief 73, 1; brief 80 §9).
#[test]
fn the_bounds_are_the_plan_s_numbers() {
    assert_eq!(PENDING_KEPT, 5);
    assert_eq!(UNAPPROVED_KEPT, 20);
    assert_eq!(BEFORE_SIGN_IN_PER_MINUTE, 60);
    assert_eq!(BODY_LIMIT, 16 * 1024);
    assert_eq!(REQUEST_LIFE, SignedDuration::from_mins(10));
}

#[test]
fn a_body_over_16_kib_is_refused() {
    let h = harness();
    let big = vec![b'a'; BODY_LIMIT + 1];
    for p in ["/oauth/register", "/oauth/token"] {
        assert_eq!(h.ask("POST", p, &[("content-type", "application/json")], big.clone()).status, StatusCode::PAYLOAD_TOO_LARGE, "{p}");
    }
    // and a GET takes none
    assert_eq!(h.ask("GET", "/.well-known/oauth-protected-resource", &[], b"x".to_vec()).status, StatusCode::PAYLOAD_TOO_LARGE);
}

// ---- keys ----

#[test]
fn the_key_path_offers_no_sign_in_and_takes_only_its_own_keys() {
    let h = harness();
    let a = h.mcp("/mcp/key", None);
    assert_eq!(a.status, StatusCode::UNAUTHORIZED);
    assert!(!a.header("www-authenticate").contains("resource_metadata"), "{}", a.header("www-authenticate"));
    for doc in ["/.well-known/oauth-protected-resource", "/.well-known/oauth-authorization-server"] {
        assert!(!h.get(doc).text().contains("/mcp/key"), "{doc} names /mcp/key");
    }
    let (mcp, api) = (h.key("mcp"), h.key("api"));
    assert_eq!(h.mcp("/mcp/key", Some(&mcp)).status, StatusCode::OK);
    assert_eq!(h.mcp("/mcp/key", Some(&api)).status, StatusCode::UNAUTHORIZED, "an API key on MCP");
    assert_eq!(h.mcp("/mcp/key", Some("bhm_wrong")).status, StatusCode::UNAUTHORIZED);
    assert_eq!(h.ask("GET", "/api/agents/lists", &[("authorization", &format!("Bearer {mcp}"))], vec![]).status, StatusCode::UNAUTHORIZED, "an MCP key on the API");
    assert_eq!(h.ask("GET", "/api/agents/lists", &[("authorization", "Bearer bhj_wrong")], vec![]).status, StatusCode::UNAUTHORIZED);
    assert_eq!(h.ask("GET", "/api/agents/lists", &[], vec![]).status, StatusCode::UNAUTHORIZED);
    // an access token is not a key, nor a key a token
    let (access, _, _) = h.sign_in();
    assert_eq!(h.mcp("/mcp/key", Some(&access)).status, StatusCode::UNAUTHORIZED);
    assert_eq!(h.mcp("/mcp", Some(&mcp)).status, StatusCode::UNAUTHORIZED);
    // cut off, at once
    let id = h.store().keys().unwrap().into_iter().find(|k| k.way == "mcp").unwrap().id;
    approve::cut_off(&h.store(), &id, h.ctx.now()).unwrap();
    assert_eq!(h.mcp("/mcp/key", Some(&mcp)).status, StatusCode::UNAUTHORIZED);
}

// ---- the gate ----

/// The person's own proxy may send a `Host` of its own: the gate takes that one
/// alone, and an agent is still given the public name, as the address, the issuer
/// and the resource its tokens are for (§4.3a).
#[test]
fn a_proxy_s_own_host_is_taken_and_the_public_name_given() {
    use tunnel::{Provider, Setup};
    let own = Setup { provider: Provider::Own, name: "Agents.Example.com".into(), funnel_port: None, sends_host: Some("bagholder.lan".into()) };
    assert_eq!(tunnel::host_of(&own, None).unwrap(), "agents.example.com");
    assert_eq!(tunnel::received_host(&own).unwrap().as_deref(), Some("bagholder.lan"));
    // every tunnel Bagholder starts sends the public name
    for p in [Provider::Tailscale, Provider::Cloudflare, Provider::Ngrok, Provider::Zrok] {
        assert_eq!(tunnel::received_host(&Setup { provider: p, sends_host: Some("bagholder.lan".into()), ..own.clone() }).unwrap(), None, "{p:?}");
    }
    let h = harness();
    *h.ctx.receives.write().unwrap() = "bagholder.lan".into();
    let a = h.ask("GET", "/.well-known/oauth-protected-resource", &[("host", "bagholder.lan")], vec![]);
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.json()["resource"], json!(format!("https://{HOST}/mcp")), "the public name, never the proxy's own");
    assert_eq!(h.ask("GET", "/.well-known/oauth-protected-resource", &[("host", HOST)], vec![]).status, StatusCode::MISDIRECTED_REQUEST, "only the Host the proxy sends");
}

#[test]
fn the_gate_takes_the_public_name_and_a_loopback_peer_and_sends_no_cors() {
    let h = harness();
    // another Host, the target's among them, and each provider's request shape
    for host in ["127.0.0.1:9000", "localhost", "evil.example", "bagholder.example.ts.net.evil.example", "evil.bagholder.example.ts.net", "xbagholder.example.ts.net", "bagholder.example.ts.net."] {
        let a = h.ask("GET", "/.well-known/oauth-protected-resource", &[("host", host)], vec![]);
        assert_eq!(a.status, StatusCode::MISDIRECTED_REQUEST, "{host}");
    }
    let shapes: [&[(&str, &str)]; 4] = [
        // Tailscale Funnel: the name kept, X-Forwarded-Host and the Funnel mark added
        &[("x-forwarded-host", HOST), ("tailscale-funnel-request", "?1"), ("x-forwarded-for", "203.0.113.9")],
        // Cloudflare: the name passed from the edge
        &[("cf-connecting-ip", "203.0.113.9"), ("x-forwarded-proto", "https")],
        // ngrok: the domain preserved
        &[("x-forwarded-for", "203.0.113.9"), ("x-forwarded-proto", "https")],
        // zrok: nothing set
        &[],
    ];
    for s in shapes {
        assert_eq!(h.ask("GET", "/.well-known/oauth-protected-resource", s, vec![]).status, StatusCode::OK, "{s:?}");
        // a forwarded host naming the public name never passes another Host
        let mut with_target = s.to_vec();
        with_target.push(("host", "127.0.0.1:9000"));
        with_target.push(("x-forwarded-host", HOST));
        assert_eq!(h.ask("GET", "/.well-known/oauth-protected-resource", &with_target, vec![]).status, StatusCode::MISDIRECTED_REQUEST);
    }
    // a peer that is not loopback
    let a = h.ask_from("203.0.113.9:4000", "GET", "/.well-known/oauth-protected-resource", &[("x-forwarded-for", "127.0.0.1")], vec![]);
    assert_eq!(a.status, StatusCode::FORBIDDEN);
    // no CORS, on any answer
    for (path, methods) in ROUTES {
        for m in methods.iter().map(|m| m.as_str()) {
            let a = h.ask(m, &path.replace("{value}", "x"), &[("origin", "https://evil.example")], vec![]);
            assert!(a.headers.keys().all(|k| !k.as_str().starts_with("access-control-")), "{m} {path} sent CORS");
        }
    }
    // a stranger's refusal is counted, and nothing it sent is kept
    assert!(listener::strangers(&h.ctx).iter().any(|(why, n)| why == "for another host" && *n > 0));
    assert!(h.store().reads("", 100).unwrap().is_empty());
}

// ---- rate and log ----

#[test]
fn the_11th_read_in_a_burst_is_refused_and_the_log_keeps_arguments_never_answers() {
    let h = harness();
    let key = h.key("api");
    for n in 0..READS_BURST {
        let a = h.ask("GET", "/api/agents/trade?id=t-1", &[("authorization", &format!("Bearer {key}"))], vec![]);
        // no server in this test: the read itself fails, after it is counted and logged
        assert_ne!(a.status, StatusCode::TOO_MANY_REQUESTS, "read {n}");
    }
    let a = h.ask("GET", "/api/agents/trade?id=t-1", &[("authorization", &format!("Bearer {key}"))], vec![]);
    assert_eq!(a.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(!a.header("retry-after").is_empty());
    let id = h.store().keys().unwrap()[0].id.clone();
    let log = approve::log(&h.store(), &id).unwrap();
    assert_eq!(log.iter().filter(|l| l.tool.as_deref() == Some("trade")).count(), READS_BURST as usize);
    assert!(log.iter().all(|l| l.arguments.as_deref().is_none_or(|a| a == "{\"id\":\"t-1\"}")), "{log:?}");
    assert!(log.iter().any(|l| l.refusal.as_deref() == Some("over the rate")));
    // a wrong key's refusal is counted, not written
    h.ask("GET", "/api/agents/lists", &[("authorization", "Bearer bhj_nope")], vec![]);
    assert!(listener::strangers(&h.ctx).contains(&("with a wrong key".to_string(), 1)));
    assert!(h.store().reads("", 10).unwrap().is_empty());
}

/// One MCP service per path: a session the POST opened is the GET's to stream,
/// and a session it did not open is not, on /mcp and /mcp/key alike (brief 80
/// §6.1; its re-review, 13).
#[test]
fn a_session_opened_by_post_is_the_get_s_to_stream() {
    for path in ["/mcp", "/mcp/key"] {
        let h = harness();
        let bearer = if path == "/mcp" { h.sign_in().0 } else { h.key("mcp") };
        let session = h.mcp_session(path, &bearer);
        let auth = format!("Bearer {bearer}");
        assert_eq!(h.status_only("GET", path, &[("accept", "text/event-stream"), ("authorization", &auth), ("mcp-session-id", &session)]), StatusCode::OK, "{path}");
        assert_ne!(h.status_only("GET", path, &[("accept", "text/event-stream"), ("authorization", &auth), ("mcp-session-id", "no-such-session")]), StatusCode::OK, "{path}");
    }
}

/// Over MCP each tool call is a read, charged to the agent's rate and kept in its
/// log, over /mcp and /mcp/key alike; initializing, listing and notifications are
/// not (brief 80 §6.2, §6.3).
#[test]
fn over_mcp_each_tool_call_is_a_read_and_nothing_else_is() {
    for path in ["/mcp", "/mcp/key"] {
        let h = harness();
        let (bearer, agent) = if path == "/mcp" {
            let (access, _, grant) = h.sign_in();
            (access, grant)
        } else {
            let k = h.key("mcp");
            (k, h.store().keys().unwrap()[0].id.clone())
        };
        let session = h.mcp_session(path, &bearer);
        for n in 0..3 * READS_BURST as u64 {
            let r = h.mcp_post(path, &bearer, &session, json!({ "jsonrpc": "2.0", "id": 100 + n, "method": "tools/list", "params": {} }));
            assert_eq!(r.status, StatusCode::OK, "{path}: list {n}");
        }
        let call = |n: u64| h.mcp_post(path, &bearer, &session, json!({ "jsonrpc": "2.0", "id": 200 + n, "method": "tools/call", "params": { "name": "trade", "arguments": { "id": "t-1" } } }));
        for n in 0..READS_BURST as u64 {
            let r = call(n);
            assert_eq!(r.status, StatusCode::OK, "{path}: read {n}");
            assert!(!r.text().contains("Too many reads"), "{path}: read {n}: {}", r.text());
        }
        assert!(call(99).text().contains("Too many reads"), "{path}: the read past the burst");
        let log = approve::log(&h.store(), &agent).unwrap();
        assert_eq!(log.iter().filter(|l| l.tool.as_deref() == Some("trade")).count(), READS_BURST as usize, "{path}: {log:?}");
        assert!(log.iter().any(|l| l.refusal.as_deref() == Some("over the rate")), "{path}");
    }
}

/// The agents' handle reads and writes their four settings alone (brief 80 §6.4).
#[test]
fn the_agents_handle_reads_and_writes_their_own_settings_alone() {
    let h = harness();
    let s = h.store();
    for key in bagholder_book::agents::SETTINGS {
        assert!(s.setting(key).is_ok(), "{key}");
        assert!(s.set_setting(key, Some("x"), h.ctx.now()).is_ok(), "{key}");
    }
    for key in ["watch.folder", "statements.said", "agents.other", "agents"] {
        assert!(s.setting(key).is_err(), "read {key}");
        assert!(s.set_setting(key, Some("x"), h.ctx.now()).is_err(), "wrote {key}");
    }
}

/// An agent's log keeps its newest `READS_KEPT` lines (the plan's 1,000).
#[test]
fn an_agent_s_log_keeps_its_newest_thousand_lines() {
    assert_eq!(bagholder_book::agents::READS_KEPT, 1_000);
    let h = harness();
    let _made = h.key("api");
    let agent = h.store().keys().unwrap()[0].id.clone();
    let s = h.store();
    for n in 0..1_005 {
        h.advance(SignedDuration::from_secs(1));
        s.log(&bagholder_book::agents::Read { agent: agent.clone(), at: h.ctx.now(), tool: Some("trades".into()), arguments: Some(format!("{{\"n\":{n}}}")), refusal: None }).unwrap();
    }
    let lines = s.reads(&agent, 5_000).unwrap();
    assert_eq!(lines.len(), 1_000);
    assert!(lines.iter().all(|l| l.arguments.as_deref() != Some("{\"n\":4}")), "the oldest went");
    assert!(lines.iter().any(|l| l.arguments.as_deref() == Some("{\"n\":1004}")), "the newest stayed");
}

// ---- the tunnels ----

/// Each Tailscale step is shown done by what `tailscale status --json` states, in
/// the shapes its versions write: signed in when the backend runs, HTTPS and Funnel
/// by the capabilities in `Self.Capabilities` or `Self.CapMap`, the name without its
/// trailing dot. Not installed or not answering, nothing is done.
#[test]
fn tailscale_s_status_shows_each_step_done() {
    let says = |v: Value| tunnel::read_tailscale_status(&v);
    let signed_out = says(json!({"BackendState": "NeedsLogin", "AuthURL": "", "Self": {"DNSName": "", "Capabilities": null}}));
    assert!(signed_out.installed && !signed_out.signed_in && !signed_out.https && !signed_out.funnel && signed_out.name.is_none());
    let listed = says(json!({"BackendState": "Running", "Self": {"DNSName": "Machine.tail0000.ts.net.", "Capabilities": ["https", "funnel", "https://tailscale.com/cap/funnel-ports?ports=443,8443,10000"]}}));
    assert!(listed.signed_in && listed.https && listed.funnel);
    assert_eq!(listed.name.as_deref(), Some("machine.tail0000.ts.net"));
    let mapped = says(json!({"BackendState": "Running", "Self": {"DNSName": "m.tail0000.ts.net.", "CapMap": {"https": null, "funnel": null}}}));
    assert!(mapped.https && mapped.funnel);
    let no_funnel = says(json!({"BackendState": "Running", "Self": {"DNSName": "m.tail0000.ts.net.", "CapMap": {"https": null}}}));
    assert!(no_funnel.signed_in && no_funnel.https && !no_funnel.funnel);
    assert_eq!(tunnel::tailscale_says(None), tunnel::TailscaleSays::default(), "not installed: nothing done");
    let not_answering = tunnel::tailscale_says(Some(std::path::Path::new("/bin/false")));
    assert!(not_answering.installed && !not_answering.signed_in);
}

#[test]
fn the_tailscale_command_never_runs_in_the_background() {
    let setups = [Setup { provider: Provider::Tailscale, name: String::new(), funnel_port: None, sends_host: None }, Setup { provider: Provider::Tailscale, name: String::new(), funnel_port: Some(8443), sends_host: None }];
    for s in setups {
        let c = tunnel::command(&s, "tailscale", 4321).unwrap();
        assert!(!c.iter().any(|a| a.starts_with("--bg")), "{c:?}");
        assert_eq!(c[1], "funnel");
        assert!(c.contains(&"127.0.0.1:4321".to_string()));
    }
    // every provider's command targets this listener alone, with fixed arguments
    for p in [Provider::Cloudflare, Provider::Ngrok, Provider::Zrok] {
        let c = tunnel::command(&Setup { provider: p, name: "agents.example.com".into(), funnel_port: None, sends_host: None }, "x", 4321).unwrap();
        assert!(c.iter().any(|a| a.contains("127.0.0.1:4321")), "{p:?}: {c:?}");
    }
    assert!(tunnel::command(&Setup { provider: Provider::Own, name: "a.example.com".into(), funnel_port: None, sends_host: None }, "x", 1).is_none());
    use tunnel::{Provider, Setup};
}

#[test]
fn a_second_tunnel_start_is_refused_naming_the_first() {
    let mut t = tunnel::Tunnels::default();
    let stand_in = if cfg!(windows) { vec!["cmd".to_string(), "/c".into(), "ping -n 30 127.0.0.1".into()] } else { vec!["sleep".to_string(), "30".into()] };
    t.start(tunnel::Provider::Ngrok, (stand_in.clone(), vec![]), Arc::new(|| {})).unwrap();
    let said = t.start(tunnel::Provider::Zrok, (stand_in, vec![]), Arc::new(|| {})).unwrap_err();
    assert!(said.contains("Ngrok"), "{said}");
    t.stop();
    assert!(t.state().running.is_none());
}

/// Agents switched off: the listener takes nothing, so every grant and key is refused at once.
#[test]
fn switching_off_closes_the_listener_and_refuses_every_agent_at_once() {
    let h = harness();
    let agents = Agents::default();
    let ctx = Ctx {
        host: std::sync::RwLock::new(HOST.into()),
        receives: Default::default(),
        store: h.ctx.store.clone(),
        read: h.ctx.read.clone(),
        now: h.ctx.now.clone(),
        outside: h.ctx.outside.clone(),
        limits: Default::default(),
        check: Default::default(),
        one_check: Default::default(),
    };
    let port = agents.switch_on(h.rt.handle(), 0, ctx).unwrap();
    let answers = |p: u16| std::net::TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], p)), std::time::Duration::from_secs(2)).is_ok();
    // served, and answering for the public name
    let mut ok = false;
    for _ in 0..50 {
        if answers(port) {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ok, "the listener answers once on");
    agents.switch_off();
    let mut closed = false;
    for _ in 0..100 {
        if !answers(port) {
            closed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(closed, "nothing answers once off");
    assert!(agents.port().is_none());
}

// ---- reachability (brief 73, 6 and 7) ----

/// Calls the listener's own check route for the fetch, as a request through the
/// tunnel would reach it, with or without the Funnel mark; recording where it went.
struct Through {
    ctx: Arc<Ctx>,
    funnel: bool,
    went: Mutex<Vec<SocketAddr>>,
    /// Addresses that close the connection before answering.
    closed: Vec<IpAddr>,
}
impl Get for Through {
    fn get(&self, to: SocketAddr, url: &str, _h: &[(&str, &str)], _within: std::time::Duration) -> Result<bagholder_net::client::Response, String> {
        self.went.lock().unwrap().push(to);
        if self.closed.contains(&to.ip()) {
            return Err("the handshake failed: unexpected EOF".into());
        }
        let path = url.split(HOST).nth(1).unwrap().to_string();
        let mut req = Request::builder().method("GET").uri(&path).header("host", HOST);
        if self.funnel {
            req = req.header("tailscale-funnel-request", "?1");
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
        let r = router(self.ctx.clone());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let res = r.oneshot(req).await.unwrap();
            let status = res.status().as_u16();
            let body = axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap().to_vec();
            Ok(bagholder_net::client::Response { status, body, headers: vec![] })
        })
    }
}

fn reach_with(system: &str, public: Option<&str>, funnel: bool, provider: tunnel::Provider) -> (tunnel::Ready, Vec<SocketAddr>) {
    reach_through(system, public, funnel, provider, &[])
}

fn reach_through(system: &str, public: Option<&str>, funnel: bool, provider: tunnel::Provider, closed: &[&str]) -> (tunnel::Ready, Vec<SocketAddr>) {
    // public DNS's answers, comma-separated
    struct Answers(Vec<IpAddr>);
    impl Resolve for Answers {
        fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
            Ok(self.0.clone())
        }
    }
    let public_dns: Arc<dyn Resolve> = match public {
        Some(p) => Arc::new(Answers(p.split(',').map(|a| a.parse().unwrap()).collect())),
        None => Arc::new(Unreachable),
    };
    // the context the stand-in reaches; the check's value is opened on it
    let slot: Arc<Mutex<Option<Arc<Through>>>> = Arc::default();
    struct Late(Arc<Mutex<Option<Arc<Through>>>>);
    impl Get for Late {
        fn get(&self, to: SocketAddr, url: &str, h: &[(&str, &str)], within: std::time::Duration) -> Result<bagholder_net::client::Response, String> {
            let t = self.0.lock().unwrap().clone().unwrap();
            t.get(to, url, h, within)
        }
    }
    let h = harness_with(Outside { resolve: Arc::new(PublicAnswer(system.parse().unwrap())), get: Arc::new(Late(slot.clone())), public_dns }, Arc::new(|| Err("none".into())));
    let through = Arc::new(Through { ctx: h.ctx.clone(), funnel, went: Mutex::new(vec![]), closed: closed.iter().map(|a| a.parse().unwrap()).collect() });
    *slot.lock().unwrap() = Some(through.clone());
    let ready = tunnel::reach(&h.ctx, provider);
    let went = through.went.lock().unwrap().clone();
    (ready, went)
}

#[test]
fn the_check_takes_the_public_path() {
    // the system resolver answers a tailnet address; public DNS the edge's
    let (ready, went) = reach_with("100.101.102.103", Some("93.184.215.14"), true, tunnel::Provider::Tailscale);
    assert_eq!(went, vec![SocketAddr::from(([93, 184, 215, 14], 443))], "connected to the public answer alone");
    assert_eq!(ready, tunnel::Ready::Ready);
    // public DNS answering what is not a public address is no public path: asked
    // from here, and never Ready
    let (ready, went) = reach_with("100.101.102.103", Some("10.1.2.3"), true, tunnel::Provider::Tailscale);
    assert_eq!(went, vec![SocketAddr::from(([100, 101, 102, 103], 443))]);
    assert_eq!(ready, tunnel::Ready::Unconfirmed { said: tunnel::UNCONFIRMED.into() });
    // and every answer must be public: one that is not, beside a public one, and the
    // public path is not taken (brief 80 §7.1)
    let (ready, went) = reach_with("100.101.102.103", Some("93.184.215.14,10.1.2.3"), true, tunnel::Provider::Tailscale);
    assert_eq!(went, vec![SocketAddr::from(([100, 101, 102, 103], 443))], "never the public answer");
    assert_eq!(ready, tunnel::Ready::Unconfirmed { said: tunnel::UNCONFIRMED.into() });
}

/// A machine may not reach its own address the public way: Tailscale's relay does
/// not carry a node's own traffic back to it. The public path closing is then asked
/// from here, and never Ready; closed both ways, it failed, in the public path's words.
#[test]
fn a_public_path_closed_to_this_computer_is_asked_from_here_and_never_ready() {
    let (ready, went) = reach_through("100.101.102.103", Some("93.184.215.14"), true, tunnel::Provider::Tailscale, &["93.184.215.14"]);
    assert_eq!(went, vec![SocketAddr::from(([93, 184, 215, 14], 443)), SocketAddr::from(([100, 101, 102, 103], 443))]);
    assert_eq!(ready, tunnel::Ready::Unconfirmed { said: tunnel::UNCONFIRMED.into() });
    let (ready, _) = reach_through("100.101.102.103", Some("93.184.215.14"), true, tunnel::Provider::Tailscale, &["93.184.215.14", "100.101.102.103"]);
    assert_eq!(ready, tunnel::Ready::Failed { said: "the handshake failed: unexpected EOF".into() });
}

/// Another program answering on 443 at this machine's tailnet address takes Funnel's
/// traffic on 443: the address is read from Tailscale's status, and a listener there
/// is found.
#[test]
fn a_program_on_443_at_the_tailnet_address_is_found() {
    let says = tunnel::read_tailscale_status(&json!({"BackendState": "Running", "Self": {"DNSName": "m.tail0000.ts.net.", "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0::1"]}}));
    assert_eq!(says.address.as_deref(), Some("100.101.102.103"));
    assert!(!says.https_port_taken, "found only when looked for");
    // a test cannot hold 443: the same check, on a port this test holds, then frees
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let at = held.local_addr().unwrap();
    assert!(tunnel::answers(at));
    drop(held);
    assert!(!tunnel::answers(at));
}

#[test]
fn a_tailscale_check_without_the_funnel_mark_is_not_ready() {
    let (ready, _) = reach_with("100.101.102.103", Some("93.184.215.14"), false, tunnel::Provider::Tailscale);
    assert_eq!(ready, tunnel::Ready::Unconfirmed { said: tunnel::UNCONFIRMED.into() });
    // another provider's edge is the public path by construction
    let (ready, _) = reach_with("93.184.215.14", Some("93.184.215.14"), false, tunnel::Provider::Cloudflare);
    assert_eq!(ready, tunnel::Ready::Ready);
}

#[test]
fn with_public_dns_unreachable_the_check_is_never_ready() {
    let (ready, went) = reach_with("192.168.1.20", None, true, tunnel::Provider::Own);
    assert_eq!(ready, tunnel::Ready::Unconfirmed { said: "Reachable from this computer; confirm from your agent.".into() });
    assert_eq!(went, vec![SocketAddr::from(([192, 168, 1, 20], 443))], "asked from here, and said so");
}

// ---- the OpenAPI description ----

#[test]
fn the_openapi_description_matches_its_routes() {
    let d = api::description(&format!("https://{HOST}"));
    assert_eq!(d["openapi"], "3.1.0");
    // the document's own parts, as OpenAPI 3.1 requires and a client reads them
    assert!(d["info"]["title"].is_string() && d["info"]["version"] == crate::app::APP_VERSION, "{}", d["info"]);
    assert_eq!(d["servers"][0]["url"], format!("https://{HOST}"));
    assert_eq!(d["components"]["securitySchemes"]["key"]["scheme"], "bearer");
    assert_eq!(d["security"][0]["key"], json!([]), "every route asks for a key");
    let paths = d["paths"].as_object().unwrap();
    let api_routes: Vec<&str> = ROUTES.iter().map(|(p, _)| *p).filter(|p| p.starts_with("/api/agents/") && !p.ends_with("openapi.json")).collect();
    assert_eq!(paths.keys().map(String::as_str).collect::<Vec<_>>().len(), api_routes.len());
    for p in api_routes {
        let op = &paths[p]["get"];
        assert!(op.is_object(), "{p} is described");
        assert!(op["responses"]["200"].is_object() && op["operationId"].is_string(), "{p}");
        for param in op["parameters"].as_array().unwrap() {
            assert_eq!(param["in"], "query", "{p}");
            assert!(param["schema"].is_object() ^ param["content"].is_object(), "{p}: a parameter has a schema or a content, not both: {param}");
        }
    }
    // the trade route needs its id
    assert!(paths["/api/agents/trade"]["get"]["parameters"].as_array().unwrap().iter().any(|p| p["name"] == "id" && p["required"] == true));
}

/// Each described route, called with its described arguments on a real book,
/// answers what the page answers.
#[test]
fn each_described_route_answers_with_its_described_arguments() {
    let d = tempfile::tempdir().unwrap();
    crate::capacity::build(d.path(), bagholder_wealthsimple::generated::Size { accounts: 4, instruments: 10, trades: 30, months: 4, seed: 3, deep: 0 }, 2).unwrap();
    let served = crate::tests_mcp::served_for(d.path());
    let base = format!("http://127.0.0.1:{}", served.port);
    let h = harness_with(Outside::network(), Arc::new(move || Ok(crate::mcp::ReadOnly::own(&base))));
    let key = h.key("api");
    let auth = format!("Bearer {key}");
    let lists = h.ask("GET", "/api/agents/lists", &[("authorization", &auth)], vec![]);
    assert_eq!(lists.status, StatusCode::OK, "{}", lists.text());
    assert!(lists.json()["note"].as_str().unwrap().contains("own words"));
    let trades = h.ask("GET", "/api/agents/trades?size=3&filters=%7B%7D", &[("authorization", &auth)], vec![]);
    assert_eq!(trades.status, StatusCode::OK, "{}", trades.text());
    let first = trades.json()["trades"][0]["id"].as_str().unwrap().to_string();
    let one = h.ask("GET", &format!("/api/agents/trade?id={first}"), &[("authorization", &auth)], vec![]);
    assert_eq!(one.status, StatusCode::OK, "{}", one.text());
    assert_eq!(one.json()["trade"]["id"], first);
    let pos = h.ask("GET", "/api/agents/positions", &[("authorization", &auth)], vec![]);
    assert_eq!(pos.status, StatusCode::OK, "{}", pos.text());
    // every route the description lists, described with every argument it takes and
    // called with every parameter it lists, each a value of its own schema (brief
    // 80's re-review, 9)
    let described = api::description(&format!("https://{HOST}"));
    for (path, _, _, schema) in api::routes() {
        let params = described["paths"][path]["get"]["parameters"].as_array().unwrap().clone();
        let named: Vec<&str> = params.iter().map(|p| p["name"].as_str().unwrap()).collect();
        let takes: Vec<&str> = schema["properties"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
        assert_eq!(named, takes, "{path} describes every argument it takes");
        let value = |p: &Value| -> String {
            let name = p["name"].as_str().unwrap();
            let types = &p["schema"]["type"];
            let is = |t: &str| types == t || types.as_array().is_some_and(|a| a.contains(&json!(t)));
            if p["content"].is_object() {
                "%7B%7D".into()
            } else if name == "id" {
                first.clone()
            } else if is("integer") {
                "2".into()
            } else {
                match name {
                    "sort" => "activity".into(),
                    "dir" => "desc".into(),
                    other => panic!("{path}: no value made for {other}"),
                }
            }
        };
        let query: Vec<String> = params.iter().map(|p| format!("{}={}", p["name"].as_str().unwrap(), value(p))).collect();
        let called = h.ask("GET", &format!("{path}?{}", query.join("&")), &[("authorization", &auth)], vec![]);
        assert_eq!(called.status, StatusCode::OK, "{path}?{}: {}", query.join("&"), called.text());
    }
    // as the stdio tools answer
    let page = crate::agents::tools::trades_page(&crate::mcp::ReadOnly::own(&format!("http://127.0.0.1:{}", served.port)), crate::agents::tools::TradesArgs { size: Some(3), ..Default::default() }).unwrap();
    let mut via_api = trades.json();
    via_api.as_object_mut().unwrap().remove("note");
    assert_eq!(via_api, page);
    drop(served);
    let _ = &h.home;
}
