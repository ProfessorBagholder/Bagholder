//! The session's rules (`docs/plans/stage-3b-wealthsimple.md`, "The session"),
//! each on a fake network that answers in order and counts what it was asked.
//! A read refused for its session and a read of what does not exist answer as
//! Wealthsimple answered them (the owner's browser, 2026-09-24).

use std::sync::{Arc, Mutex};

use bagholder_broker::Failure;
use bagholder_core::json::{self, Value};
use bagholder_net::{Ask, Limiter, ManualClock, Net, NetError, Transport};
use bagholder_wealthsimple::client::Client;
use bagholder_wealthsimple::session::{refresh, SessionFile, Tokens};

/// Answers each request with the next reply queued, and fails a test on a
/// request with none left.
#[derive(Default)]
struct Queue {
    replies: Mutex<Vec<(u16, String)>>,
    asked: Mutex<Vec<String>>,
}

struct Shared(Arc<Queue>);

impl Transport for Shared {
    fn answer(&self, ask: &Ask) -> Result<bagholder_net::Answer, NetError> {
        self.0.asked.lock().unwrap().push(ask.url.to_string());
        let mut r = self.0.replies.lock().unwrap();
        assert!(!r.is_empty(), "a request nothing answers: {}", ask.url);
        let (status, body) = r.remove(0);
        Ok((status, ask.url.to_string(), vec![], body.into_bytes()))
    }
}

fn net(q: &Arc<Queue>) -> Net {
    Net::answered_by(Arc::new(ManualClock::at("2026-09-24T12:00:00Z".parse().unwrap())), Arc::new(Limiter::new()), Box::new(Shared(q.clone())))
}

fn queue(replies: &[(u16, &str)]) -> Arc<Queue> {
    Arc::new(Queue { replies: Mutex::new(replies.iter().map(|(s, b)| (*s, b.to_string())).collect()), asked: Mutex::new(vec![]) })
}

/// Wealthsimple's answer to a read whose session is not valid.
const UNAUTHENTICATED: &str = r#"{"errors":[{"message":"Not Authorized","extensions":{"code":"UNAUTHENTICATED"}}]}"#;

fn session_file(refresh_token: &str) -> (tempfile::TempDir, SessionFile) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    std::fs::write(&path, format!(r#"{{"access_token":"old-access","refresh_token":"{refresh_token}","client_id":"the-client","identity_canonical_id":"identity-1","expires_at":"2026-09-24T13:00:00Z","wssdi":"kept"}}"#)).unwrap();
    (dir, SessionFile { path })
}

fn held(refresh_token: &str) -> Tokens {
    Tokens { access: "old-access".into(), refresh: refresh_token.into(), client_id: "the-client".into(), identity: "identity-1".into(), expires_at: None }
}

#[test]
fn a_token_another_caller_rotated_is_adopted_and_nothing_is_posted() {
    let q = queue(&[]);
    let (_d, file) = session_file("rotated-by-another");
    let got = refresh(&net(&q), &file, &held("the-one-we-hold")).unwrap();
    assert_eq!(got.refresh, "rotated-by-another");
    assert!(q.asked.lock().unwrap().is_empty());
}

#[test]
fn a_refresh_saves_the_new_tokens_and_keeps_every_other_key() {
    let q = queue(&[(200, r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":1800,"token_type":"Bearer"}"#)]);
    let (_d, file) = session_file("r1");
    let got = refresh(&net(&q), &file, &held("r1")).unwrap();
    assert_eq!((got.access.as_str(), got.refresh.as_str()), ("new-access", "new-refresh"));
    let saved = json::parse(&std::fs::read_to_string(&file.path).unwrap()).unwrap();
    let Value::Object(m) = saved else { panic!() };
    assert_eq!(m["refresh_token"], Value::String("new-refresh".into()));
    assert_eq!(m["wssdi"], Value::String("kept".into()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file.path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn a_refused_refresh_token_is_never_posted_again() {
    let q = queue(&[(400, r#"{"error":"invalid_grant","error_description":"The provided authorization grant is invalid"}"#)]);
    let (_d, file) = session_file("refused-token");
    let n = net(&q);
    assert!(matches!(refresh(&n, &file, &held("refused-token")), Err(Failure::Lapsed(_))));
    assert!(matches!(refresh(&n, &file, &held("refused-token")), Err(Failure::Lapsed(_))));
    assert_eq!(q.asked.lock().unwrap().len(), 1);
}

#[test]
fn a_read_refused_for_its_session_is_refreshed_once_and_asked_once_more_and_never_again() {
    let ok = r#"{"data":{"securities":[]}}"#;
    let token = r#"{"access_token":"a2","refresh_token":"r2","expires_in":1800}"#;
    // refused, refreshed, answered
    let q = queue(&[(401, UNAUTHENTICATED), (200, token), (200, ok)]);
    let (_d, file) = session_file("r1");
    let n = net(&q);
    let mut c = Client::new(&n, file);
    assert!(c.graphql("Securities", json::parse(r#"{"ids":[]}"#).unwrap()).is_ok());
    assert_eq!(q.asked.lock().unwrap().len(), 3);
    // refused again after its refresh: a lapse, and nothing more is asked
    let q = queue(&[(401, UNAUTHENTICATED), (200, r#"{"access_token":"a3","refresh_token":"r3","expires_in":1800}"#), (401, UNAUTHENTICATED)]);
    let (_d2, file) = session_file("r9");
    let n = net(&q);
    let mut c = Client::new(&n, file);
    assert!(matches!(c.graphql("Securities", json::parse(r#"{"ids":[]}"#).unwrap()), Err(Failure::Lapsed(_))));
    assert_eq!(q.asked.lock().unwrap().len(), 3);
}

#[test]
fn a_graphql_error_is_a_refusal_naming_wealthsimple_s_message() {
    // a read of a transfer that does not exist: answered 200, with the error
    let q = queue(&[(200, r#"{"data":{"internalTransfer":null},"errors":[{"message":"NOT_FOUND","path":["internalTransfer"],"extensions":{"code":"NOT_FOUND"}}]}"#)]);
    let (_d, file) = session_file("r1");
    let n = net(&q);
    let mut c = Client::new(&n, file);
    match c.graphql("FetchInternalTransfer", json::parse(r#"{"id":"internal_transfer-none"}"#).unwrap()) {
        Err(Failure::Refused(w)) => assert!(w.contains("NOT_FOUND"), "{w}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_request_goes_through_the_limiter_at_wealthsimple_s_pace() {
    let q = queue(&[(200, r#"{"data":{"securities":[]}}"#), (200, r#"{"data":{"securities":[]}}"#)]);
    let (_d, file) = session_file("r1");
    let n = net(&q);
    let mut c = Client::new(&n, file);
    c.graphql("Securities", json::parse(r#"{"ids":[]}"#).unwrap()).unwrap();
    assert_eq!(n.limiter().pace("my.wealthsimple.com").gap, bagholder_wealthsimple::client::PACE);
    // the manual clock does not move: the second request waits its turn on it
    // and is answered once the gap has passed on that clock
    assert_eq!(q.asked.lock().unwrap().len(), 1);
}
