//! The Wealthsimple client against a stand-in server on 127.0.0.1
//! and the store it writes to.
//!
//! Every test here sets `BAGHOLDER_WS_BASE`, so they run one at a time under
//! one lock. The server harness itself lives in `tests/common/mod.rs`, shared
//! with the other tests here.

mod common;

use bagholder_ws::session::{self, Session, REFUSED_LOGIN_MESSAGE};
use common::{fixture, gzip, ok_json, unused};
use serde_json::json;


const FAKE_CLIENT_ID: &str = common::FAKE_CLIENT_ID;

fn sess(over: serde_json::Value) -> Session {
    let mut v = json!({});
    for (k, val) in over.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).unwrap()
}

#[test]
fn test_session_client_id_is_written_to_disk() {
    let fx = fixture(unused());
    let s = sess(json!({"client_id": FAKE_CLIENT_ID}));
    let found = fx.client().client_id_for(&s);
    assert_eq!(found, FAKE_CLIENT_ID);
    assert!(fx.home.client_id_path().exists());
    assert_eq!(fx.home.cached_client_id(), FAKE_CLIENT_ID);
}

#[test]
fn test_token_info_uid_is_stored() {
    // `apply_token_info_client_id` lives in the server crate; the ws part is the uid read
    let _fx = fixture(unused());
    let info1: session::TokenInfo = serde_json::from_value(json!({"application_uid": FAKE_CLIENT_ID})).unwrap();
    assert_eq!(session::client_id_from_token_info(&info1), FAKE_CLIENT_ID);
    let info2: session::TokenInfo = serde_json::from_value(json!({"application": {"uid": FAKE_CLIENT_ID}})).unwrap();
    assert_eq!(session::client_id_from_token_info(&info2), FAKE_CLIENT_ID);
}

#[test]
fn test_refresh_session_without_client_id_does_not_scrape_or_post() {
    let fx = fixture(unused());
    fx.home.save_session(&sess(json!({"refresh_token": "r"}))).unwrap();
    assert!(!fx.home.client_id_path().exists());
    let mut s = sess(json!({"refresh_token": "r"}));
    let res = fx.client().refresh_session(&mut s, true);
    assert_eq!(res, Err("session has no client id".to_string()));
    assert!(fx.requests().is_empty());
    assert!(fx.home.session_path().exists());
    assert_eq!(fx.home.load_session().unwrap().refresh_token, "r");
}


#[test]
fn test_refresh_session_uses_cached_client_id_file() {
    let fx = fixture(Box::new(|_| ok_json(json!({"access_token": "tok", "expires_in": 3600}))));
    fx.home.save_client_id(FAKE_CLIENT_ID);
    let mut s = sess(json!({"refresh_token": "r"}));
    let res = fx.client().refresh_session(&mut s, true);
    assert_eq!(res, Ok(()));
    let calls = fx.requests();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].method.as_str(), calls[0].path.as_str()), ("POST", "/oauth/token"));
    assert_eq!(s.client_id, FAKE_CLIENT_ID);
    let exp = match s.expires_at { Some(session::Expiry::Text(t)) => t, other => panic!("expires_at is not a text stamp: {:?}", other) };
    assert!(exp.contains('T'));
    assert!(exp.ends_with('Z'));
}

#[test]
fn test_refresh_session_sets_http_and_oauth_error() {
    let fx = fixture(Box::new(|_| {
        let (_, h, b) = ok_json(json!({"error": "invalid_client"}));
        (401, h, b)
    }));
    let mut s = sess(json!({"refresh_token": "r", "client_id": FAKE_CLIENT_ID}));
    fx.home.save_session(&s).unwrap();
    let err = fx.client().refresh_session(&mut s, true).unwrap_err();
    assert!(err.contains("HTTP 401"));
    assert!(err.contains("invalid_client"));
    assert!(err.starts_with("Wealthsimple token refresh HTTP 401"));
    assert!(!err.contains(FAKE_CLIENT_ID));
    assert!(!err.split_whitespace().any(|w| w == "r"));
    assert!(fx.home.session_path().exists());
    assert_eq!(fx.home.load_session().unwrap().refresh_token, "r");
}

#[test]
fn test_refresh_session_sets_http_error() {
    let fx = fixture(Box::new(|_| {
        let (_, h, b) = ok_json(json!({"error": "invalid_grant"}));
        (400, h, b)
    }));
    let mut s = sess(json!({"refresh_token": "r", "client_id": FAKE_CLIENT_ID}));
    fx.home.save_session(&s).unwrap();
    let err = fx.client().refresh_session(&mut s, true).unwrap_err();
    assert_eq!(err, REFUSED_LOGIN_MESSAGE);
    assert!(fx.home.session_path().exists());
    assert_eq!(fx.home.load_session().unwrap().refresh_token, "r");
}

#[test]
fn test_refresh_session_sets_oauth_error_text() {
    let fx = fixture(Box::new(|_| ok_json(json!({"error": "invalid_grant"}))));
    let mut s = sess(json!({"refresh_token": "r", "client_id": FAKE_CLIENT_ID}));
    let err = fx.client().refresh_session(&mut s, true).unwrap_err();
    assert_eq!(err, REFUSED_LOGIN_MESSAGE);
}

#[test]
fn test_http_json_invalid_body_returns_error_dict() {
    let _fx = fixture(Box::new(|_| (200, vec![], b"not-json{".to_vec())));
    let url = format!("{}/token", session::oauth_url());
    let data = session::http_json("GET", &url, None, &[]);
    assert_eq!(data["error"], "invalid_json");
    assert!(data.get("_http_status").is_some());
}

#[test]
fn test_http_json_reads_gzip_json() {
    let raw = gzip(br#"{"access_token":"tok","expires_in":3600}"#);
    let _fx = fixture(Box::new(move |_| (200, vec![("Content-Encoding".into(), "gzip".into())], raw.clone())));
    let url = format!("{}/token", session::oauth_url());
    let data = session::http_json("POST", &url, Some(&json!({"grant_type": "refresh_token"})), &[]);
    assert_eq!(data["access_token"], "tok");
    assert!(data.get("_http_status").is_none());
}

