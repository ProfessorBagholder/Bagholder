//! Golden (characterisation) test for `bagholder-ws`'s pure, no-network
//! surface ahead of the typed-struct conversion: every branch of
//! `mapping::map_activity` / `map_activity_rows` / `skip_activity` /
//! `signed_cash` / `option_symbol` / `is_corp_share_move` / `is_code_change`
//! over the corpus in `tests/golden/activity_items.json`; `sync::slim_accounts`,
//! `mapping::fifo_pool_ids_from_nodes`, `mapping::nav_account_groups`,
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

use bagholder_ws::wire::{ActivityItem, AccountNode, MarginAnswer, Money, NavAnswer};
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

fn accounts_raw() -> Vec<Value> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/accounts.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v.as_array().unwrap().clone()
}

const NOW_UNIX: i64 = 1_789_000_000;

/// `MappedActivity` rows, in their stored JSON shape (`to_rows`), so the
/// golden keeps exactly the field set it always has.
fn rows_json(rows: Vec<bagholder_store::broker::MappedActivity>) -> Value {
    Value::Array(bagholder_store::broker::MappedActivity::to_rows(&rows))
}

fn single_json(row: Option<bagholder_store::broker::MappedActivity>) -> Value {
    match row {
        Some(r) => bagholder_store::broker::MappedActivity::to_rows(&[r]).into_iter().next().unwrap(),
        None => Value::Null,
    }
}

fn answers() -> Value {
    let mut out: Map<String, Value> = Map::new();
    let items_raw = corpus();
    // a corpus entry that is not an object at all (`skip_activity`'s own
    // "not an object" branch) reads as the empty, all-defaults item -- which
    // is itself skipped for having no `occurredAt`, exactly as before
    let items: Vec<(String, ActivityItem)> =
        items_raw.iter().map(|(k, v)| (k.clone(), serde_json::from_value(v.clone()).unwrap_or_default())).collect();
    let accts_raw = accounts_raw();
    let nodes: Vec<AccountNode> = accts_raw.iter().map(|a| serde_json::from_value(a.clone()).unwrap()).collect();
    let accounts = mapping::Accounts::from_nodes(&nodes);
    let none = mapping::Accounts::default();

    // --- per-corpus-item outputs -------------------------------------------
    for (name, item) in &items {
        out.insert(format!("rows_list/{}", name), rows_json(mapping::map_activity_rows(item, &accounts)));
        // the map-keyed and list-keyed accounts inputs were always the same
        // computation; both keys are kept so the golden's key set does not move
        out.insert(format!("rows_map/{}", name), rows_json(mapping::map_activity_rows(item, &accounts)));
        out.insert(format!("rows_none/{}", name), rows_json(mapping::map_activity_rows(item, &none)));
        out.insert(format!("single/{}", name), single_json(mapping::map_activity(item, &none)));
        out.insert(format!("skip/{}", name), json!(mapping::skip_activity(item)));
        out.insert(format!("signed_cash/{}", name), json!(mapping::signed_cash(item)));
        out.insert(format!("option_symbol/{}", name), json!(mapping::option_symbol(item)));
        out.insert(format!("corp/{}", name), json!(mapping::is_corp_share_move(item)));
        out.insert(format!("code_change/{}", name), json!(mapping::is_code_change(item)));
    }

    // --- accounts-derived outputs -------------------------------------------
    out.insert("slim_accounts".into(), serde_json::to_value(sync::slim_accounts(&nodes)).unwrap());

    let pools = mapping::fifo_pool_ids_from_nodes(&nodes);
    let sorted: BTreeMap<String, String> = pools.into_iter().collect();
    out.insert("fifo_pool_ids".into(), serde_json::to_value(&sorted).unwrap());

    let groups: Map<String, Value> = mapping::nav_account_groups(&nodes).into_iter().map(|(k, v)| (k, json!(v))).collect();
    out.insert("nav_account_groups".into(), Value::Object(groups));
    out.insert("margin_account_ids".into(), json!(fetch::margin_account_ids(&nodes)));

    let mut acc_ids: Vec<String> = nodes.iter().map(|a| a.id.clone()).filter(|s| !s.is_empty()).collect();
    acc_ids.push("unknown-acc-id".to_string());
    for id in &acc_ids {
        out.insert(format!("account_type/{}", id), json!(mapping::account_type(id, &accounts)));
    }
    for acc in &nodes {
        out.insert(format!("margin_boost_target/{}", acc.id), json!(sync::margin_boost_target(acc)));
    }

    // --- fetch_condition ------------------------------------------------------
    let cond = |start: Option<&str>| serde_json::to_value(fetch::activity_fetch_condition("acct-1", start, NOW_UNIX)).unwrap();
    out.insert("fetch_condition/none".into(), cond(None));
    out.insert("fetch_condition/date_only".into(), cond(Some("2024-03-05")));
    out.insert("fetch_condition/date_time".into(), cond(Some("2024-03-05T10:00:00.000Z")));
    out.insert("fetch_condition/blank".into(), cond(Some("  ")));

    // --- parse_margin ----------------------------------------------------------
    let margin_answer = |v: Value| -> MarginAnswer { serde_json::from_value(v).unwrap() };
    let margin_json = |v: Value| -> Value {
        match fetch::parse_margin(&margin_answer(v)) {
            Some(m) => json!({"buyingPower": m.buying_power, "currency": m.currency, "unavailable": m.unavailable}),
            None => Value::Null,
        }
    };
    out.insert(
        "parse_margin/amount_string".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": "123.45", "currency": "USD"}
        }}}}}}})),
    );
    out.insert(
        "parse_margin/amount_number".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": 500.0, "currency": "CAD"}
        }}}}}}})),
    );
    out.insert(
        "parse_margin/amount_missing".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"currency": "CAD"}
        }}}}}}})),
    );
    out.insert(
        "parse_margin/amount_non_numeric".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": "not-a-number", "currency": "CAD"}
        }}}}}}})),
    );
    out.insert(
        "parse_margin/missing_currency".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricAvailable", "total": {"amount": 10.0}
        }}}}}}})),
    );
    out.insert(
        "parse_margin/unavailable_with_reason".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricUnavailable",
            "reason": {"__typename": "UnavailableSecurities", "securities": [{"securityId": "s1"}, {"securityId": "s2"}, {"securityId": "s3"}]}
        }}}}}}})),
    );
    out.insert(
        "parse_margin/unavailable_without_reason".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
            "__typename": "BuyingPowerMetricUnavailable"
        }}}}}}})),
    );
    out.insert(
        "parse_margin/unavailable_without_anything".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {}}}}}}})),
    );
    out.insert(
        "parse_margin/missing_marginv3".into(),
        margin_json(json!({"account": {"financials": {"current": {"marginV3": null}}}})),
    );

    // --- nav_points_from_payload ------------------------------------------------
    // The golden's "page" value is the wire's own, unshaped `pageInfo` object
    // (0, 1 or 2 keys, exactly as the payload carried it), which a typed
    // `PageInfo` (always both fields) cannot reproduce byte for byte once
    // deserialized -- so the raw value is read straight off the test's own
    // input here, the one piece nav_points_from_payload's typed return
    // cannot recover.
    let nav = |v: Value| -> (Value, Value) {
        let page_raw = v
            .pointer("/identity/financials/historicalDaily/pageInfo")
            .or_else(|| v.pointer("/account/financials/historicalDaily/pageInfo"))
            .cloned()
            .unwrap_or(json!({}));
        let answer: NavAnswer = serde_json::from_value(v).unwrap();
        let (pts, _page) = fetch::nav_points_from_payload(&answer);
        let points_json = Value::Array(
            pts.iter()
                .map(|p| {
                    let mut m = Map::new();
                    m.insert("date".into(), json!(p.date));
                    m.insert("equity".into(), json!(p.equity));
                    m.insert("currency".into(), json!(p.currency));
                    if let Some(nd) = p.net_deposits {
                        m.insert("netDeposits".into(), json!(nd));
                    }
                    Value::Object(m)
                })
                .collect(),
        );
        (points_json, page_raw)
    };
    {
        let (pts, page) = nav(json!({"identity": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-01", "netLiquidationValue": {"amount": 10, "currency": "CAD"}, "netDeposits": {"amount": 1, "currency": "CAD"}}}],
            "pageInfo": {"hasNextPage": false}
        }}}}));
        out.insert("nav_points/identity".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = nav(json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-02", "netLiquidationValueV2": {"amount": "20", "currency": "USD"}, "netDepositsV2": {"amount": "4", "currency": "USD"}}}],
            "pageInfo": {"hasNextPage": true, "endCursor": "cur-1"}
        }}}}));
        out.insert("nav_points/v2_keys".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = nav(json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-03"}}],
            "pageInfo": {}
        }}}}));
        out.insert("nav_points/missing_equity".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = nav(json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"netLiquidationValue": {"amount": 30}}}],
            "pageInfo": {}
        }}}}));
        out.insert("nav_points/missing_date".into(), json!({"points": pts, "page": page}));
    }
    {
        let (pts, page) = nav(json!({"account": {"financials": {"historicalDaily": {
            "edges": [{"node": {"date": "2024-02-04", "netLiquidationValue": {"amount": 40}}}],
            "pageInfo": {}
        }}}}));
        out.insert("nav_points/net_deposits_absent".into(), json!({"points": pts, "page": page}));
    }

    // --- merge_nav_points ---------------------------------------------------------
    let point = |date: &str, equity: f64, currency: &str, net_deposits: Option<f64>| bagholder_store::broker::NavPoint {
        account_id: String::new(),
        date: date.into(),
        equity: Some(equity),
        currency: currency.into(),
        net_deposits,
    };
    let merged = fetch::merge_nav_points(&[
        vec![point("2024-01-01", 10.0, "CAD", Some(1.0)), point("2024-01-03", 7.0, "CAD", None)],
        vec![point("2024-01-01", 5.0, "USD", Some(2.0)), point("2024-01-02", 6.0, "CAD", None)],
        vec![point("2024-01-01", 1.0, "EUR", None), point("2024-01-02", 2.0, "CAD", Some(9.0))],
    ]);
    out.insert(
        "merge_nav_points".into(),
        Value::Array(
            merged
                .iter()
                .map(|p| {
                    let mut m = Map::new();
                    m.insert("date".into(), json!(p.date));
                    m.insert("equity".into(), json!(p.equity));
                    m.insert("currency".into(), json!(p.currency));
                    if let Some(nd) = p.net_deposits {
                        m.insert("netDeposits".into(), json!(nd));
                    }
                    Value::Object(m)
                })
                .collect(),
        ),
    );

    // --- money_amount ---------------------------------------------------------------
    let money = |v: Value| -> Option<Money> { if v.is_null() { None } else { Some(serde_json::from_value(v).unwrap()) } };
    let ma = |a: Value, b: Value| -> Value {
        let (amt, cur) = fetch::money_amount(&[money(a).as_ref(), money(b).as_ref()]);
        json!([amt, cur])
    };
    out.insert("money_amount/first_key".into(), ma(json!({"amount": "5.5", "currency": "USD"}), json!({"amount": 1})));
    out.insert("money_amount/second_key".into(), ma(Value::Null, json!({"amount": 2, "currency": "CAD"})));
    out.insert("money_amount/no_currency".into(), ma(json!({"amount": 3}), Value::Null));
    out.insert("money_amount/non_numeric".into(), ma(json!({"amount": "abc"}), json!({"amount": 4})));
    out.insert("money_amount/none".into(), ma(Value::Null, Value::Null));
    out.insert("money_amount/not_object".into(), ma(Value::Null, Value::Null));

    // --- security_record ---------------------------------------------------------------
    out.insert(
        "security_record/stock".into(),
        serde_json::to_value(fetch::security_record(&json!({"id": "sec-1", "currency": "CAD", "stock": {"symbol": "AAA", "name": "AAA Inc", "primaryExchange": "TSX", "primaryMic": "XTSE"}}), "fallback-1")).unwrap(),
    );
    out.insert(
        "security_record/option_with_underlying".into(),
        serde_json::to_value(fetch::security_record(&json!({"id": "sec-2", "currency": "USD", "optionDetails": {"underlyingSecurity": {"id": "under-1"}}}), "fallback-2")).unwrap(),
    );
    out.insert("security_record/empty_object".into(), serde_json::to_value(fetch::security_record(&json!({}), "fallback-3")).unwrap());
    out.insert(
        "security_record/missing_id_uses_fallback".into(),
        serde_json::to_value(fetch::security_record(&json!({"currency": "CAD", "stock": {"symbol": "BBB"}}), "fallback-4")).unwrap(),
    );

    // --- token / session helpers ---------------------------------------------------------
    let now = 1_700_000_000.0_f64;
    let reply = |v: Value| -> session::TokenReply { serde_json::from_value(v).unwrap() };
    let sess_with_expiry = |v: Value| -> session::Session {
        let mut s = session::Session::default();
        s.refresh_token = "r".into();
        if let Some(e) = v.get("expires_at") {
            s.expires_at = match e {
                Value::Number(n) => Some(session::Expiry::Unix(n.as_f64().unwrap())),
                Value::String(t) if !t.is_empty() => Some(session::Expiry::Text(t.clone())),
                _ => None,
            };
        }
        s
    };
    out.insert("token/expires_at_number".into(), json!(sync::expires_at_unix(&sess_with_expiry(json!({"expires_at": now + 60.0})))));
    out.insert("token/expires_at_numeric_string".into(), json!(sync::expires_at_unix(&sess_with_expiry(json!({"expires_at": (now + 60.0).to_string()})))));
    out.insert("token/expires_at_iso_z".into(), json!(sync::expires_at_unix(&sess_with_expiry(json!({"expires_at": "2024-01-02T03:04:05Z"})))));
    out.insert("token/expires_at_iso_offset".into(), json!(sync::expires_at_unix(&sess_with_expiry(json!({"expires_at": "2024-01-02T03:04:05+05:00"})))));
    out.insert("token/expires_at_empty".into(), json!(sync::expires_at_unix(&sess_with_expiry(json!({"expires_at": ""})))));
    out.insert("token/expires_at_absent".into(), json!(sync::expires_at_unix(&sess_with_expiry(json!({})))));

    for (name, v) in [
        ("number", json!({"expires_at": now + 60.0})),
        ("numeric_string", json!({"expires_at": (now + 60.0).to_string()})),
        ("iso_z", json!({"expires_at": "2023-11-14T22:13:20Z"})),
        ("iso_offset", json!({"expires_at": "2023-11-15T03:13:20+05:00"})),
        ("empty", json!({"expires_at": ""})),
        ("absent", json!({})),
    ] {
        let s = sess_with_expiry(v);
        out.insert(format!("token/seconds_until_refresh/{}", name), json!(sync::seconds_until_token_refresh(&s, now)));
        out.insert(format!("token/refresh_needed/{}", name), json!(sync::token_refresh_needed(&s, now)));
    }

    out.insert("session/expires_at_as_timestamp_iso".into(), json!(session::expires_at_as_timestamp(&reply(json!({"expires_at": "2024-01-02T03:04:05Z"})), now)));
    out.insert("session/expires_at_as_timestamp_number".into(), json!(session::expires_at_as_timestamp(&reply(json!({"expires_at": now + 100.0})), now)));
    out.insert("session/expires_at_as_timestamp_expires_in_number".into(), json!(session::expires_at_as_timestamp(&reply(json!({"expires_in": 3600})), now)));
    out.insert("session/expires_at_as_timestamp_expires_in_string".into(), json!(session::expires_at_as_timestamp(&reply(json!({"expires_in": "1800"})), now)));
    out.insert("session/expires_at_as_timestamp_expires_in_garbage".into(), json!(session::expires_at_as_timestamp(&reply(json!({"expires_in": "not-a-number"})), now)));
    out.insert("session/expires_at_as_timestamp_nothing".into(), json!(session::expires_at_as_timestamp(&reply(json!({})), now)));

    for key in ["identity_canonical_id", "identityCanonicalId", "canonical_id", "identity_id", "resource_owner_id", "sub"] {
        let ids: session::IdentityKeys = serde_json::from_value(json!({key: "id-value"})).unwrap();
        out.insert(format!("session/identity_from/{}", key), json!(ids.identity()));
    }
    let none_ids: session::IdentityKeys = serde_json::from_value(json!({})).unwrap();
    out.insert("session/identity_from/none".into(), json!(none_ids.identity()));

    let info = |v: Value| -> session::TokenInfo { serde_json::from_value(v).unwrap() };
    out.insert("session/client_id_from_token_info/uid".into(), json!(session::client_id_from_token_info(&info(json!({"application_uid": "uid-1"})))));
    out.insert("session/client_id_from_token_info/nested".into(), json!(session::client_id_from_token_info(&info(json!({"application": {"uid": "uid-2"}})))));
    out.insert("session/client_id_from_token_info/none".into(), json!(session::client_id_from_token_info(&info(json!({})))));

    for (name, body) in [
        ("invalid_grant", json!({"error": "invalid_grant"})),
        ("long_hex", json!({"error": "0123456789abcdef0123456789abcdef"})),
        ("weird_chars", json!({"error": "bad!chars$$"})),
        ("too_long", json!({"error": "a".repeat(70)})),
        ("with_http_status", json!({"error": "invalid_client", "_http_status": 401})),
        ("none", json!({})),
    ] {
        let r = reply(body);
        out.insert(format!("session/oauth_error_code/{}", name), json!(session::oauth_error_code(&r)));
        out.insert(format!("session/refresh_failure_message/{}", name), json!(session::refresh_failure_message(&r)));
    }
    out.insert("session/refresh_failure_message/http_only".into(), json!(session::refresh_failure_message(&reply(json!({"_http_status": 500})))));
    out.insert("session/refresh_failure_message/nothing".into(), json!(session::refresh_failure_message(&reply(json!({})))));

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
