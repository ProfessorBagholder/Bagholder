//! The Wealthsimple client against a stand-in server on 127.0.0.1
//! (tests/test_store.py, WealthsimpleHttpTest and StoreTest).
//!
//! Every test here sets `BAGHOLDER_WS_BASE`, so they run one at a time under
//! one lock.

use bagholder_ws::session::{self, Client, Home, REFUSED_LOGIN_MESSAGE};
use bagholder_ws::{fetch, queries, sync};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

const FAKE_CLIENT_ID: &str = "abababababababababababababababababababababababababababababababab";

static ENV: Mutex<()> = Mutex::new(());
static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    body: Value,
}

type Handler = dyn Fn(&Req) -> (u16, Vec<(String, String)>, Vec<u8>) + Send + Sync;

struct Fixture {
    _guard: MutexGuard<'static, ()>,
    home: Home,
    reqs: Arc<Mutex<Vec<Req>>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::env::remove_var("BAGHOLDER_WS_BASE");
        let _ = std::fs::remove_dir_all(&self.home.dir);
    }
}

impl Fixture {
    fn client(&self) -> Client<'_> {
        Client { home: &self.home }
    }
    fn requests(&self) -> Vec<Req> {
        self.reqs.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Req> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).ok()?;
    let body = serde_json::from_slice(&buf).unwrap_or(Value::Null);
    Some(Req { method, path, body })
}

/// A fresh home and a server answering with `handler`, `BAGHOLDER_WS_BASE`
/// pointed at it.
fn fixture(handler: Box<Handler>) -> Fixture {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir: PathBuf = std::env::temp_dir().join(format!("bh-ws-http-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = Home::new(&dir);
    // clears a refused refresh token another test left behind
    home.delete_session();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reqs: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = reqs.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream { Ok(s) => s, Err(_) => continue };
            while let Some(req) = read_request(&mut stream) {
                seen.lock().unwrap().push(req.clone());
                let (status, headers, body) = handler(&req);
                let mut head = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", status, body.len());
                for (k, v) in headers {
                    head.push_str(&format!("{}: {}\r\n", k, v));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                break;
            }
        }
    });
    std::env::set_var("BAGHOLDER_WS_BASE", format!("http://127.0.0.1:{}", port));
    Fixture { _guard: guard, home, reqs }
}

fn ok_json(v: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    (200, vec![("Content-Type".into(), "application/json".into())], serde_json::to_vec(&v).unwrap())
}

fn graphql(data: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    ok_json(json!({"data": data}))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// `gzip.compress`, with one stored deflate block.
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let len = data.len() as u16;
    out.push(1);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

fn unused() -> Box<Handler> {
    Box::new(|_| ok_json(json!({})))
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {}", v))
}

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
