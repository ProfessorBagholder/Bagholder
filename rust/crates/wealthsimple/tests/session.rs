//! The session's rules (`docs/plans/stage-3b-wealthsimple.md`, "The session"),
//! each on a fake network that answers in order and counts what it was asked.
//! A read refused for its session and a read of what does not exist answer as
//! Wealthsimple answered them (the owner's browser, 2026-09-24).

use std::sync::{Arc, Mutex};

use bagholder_broker::Failure;
use bagholder_core::json::{self, Value};
use bagholder_net::{Ask, Limiter, ManualClock, Net, NetError, Transport};
use bagholder_wealthsimple::client::Client;
use bagholder_wealthsimple::session::{refresh, take_over, SessionFile, Tokens, LOST_REFRESH, REFUSED_SIGN_IN};

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
        // status 0: the request went out and no answer came back
        if status == 0 {
            return Err(NetError::Unreachable(body));
        }
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
    Tokens { access: "old-access".into(), refresh: refresh_token.into(), client_id: "the-client".into(), identity: "identity-1".into(), expires_at: None, device: None }
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

#[test]
fn two_refreshes_at_once_post_the_refresh_token_once() {
    // a read and an order finding the access token stale at the same moment:
    // one posts, the other adopts what it saved
    let q = queue(&[(200, r#"{"access_token":"fresh-access","refresh_token":"fresh-refresh","expires_in":1800}"#)]);
    let (_d, file) = session_file("shared-token");
    let n = net(&q);
    let start = std::sync::Barrier::new(2);
    let got: Vec<Tokens> = std::thread::scope(|s| {
        let both: Vec<_> = (0..2)
            .map(|_| {
                s.spawn(|| {
                    start.wait();
                    refresh(&n, &file, &held("shared-token")).unwrap()
                })
            })
            .collect();
        both.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(q.asked.lock().unwrap().len(), 1, "the refresh token is posted once");
    assert!(got.iter().all(|t| t.refresh == "fresh-refresh"), "{got:?}");
}

const NEW_TOKENS: &str = r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":1800,"token_type":"Bearer"}"#;
const INVALID_GRANT: &str = r#"{"error":"invalid_grant","error_description":"The provided authorization grant is invalid"}"#;

#[test]
fn a_captured_sign_in_is_written_only_once_wealthsimple_has_answered_with_new_tokens() {
    let (_d, file) = session_file("the-saved-one");
    let before = std::fs::read_to_string(&file.path).unwrap();
    let write = |t: &Tokens| file.save(&json::parse(&before).unwrap(), t);
    // refused at first sight: the saved sign-in stays, and the reason is the refusal
    let q = queue(&[(400, INVALID_GRANT)]);
    assert_eq!(take_over(&net(&q), &held("captured-refused"), write), Err(Failure::Lapsed(REFUSED_SIGN_IN.into())));
    assert_eq!(std::fs::read_to_string(&file.path).unwrap(), before, "nothing written for a refused capture");
    // no answer: nothing written either
    let q = queue(&[(0, "the connection dropped")]);
    assert!(matches!(take_over(&net(&q), &held("captured-unanswered"), write), Err(Failure::Unreachable(_))));
    assert_eq!(std::fs::read_to_string(&file.path).unwrap(), before);
    // answered: written, with Wealthsimple's tokens
    let q = queue(&[(200, NEW_TOKENS)]);
    let got = take_over(&net(&q), &held("captured-good"), write).unwrap();
    assert_eq!(got.refresh, "new-refresh");
    assert!(std::fs::read_to_string(&file.path).unwrap().contains("new-refresh"));
}

#[test]
fn a_refusal_after_a_lost_answer_is_said_as_the_lost_answer_not_as_a_refusal() {
    let (_d, file) = session_file("lost-one");
    let q = queue(&[(0, "the connection dropped"), (400, INVALID_GRANT)]);
    let n = net(&q);
    assert!(matches!(refresh(&n, &file, &held("lost-one")), Err(Failure::Unreachable(_))));
    assert_eq!(refresh(&n, &file, &held("lost-one")), Err(Failure::Lapsed(LOST_REFRESH.into())), "the first post was taken and its answer lost");
}

/// What the app knows of each refresh token is kept per token: a second token posted
/// meanwhile (a sign-in captured) neither hides the first's lost answer nor lets a
/// refused one be posted again.
#[test]
fn a_second_token_posted_meanwhile_hides_nothing_known_of_the_first() {
    let (_d, file) = session_file("per-token-a");
    // the first token's post goes unanswered; a captured sign-in's goes unanswered too
    let q = queue(&[(0, "the connection dropped"), (0, "the connection dropped"), (400, INVALID_GRANT)]);
    let n = net(&q);
    assert!(matches!(refresh(&n, &file, &held("per-token-a")), Err(Failure::Unreachable(_))));
    assert!(matches!(take_over(&n, &held("per-token-b"), |_| Ok(())), Err(Failure::Unreachable(_))));
    // the first refused now: its answer was lost, not refused at first sight
    assert_eq!(refresh(&n, &file, &held("per-token-a")), Err(Failure::Lapsed(LOST_REFRESH.into())));
    // another token refused after it: the first is still never posted again
    let q = queue(&[(400, INVALID_GRANT)]);
    let n = net(&q);
    assert_eq!(take_over(&n, &held("per-token-c"), |_| Ok(())), Err(Failure::Lapsed(REFUSED_SIGN_IN.into())));
    assert_eq!(refresh(&n, &file, &held("per-token-a")), Err(Failure::Lapsed(REFUSED_SIGN_IN.into())));
    assert_eq!(q.asked.lock().unwrap().len(), 1, "the refused first token was not posted again");
}
