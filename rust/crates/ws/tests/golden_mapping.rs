//! Golden (characterisation) test for `bagholder-ws`'s pure, no-network
//! surface ahead of the typed-struct conversion: every branch of
//! `mapping::map_activity` / `map_activity_rows` / `skip_activity` /
//! `signed_cash` / `option_symbol` / `is_corp_share_move` / `is_code_change`
//! over the corpus in `tests/golden/activity_items.json`; `sync::slim_accounts`,
//! `mapping::fifo_pool_ids`, `mapping::nav_account_groups`,
//! `fetch::margin_account_ids`, `mapping::account_type` and
//! `sync::margin_boost_target` over `tests/golden/accounts.json`;
//! `fetch::activity_fetch_condition`; hand-written payloads for
//! `fetch::parse_margin`, `fetch::nav_points_from_payload`,
//! `fetch::merge_nav_points`, `fetch::money_amount`, `fetch::security_record`;
//! and the token/session helpers in `sync` and `session`.
//!
//! After an intended change to any of these: rebuild the golden with
//! `BAGHOLDER_BLESS=1 cargo test -p bagholder-ws --test golden_mapping`,
//! then read the diff in `tests/golden/mapping.json` before committing it.

use bagholder_ws::{fetch, mapping, session, sync};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

fn corpus() -> Map<String, Value> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/activity_items.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v.as_object().unwrap().clone()
}

fn accounts() -> Vec<Value> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/accounts.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v.as_array().unwrap().clone()
}

const NOW_UNIX: i64 = 1_789_000_000;

fn answers() -> Value {
    let mut out: Map<String, Value> = Map::new();
    let items = corpus();
    let accts = accounts();
    let accts_arr = Value::Array(accts.clone());
    let accts_map: Value = {
        let mut m = Map::new();
        for a in &accts {
            let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !id.is_empty() {
                m.insert(id, a.clone());
            }
        }
        Value::Object(m)
    };

    // --- per-corpus-item outputs -------------------------------------------
    for (name, item) in &items {
        out.insert(format!("rows_list/{}", name), Value::Array(mapping::map_activity_rows(item, Some(&accts_arr))));
        out.insert(format!("rows_map/{}", name), Value::Array(mapping::map_activity_rows(item, Some(&accts_map))));
        out.insert(format!("rows_none/{}", name), Value::Array(mapping::map_activity_rows(item, None)));
        out.insert(format!("single/{}", name), mapping::map_activity(item, None).unwrap_or(Value::Null));
        out.insert(format!("skip/{}", name), json!(mapping::skip_activity(item)));
        out.insert(format!("signed_cash/{}", name), json!(mapping::signed_cash(item)));
        out.insert(format!("option_symbol/{}", name), json!(mapping::option_symbol(item)));
        out.insert(format!("corp/{}", name), json!(mapping::is_corp_share_move(item)));
        out.insert(format!("code_change/{}", name), json!(mapping::is_code_change(item)));
    }

    // --- accounts-derived outputs -------------------------------------------
    out.insert("slim_accounts".into(), Value::Array(sync::slim_accounts(&accts)));

    let pools = mapping::fifo_pool_ids(Some(&accts_arr));
    let sorted: BTreeMap<String, String> = pools.into_iter().collect();
    out.insert("fifo_pool_ids".into(), serde_json::to_value(&sorted).unwrap());

    out.insert("nav_account_groups".into(), Value::Object(mapping::nav_account_groups(Some(&accts_arr))));
    out.insert("margin_account_ids".into(), json!(fetch::margin_account_ids(&accts)));

    let mut acc_ids: Vec<String> = accts.iter().filter_map(|a| a.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())).collect();
    acc_ids.push("unknown-acc-id".to_string());
    for id in &acc_ids {
        out.insert(format!("account_type/{}", id), json!(mapping::account_type(id, Some(&accts_arr))));
    }
    for acc in &accts {
        let id = acc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        out.insert(format!("margin_boost_target/{}", id), json!(sync::margin_boost_target(acc)));
    }

    // --- fetch_condition ------------------------------------------------------
    out.insert("fetch_condition/none".into(), fetch::activity_fetch_condition("acct-1", None, NOW_UNIX));
    out.insert("fetch_condition/date_only".into(), fetch::activity_fetch_condition("acct-1", Some("2024-03-05"), NOW_UNIX));
    out.insert(
        "fetch_condition/date_time".into(),
        fetch::activity_fetch_condition("acct-1", Some("2024-03-05T10:00:00.000Z"), NOW_UNIX),
    );
    out.insert("fetch_condition/blank".into(), fetch::activity_fetch_condition("acct-1", Some("  "), NOW_UNIX));

    // --- parse_margin ----------------------------------------------------------
    out.insert(
        "parse_margin/amount_string".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": "123.45", "currency": "USD"}
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/amount_number".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": 500.0, "currency": "CAD"}
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/amount_missing".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"currency": "CAD"}
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/amount_non_numeric".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": "not-a-number", "currency": "CAD"}
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/missing_currency".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": 10.0}
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/unavailable_with_reason".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricUnavailable",
            "reason": {"__typename": "UnavailableSecurities", "securities": [{"securityId": "s1"}, {"securityId": "s2"}, {"securityId": "s3"}]}
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/unavailable_without_reason".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricUnavailable"
        }}}}}}}))),
    );
    out.insert(
        "parse_margin/unavailable_without_anything".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {}}}}}}}))),
    );
    out.insert(
        "parse_margin/missing_marginv3".into(),
        json!(fetch::parse_margin(&json!({"account": {"financials": {"current": {"marginV3": null}}}}))),
    );

    // --- nav_points_from_payload ------------------------------------------------
    {
        let (pts, page) = fetch::nav_points_from_payload(&json!({"identity": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-01", "netLiquidationValue": {"amount": 10, "currency": "CAD"}, "netDeposits": {"amount": 1, "currency": "CAD"}}}],
            "pageInfo": {"hasNextPage": false}
        }}}}));
        out.insert("nav_points/identity".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = fetch::nav_points_from_payload(&json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-02", "netLiquidationValueV2": {"amount": "20", "currency": "USD"}, "netDepositsV2": {"amount": "4", "currency": "USD"}}}],
            "pageInfo": {"hasNextPage": true, "endCursor": "cur-1"}
        }}}}));
        out.insert("nav_points/v2_keys".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = fetch::nav_points_from_payload(&json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-03"}}],
            "pageInfo": {}
        }}}}));
        out.insert("nav_points/missing_equity".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = fetch::nav_points_from_payload(&json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"netLiquidationValue": {"amount": 30}}}],
            "pageInfo": {}
        }}}}));
        out.insert("nav_points/missing_date".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = fetch::nav_points_from_payload(&json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-04", "netLiquidationValue": {"amount": 40}}}],
            "pageInfo": {}
        }}}}));
        out.insert("nav_points/net_deposits_absent".into(), json!({"points": pts, "page": page}));
    }

    // --- merge_nav_points ---------------------------------------------------------
    out.insert(
        "merge_nav_points".into(),
        json!(fetch::merge_nav_points(&[
            vec![
                json!({"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1}),
                json!({"date": "2024-01-03", "equity": 7, "currency": "CAD"}),
            ],
            vec![
                json!({"date": "2024-01-01", "equity": 5, "currency": "USD", "netDeposits": 2}),
                json!({"date": "2024-01-02", "equity": 6, "currency": "CAD"}),
            ],
            vec![
                json!({"date": "2024-01-01", "equity": 1, "currency": "EUR"}),
                json!({"date": "2024-01-02", "equity": 2, "currency": "CAD", "netDeposits": 9}),
            ],
        ])),
    );

    // --- money_amount ---------------------------------------------------------------
    out.insert("money_amount/first_key".into(), json!(fetch::money_amount(&json!({"a": {"amount": "5.5", "currency": "USD"}, "b": {"amount": 1}}), &["a", "b"])));
    out.insert("money_amount/second_key".into(), json!(fetch::money_amount(&json!({"b": {"amount": 2, "currency": "CAD"}}), &["a", "b"])));
    out.insert("money_amount/no_currency".into(), json!(fetch::money_amount(&json!({"a": {"amount": 3}}), &["a", "b"])));
    out.insert("money_amount/non_numeric".into(), json!(fetch::money_amount(&json!({"a": {"amount": "abc"}, "b": {"amount": 4}}), &["a", "b"])));
    out.insert("money_amount/none".into(), json!(fetch::money_amount(&json!({}), &["a", "b"])));
    out.insert("money_amount/not_object".into(), json!(fetch::money_amount(&json!("nope"), &["a", "b"])));

    // --- security_record ---------------------------------------------------------------
    out.insert(
        "security_record/stock".into(),
        json!(fetch::security_record(&json!({"id": "sec-1", "currency": "CAD", "stock": {"symbol": "AAA", "name": "AAA Inc", "primaryExchange": "TSX", "primaryMic": "XTSE"}}), "fallback-1")),
    );
    out.insert(
        "security_record/option_with_underlying".into(),
        json!(fetch::security_record(&json!({"id": "sec-2", "currency": "USD", "optionDetails": {"underlyingSecurity": {"id": "under-1"}}}), "fallback-2")),
    );
    out.insert("security_record/empty_object".into(), json!(fetch::security_record(&json!({}), "fallback-3")));
    out.insert(
        "security_record/missing_id_uses_fallback".into(),
        json!(fetch::security_record(&json!({"currency": "CAD", "stock": {"symbol": "BBB"}}), "fallback-4")),
    );

    // --- token / session helpers ---------------------------------------------------------
    let now = 1_700_000_000.0_f64;
    out.insert("token/expires_at_number".into(), json!(sync::expires_at_unix(&json!({"expires_at": now + 60.0}))));
    out.insert("token/expires_at_numeric_string".into(), json!(sync::expires_at_unix(&json!({"expires_at": (now + 60.0).to_string()}))));
    out.insert("token/expires_at_iso_z".into(), json!(sync::expires_at_unix(&json!({"expires_at": "2024-01-02T03:04:05Z"}))));
    out.insert("token/expires_at_iso_offset".into(), json!(sync::expires_at_unix(&json!({"expires_at": "2024-01-02T03:04:05+05:00"}))));
    out.insert("token/expires_at_empty".into(), json!(sync::expires_at_unix(&json!({"expires_at": ""}))));
    out.insert("token/expires_at_absent".into(), json!(sync::expires_at_unix(&json!({}))));

    for (name, sess) in [
        ("number", json!({"expires_at": now + 60.0})),
        ("numeric_string", json!({"expires_at": (now + 60.0).to_string()})),
        ("iso_z", json!({"expires_at": "2023-11-14T22:13:20Z"})),
        ("iso_offset", json!({"expires_at": "2023-11-15T03:13:20+05:00"})),
        ("empty", json!({"expires_at": ""})),
        ("absent", json!({})),
    ] {
        out.insert(format!("token/seconds_until_refresh/{}", name), json!(sync::seconds_until_token_refresh(&sess, now)));
        out.insert(format!("token/refresh_needed/{}", name), json!(sync::token_refresh_needed(&sess, now)));
    }

    out.insert("session/expires_at_as_timestamp_iso".into(), json!(session::expires_at_as_timestamp(&json!({"expires_at": "2024-01-02T03:04:05Z"}), now)));
    out.insert("session/expires_at_as_timestamp_number".into(), json!(session::expires_at_as_timestamp(&json!({"expires_at": now + 100.0}), now)));
    out.insert("session/expires_at_as_timestamp_expires_in_number".into(), json!(session::expires_at_as_timestamp(&json!({"expires_in": 3600}), now)));
    out.insert("session/expires_at_as_timestamp_expires_in_string".into(), json!(session::expires_at_as_timestamp(&json!({"expires_in": "1800"}), now)));
    out.insert("session/expires_at_as_timestamp_expires_in_garbage".into(), json!(session::expires_at_as_timestamp(&json!({"expires_in": "not-a-number"}), now)));
    out.insert("session/expires_at_as_timestamp_nothing".into(), json!(session::expires_at_as_timestamp(&json!({}), now)));

    for key in ["identity_canonical_id", "identityCanonicalId", "canonical_id", "identity_id", "resource_owner_id", "sub"] {
        out.insert(format!("session/identity_from/{}", key), json!(session::identity_from(&json!({key: "id-value"}))));
    }
    out.insert("session/identity_from/none".into(), json!(session::identity_from(&json!({}))));

    out.insert("session/client_id_from_token_info/uid".into(), json!(session::client_id_from_token_info(&json!({"application_uid": "uid-1"}))));
    out.insert("session/client_id_from_token_info/nested".into(), json!(session::client_id_from_token_info(&json!({"application": {"uid": "uid-2"}}))));
    out.insert("session/client_id_from_token_info/none".into(), json!(session::client_id_from_token_info(&json!({}))));

    for (name, body) in [
        ("invalid_grant", json!({"error": "invalid_grant"})),
        ("long_hex", json!({"error": "0123456789abcdef0123456789abcdef"})),
        ("weird_chars", json!({"error": "bad!chars$$"})),
        ("too_long", json!({"error": "a".repeat(70)})),
        ("with_http_status", json!({"error": "invalid_client", "_http_status": 401})),
        ("none", json!({})),
    ] {
        out.insert(format!("session/oauth_error_code/{}", name), json!(session::oauth_error_code(&body)));
        out.insert(format!("session/refresh_failure_message/{}", name), json!(session::refresh_failure_message(&body)));
    }
    out.insert("session/refresh_failure_message/http_only".into(), json!(session::refresh_failure_message(&json!({"_http_status": 500}))));
    out.insert("session/refresh_failure_message/nothing".into(), json!(session::refresh_failure_message(&json!({}))));

    Value::Object(out)
}

#[test]
fn test_ws_pure_surface_is_pinned() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/mapping.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/mapping.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
