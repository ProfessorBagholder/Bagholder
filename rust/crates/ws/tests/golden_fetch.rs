//! Golden (characterisation) test for `bagholder-ws`'s network-facing
//! surface ahead of the typed-struct conversion: every paged `fetch::*` call
//! against a stand-in GraphQL server (the harness in `tests/common/mod.rs`,
//! shared with `tests/http.rs`), the exact `variables` sent on the wire, the
//! `Client::graphql` error mapping, and what the results look like once
//! written into a fresh store through the same pipeline
//! `crates/server/src/session.rs`'s `sync_body` uses
//! (`mapping::map_activity_rows` + `fifo_pool_ids`, `merge::apply_wealthsimple_mapped`,
//! `tables::replace_accounts`/`replace_balances`/`replace_margin`/`upsert_nav`,
//! `admin::upsert_securities`).
//!
//! After an intended change to any of these: rebuild the golden with
//! `BAGHOLDER_BLESS=1 cargo test -p bagholder-ws --test golden_fetch`,
//! then read the diff in `tests/golden/fetch.json` before committing it.

mod common;

use bagholder_ws::{fetch, mapping, sync};
use common::{fixture, graphql, graphql_errors, ok_json, Req};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::cell::Cell;

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

fn vars_of(reqs: &[Req]) -> Vec<Value> {
    reqs.iter().map(|r| r.body["variables"].clone()).collect()
}

const IDENTITY: &str = "ident-1";
const TODAY_MULTI_YEAR: &str = "2021-02-10";
const TODAY_MID_YEAR: &str = "2024-09-01";

/// `fetch_all_accounts`: two pages, one edge's node null.
fn accounts_scenario(out: &mut Map<String, Value>) -> Vec<Value> {
    let fx = fixture(Box::new(|req| {
        let cursor = req.body["variables"]["cursor"].clone();
        if cursor.is_null() {
            graphql(json!({"identity": {"accounts": {
                "edges": [
                    {"node": {"id": "acc-a", "nickname": "A", "currency": "CAD", "status": "open", "type": "SelfDirected"}},
                    {"node": null},
                ],
                "pageInfo": {"hasNextPage": true, "endCursor": "page-2"}
            }}}))
        } else {
            graphql(json!({"identity": {"accounts": {
                "edges": [
                    {"node": {"id": "acc-b", "nickname": "B", "currency": "USD", "status": "open", "type": "SelfDirected"}},
                ],
                "pageInfo": {"hasNextPage": false}
            }}}))
        }
    }));
    let sess = json!({"access_token": "tok"});
    let accounts = fetch::fetch_all_accounts(&fx.client(), &sess, IDENTITY).unwrap();
    out.insert("fetch_all_accounts/result".into(), json!(accounts));
    out.insert("fetch_all_accounts/request_variables".into(), json!(vars_of(&fx.requests())));
    accounts
}

/// `fetch_activities_for_account`: three pages, a null node on page one, and a
/// third page that says `hasNextPage: true` but carries no cursor -- which
/// must stop the walk rather than loop forever.
fn activities_scenario(out: &mut Map<String, Value>) -> Vec<Value> {
    let fx = fixture(Box::new(|req| {
        let cursor = req.body["variables"]["cursor"].clone();
        match cursor.as_str() {
            None => graphql(json!({"activityFeedItems": {
                "edges": [
                    {"node": {"canonicalId": "ws-fetch-0001", "occurredAt": "2024-05-01T10:00:00Z", "accountId": "acc-a", "type": "DIY_BUY", "subType": "BUY", "status": "POSTED", "assetSymbol": "AAA", "assetQuantity": "10", "amount": "100", "amountSign": "negative", "currency": "CAD"}},
                    {"node": null},
                ],
                "pageInfo": {"hasNextPage": true, "endCursor": "cur-1"}
            }})),
            Some("cur-1") => graphql(json!({"activityFeedItems": {
                "edges": [
                    {"node": {"canonicalId": "ws-fetch-0002", "occurredAt": "2024-05-02T10:00:00Z", "accountId": "acc-a", "type": "DIY_SELL", "subType": "SELL", "status": "POSTED", "assetSymbol": "AAA", "assetQuantity": "5", "amount": "60", "amountSign": "positive", "currency": "CAD"}},
                ],
                "pageInfo": {"hasNextPage": true, "endCursor": "cur-2"}
            }})),
            Some("cur-2") => graphql(json!({"activityFeedItems": {
                "edges": [
                    {"node": {"canonicalId": "ws-fetch-0003", "occurredAt": "2024-05-03T10:00:00Z", "accountId": "acc-a", "type": "DEPOSIT", "subType": "EFT", "status": "POSTED", "assetQuantity": "0", "amount": "50", "amountSign": "positive", "currency": "CAD"}},
                ],
                "pageInfo": {"hasNextPage": true, "endCursor": ""}
            }})),
            _ => panic!("unexpected cursor {:?}", cursor),
        }
    }));
    let sess = json!({"access_token": "tok"});
    let items = fetch::fetch_activities_for_account(&fx.client(), &sess, "acc-a", None, 1_789_000_000).unwrap();
    out.insert("fetch_activities_for_account/result".into(), json!(items));
    out.insert("fetch_activities_for_account/request_variables".into(), json!(vars_of(&fx.requests())));
    items
}

/// `fetch_balances`: 23 ids, chunked 20 + 3; balance as an array, as a single
/// object, and missing altogether.
fn balances_scenario(out: &mut Map<String, Value>) -> Vec<Value> {
    let ids: Vec<String> = (1..=23).map(|i| format!("bal-{:02}", i)).collect();
    let fx = fixture(Box::new(|req| {
        let list = req.body["variables"]["ids"].as_array().cloned().unwrap_or_default();
        let accounts: Vec<Value> = list
            .iter()
            .map(|idv| {
                let id = idv.as_str().unwrap_or("").to_string();
                let custodian = if id == "bal-01" {
                    json!([{"id": format!("cust-{}", id), "financials": {"balance": [
                        {"securityId": "sec-1", "quantity": "10"},
                        {"securityId": "sec-2", "quantity": 5},
                    ]}}])
                } else if id == "bal-05" {
                    json!([{"id": format!("cust-{}", id), "financials": {"balance": {"securityId": "sec-3", "quantity": "2.5"}}}])
                } else if id == "bal-21" {
                    json!([{"id": format!("cust-{}", id), "financials": {}}])
                } else {
                    json!([])
                };
                json!({"id": id, "custodianAccounts": custodian})
            })
            .collect();
        graphql(json!({"accounts": accounts}))
    }));
    let sess = json!({"access_token": "tok"});
    let balances = fetch::fetch_balances(&fx.client(), &sess, &ids).unwrap();
    let reqs = fx.requests();
    out.insert("fetch_balances/result".into(), json!(balances));
    out.insert("fetch_balances/chunk_sizes".into(), json!(reqs.iter().map(|r| r.body["variables"]["ids"].as_array().map(|a| a.len()).unwrap_or(0)).collect::<Vec<_>>()));
    out.insert("fetch_balances/request_variables".into(), json!(vars_of(&reqs)));
    balances
}

/// `fetch_margin`: one available account, one unavailable, one whose request
/// answers a GraphQL error -- which must not fail the whole call.
fn margin_scenario(out: &mut Map<String, Value>) -> Vec<Value> {
    let fx = fixture(Box::new(|req| {
        match req.body["variables"]["accountId"].as_str().unwrap_or("") {
            "margin-ok" => graphql(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
                "__typename": "BuyingPowerMetricAvailable", "total": {"amount": "999.99", "currency": "CAD"}
            }}}}}}})),
            "margin-unavailable" => graphql(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
                "__typename": "BuyingPowerMetricUnavailable",
                "reason": {"__typename": "UnavailableSecurities", "securities": [{"securityId": "s1"}]}
            }}}}}}})),
            "margin-error" => graphql_errors(json!([{"message": "margin service unavailable"}])),
            other => panic!("unexpected accountId {}", other),
        }
    }));
    let sess = json!({"access_token": "tok"});
    let ids = vec!["margin-ok".to_string(), "margin-unavailable".to_string(), "margin-error".to_string()];
    let rows = fetch::fetch_margin(&fx.client(), &sess, &ids, "2026-09-22T00:00:00Z");
    out.insert("fetch_margin/result".into(), json!(rows));
    out.insert("fetch_margin/request_variables".into(), json!(vars_of(&fx.requests())));
    rows
}

/// `fetch_nav_history` (identity-wide): `since` is `None`, so the walk covers
/// every year up to `today`; year 2020 pages twice and revises one date on the
/// second page.
fn nav_history_scenario(out: &mut Map<String, Value>) -> Vec<Value> {
    let fx = fixture(Box::new(|req| {
        let start = req.body["variables"]["startDate"].as_str().unwrap_or("").to_string();
        let cursor = req.body["variables"]["cursor"].clone();
        if start == "2020-01-01" && cursor.is_null() {
            graphql(json!({"identity": {"financials": {"historicalDaily": {
                "edges": [{"node": {"date": "2020-03-01", "netLiquidationValue": {"amount": "100", "currency": "CAD"}, "netDeposits": {"amount": "10", "currency": "CAD"}}}],
                "pageInfo": {"hasNextPage": true, "endCursor": "y2020-p2"}
            }}}}))
        } else if start == "2020-01-01" && cursor.as_str() == Some("y2020-p2") {
            graphql(json!({"identity": {"financials": {"historicalDaily": {
                "edges": [
                    {"node": {"date": "2020-03-01", "netLiquidationValue": {"amount": "200", "currency": "CAD"}, "netDeposits": {"amount": "20", "currency": "CAD"}}},
                    {"node": {"date": "2020-04-01", "netLiquidationValue": {"amount": "50", "currency": "CAD"}}},
                ],
                "pageInfo": {"hasNextPage": false}
            }}}}))
        } else {
            graphql(json!({"identity": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {"hasNextPage": false}}}}}))
        }
    }));
    let sess = json!({"access_token": "tok"});
    let points = fetch::fetch_nav_history(&fx.client(), &sess, IDENTITY, None, TODAY_MULTI_YEAR).unwrap();
    out.insert("fetch_nav_history/result".into(), json!(points));
    out.insert("fetch_nav_history/request_variables".into(), json!(vars_of(&fx.requests())));
    points
}

/// `fetch_account_nav_history`: `since` mid-year, one page, one year.
fn account_nav_history_scenario(out: &mut Map<String, Value>) -> Vec<Value> {
    let fx = fixture(Box::new(|_req| {
        graphql(json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-07-01", "netLiquidationValueV2": {"amount": "300", "currency": "USD"}, "netDepositsV2": {"amount": "30", "currency": "USD"}}}],
            "pageInfo": {"hasNextPage": false}
        }}}}))
    }));
    let sess = json!({"access_token": "tok"});
    let points = fetch::fetch_account_nav_history(&fx.client(), &sess, "acc-b", Some("2024-06-15"), TODAY_MID_YEAR).unwrap();
    out.insert("fetch_account_nav_history/result".into(), json!(points));
    out.insert("fetch_account_nav_history/request_variables".into(), json!(vars_of(&fx.requests())));
    points
}

/// `fetch_securities`: a batch that answers directly, and a batch whose
/// `FetchSecurities` call errors and falls back to one `FetchSecurity` call
/// per id, one of which also errors.
fn securities_scenario(out: &mut Map<String, Value>) {
    let sess = json!({"access_token": "tok"});
    // Each `fixture()` takes the same process-wide lock for its whole life,
    // so the first stand-in server must be dropped before the second is
    // created -- two live at once would deadlock this thread against itself.
    {
        let fx_ok = fixture(Box::new(|req| {
            assert_eq!(req.body["operationName"], "FetchSecurities");
            graphql(json!({"securities": [
                {"id": "sec-a", "currency": "CAD", "stock": {"symbol": "AAA", "name": "AAA Inc", "primaryExchange": "TSX", "primaryMic": "XTSE"}},
                {"id": "sec-b", "currency": "USD", "optionDetails": {"underlyingSecurity": {"id": "sec-a"}}},
            ]}))
        }));
        let ok = fetch::fetch_securities(&fx_ok.client(), &sess, &["sec-a".to_string(), "sec-b".to_string()]);
        out.insert("fetch_securities/batch_ok/result".into(), json!(ok));
        out.insert("fetch_securities/batch_ok/request_variables".into(), json!(vars_of(&fx_ok.requests())));
    }

    let fx_fallback = fixture(Box::new(|req| {
        if req.body["operationName"] == "FetchSecurities" {
            graphql_errors(json!([{"message": "batch lookup failed"}]))
        } else {
            match req.body["variables"]["securityId"].as_str().unwrap_or("") {
                "sec-x" => graphql(json!({"security": {"id": "sec-x", "currency": "CAD", "stock": {"symbol": "XXX"}}})),
                "sec-y" => graphql_errors(json!([{"message": "security lookup failed"}])),
                other => panic!("unexpected securityId {}", other),
            }
        }
    }));
    let fallback = fetch::fetch_securities(&fx_fallback.client(), &sess, &["sec-x".to_string(), "sec-y".to_string()]);
    let reqs = fx_fallback.requests();
    out.insert("fetch_securities/batch_error_fallback/result".into(), json!(fallback));
    out.insert(
        "fetch_securities/batch_error_fallback/operations".into(),
        json!(reqs.iter().map(|r| r.body["operationName"].clone()).collect::<Vec<_>>()),
    );
    out.insert("fetch_securities/batch_error_fallback/request_variables".into(), json!(vars_of(&reqs)));
}

/// `Client::graphql`'s own error mapping: an HTTP 401, an `errors` array with
/// a message, an `errors` entry that only carries an `error` key, and a
/// response with `data: null` and no `errors` at all.
fn graphql_error_scenarios(out: &mut Map<String, Value>) {
    let sess = json!({"access_token": "tok"});

    // Each `fixture()` holds the process-wide lock for its whole life, so
    // every one here is scoped to its own block and dropped before the next
    // is created -- two alive at once would deadlock this thread.
    {
        let fx = fixture(Box::new(|_| (401, vec![], serde_json::to_vec(&json!({"error": "unauthorized"})).unwrap())));
        let err = fx.client().graphql(&sess, "FetchSecurity", &json!({}), None).unwrap_err();
        out.insert("graphql_error/401".into(), json!(format!("{:?}", err)));
    }
    {
        let fx = fixture(Box::new(|_| ok_json(json!({"errors": [{"message": "field X does not exist"}]}))));
        let err = fx.client().graphql(&sess, "FetchSecurity", &json!({}), None).unwrap_err();
        out.insert("graphql_error/errors_array_message".into(), json!(format!("{:?}", err)));
    }
    {
        let fx = fixture(Box::new(|_| ok_json(json!({"errors": [{"error": "rate_limited"}]}))));
        let err = fx.client().graphql(&sess, "FetchSecurity", &json!({}), None).unwrap_err();
        out.insert("graphql_error/errors_object_error_key".into(), json!(format!("{:?}", err)));
    }
    {
        let fx = fixture(Box::new(|_| ok_json(json!({"data": null}))));
        let err = fx.client().graphql(&sess, "FetchSecurity", &json!({}), None).unwrap_err();
        out.insert("graphql_error/data_null".into(), json!(format!("{:?}", err)));
    }
}

/// A deterministic id generator: shaped like a uuid, counting from one.
fn id_gen() -> impl Fn() -> String {
    let n = Cell::new(0u64);
    move || {
        n.set(n.get() + 1);
        format!("00000000-0000-4000-8000-{:012}", n.get())
    }
}

/// Every fetched result, written into a fresh store exactly as
/// `crates/server/src/session.rs`'s `sync_body` assembles it, then dumped back
/// out with `snapshot`. Generated ids come from a counter fixed at zero for
/// each write, so the dump is the same on every run.
fn store_scenario(
    out: &mut Map<String, Value>,
    accounts: &[Value],
    activity_items: &[Value],
    balances: &[Value],
    margin: &[Value],
    nav_identity: &[Value],
    nav_account: &[Value],
) {
    let conn = Connection::open_in_memory().unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();

    let acc_by_id: Value = {
        let mut m = Map::new();
        for a in accounts {
            let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !id.is_empty() {
                m.insert(id, a.clone());
            }
        }
        Value::Object(m)
    };
    let mut mapped: Vec<Value> = Vec::new();
    for it in activity_items {
        mapped.extend(mapping::map_activity_rows(it, Some(&acc_by_id)));
    }
    let pools = mapping::fifo_pool_ids(Some(&Value::Array(accounts.to_vec())));
    for row in mapped.iter_mut() {
        let aid = row.get("accountId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pool = pools.get(&aid).cloned().unwrap_or_else(|| aid.clone());
        row["fifoId"] = json!(pool);
    }

    let new_id = id_gen();
    bagholder_store::merge::apply_wealthsimple_mapped(&conn, &mapped, &new_id).unwrap();

    bagholder_store::tables::replace_accounts(&conn, &sync::slim_accounts(accounts)).unwrap();
    bagholder_store::tables::replace_balances(&conn, balances).unwrap();
    bagholder_store::tables::replace_margin(&conn, margin, "2026-09-22T00:00:00Z").unwrap();

    // identity-wide points carry accountId "", per-nickname points the
    // nickname -- exactly as `fetch_nickname_nav_history` assembles them.
    let mut combined: Vec<Value> = nav_identity
        .iter()
        .map(|r| {
            let mut m = r.as_object().cloned().unwrap_or_default();
            m.insert("accountId".into(), json!(""));
            Value::Object(m)
        })
        .collect();
    combined.extend(nav_account.iter().map(|r| {
        let mut m = r.as_object().cloned().unwrap_or_default();
        m.insert("accountId".into(), json!("B"));
        Value::Object(m)
    }));
    bagholder_store::tables::upsert_nav(&conn, &combined).unwrap();

    let securities = json!([
        {"id": "sec-a", "symbol": "AAA", "name": "AAA Inc", "primaryExchange": "TSX", "primaryMic": "XTSE", "currency": "CAD"},
        {"id": "sec-b", "symbol": "", "name": "", "primaryExchange": "", "primaryMic": "", "currency": "USD", "underlyingId": "sec-a"},
    ]);
    bagholder_store::admin::upsert_securities(&conn, securities.as_array().unwrap(), "2026-09-22T00:00:00Z").unwrap();

    let snap = bagholder_store::snapshot::snapshot(&conn, true).unwrap();
    out.insert("store/activities".into(), snap["activities"].clone());
    out.insert("store/accounts".into(), snap["accounts"].clone());
    out.insert("store/balances".into(), snap["balances"].clone());
    out.insert("store/margin".into(), snap["margin"].clone());
    out.insert("store/navHistory".into(), snap["navHistory"].clone());
    out.insert("store/navByAccount".into(), snap["navByAccount"].clone());
    out.insert("store/securities".into(), snap["securities"].clone());
}

fn answers() -> Value {
    let mut out: Map<String, Value> = Map::new();
    let accounts = accounts_scenario(&mut out);
    let items = activities_scenario(&mut out);
    let balances = balances_scenario(&mut out);
    let margin = margin_scenario(&mut out);
    let nav_identity = nav_history_scenario(&mut out);
    let nav_account = account_nav_history_scenario(&mut out);
    securities_scenario(&mut out);
    graphql_error_scenarios(&mut out);
    store_scenario(&mut out, &accounts, &items, &balances, &margin, &nav_identity, &nav_account);
    Value::Object(out)
}

#[test]
fn test_ws_network_surface_and_store_writes_are_pinned() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fetch.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/fetch.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}

#[test]
fn test_running_twice_is_deterministic() {
    let a = norm(answers());
    let b = norm(answers());
    assert_eq!(a, b, "the golden must not depend on run order or generated ids");
}
