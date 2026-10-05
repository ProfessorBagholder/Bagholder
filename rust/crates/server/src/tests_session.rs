//! The sign-in is real before it is used (`docs/plans/stage-money.md`, part C): a
//! captured login is written only once Wealthsimple has answered with new tokens,
//! and "connected" has one writer.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::app::App;
use crate::session::{self, Capture, Taken};

/// Answers the token endpoint with the next reply queued; status 0 is a request
/// that went out and got no answer.
struct Token(Mutex<Vec<(u16, &'static str)>>);
struct Shared(Arc<Token>);

impl bagholder_net::Transport for Shared {
    fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
        assert_eq!(ask.url, bagholder_wealthsimple::session::TOKEN_URL, "only the token endpoint is asked");
        let (status, body) = self.0 .0.lock().unwrap().remove(0);
        if status == 0 {
            return Err(bagholder_net::NetError::Unreachable(body.into()));
        }
        Ok((status, ask.url.to_string(), vec![], body.as_bytes().to_vec()))
    }
}

fn app_answering(name: &str, replies: Vec<(u16, &'static str)>) -> Arc<App> {
    let dir = std::env::temp_dir().join(format!("bh-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let net = bagholder_net::Net::answered_by(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()), Box::new(Shared(Arc::new(Token(Mutex::new(replies))))));
    App::with_net(dir, PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net)
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
    let app = app_answering("refused", vec![(400, INVALID_GRANT)]);
    let before = saved(&app);
    assert!(matches!(session::take_capture(&app, &captured("captured-refused")), Taken::Refused(_)));
    assert_eq!(std::fs::read_to_string(app.ws_home().session_path()).unwrap(), before, "nothing written before Wealthsimple accepts");
}

#[test]
fn a_capture_with_no_answer_leaves_the_saved_login_and_is_tried_again() {
    let _g = crate::tests_common::guard();
    let app = app_answering("unanswered", vec![(0, "the connection dropped"), (200, NEW_TOKENS)]);
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
            // a file's own test module is a test's set-up, not a writer
            let code = text.split("#[cfg(test)]\nmod ").next().unwrap();
            for (i, line) in code.lines().enumerate() {
                if line.contains(".connected =") {
                    writers.push(format!("{name}:{}", i + 1));
                }
            }
        }
    }
    assert!(writers.iter().all(|w| w.starts_with("session.rs:")), "connected written outside session::heard: {writers:?}");
    assert_eq!(writers.len(), 3, "the three arms of heard: {writers:?}");
}

#[test]
fn a_refresh_with_no_answer_while_the_token_held_is_live_stays_connected_and_one_refused_does_not() {
    let _g = crate::tests_common::guard();
    let app = app_answering("live", vec![(0, "the connection dropped"), (400, INVALID_GRANT)]);
    let mut sess = bagholder_ws::session::Session {
        access_token: "live-access".into(),
        refresh_token: "live-refresh".into(),
        client_id: bagholder_ws::standin::FAKE_CLIENT_ID.into(),
        ids: bagholder_ws::session::IdentityKeys { identity_canonical_id: "identity-1".into(), ..Default::default() },
        ..Default::default()
    };
    sess.expires_at = Some(bagholder_ws::session::Expiry::Unix(crate::app::now_unix() + 3600.0));
    session::save_session(&app, &sess).unwrap();
    session::heard(&app, session::Heard::Issued);
    let a = session::refresh_now(&app);
    assert!(!a.ok && a.connected, "the token held is live: still connected ({})", a.error);
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
