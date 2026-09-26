//! Golden: every answer Wealthsimple gives the order code, read as the order code
//! reads it -- an order's extended reading, the pending-order feed page by page, the
//! securities summary behind a ticket's quote, the listing search, and the replies to
//! a create, a cancel and a modify. Each case is a realistic answer or a malformed one
//! (numbers as text, text as numbers, nulls, missing keys, the wrong shape), so a
//! change to how any of them is read shows here as a diff. The order answers and the
//! feed are read strictly: what does not match is a failure, named.
//!
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-server
//! wire_golden`, then read the diff in `tests/golden/orders_wire.json`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::orders::{self as o, gate};

/// An answer read as the order code reads it, or the failure it reads as.
fn read<T: serde::de::DeserializeOwned>(data: &Value, then: impl FnOnce(T) -> Value) -> Value {
    match o::read_answer::<T>("golden", data.clone()) {
        Ok(d) => then(d),
        Err(e) => json!({"failed": e.to_string()}),
    }
}

fn dbg<T: std::fmt::Debug>(v: &T) -> Value {
    json!(format!("{v:?}"))
}

fn extended_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("filled", json!({"soOrdersExtendedOrder": {"status": "FILLED", "filledQuantity": 25, "averageFilledPrice": 165.38, "submittedAtUtc": "2026-09-10T13:30:00Z",
            "expiredAtUtc": null, "firstFilledAtUtc": "2026-09-10T13:31:00Z", "lastFilledAtUtc": "2026-09-10T13:32:00Z", "rejectionCause": null, "rejectionCode": null,
            "submittedQuantity": 25, "limitPrice": 165.4, "stopPrice": null, "timeInForce": "day", "securityCurrency": "usd", "canonicalAccountId": "acct-tfsa",
            "accountId": "cust-1", "securityId": "sec-s-us", "orderType": "buy_quantity"}})),
        ("lowercase status, numbers as text", json!({"soOrdersExtendedOrder": {"status": "partially_filled", "filledQuantity": "10", "averageFilledPrice": "1.7350",
            "submittedQuantity": " 40 ", "limitPrice": "", "stopPrice": "abc", "timeInForce": "UNTIL_CANCEL"}})),
        ("cancel pending", json!({"soOrdersExtendedOrder": {"status": "CANCEL_PENDING"}})),
        ("unknown status", json!({"soOrdersExtendedOrder": {"status": "SOMETHING_NEW"}})),
        ("status as a number", json!({"soOrdersExtendedOrder": {"status": 5}})),
        ("rejected with a code only", json!({"soOrdersExtendedOrder": {"status": "REJECTED", "rejectionCause": null, "rejectionCode": "INSUFFICIENT_FUNDS"}})),
        ("rejected with both", json!({"soOrdersExtendedOrder": {"status": "REJECTED", "rejectionCause": "Not enough cash", "rejectionCode": "INSUFFICIENT_FUNDS"}})),
        ("rejected with an empty cause", json!({"soOrdersExtendedOrder": {"status": "REJECTED", "rejectionCause": "", "rejectionCode": "X"}})),
        ("account id only", json!({"soOrdersExtendedOrder": {"status": "EXPIRED", "accountId": "cust-9", "canonicalAccountId": ""}})),
        ("booleans and numbers where text is", json!({"soOrdersExtendedOrder": {"status": "POSTED", "filledQuantity": true, "securityId": 1234, "timeInForce": false}})),
        ("status empty", json!({"soOrdersExtendedOrder": {"status": "", "filledQuantity": 3}})),
        ("status missing", json!({"soOrdersExtendedOrder": {"filledQuantity": 3}})),
        ("order null", json!({"soOrdersExtendedOrder": null})),
        ("order not an object", json!({"soOrdersExtendedOrder": ["FILLED"]})),
        ("no order key", json!({})),
        ("data null", Value::Null),
    ]
}

fn feed_node(id: Value, extra: Value) -> Value {
    let mut n = json!({"id": id, "orderId": "ws-9", "canonicalAccountId": "acct-tfsa", "createdAtUtc": "2026-09-10T01:00:00Z", "status": "SUBMITTED", "side": "BUY",
        "executionType": "LIMIT", "submittedQuantity": 3, "limitPrice": 1.76, "stopPrice": null, "averageFillPrice": null, "securityCurrency": "usd",
        "securityId": "sec-s-us", "symbol": "QNC", "security": {"id": "sec-s-us", "stock": {"symbol": "QNC", "name": "Quantum Emotion Corp"}}});
    for (k, v) in extra.as_object().unwrap() {
        n[k] = v.clone();
    }
    n
}

/// Feed pages keyed by the cursor asked for (`null` for the first).
fn feed_cases() -> Vec<(&'static str, Vec<Value>)> {
    let page = |edges: Vec<Value>, info: Value| json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": edges, "pageInfo": info}}});
    let edge = |node: Value| json!({"cursor": "c", "node": node});
    vec![
        ("one page", vec![page(vec![edge(feed_node(json!("o-1"), json!({})))], json!({"hasNextPage": false, "endCursor": "c1"}))]),
        ("two pages", vec![
            page(vec![edge(feed_node(json!("o-1"), json!({})))], json!({"hasNextPage": true, "endCursor": "c1"})),
            page(vec![edge(feed_node(json!("o-2"), json!({"side": "SELL_SHORT", "executionType": "stop_limit", "stopPrice": "1.5"})))], json!({"hasNextPage": false, "endCursor": "c2"})),
        ]),
        ("next page with no cursor", vec![page(vec![edge(feed_node(json!("o-1"), json!({})))], json!({"hasNextPage": true, "endCursor": null}))]),
        ("next page spelled as text", vec![
            page(vec![edge(feed_node(json!("o-1"), json!({})))], json!({"hasNextPage": "true", "endCursor": "c1"})),
            page(vec![], json!({"hasNextPage": false})),
        ]),
        ("rows that are not orders", vec![page(vec![
            edge(feed_node(json!(""), json!({}))),
            edge(feed_node(Value::Null, json!({}))),
            edge(json!("not a node")),
            json!({"cursor": "c"}),
            json!("not an edge"),
            edge(feed_node(json!(77), json!({}))),
        ], json!({"hasNextPage": false}))]),
        ("fallbacks", vec![page(vec![
            edge(feed_node(json!("o-3"), json!({"securityId": null, "symbol": null, "security": {"id": "sec-unknown", "stock": {"symbol": "ZZZ"}}}))),
            edge(feed_node(json!("o-4"), json!({"securityId": "sec-other", "symbol": "", "security": null, "executionType": null, "submittedQuantity": null, "side": null, "status": null}))),
            edge(feed_node(json!("o-5"), json!({"executionType": "market", "submittedQuantity": "12", "limitPrice": "", "averageFillPrice": "1.1", "canonicalAccountId": "acct-nope"}))),
        ], json!({"hasNextPage": false}))]),
        ("no feed", vec![json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": null}})]),
        ("no identity", vec![json!({"identity": null})]),
        ("edges not a list", vec![json!({"identity": {"orderServiceExtendedOrderFeed": {"edges": {"node": {}}, "pageInfo": {}}}})]),
    ]
}

fn summary_node(extra: Value, quote: Value) -> Value {
    let mut n = json!({"id": "sec-s-us", "buyable": true, "sellable": true, "wsTradeEligible": true, "securityType": "EQUITY", "currency": "USD", "status": "TRADING",
        "stock": {"name": "Quantum Emotion Corp", "symbol": "QNC", "primaryExchange": "NYSE"}, "quoteV2": quote});
    for (k, v) in extra.as_object().unwrap() {
        n[k] = v.clone();
    }
    n
}

fn quote_cases() -> Vec<(&'static str, Value)> {
    let q = json!({"__typename": "EquityQuote", "ask": 3.02, "bid": 3.0, "currency": "USD", "price": 3.01, "previousBaseline": 2.9, "marketStatus": "OPEN",
        "askSize": 5, "bidSize": 7, "quotedAsOf": "2026-09-10T15:00:00Z"});
    let with = |over: Value| {
        let mut x = q.clone();
        for (k, v) in over.as_object().unwrap() {
            x[k] = v.clone();
        }
        x
    };
    vec![
        ("equity", summary_node(json!({}), q.clone())),
        ("mid given", summary_node(json!({}), with(json!({"mid": 3.05})))),
        ("mid as text", summary_node(json!({}), with(json!({"mid": "3.04"})))),
        ("mid null", summary_node(json!({}), with(json!({"mid": null})))),
        ("mid empty text", summary_node(json!({}), with(json!({"mid": ""})))),
        ("mid unreadable", summary_node(json!({}), with(json!({"mid": "n/a"})))),
        ("last instead of price", summary_node(json!({}), with(json!({"price": null, "last": "3.2", "previousBaseline": null, "referenceClose": 3.0})))),
        ("zero baseline", summary_node(json!({}), with(json!({"previousBaseline": 0})))),
        ("no bid", summary_node(json!({}), with(json!({"bid": null})))),
        ("currency only on the node", summary_node(json!({"currency": "cad"}), with(json!({"currency": ""})))),
        ("no quote", summary_node(json!({}), Value::Null)),
        ("quote not an object", summary_node(json!({}), json!("3.01"))),
        ("option", summary_node(json!({"securityType": "OPTION", "optionDetails": {"multiplier": "100", "maturityDate": "2026-12-18"}}), q.clone())),
        ("option details empty", summary_node(json!({"optionDetails": {}}), q.clone())),
        ("option details without a multiplier", summary_node(json!({"optionDetails": {"maturityDate": "2026-12-18"}}), q.clone())),
        ("flags spelled oddly", summary_node(json!({"buyable": "yes", "sellable": 0, "wsTradeEligible": null, "status": 3}), q.clone())),
        ("no stock", summary_node(json!({"stock": null}), q.clone())),
        ("no id", summary_node(json!({"id": ""}), q.clone())),
        ("numeric id", summary_node(json!({"id": 42}), q.clone())),
        ("not an object", json!("sec-s-us")),
    ]
}

fn search_answer() -> Value {
    json!({"securitySearch": {"results": [
        "not a row",
        {"id": "", "securityType": "EQUITY", "currency": "USD", "stock": {"symbol": "BBAI", "primaryExchange": "NYSE"}},
        {"id": "sec-o-bbai", "securityType": "OPTION", "currency": "USD", "stock": {"symbol": "BBAI", "primaryExchange": "NYSE"}},
        {"id": "sec-s-bbai", "buyable": true, "status": "TRADING", "currency": "usd", "securityType": "equity", "stock": {"symbol": "bbai", "name": "BigBear.ai Holdings Inc", "primaryExchange": "nyse", "primaryMic": "XNYS"}},
        {"id": "sec-s-baig", "currency": "USD", "securityType": "EXCHANGE_TRADED_FUND", "stock": {"symbol": "BAIG", "name": "2X Long Bbai Daily ETF", "primaryExchange": "NASDAQ", "primaryMic": "XNAS"}},
        {"id": "sec-s-qnc-ca", "currency": "CAD", "securityType": "EQUITY", "stock": {"symbol": "QNC.TO", "name": "Quantum Emotion Corp", "primaryExchange": "TSX-V", "primaryMic": "XTSX"}},
        {"id": "sec-s-nostock", "currency": "CAD", "securityType": "EQUITY", "stock": null},
        {"id": 991, "currency": 5, "securityType": "EQUITY", "stock": {"symbol": "NUM", "name": 7, "primaryExchange": "TSX"}},
    ]}})
}

fn search_cases() -> Vec<(&'static str, Value, &'static str, &'static str)> {
    vec![
        ("the equity, not the option", search_answer(), "BBAI", "NYSE"),
        ("an ETF", search_answer(), "BAIG", "nasdaq"),
        ("a Canadian suffix on either side", search_answer(), "QNC.V", "TSX-V"),
        ("the wrong exchange", search_answer(), "BBAI", "NASDAQ"),
        ("a listing without a stock", search_answer(), "", ""),
        ("numbers where text is", search_answer(), "NUM", "TSX"),
        ("no results", json!({"securitySearch": {"results": []}}), "BBAI", "NYSE"),
        ("results not a list", json!({"securitySearch": {"results": {"id": "x"}}}), "BBAI", "NYSE"),
        ("no search", json!({"securitySearch": null}), "BBAI", "NYSE"),
        ("data null", Value::Null, "BBAI", "NYSE"),
    ]
}

fn create_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("accepted", json!({"soOrdersCreateOrder": {"errors": [], "order": {"orderId": "ws-123", "createdAt": "2026-09-10T15:31:00Z"}}})),
        ("accepted, errors null", json!({"soOrdersCreateOrder": {"errors": null, "order": {"orderId": "ws-124"}}})),
        ("accepted, numeric order id", json!({"soOrdersCreateOrder": {"order": {"orderId": 125}}})),
        ("accepted, no order", json!({"soOrdersCreateOrder": {"errors": [], "order": null}})),
        ("no result", json!({"soOrdersCreateOrder": null})),
        ("empty answer", json!({})),
        ("refused with a message", json!({"soOrdersCreateOrder": {"errors": [{"code": "ORDER.insufficient_funds", "message": "Insufficient funds"}], "order": null}})),
        ("refused with a code only", json!({"soOrdersCreateOrder": {"errors": [{"code": "ORDER.halted", "message": null}]}})),
        ("refused with an empty message", json!({"soOrdersCreateOrder": {"errors": [{"code": "ORDER.x", "message": ""}]}})),
        ("refused with neither", json!({"soOrdersCreateOrder": {"errors": [{}]}})),
        ("refused with text", json!({"soOrdersCreateOrder": {"errors": ["market closed"]}})),
        ("refused with null", json!({"soOrdersCreateOrder": {"errors": [null]}})),
        ("refused, errors an object", json!({"soOrdersCreateOrder": {"errors": {"message": "Account locked"}, "order": {"orderId": "ws-126"}}})),
        ("refused, errors text", json!({"soOrdersCreateOrder": {"errors": "Account locked", "order": {"orderId": "ws-127"}}})),
    ]
}

fn mutation_error_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("accepted", json!({"errors": []})),
        ("accepted, errors null", json!({"errors": null})),
        ("accepted, no result", Value::Null),
        ("refused with a message", json!({"errors": [{"code": "C", "message": "Order already filled"}]})),
        ("refused with a code only", json!({"errors": [{"code": "ORDER.not_cancellable"}]})),
        ("refused with text", json!({"errors": ["too late"]})),
        ("refused, errors an object", json!({"errors": {"message": "Account locked"}})),
        ("refused, errors text", json!({"errors": "Account locked"})),
    ]
}

#[test]
fn test_every_order_answer_wealthsimple_gives_is_read_as_the_golden_pins() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = crate::tests_execution::fresh();
    let mut got = Map::new();

    let mut ext = Map::new();
    for (name, data) in extended_cases() {
        ext.insert(name.into(), match gate::read_extended(&data) {
            Ok(f) => dbg(&f),
            Err(e) => json!({"failed": e}),
        });
    }
    got.insert("extendedOrder".into(), Value::Object(ext));

    let sess: bagholder_ws::session::Session = serde_json::from_value(json!({"access_token": "t", "identity_canonical_id": "ident-1"})).unwrap();
    let mut feed = Map::new();
    for (name, pages) in feed_cases() {
        let asked: Arc<Mutex<Vec<Value>>> = Arc::default();
        let a2 = asked.clone();
        *app.orders.seam.gql.lock().unwrap() = Some(Arc::new(move |op: &str, vars: &Value| {
            assert_eq!(op, "OrderServiceExtendedOrderFeed");
            let mut a = a2.lock().unwrap();
            let n = a.len();
            a.push(vars["cursor"].clone());
            Ok(pages.get(n).cloned().unwrap_or_else(|| panic!("page {n} asked for")))
        }));
        let rows = match o::read_feed(&app, &sess) {
            Ok(rows) => json!(rows.iter().map(dbg).collect::<Vec<_>>()),
            Err(e) => json!({"failed": e}),
        };
        feed.insert(name.into(), json!({"cursors": *asked.lock().unwrap(), "rows": rows}));
    }
    got.insert("feed".into(), Value::Object(feed));

    let mut quotes = Map::new();
    for (name, node) in quote_cases() {
        quotes.insert(name.into(), read(&json!({"securities": [node]}), |d: bagholder_ws::wire::SecuritiesSummary| serde_json::to_value(d.securities.first().and_then(o::parse_quote)).unwrap()));
    }
    let summary = json!({"securities": quote_cases().into_iter().map(|(_, n)| n).collect::<Vec<_>>()});
    *app.orders.seam.gql.lock().unwrap() = Some(Arc::new(move |_: &str, _: &Value| Ok(summary.clone())));
    let all = o::fetch_quotes(&app, &sess, &["sec-s-us".into()]).unwrap();
    *app.orders.seam.gql.lock().unwrap() = Some(Arc::new(|_: &str, _: &Value| Ok(json!({"securities": {"id": "sec-s-us"}}))));
    let none = o::fetch_quotes(&app, &sess, &["sec-s-us".into()]).map(|q| q.len()).map_err(|e| e.to_string());
    let mut keys: Vec<&String> = all.keys().collect();
    keys.sort();
    quotes.insert("summary: ids kept".into(), json!(keys));
    quotes.insert("summary: securities not a list".into(), dbg(&none));
    got.insert("quote".into(), Value::Object(quotes));

    let mut search = Map::new();
    for (name, data, sym, ex) in search_cases() {
        search.insert(name.into(), read(&data, |d: bagholder_ws::wire::SecuritySearchAnswer| serde_json::to_value(o::parse_listing_search(&d, sym, ex)).unwrap()));
    }
    got.insert("listingSearch".into(), Value::Object(search));

    use bagholder_ws::session::Mutation;
    let answer = |data: Value| -> Result<Value, String> { if data.is_null() { Err("no answer".into()) } else { Ok(data) } };
    let mut create = Map::new();
    for (name, reply) in create_cases() {
        let m = match answer(reply).and_then(|d| o::read_answer::<bagholder_ws::wire::CreateOrderAnswer>("golden", d).map_err(|e| e.to_string())) {
            Ok(a) => Mutation::Answer(a),
            Err(e) => Mutation::Unclear(e),
        };
        create.insert(name.into(), dbg(&gate::created(m)));
    }
    got.insert("create".into(), Value::Object(create));

    let mut cancel = Map::new();
    let mut modify = Map::new();
    for (name, result) in mutation_error_cases() {
        let c = if result.is_null() { json!({}) } else { json!({"orderServiceCancelOrder": result.clone()}) };
        let m = match o::read_answer::<bagholder_ws::wire::CancelOrderAnswer>("golden", c) {
            Ok(a) => Mutation::Answer(a),
            Err(e) => Mutation::Unclear(e.to_string()),
        };
        cancel.insert(name.into(), dbg(&gate::cancelled(m)));
        let md = if result.is_null() { json!({}) } else { json!({"soOrdersModifyOrder": result.clone()}) };
        let m = match o::read_answer::<bagholder_ws::wire::ModifyOrderAnswer>("golden", md) {
            Ok(a) => Mutation::Answer(a),
            Err(e) => Mutation::Unclear(e.to_string()),
        };
        modify.insert(name.into(), dbg(&gate::modified(m)));
    }
    got.insert("cancel".into(), Value::Object(cancel));
    got.insert("modify".into(), Value::Object(modify));
    // the transport's own outcomes
    got.insert("transport".into(), json!({
        "refused": dbg(&gate::created(Mutation::Refused("Bad request".into()))),
        "session lapsed": dbg(&gate::created(Mutation::NotAuthorized)),
        "no answer": dbg(&gate::created(Mutation::Unclear("the connection dropped".into()))),
    }));

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/orders_wire.json");
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(got)).unwrap() + "\n").unwrap();
        return;
    }
    let want: Map<String, Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or_default();
    for (group, cases) in &got {
        let empty = Map::new();
        let w = want.get(group).and_then(|v| v.as_object()).unwrap_or(&empty);
        let g = cases.as_object().unwrap();
        let mut names: Vec<&String> = g.keys().chain(w.keys()).collect();
        names.sort();
        names.dedup();
        for name in names {
            assert_eq!(g.get(name), w.get(name), "{group} / {name} reads differently than the golden pins");
        }
    }
    assert_eq!(got.len(), want.len(), "the golden's groups");
}
