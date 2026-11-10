//! The sign-in is real before it is used (`docs/plans/stage-money.md`, part C): a
//! captured login is written only once Wealthsimple has answered with new tokens,
//! and "connected" has one writer.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::app::App;
use crate::session::{self, Capture, Taken};

/// Answers the token endpoint with the next reply queued; status 0 is a request
/// that went out and got no answer.
struct Token(Mutex<Vec<(u16, &'static str)>>);
struct Shared(Arc<Token>);

impl bagholder_net::Transport for Shared {
    fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
        assert_eq!(ask.url, bagholder_wealthsimple::hosts::token_url(), "only the token endpoint is asked");
        let (status, body) = self.0 .0.lock().unwrap().remove(0);
        if status == 0 {
            return Err(bagholder_net::NetError::Unreachable(body.into()));
        }
        Ok((status, ask.url.to_string(), vec![], body.as_bytes().to_vec()))
    }
}

fn app_answering(replies: Vec<(u16, &'static str)>) -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let net = bagholder_net::Net::answered_by(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()), Box::new(Shared(Arc::new(Token(Mutex::new(replies))))));
    let app = App::with_net(dir.path().to_path_buf(), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net);
    (dir, app)
}

/// The refresh posts this capture made, and the replies queued for them.
struct Recorded {
    posts: Arc<Mutex<Vec<String>>>,
    replies: Mutex<Vec<(u16, String)>>,
}

impl bagholder_net::Transport for Recorded {
    fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
        assert_eq!(ask.url, bagholder_wealthsimple::hosts::token_url(), "only the token endpoint is asked");
        self.posts.lock().unwrap().push(String::from_utf8_lossy(ask.body.unwrap_or_default()).into_owned());
        let (status, body) = self.replies.lock().unwrap().remove(0);
        if status == 0 {
            return Err(bagholder_net::NetError::Unreachable(body));
        }
        Ok((status, ask.url.to_string(), vec![], body.into_bytes()))
    }
}

fn app_recorded(replies: Vec<(u16, String)>) -> (tempfile::TempDir, Arc<App>, Arc<Mutex<Vec<String>>>) {
    let dir = tempfile::tempdir().unwrap();
    let posts = Arc::new(Mutex::new(Vec::new()));
    let net = bagholder_net::Net::answered_by(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()), Box::new(Recorded { posts: posts.clone(), replies: Mutex::new(replies) }));
    let app = App::with_net(dir.path().to_path_buf(), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net);
    (dir, app, posts)
}

/// A good saved login, as a sign-in before this one left it.
fn saved(app: &Arc<App>) -> String {
    let sess = bagholder_ws::session::Session {
        access_token: "saved-access".into(),
        refresh_token: "saved-refresh".into(),
        client_id: bagholder_ws::standin::FAKE_CLIENT_ID.into(),
        ids: bagholder_ws::session::IdentityKeys { identity_canonical_id: "identity-1".into(), ..Default::default() },
        ..Default::default()
    };
    session::save_session(app, &sess).unwrap();
    std::fs::read_to_string(app.ws_home().session_path()).unwrap()
}

fn captured(refresh: &str) -> Capture {
    Capture {
        access_token: "captured-access".into(),
        refresh_token: refresh.into(),
        client_id: bagholder_ws::standin::FAKE_CLIENT_ID.into(),
        ids: bagholder_ws::session::IdentityKeys { identity_canonical_id: "identity-1".into(), ..Default::default() },
        ..Default::default()
    }
}

const NEW_TOKENS: &str = r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":1800}"#;
const INVALID_GRANT: &str = r#"{"error":"invalid_grant"}"#;

#[test]
fn a_capture_wealthsimple_refuses_leaves_the_saved_login_and_is_not_tried_again() {
    let _g = crate::tests_common::guard();
    let (_home, app) = app_answering(vec![(400, INVALID_GRANT)]);
    let before = saved(&app);
    assert!(matches!(session::take_capture(&app, &captured("captured-refused")), Taken::Refused(_)));
    assert_eq!(std::fs::read_to_string(app.ws_home().session_path()).unwrap(), before, "nothing written before Wealthsimple accepts");
}

#[test]
fn a_capture_with_no_answer_leaves_the_saved_login_and_is_tried_again() {
    let _g = crate::tests_common::guard();
    let (_home, app) = app_answering(vec![(0, "the connection dropped"), (200, NEW_TOKENS)]);
    let before = saved(&app);
    assert!(matches!(session::take_capture(&app, &captured("captured-1")), Taken::NotAsked(_)), "no answer is not a refusal");
    assert_eq!(std::fs::read_to_string(app.ws_home().session_path()).unwrap(), before);
    // the next capture of the window is asked, and taken over
    assert_eq!(session::take_capture(&app, &captured("captured-2")), Taken::Taken);
    let now = session::load_session(&app).unwrap().unwrap();
    assert_eq!((now.access_token.as_str(), now.refresh_token.as_str()), ("new-access", "new-refresh"));
    assert!(app.state.lock().unwrap().connected);
}

/// Only `session::heard` writes "connected": every other path reports to it.
#[test]
fn connected_has_one_writer() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut writers = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".rs") || name.starts_with("tests_") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            // a file's own test items are a test's set-up, not a writer
            let code = crate::tests_source::production(&text);
            for (i, line) in code.lines().enumerate() {
                if line.contains(".connected =") {
                    writers.push(format!("{name}:{}", i + 1));
                }
            }
        }
    }
    assert!(writers.iter().all(|w| w.starts_with("session.rs:")), "connected written outside session::heard: {writers:?}");
    assert_eq!(writers.len(), 2, "heard's two writes, on and off: {writers:?}");
}

/// A sign-in saved with its access token `expires_in` seconds from now. Its refresh
/// token is the test's own: one Wealthsimple refused is never posted again in the
/// process.
fn saved_expiring(app: &Arc<App>, refresh: &str, expires_in: f64) {
    let mut sess = bagholder_ws::session::Session {
        access_token: "held-access".into(),
        refresh_token: refresh.into(),
        client_id: bagholder_ws::standin::FAKE_CLIENT_ID.into(),
        ids: bagholder_ws::session::IdentityKeys { identity_canonical_id: "identity-1".into(), ..Default::default() },
        ..Default::default()
    };
    sess.expires_at = Some(bagholder_ws::session::Expiry::Unix(crate::app::now_unix() + expires_in));
    session::save_session(app, &sess).unwrap();
}

/// No answer from Wealthsimple at start is not a sign-out: the page offers Sync
/// now, not Connect Wealthsimple, and says what went unanswered.
#[test]
fn a_start_wealthsimple_does_not_answer_keeps_the_saved_sign_in() {
    let _g = crate::tests_common::guard();
    let (_home, app) = app_answering(vec![(0, "the connection dropped"); 3]);
    saved_expiring(&app, "start-unanswered", 3600.0);
    session::boot_session(&app);
    let page = crate::status::status(&app);
    assert!(page.connected, "a saved sign-in Wealthsimple did not refuse is still the person's");
    assert!(page.error.contains("the connection dropped"), "{}", page.error);
}

/// At start, a sign-in Wealthsimple refuses ends: connect again.
#[test]
fn a_start_wealthsimple_refuses_ends_the_saved_sign_in() {
    let _g = crate::tests_common::guard();
    let (_home, app) = app_answering(vec![(400, INVALID_GRANT); 3]);
    saved_expiring(&app, "start-refused", 3600.0);
    session::boot_session(&app);
    let page = crate::status::status(&app);
    assert!(!page.connected);
    assert_eq!(page.error, bagholder_wealthsimple::session::REFUSED_SIGN_IN);
}

/// A sign-in Wealthsimple refused ends on disk too: a later start with no answer
/// reads it ended, never saved.
#[test]
fn a_refused_sign_in_is_gone_from_disk_so_a_later_start_with_no_answer_is_not_connected() {
    let _g = crate::tests_common::guard();
    let (_home, app) = app_answering(vec![(400, INVALID_GRANT), (0, "the connection dropped"), (0, "the connection dropped")]);
    saved_expiring(&app, "refused-then-unanswered", 3600.0);
    session::boot_session(&app);
    assert!(session::load_session(&app).unwrap().is_none(), "the refused sign-in is gone from disk");
    session::boot_session(&app);
    assert!(!crate::status::status(&app).connected);
}

/// The access token running out while Wealthsimple cannot be reached is not a
/// sign-out either: the refresh renews it once it can be asked.
#[test]
fn a_refresh_with_no_answer_stays_connected_past_the_access_token_s_expiry_and_one_refused_does_not() {
    let _g = crate::tests_common::guard();
    let (_home, app) = app_answering(vec![(0, "the connection dropped"), (400, INVALID_GRANT)]);
    saved_expiring(&app, "expired-offline", -60.0);
    session::heard(&app, session::Heard::Issued);
    let a = session::refresh_now(&app);
    assert!(!a.ok && a.connected, "no answer: still connected ({})", a.error);
    let a = session::refresh_now(&app);
    assert!(!a.ok && !a.connected, "refused: connect again");
    assert_eq!(a.error, bagholder_wealthsimple::session::LOST_REFRESH, "the refusal follows a post whose answer was lost");
}

/// Every on/off setting is read by the one parser: no crate reads one of them on its own.
#[test]
fn every_on_off_setting_is_read_by_the_one_parser() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut readers = Vec::new();
    let mut stack = vec![crates];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                if name != "target" && name != "tests" {
                    stack.push(p);
                }
                continue;
            }
            if !name.ends_with(".rs") || name.starts_with("tests_") || p.ends_with("net/src/switch.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            for s in bagholder_net::switch::SWITCHES {
                for form in [format!("var(\"{s}\")"), format!("var_os(\"{s}\")")] {
                    if text.contains(&form) {
                        readers.push(format!("{}: {s}", p.display()));
                    }
                }
            }
        }
    }
    assert!(readers.is_empty(), "read outside bagholder_net::switch: {readers:?}");
}

/// 64 hex digits built from one byte. Not a client id anyone was issued.
fn generated_id(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

/// Wealthsimple's login script as the current build writes the two ids: a sandbox
/// object and the production object a sign-in uses.
fn login_script(sandbox: &str, production: &str) -> String {
    format!(r#"let T={{env:"sandbox",scope:"read write",clientId:"{sandbox}"}},P={{env:"production",scope:"read write",clientId:"{production}"}};"#)
}

const SCRIPT_PATH: &str = "/assets/app-0123456789abcdef.js";

fn login_html() -> String {
    format!("<html><script src=\"https://example.invalid/p.js\"></script><script\n      src=\"{SCRIPT_PATH}\"\n      crossorigin=\"anonymous\"\n    ></script></html>", SCRIPT_PATH = SCRIPT_PATH)
}

/// Answers the paths a client-id discovery asks, and records each path. Headers
/// are not recorded: they carry the access token.
struct Pages {
    port: u16,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

fn pages(routes: Vec<(String, u16, Vec<u8>)>) -> Pages {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (hits2, stop2) = (hits.clone(), stop.clone());
    let join = thread::spawn(move || {
        while !stop2.load(Ordering::SeqCst) {
            let Ok((mut sock, _)) = listener.accept() else { break };
            sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
            sock.set_write_timeout(Some(Duration::from_secs(2))).ok();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 65536 {
                match sock.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(_) => break,
                }
            }
            let head = String::from_utf8_lossy(&buf);
            let path = head.split('\r').next().unwrap_or("").split(' ').nth(1).unwrap_or("").split('?').next().unwrap_or("").to_string();
            if !path.is_empty() {
                hits2.lock().unwrap().push(path.clone());
            }
            let (status, body) = routes.iter().find(|(p, _, _)| p == &path).map(|(_, s, b)| (*s, b.as_slice())).unwrap_or((404, b"no"));
            let resp = format!("HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.write_all(body);
        }
    });
    Pages { port, hits, stop, join: Some(join) }
}

impl Drop for Pages {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for _ in 0..50 {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Pages {
    fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

/// Points every Wealthsimple address at `url` until dropped.
struct StandIn {
    prev: Option<String>,
}

impl StandIn {
    fn at(url: &str) -> Self {
        let prev = std::env::var(bagholder_wealthsimple::hosts::BASE_VAR).ok();
        std::env::set_var(bagholder_wealthsimple::hosts::BASE_VAR, url);
        StandIn { prev }
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var(bagholder_wealthsimple::hosts::BASE_VAR, v),
            None => std::env::remove_var(bagholder_wealthsimple::hosts::BASE_VAR),
        }
    }
}

fn posted_client_id(posts: &[String]) -> String {
    let body: serde_json::Value = serde_json::from_str(posts.last().unwrap()).unwrap();
    body["client_id"].as_str().unwrap().to_string()
}

/// Every accepted identity key, one table: token info refuses the access token,
/// the login script names a sandbox id and a production id, and the refresh that
/// follows posts the production id. The file keeps that key, `wssdi`, and any
/// other key it had.
#[test]
fn a_login_under_any_identity_key_refreshes_with_the_production_client_id() {
    let _g = crate::tests_common::guard();
    let keys = ["identity_canonical_id", "identityCanonicalId", "canonical_id", "identity_id", "resource_owner_id", "sub"];
    let production = generated_id(0x11);
    let sandbox = generated_id(0x22);
    let (_dir, app, posts) = app_recorded(keys.iter().map(|_| (200, NEW_TOKENS.into())).collect());
    let site = pages(vec![
        ("/oauth/token/info".into(), 401, b"{}".to_vec()),
        ("/app/login".into(), 200, login_html().into_bytes()),
        (SCRIPT_PATH.into(), 200, login_script(&sandbox, &production).into_bytes()),
    ]);
    let _base = StandIn::at(&format!("http://127.0.0.1:{}", site.port));
    for (n, key) in keys.iter().enumerate() {
        let refresh = format!("refresh-{n}");
        let raw = format!(r#"{{"access_token":"saved-access","refresh_token":"{refresh}","{key}":"person-1","wssdi":"device-7","note":"kept-extra"}}"#);
        std::fs::write(app.ws_home().session_path(), &raw).unwrap();
        let _ = std::fs::remove_file(app.ws_home().client_id_path());
        let before = posts.lock().unwrap().len();
        session::boot_session(&app);
        let posted = posts.lock().unwrap();
        assert_eq!(posted.len(), before + 1, "{key}: token info refused, so the script's id is posted");
        let cid = posted_client_id(&posted);
        assert_eq!(cid, production, "{key}");
        assert_ne!(cid, sandbox, "{key}");
        drop(posted);
        let saved = std::fs::read_to_string(app.ws_home().session_path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(v[key], "person-1", "{key} stays");
        assert_eq!(v["wssdi"], "device-7");
        assert_eq!(v["note"], "kept-extra");
        assert_eq!(v["client_id"], production);
    }
}

/// A client id already stored is the one posted. Two production ids, or a page
/// that is not the app, post nothing and leave the file as it was; the page
/// that is not the app does not block a later script.
#[test]
fn a_stored_client_id_is_posted_and_anything_ambiguous_is_not() {
    let _g = crate::tests_common::guard();
    let stored = generated_id(0x44);
    let production = generated_id(0x11);
    let sandbox = generated_id(0x22);
    let other = generated_id(0x55);
    let (_dir, app, posts) = app_recorded(vec![(200, NEW_TOKENS.into()), (200, NEW_TOKENS.into())]);
    let login = format!(r#"{{"access_token":"saved-access","refresh_token":"kept-refresh","client_id":"{stored}","identity_canonical_id":"person-1","wssdi":"device-7","note":"kept-extra"}}"#);
    std::fs::write(app.ws_home().session_path(), &login).unwrap();
    let site = pages(vec![
        ("/oauth/token/info".into(), 200, format!(r#"{{"application_uid":"{other}"}}"#).into_bytes()),
        ("/app/login".into(), 200, login_html().into_bytes()),
        (SCRIPT_PATH.into(), 200, login_script(&sandbox, &production).into_bytes()),
    ]);
    let _base = StandIn::at(&format!("http://127.0.0.1:{}", site.port));
    session::boot_session(&app);
    assert!(posts.lock().unwrap().is_empty(), "a stored id is not replaced, and an accepted token is not refreshed");
    assert!(!site.hits().iter().any(|p| p == SCRIPT_PATH || p == "/app/login"), "the script is not read when an id is stored: {:?}", site.hits());
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(app.ws_home().session_path()).unwrap()).unwrap();
    assert_eq!(saved["client_id"], stored);
    assert_eq!(saved["wssdi"], "device-7");
    assert_eq!(saved["note"], "kept-extra");
    let no_id = r#"{"access_token":"saved-access","refresh_token":"kept-refresh","identity_canonical_id":"person-1","wssdi":"device-7","note":"kept-extra"}"#;
    std::fs::write(app.ws_home().session_path(), no_id).unwrap();
    session::boot_session(&app);
    assert!(posts.lock().unwrap().is_empty(), "an accepted token is not refreshed");
    assert!(!site.hits().iter().any(|p| p == SCRIPT_PATH), "token info supplies the id, so the script is not read");
    assert!(!app.ws_home().client_id_path().exists(), "an id from token info is not written as if the script had been read");
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(app.ws_home().session_path()).unwrap()).unwrap();
    assert_eq!(saved["client_id"], other);
    drop(_base);
    drop(site);

    let bare = r#"{"access_token":"saved-access","refresh_token":"kept-refresh","identity_id":"person-1","wssdi":"device-7","note":"kept-extra"}"#;
    std::fs::write(app.ws_home().session_path(), bare).unwrap();
    let two = format!(r#"{{env:"production",clientId:"{sandbox}"}},{{env:"production",clientId:"{production}"}}"#);
    let site = pages(vec![
        ("/oauth/token/info".into(), 401, b"{}".to_vec()),
        ("/app/login".into(), 200, format!("<html><script src=\"{SCRIPT_PATH}\"></script></html>", SCRIPT_PATH = SCRIPT_PATH).into_bytes()),
        (SCRIPT_PATH.into(), 200, two.into_bytes()),
    ]);
    let _base = StandIn::at(&format!("http://127.0.0.1:{}", site.port));
    session::boot_session(&app);
    assert!(posts.lock().unwrap().is_empty(), "two production ids are not a guess");
    assert_eq!(std::fs::read_to_string(app.ws_home().session_path()).unwrap(), bare);
    drop(_base);
    drop(site);

    std::fs::write(app.ws_home().session_path(), bare).unwrap();
    let site = pages(vec![
        ("/oauth/token/info".into(), 401, b"{}".to_vec()),
        ("/app/login".into(), 200, b"<html><p>not the app</p></html>".to_vec()),
    ]);
    let _base = StandIn::at(&format!("http://127.0.0.1:{}", site.port));
    session::boot_session(&app);
    assert!(posts.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(app.ws_home().session_path()).unwrap(), bare, "a page that is not the app changes nothing");
    drop(_base);
    drop(site);

    let site = pages(vec![
        ("/oauth/token/info".into(), 401, b"{}".to_vec()),
        ("/app/login".into(), 200, login_html().into_bytes()),
        (SCRIPT_PATH.into(), 200, login_script(&sandbox, &production).into_bytes()),
    ]);
    let _base = StandIn::at(&format!("http://127.0.0.1:{}", site.port));
    session::boot_session(&app);
    assert_eq!(posted_client_id(&posts.lock().unwrap()), production, "the next page that is the app still refreshes");
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(app.ws_home().session_path()).unwrap()).unwrap();
    assert_eq!(saved["wssdi"], "device-7");
    assert_eq!(saved["note"], "kept-extra");
    assert_eq!(saved["identity_id"], "person-1");
}

