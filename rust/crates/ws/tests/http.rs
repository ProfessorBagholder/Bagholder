//! The Wealthsimple client against a stand-in server on 127.0.0.1
//! and the store it writes to.
//!
//! Every test here sets `BAGHOLDER_WS_BASE`, so they run one at a time under
//! one lock. The server harness itself lives in `tests/common/mod.rs`, shared
//! with `tests/golden_fetch.rs`.

mod common;

use bagholder_ws::session::{self, REFUSED_LOGIN_MESSAGE};
use bagholder_ws::{fetch, queries, sync};
use common::{f, fixture, gzip, ok_json, unused, Handler};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use common::graphql;

const FAKE_CLIENT_ID: &str = common::FAKE_CLIENT_ID;

#[test]
fn test_session_client_id_is_written_to_disk() {
    let fx = fixture(unused());
    let sess = json!({"client_id": FAKE_CLIENT_ID});
    let found = fx.client().client_id_for(&sess);
    assert_eq!(found, FAKE_CLIENT_ID);
    assert!(fx.home.client_id_path().exists());
    assert_eq!(fx.home.cached_client_id(), FAKE_CLIENT_ID);
}

#[test]
fn test_token_info_uid_is_stored() {
    // `apply_token_info_client_id` lives in the server crate; the ws part is the uid read
    let _fx = fixture(unused());
    assert_eq!(session::client_id_from_token_info(&json!({"application_uid": FAKE_CLIENT_ID})), FAKE_CLIENT_ID);
    assert_eq!(session::client_id_from_token_info(&json!({"application": {"uid": FAKE_CLIENT_ID}})), FAKE_CLIENT_ID);
}

#[test]
fn test_refresh_session_without_client_id_does_not_scrape_or_post() {
    let fx = fixture(unused());
    fx.home.save_session(&json!({"refresh_token": "r"})).unwrap();
    assert!(!fx.home.client_id_path().exists());
    let mut sess = json!({"refresh_token": "r"});
    let res = fx.client().refresh_session(&mut sess, true);
    assert_eq!(res, Err("session has no client id".to_string()));
    assert!(fx.requests().is_empty());
    assert!(fx.home.session_path().exists());
    assert_eq!(fx.home.load_session().unwrap()["refresh_token"], "r");
}

#[test]
fn test_parse_margin_and_fetch_margin() {
    let available = json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {"__typename": "BuyingPowerMetricAvailable", "total": {"amount": "6817.33", "currency": "CAD"}, "restrictions": []}}}}}}});
    let unavailable = json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {"__typename": "BuyingPowerMetricUnavailable", "reason": {"__typename": "UnavailableSecurities", "securities": [{"securityId": "s1", "status": "x"}, {"securityId": "s2", "status": "x"}]}}}}}}}});
    let none = json!({"account": {"financials": {"current": {"marginV3": null}}}});
    assert_eq!(fetch::parse_margin(&available), Some(json!({"buyingPower": 6817.33, "currency": "CAD", "unavailable": ""})));
    assert_eq!(fetch::parse_margin(&unavailable), Some(json!({"buyingPower": null, "currency": "CAD", "unavailable": "UnavailableSecurities (2 securities)"})));
    assert_eq!(fetch::parse_margin(&none), None);
    assert_eq!(fetch::parse_margin(&json!({})), None);

    let answers = json!({"acct-1": available, "acct-2": none, "acct-3": unavailable});
    let fx = fixture(Box::new(move |req| graphql(answers[req.body["variables"]["accountId"].as_str().unwrap()].clone())));
    let ids: Vec<String> = ["acct-1", "acct-2", "acct-3", ""].iter().map(|s| s.to_string()).collect();
    let rows = fetch::fetch_margin(&fx.client(), &json!({"access_token": "x"}), &ids, "2026-09-16T12:00:00Z");
    let calls = fx.requests();
    assert_eq!(calls.iter().map(|c| c.body["operationName"].clone()).collect::<Vec<_>>(), vec![json!("FetchAccountCurrentMarginBuyingPowerV2"); 3]);
    assert_eq!(calls.iter().map(|c| c.body["variables"]["currency"].clone()).collect::<Vec<_>>(), vec![json!("CAD"); 3]);
    let got: Vec<(Value, Value, Value)> = rows.iter().map(|r| (r["accountId"].clone(), r["buyingPower"].clone(), r["unavailable"].clone())).collect();
    assert_eq!(got, vec![
        (json!("acct-1"), json!(6817.33), json!("")),
        (json!("acct-3"), Value::Null, json!("UnavailableSecurities (2 securities)")),
    ], "an account without margin figures is not a row");
    assert!(rows.iter().all(|r| !r["fetchedAt"].as_str().unwrap_or("").is_empty()));
}

#[test]
fn test_refresh_session_uses_cached_client_id_file() {
    let fx = fixture(Box::new(|_| ok_json(json!({"access_token": "tok", "expires_in": 3600}))));
    fx.home.save_client_id(FAKE_CLIENT_ID);
    let mut sess = json!({"refresh_token": "r"});
    let res = fx.client().refresh_session(&mut sess, true);
    assert_eq!(res, Ok(()));
    let calls = fx.requests();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].method.as_str(), calls[0].path.as_str()), ("POST", "/oauth/token"));
    assert_eq!(sess["client_id"], FAKE_CLIENT_ID);
    let exp = sess["expires_at"].as_str().expect("expires_at is a string");
    assert!(exp.contains('T'));
    assert!(exp.ends_with('Z'));
}

#[test]
fn test_refresh_session_sets_http_and_oauth_error() {
    let fx = fixture(Box::new(|_| {
        let (_, h, b) = ok_json(json!({"error": "invalid_client"}));
        (401, h, b)
    }));
    let mut sess = json!({"refresh_token": "r", "client_id": FAKE_CLIENT_ID});
    fx.home.save_session(&sess).unwrap();
    let err = fx.client().refresh_session(&mut sess, true).unwrap_err();
    assert!(err.contains("HTTP 401"));
    assert!(err.contains("invalid_client"));
    assert!(err.starts_with("Wealthsimple token refresh HTTP 401"));
    assert!(!err.contains(FAKE_CLIENT_ID));
    assert!(!err.split_whitespace().any(|w| w == "r"));
    assert!(fx.home.session_path().exists());
    assert_eq!(fx.home.load_session().unwrap()["refresh_token"], "r");
}

#[test]
fn test_refresh_session_sets_http_error() {
    let fx = fixture(Box::new(|_| {
        let (_, h, b) = ok_json(json!({"error": "invalid_grant"}));
        (400, h, b)
    }));
    let mut sess = json!({"refresh_token": "r", "client_id": FAKE_CLIENT_ID});
    fx.home.save_session(&sess).unwrap();
    let err = fx.client().refresh_session(&mut sess, true).unwrap_err();
    assert_eq!(err, REFUSED_LOGIN_MESSAGE);
    assert!(fx.home.session_path().exists());
    assert_eq!(fx.home.load_session().unwrap()["refresh_token"], "r");
}

#[test]
fn test_refresh_session_sets_oauth_error_text() {
    let fx = fixture(Box::new(|_| ok_json(json!({"error": "invalid_grant"}))));
    let mut sess = json!({"refresh_token": "r", "client_id": FAKE_CLIENT_ID});
    let err = fx.client().refresh_session(&mut sess, true).unwrap_err();
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

fn empty_identity_history() -> Box<Handler> {
    Box::new(|_| graphql(json!({"identity": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {}}}}})))
}

#[test]
fn test_fetch_nav_history_since_date_skips_older_years() {
    let fx = fixture(empty_identity_history());
    fetch::fetch_nav_history(&fx.client(), &json!({"access_token": "t"}), "ident-1", Some("2026-08-30"), "2026-09-16").unwrap();
    let calls = fx.requests();
    assert!(!calls.is_empty());
    for c in calls {
        let start = c.body["variables"]["startDate"].as_str().unwrap().to_string();
        assert!(start.as_str() >= "2026-08-30");
        assert!(start.starts_with("2026"));
    }
}

#[test]
fn test_fetch_nav_history_identity_wide_omits_account_ids() {
    let fx = fixture(empty_identity_history());
    fetch::fetch_nav_history(&fx.client(), &json!({"access_token": "t"}), "ident-1", None, "2026-09-16").unwrap();
    let calls = fx.requests();
    assert!(!calls.is_empty());
    let q = queries::query("IdentityHistoricalFinancialsQuery").unwrap();
    for c in calls {
        assert_eq!(c.body["operationName"], "IdentityHistoricalFinancialsQuery");
        assert!(c.body["variables"].get("accountIds").is_none());
        assert_eq!(c.body["query"], q);
        assert_eq!(c.body["variables"]["limit"], 400);
    }
    assert!(!q.contains("$accountIds"));
    assert!(!q.contains("accounts: $accountIds"));
}

#[test]
fn test_fetch_account_nav_history_uses_account_query() {
    let fx = fixture(Box::new(|_| graphql(json!({"account": {"financials": {"historicalDaily": {"edges": [{"node": {"date": "2024-01-02", "netLiquidationValueV2": {"amount": "12.5", "currency": "CAD"}, "netDepositsV2": {"amount": "3", "currency": "CAD"}}}], "pageInfo": {}}}}}))));
    let pts = fetch::fetch_account_nav_history(&fx.client(), &json!({"access_token": "t"}), "acct-1", None, "2026-09-16").unwrap();
    let calls = fx.requests();
    assert!(!calls.is_empty());
    assert_eq!(pts[0]["date"], "2024-01-02");
    assert_eq!(f(&pts[0]["equity"]), 12.5);
    assert_eq!(f(&pts[0]["netDeposits"]), 3.0);
    let q = queries::query("FetchAccountHistoricalFinancials").unwrap();
    for c in calls {
        let v = &c.body["variables"];
        assert_eq!(c.body["operationName"], "FetchAccountHistoricalFinancials");
        assert_eq!(v["id"], "acct-1");
        assert_eq!(v["first"], 400);
        assert_eq!(v["resolution"], "DAILY");
        assert!(v.get("accountIds").is_none());
        assert!(v.get("identityId").is_none());
        assert_eq!(c.body["query"], q);
    }
    assert!(q.contains("account(id: $id)"));
    assert!(q.contains("$resolution: DateResolution!"));
}

#[test]
fn test_daily_path_does_not_page_whole_history_when_rows_exist() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    let item = json!({"occurredAt": "2024-06-15T13:45:22.123Z", "canonicalId": "ws-cid-aaa-001", "status": "POSTED", "type": "DIY_BUY", "subType": "BUY", "assetSymbol": "AAA", "assetQuantity": 10, "amount": 100, "accountId": "acct-1", "currency": "CAD"});
    let row = bagholder_ws::mapping::map_activity(&item, None).unwrap();
    let id = || "00000000-0000-4000-8000-000000000001".to_string();
    bagholder_store::merge::apply_wealthsimple_mapped(&conn, &[row], &id).unwrap();
    let (start, full) = sync::activity_sync_bounds(&conn).unwrap();
    assert!(!full);
    let start = start.expect("a start date");
    assert_eq!(&start[..10], "2024-06-01", "fourteen days before the newest stored row");

    let n = Arc::new(AtomicU64::new(0));
    let fx = fixture(Box::new(move |_| {
        let seen = n.fetch_add(1, Ordering::SeqCst) + 1;
        // the server bounds the walk by startDate; every page it returns is read
        graphql(json!({"activityFeedItems": {"edges": [{"node": item.clone()}], "pageInfo": {"hasNextPage": seen < 2, "endCursor": "cursor-page-2"}}}))
    }));
    fetch::fetch_activities_for_account(&fx.client(), &json!({"access_token": "x"}), "acct-1", Some(&start), 1_789_000_000).unwrap();
    let calls = fx.requests();
    assert_eq!(calls.len(), 2, "a page of known rows does not end the walk");
    let cond = &calls[0].body["variables"]["condition"];
    assert!(cond.get("startDate").is_some());
    assert!(cond["startDate"].as_str().unwrap().starts_with("2024-06-01"));
}
