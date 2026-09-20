//! The order ticket, reading orders back, stop fills booked locally, the
//! Orders panel, feed matching, the order tick, the ninety-day roll and stop
//! expiry. Wealthsimple is always a fake here
//! (`orders::seam`); nothing reaches the network and nothing is placed.
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

use bagholder_store::orders as so;
use bagholder_ws::session::CallError;
use serde_json::{json, Value};

use crate::app::{app, now_iso, now_unix, stamp_of, uuid4};
use crate::orders::{self as o, seam};

type Sent = Arc<Mutex<Vec<(String, Value)>>>;

fn conn() -> bagholder_store::pool::Pooled<'static> {
    app().open().unwrap()
}

fn n(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or_else(|| panic!("{} not a number in {}", k, v))
}

fn st(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(x) => x.to_string(),
    }
}

fn get_order(id: &str) -> Value {
    so::get_order(&conn(), id).unwrap().unwrap_or(Value::Null)
}
fn update_order(id: &str, patch: Value) {
    so::update_order(&conn(), id, &patch, &now_iso()).unwrap()
}
fn get_bracket(id: &str) -> Value {
    so::get_bracket(&conn(), id).unwrap().unwrap_or(Value::Null)
}
fn update_bracket(id: &str, patch: Value) {
    so::update_bracket(&conn(), id, &patch, &now_iso()).unwrap()
}
fn list_orders() -> Vec<Value> {
    so::list_orders(&conn(), 200).unwrap()
}
fn activities() -> Vec<Value> {
    let s = bagholder_store::snapshot::snapshot(&conn(), true).unwrap();
    s["activities"].as_array().cloned().unwrap_or_default()
}
fn apply_ws(rows: &[Value]) -> bagholder_store::merge::Applied {
    bagholder_store::merge::apply_wealthsimple_mapped(&conn(), rows, &uuid4).unwrap()
}

fn set_gql<F: Fn(&str, &Value) -> Result<Value, CallError> + Send + Sync + 'static>(f: F) {
    *seam::GQL.lock().unwrap() = Some(Arc::new(f));
}
fn set_live(v: Option<bool>) {
    *seam::LIVE.lock().unwrap() = v;
}
fn set_session(v: Option<Value>) {
    *seam::SESSION.lock().unwrap() = Some(v);
}
fn unpatch() {
    seam::reset();
}

/// A wiped store, set up for an order test.
fn setup() -> MutexGuard<'static, ()> {
    let g = crate::tests_common::guard();
    seam::reset();
    let c = conn();
    bagholder_store::relabel::ensure(&c).unwrap();
    let names: Vec<String> = {
        let mut q = c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").unwrap();
        let r = q.query_map([], |r| r.get::<_, String>(0)).unwrap().filter_map(|x| x.ok()).collect();
        r
    };
    for t in names {
        if t == "meta" {
            let _ = c.execute("DELETE FROM meta WHERE key != 'schema_version'", []);
        } else {
            let _ = c.execute(&format!("DELETE FROM \"{}\"", t), []);
        }
    }
    app().invalidate();
    *o::REFRESHED_AT.lock().unwrap() = String::new();
    {
        let mut s = app().state.lock().unwrap();
        s.connected = false;
        s.syncing = false;
    }
    bagholder_store::tables::replace_accounts(&c, &[
        json!({"id": "acct-margin", "nickname": "Trading", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "currency": "CAD", "status": "open", "type": "non_registered"}),
        json!({"id": "acct-tfsa", "nickname": "TFSA", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa", "marginAccountId": "acct-margin"}),
        json!({"id": "acct-crypto", "nickname": "Crypto", "unifiedAccountType": "SELF_DIRECTED_CRYPTO", "currency": "CAD", "status": "open", "type": "crypto"}),
        json!({"id": "acct-old", "nickname": "Old", "unifiedAccountType": "SELF_DIRECTED_RRSP", "currency": "CAD", "status": "closed", "type": "rrsp"}),
        json!({"id": "acct-managed", "nickname": "Managed", "unifiedAccountType": "MANAGED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa"}),
    ]).unwrap();
    bagholder_store::admin::upsert_securities(&c, &[
        json!({"id": "sec-o-1", "symbol": "QNC", "name": "", "primaryExchange": "", "primaryMic": "", "currency": "USD", "underlyingId": "sec-s-us"}),
        json!({"id": "sec-s-us", "symbol": "QNC", "name": "Quantum Emotion Corp", "primaryExchange": "NYSE", "primaryMic": "XNYS", "currency": "USD", "underlyingId": null}),
        json!({"id": "sec-s-ca", "symbol": "QNC.TO", "name": "Quantum Emotion Corp", "primaryExchange": "TSX-V", "primaryMic": "XTSX", "currency": "CAD", "underlyingId": null}),
    ], &now_iso()).unwrap();
    bagholder_store::tables::replace_margin(&c, &[json!({"accountId": "acct-margin", "buyingPower": 12680.45, "currency": "CAD"})], &now_iso()).unwrap();
    g
}

fn ticket(over: Value) -> Value {
    let mut body = json!({"symbol": "QNC", "securityId": "sec-s-us", "accountId": "acct-margin", "side": "BUY", "type": "LIMIT", "tif": "DAY",
        "quantity": 25, "limitPrice": 165.4, "stopPrice": null, "currency": "USD",
        "stopLoss": {"kind": "stop", "price": 157.13}, "takeProfit": {"price": 181.94}});
    for (k, v) in over.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

fn tok() -> Value {
    json!({"access_token": "t"})
}

/// A ticket sent live against a fake that answers ws-1.
fn sent_order() -> String {
    set_gql(|_, _| Ok(json!({"soOrdersCreateOrder": {"errors": [], "order": {"orderId": "ws-1"}}})));
    set_live(Some(true));
    set_session(Some(tok()));
    let r = o::place_order(&ticket(json!({})));
    unpatch();
    st(&r, "id")
}

// ---------------------------------------------------------------------------
// OrderTicketTest
// ---------------------------------------------------------------------------

#[test]
fn test_tradable_accounts_are_open_self_directed_securities_accounts() {
    let _g = setup();
    let accts = o::order_accounts(None);
    let ids: Vec<String> = accts.iter().map(|a| st(a, "id")).collect();
    assert_eq!(ids, ["acct-margin", "acct-tfsa"], "crypto, managed and closed accounts are not offered");
    let m: HashMap<String, Value> = accts.iter().map(|a| (st(a, "id"), a["margin"].clone())).collect();
    assert!(crate::app::truthy(m.get("acct-margin")));
    assert!(!crate::app::truthy(m.get("acct-tfsa")));
}

#[test]
fn test_a_symbol_resolves_to_its_share_listing_before_an_option_contract() {
    let _g = setup();
    assert_eq!(st(&o::resolve_security("QNC", "").unwrap(), "id"), "sec-s-us");
    assert_eq!(st(&o::resolve_security("qnc.to", "").unwrap(), "id"), "sec-s-ca");
    assert_eq!(st(&o::resolve_security("", "sec-s-ca").unwrap(), "id"), "sec-s-ca");
    assert!(o::resolve_security("NOPE", "").is_none());
    assert_eq!(st(&o::resolve_security("NVDA", "sec-nvda").unwrap(), "id"), "sec-nvda");
}

#[test]
fn test_the_quote_card_is_read_from_wealthsimples_summary() {
    let node = json!({"id": "sec-s-us", "buyable": true, "sellable": true, "wsTradeEligible": true, "securityType": "EQUITY", "currency": "USD", "status": "ACTIVE",
        "stock": {"name": "NVIDIA Corp", "symbol": "NVDA", "primaryExchange": "NASDAQ", "primaryMic": "XNAS"},
        "quoteV2": {"__typename": "EquityQuote", "ask": 165.42, "bid": 165.38, "currency": "USD", "price": 165.40, "previousBaseline": 163.42,
                    "marketStatus": "OPEN", "askSize": 300, "bidSize": 100, "mid": 165.40, "quotedAsOf": "2026-09-10T15:30:00Z"}});
    let q = o::parse_quote(&node).unwrap();
    assert_eq!((st(&q, "symbol"), st(&q, "exchange"), st(&q, "currency")), ("NVDA".into(), "NASDAQ".into(), "USD".into()));
    assert_eq!((n(&q, "last"), n(&q, "bid"), n(&q, "ask"), n(&q, "bidSize"), n(&q, "askSize"), n(&q, "mid")), (165.40, 165.38, 165.42, 100.0, 300.0, 165.40));
    assert!((n(&q, "change") - 1.98).abs() < 1e-6);
    assert!((n(&q, "changePct") - 1.98 / 163.42).abs() < 1e-9);
    assert_eq!(st(&q, "marketStatus"), "OPEN");
    assert!(o::parse_quote(&json!({"stock": {}})).is_none(), "no id, no quote");
    let md = o::parse_market_data(&json!({"security": {"allowedOrderSubtypes": ["LIMIT", "FRACTIONAL", "MARKET"], "marginRates": {"clientMarginRate": 30}}}));
    assert_eq!(md["orderTypes"], json!(["MARKET", "LIMIT"]), "only the ticket's types, in the ticket's order");
    assert!((n(&md, "marginRate") - 0.30).abs() < 1e-7, "a percentage becomes a fraction");
    let bp = o::parse_buying_power(&json!({"account": {"financials": {"current": {"tradingBalanceViewV2": {"buyingPower": {"quantity": 12680.45, "currency": "USD"}, "cash": {"quantity": 3420.18, "currency": "USD"}}}}}}));
    assert_eq!((n(&bp, "buyingPower"), n(&bp, "cash"), st(&bp, "currency")), (12680.45, 3420.18, "USD".into()));
}

fn search_answer() -> Value {
    json!({"securitySearch": {"results": [
        {"id": "sec-s-bbai", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "EQUITY", "wsTradeEligible": true, "stock": {"symbol": "BBAI", "name": "BigBear.ai Holdings Inc", "primaryExchange": "NYSE", "primaryMic": "XNYS"}},
        {"id": "sec-s-baig", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "EXCHANGE_TRADED_FUND", "wsTradeEligible": true, "stock": {"symbol": "BAIG", "name": "2X Long Bbai Daily ETF", "primaryExchange": "NASDAQ", "primaryMic": "XNAS"}},
        {"id": "sec-s-qnc-ca", "buyable": true, "status": "TRADING", "currency": "CAD", "securityType": "EQUITY", "wsTradeEligible": true, "stock": {"symbol": "QNC.TO", "name": "Quantum Emotion Corp", "primaryExchange": "TSX-V", "primaryMic": "XTSX"}},
        {"id": "sec-o-qnc", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "OPTION", "stock": {"symbol": "QNC", "name": "", "primaryExchange": "NYSE"}},
    ]}})
}

#[test]
fn test_listing_search_picks_the_symbol_on_its_exchange() {
    let a = search_answer();
    let pick = |sym: &str, ex: &str| o::parse_listing_search(&a, sym, ex);
    assert_eq!(st(&pick("BBAI", "NYSE").unwrap(), "id"), "sec-s-bbai");
    assert_eq!(st(&pick("bbai", "nyse").unwrap(), "currency"), "USD");
    assert_eq!(st(&pick("QNC", "TSX-V").unwrap(), "id"), "sec-s-qnc-ca");
    assert_eq!(st(&pick("QNC", "TSX-V").unwrap(), "symbol"), "QNC.TO");
    assert!(pick("QNC", "NYSE").is_none(), "the NYSE result is an option, not the share");
    assert!(pick("BBAI", "NASDAQ").is_none());
}

#[test]
fn test_ticket_on_a_never_held_symbol_asks_wealthsimple_once_and_keeps_the_listing() {
    let _g = setup();
    let searches: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = searches.clone();
    set_gql(move |op, vars| match op {
        "FetchSecuritySearchResult" => {
            s2.lock().unwrap().push(st(vars, "query"));
            Ok(search_answer())
        }
        "FetchSecuritiesSummary" => {
            assert_eq!(vars["ids"], json!(["sec-s-bbai"]), "the quote is asked for the id the search gave");
            Ok(json!({"securities": [{"id": "sec-s-bbai", "buyable": true, "sellable": true, "wsTradeEligible": true, "securityType": "EQUITY", "currency": "USD",
                "stock": {"name": "BigBear.ai Holdings Inc", "symbol": "BBAI", "primaryExchange": "NYSE"},
                "quoteV2": {"__typename": "EquityQuote", "ask": 3.02, "bid": 3.0, "currency": "USD", "price": 3.01, "previousBaseline": 2.9, "marketStatus": "OPEN", "askSize": 5, "bidSize": 7}}]}))
        }
        "FetchSecurityMarketData" => Ok(json!({"security": {"id": "sec-s-bbai", "allowedOrderSubtypes": ["MARKET", "LIMIT"], "marginRates": {"clientMarginRate": 0.5}}})),
        "FetchTradingBalanceBuyingPower" => Ok(json!({"account": {"financials": {"current": {"tradingBalanceViewV2": {"buyingPower": {"quantity": 9000.0, "currency": "USD"}, "cash": {"quantity": 100.0, "currency": "USD"}}}}}})),
        _ => panic!("{}", op),
    });
    set_session(Some(tok()));
    let r = o::ticket_quote("BBAI", "", "acct-margin", "NYSE");
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!(st(&r["quote"], "securityId"), "sec-s-bbai");
    let again = o::ticket_quote("BBAI", "", "acct-margin", "NYSE");
    unpatch();
    assert_eq!(again["ok"], json!(true));
    assert_eq!(*searches.lock().unwrap(), vec!["BBAI".to_string()], "Wealthsimple's search is asked once");
    let stored: Vec<Value> = bagholder_store::admin::list_securities(&conn()).unwrap().into_iter().filter(|x| st(x, "id") == "sec-s-bbai").collect();
    assert_eq!((st(&stored[0], "symbol"), st(&stored[0], "primaryExchange"), st(&stored[0], "currency")), ("BBAI".into(), "NYSE".into(), "USD".into()));
    assert_eq!(st(&o::resolve_security("BBAI", "").unwrap(), "id"), "sec-s-bbai", "a book symbol from now on");
}

#[test]
fn test_ticket_quote_for_a_listing_wealthsimple_lacks_says_so() {
    let _g = setup();
    set_session(Some(tok()));
    set_gql(|_, _| Ok(json!({"securitySearch": {"results": []}})));
    let r = o::ticket_quote("NEWCO", "", "acct-margin", "NYSE");
    unpatch();
    assert_eq!(r["ok"], json!(false));
    assert!(st(&r, "error").contains("No listing stored for NEWCO"), "{}", r);
    set_gql(|_, _| panic!("no call"));
    assert!(st(&o::ticket_quote("NEWCO", "", "acct-margin", ""), "error").contains("No listing stored"));
    unpatch();
}

#[test]
fn test_collateral_account_names_the_margin_account_it_backs() {
    let _g = setup();
    let raw = vec![
        json!({"id": "acct-margin", "nickname": "Trading", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "currency": "CAD", "status": "open", "type": "non_registered",
         "custodianAccounts": [{"id": "cust-margin-1"}], "accountFeatures": [{"name": "MARGIN", "enabled": true, "functional": true, "metadata": null}]}),
        json!({"id": "acct-tfsa", "nickname": "TFSA", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa",
         "custodianAccounts": [{"id": "cust-tfsa-1"}], "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": true, "functional": true, "metadata": {"__typename": "MarginBoostFeatureMetadata", "targetMarginAccountId": "cust-margin-1"}}]}),
        json!({"id": "acct-rrsp", "nickname": "RRSP", "unifiedAccountType": "SELF_DIRECTED_RRSP", "currency": "CAD", "status": "open", "type": "rrsp",
         "custodianAccounts": [{"id": "cust-rrsp-1"}], "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": false, "functional": false, "metadata": {"__typename": "MarginBoostFeatureMetadata", "targetMarginAccountId": "cust-margin-1"}}]}),
        json!({"id": "acct-lira", "nickname": "LIRA", "unifiedAccountType": "SELF_DIRECTED_LIRA", "currency": "CAD", "status": "open", "type": "lira", "custodianAccounts": [], "accountFeatures": []}),
    ];
    let slim_v = bagholder_ws::sync::slim_accounts(&raw);
    let slim: HashMap<String, Value> = slim_v.iter().map(|a| (st(a, "id"), a.clone())).collect();
    assert_eq!(st(&slim["acct-tfsa"], "marginAccountId"), "acct-margin");
    assert_eq!(st(&slim["acct-rrsp"], "marginAccountId"), "", "a feature that is not enabled links nothing");
    assert_eq!((st(&slim["acct-margin"], "marginAccountId"), st(&slim["acct-lira"], "marginAccountId")), (String::new(), String::new()));
    bagholder_store::tables::replace_accounts(&conn(), &slim_v).unwrap();
    let snap = bagholder_store::snapshot::snapshot(&conn(), false).unwrap();
    let kept: HashMap<String, Value> = snap["accounts"].as_array().unwrap().iter().map(|a| (st(a, "id"), a.clone())).collect();
    assert_eq!(st(&kept["acct-tfsa"], "marginAccountId"), "acct-margin", "the link survives the store");
    let by_id: HashMap<String, Value> = o::order_accounts(None).into_iter().map(|a| (st(&a, "id"), a)).collect();
    assert_eq!(st(&by_id["acct-margin"], "marginAccountId"), "acct-margin");
    assert_eq!(st(&by_id["acct-tfsa"], "marginAccountId"), "acct-margin");
    assert_eq!(st(&by_id["acct-rrsp"], "marginAccountId"), "");
}

fn qnc_summary() -> Value {
    json!({"securities": [{"id": "sec-s-us", "buyable": true, "sellable": true, "wsTradeEligible": true, "securityType": "EQUITY", "currency": "USD",
        "stock": {"name": "Quantum Emotion Corp", "symbol": "QNC", "primaryExchange": "NYSE"},
        "quoteV2": {"__typename": "EquityQuote", "ask": 3.02, "bid": 3.0, "currency": "USD", "price": 3.01, "previousBaseline": 2.9, "marketStatus": "OPEN", "askSize": 5, "bidSize": 7}}]})
}

#[test]
fn test_ticket_quote_on_a_collateral_account_carries_the_margin_it_backs() {
    let _g = setup();
    set_gql(|op, vars| match op {
        "FetchSecuritiesSummary" => Ok(qnc_summary()),
        "FetchSecurityMarketData" => Ok(json!({"security": {"id": "sec-s-us", "allowedOrderSubtypes": ["MARKET", "LIMIT"], "marginRates": {"clientMarginRate": 0.5}}})),
        "FetchTradingBalanceBuyingPower" => {
            assert_eq!(st(vars, "accountCanonicalId"), "acct-tfsa", "cash and buying power are the TFSA's own");
            Ok(json!({"account": {"financials": {"current": {"tradingBalanceViewV2": {"buyingPower": {"quantity": 500.0, "currency": "USD"}, "cash": {"quantity": 500.0, "currency": "USD"}}}}}}))
        }
        _ => panic!("{}", op),
    });
    set_session(Some(tok()));
    let r = o::ticket_quote("QNC", "", "acct-tfsa", "");
    unpatch();
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!(n(&r, "cash"), 500.0);
    assert_eq!(n(&r, "marginAvailable"), 12680.45, "the margin account the TFSA backs");
    assert_eq!(st(&r["account"], "marginAccountId"), "acct-margin");
}

#[test]
fn test_ticket_quote_answers_with_everything_the_panel_shows() {
    let _g = setup();
    set_gql(|op, vars| match op {
        "FetchSecuritiesSummary" => {
            assert_eq!(vars["ids"], json!(["sec-s-us"]));
            Ok(qnc_summary())
        }
        "FetchSecurityMarketData" => Ok(json!({"security": {"id": "sec-s-us", "allowedOrderSubtypes": ["MARKET", "LIMIT", "STOP_LIMIT"], "marginRates": {"clientMarginRate": 0.5}}})),
        "FetchTradingBalanceBuyingPower" => {
            assert_eq!((st(vars, "accountCanonicalId"), st(vars, "currency"), st(vars, "securityId")), ("acct-margin".into(), "USD".into(), "sec-s-us".into()));
            Ok(json!({"account": {"financials": {"current": {"tradingBalanceViewV2": {"buyingPower": {"quantity": 9000.0, "currency": "USD"}, "cash": {"quantity": 100.0, "currency": "USD"}}}}}}))
        }
        _ => panic!("{}", op),
    });
    set_session(Some(tok()));
    // Orders are live by default; the test process runs with BAGHOLDER_DRY_ORDERS=1, so the switch is set here.
    set_live(Some(true));
    let r = o::ticket_quote("QNC", "", "acct-margin", "");
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!(st(&r["quote"], "symbol"), "QNC");
    assert_eq!(r["orderTypes"], json!(["MARKET", "LIMIT", "STOP_LIMIT"]));
    assert_eq!(n(&r, "marginRate"), 0.5);
    assert_eq!(n(&r, "marginAvailable"), 12680.45);
    assert_eq!((n(&r, "buyingPower"), n(&r, "cash")), (9000.0, 100.0));
    let ids: Vec<String> = r["accounts"].as_array().unwrap().iter().map(|a| st(a, "id")).collect();
    assert_eq!(ids, ["acct-margin", "acct-tfsa"]);
    assert_eq!(r["live"], json!(true));
    set_session(None);
    assert_eq!(st(&o::ticket_quote("QNC", "", "acct-margin", ""), "error"), "Not connected.");
    unpatch();
    assert!(st(&o::ticket_quote("NOPE", "", "acct-margin", ""), "error").contains("No listing stored"));
}

#[test]
fn test_the_request_is_the_one_wealthsimples_web_app_sends() {
    let _g = setup();
    let (row, req) = o::order_request(&ticket(json!({}))).unwrap();
    assert!(st(&req, "externalId").starts_with("order-"));
    let mut rest = req.clone();
    rest.as_object_mut().unwrap().shift_remove("externalId");
    assert_eq!(rest, json!({"canonicalAccountId": "acct-margin", "executionType": "LIMIT", "orderType": "BUY_QUANTITY", "quantity": 25.0, "securityId": "sec-s-us", "timeInForce": "DAY", "limitPrice": 165.4}));
    assert_eq!(row["stopLoss"], json!({"kind": "stop", "price": 157.13, "trail": null, "trailUnit": "pct"}));
    assert_eq!(row["takeProfit"], json!({"price": 181.94}));
    assert_eq!((st(&row, "symbol"), st(&row, "currency"), st(&row, "account")), ("QNC".into(), "USD".into(), "Trading".into()));
    let (_, req) = o::order_request(&ticket(json!({"type": "MARKET", "tif": "UNTIL_CANCEL"}))).unwrap();
    assert!(req.get("limitPrice").is_none());
    assert!(req.get("stopPrice").is_none());
    assert_eq!(st(&req, "timeInForce"), "UNTIL_CANCEL");
    let (_, req) = o::order_request(&ticket(json!({"type": "STOP_LIMIT", "stopPrice": 170.0}))).unwrap();
    assert_eq!((st(&req, "executionType"), n(&req, "stopPrice"), n(&req, "limitPrice")), ("STOP_LIMIT".into(), 170.0, 165.4));
    let (row, req) = o::order_request(&ticket(json!({"side": "SELL", "type": "STOP", "stopPrice": 150.0}))).unwrap();
    assert_eq!((st(&req, "executionType"), st(&req, "orderType"), n(&req, "stopPrice")), ("STOP".into(), "SELL_QUANTITY".into(), 150.0));
    assert!(row["stopLoss"].is_null(), "a sell has nothing to protect");
    assert!(row["takeProfit"].is_null());
    let (row, _) = o::order_request(&ticket(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "pct"}}))).unwrap();
    assert_eq!(row["stopLoss"], json!({"kind": "trail", "price": null, "trail": 5.0, "trailUnit": "pct"}));
}

#[test]
fn test_a_bad_ticket_is_refused_with_the_reason() {
    let _g = setup();
    let bad = |over: Value| o::order_request(&ticket(over)).err().unwrap_or_default();
    assert!(bad(json!({"quantity": 0})).contains("Quantity"));
    assert!(bad(json!({"limitPrice": null})).contains("limit price"));
    assert!(bad(json!({"type": "STOP", "stopPrice": null})).contains("stop price"));
    assert!(bad(json!({"accountId": "acct-crypto"})).contains("account"));
    assert!(bad(json!({"symbol": "NOPE", "securityId": ""})).contains("No listing"));
    assert!(bad(json!({"side": "HOLD"})).contains("Side"));
    assert!(bad(json!({"type": "TRAILING"})).contains("Order type"));
    assert!(bad(json!({"tif": "WEEK"})).contains("Time in force"));
    assert!(bad(json!({"stopLoss": {"kind": "stop", "price": 0}})).contains("stop loss price"));
    assert!(bad(json!({"takeProfit": {"price": null}})).contains("take profit"));
}

#[test]
fn test_orders_are_live_unless_the_dry_setting_is_on() {
    // Orders are live without BAGHOLDER_DRY_ORDERS; this process
    // always sets it (tests_common), so the same rule is checked the other way.
    let _g = setup();
    assert_eq!(std::env::var("BAGHOLDER_DRY_ORDERS").as_deref(), Ok("1"));
    assert!(!o::orders_live(), "BAGHOLDER_DRY_ORDERS=1: not live");
}

#[test]
fn test_under_the_dry_setting_a_submit_is_recorded_and_nothing_is_sent() {
    let _g = setup();
    set_gql(|_, _| panic!("must not be called"));
    set_live(Some(false));
    let r = o::place_order(&ticket(json!({})));
    assert_eq!(crate::status::payload()["ordersLive"], json!(false));
    unpatch();
    assert_eq!(r["ok"], json!(true));
    assert_eq!(st(&r, "status"), "dry");
    let rows = list_orders();
    assert_eq!(rows.len(), 1);
    assert_eq!((st(&rows[0], "id"), st(&rows[0], "status"), st(&rows[0], "side"), st(&rows[0], "type"), n(&rows[0], "quantity")), (st(&r, "id"), "dry".into(), "BUY".into(), "LIMIT".into(), 25.0));
    assert_eq!(st(&rows[0]["request"], "executionType"), "LIMIT");
    assert_eq!(n(&rows[0]["stopLoss"], "price"), 157.13);
    assert_eq!(get_order(&st(&r, "id"))["takeProfit"], json!({"price": 181.94}));
}

#[test]
fn test_by_default_the_order_goes_to_wealthsimple_and_the_answer_is_kept() {
    let _g = setup();
    let sent: Sent = Arc::default();
    let s2 = sent.clone();
    set_gql(move |op, vars| {
        s2.lock().unwrap().push((op.to_string(), vars.clone()));
        Ok(json!({"soOrdersCreateOrder": {"errors": [], "order": {"orderId": "ws-123", "createdAt": "2026-09-10T15:31:00Z"}}}))
    });
    set_live(Some(true));
    set_session(Some(tok()));
    let r = o::place_order(&ticket(json!({})));
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!((st(&r, "status"), st(&r, "wsOrderId")), ("sent".into(), "ws-123".into()));
    {
        let s = sent.lock().unwrap();
        assert_eq!(s[0].0, "SoOrdersOrderCreate");
        assert_eq!(st(&s[0].1["input"], "externalId"), st(&r, "id"));
    }
    let row = get_order(&st(&r, "id"));
    assert_eq!((st(&row, "status"), st(&row, "wsOrderId")), ("sent".into(), "ws-123".into()));
    set_gql(|_, _| Ok(json!({"soOrdersCreateOrder": {"errors": [{"code": "ORDER.insufficient_funds", "message": "Insufficient funds"}], "order": null}})));
    let r2 = o::place_order(&ticket(json!({})));
    assert_eq!(r2["ok"], json!(false));
    assert!(st(&r2, "error").contains("Insufficient funds"));
    let row2 = get_order(&st(&r2, "id"));
    assert_eq!((st(&row2, "status"), st(&row2, "error")), ("rejected".into(), "Insufficient funds".into()));
    set_gql(|_, _| Err(CallError::NotAuthorized));
    let r3 = o::place_order(&ticket(json!({})));
    unpatch();
    assert!(st(&r3, "error").contains("refused the session"));
    assert_eq!(st(&get_order(&st(&r3, "id")), "status"), "failed");
    assert_eq!(list_orders().len(), 3, "every attempt is a row");
}

#[test]
fn test_the_orders_table_survives_clear_synced_data() {
    let _g = setup();
    set_live(Some(false));
    let r = o::place_order(&ticket(json!({})));
    unpatch();
    bagholder_store::admin::clear_synced_data(&conn(), false, false).unwrap();
    assert_eq!(list_orders().len(), 1);
    assert_eq!(st(&list_orders()[0], "id"), st(&r, "id"));
}

// ---------------------------------------------------------------------------
// OrdersReadBackTest
// ---------------------------------------------------------------------------

#[test]
fn test_wealthsimple_statuses_group_as_the_page_shows_them() {
    for ws in ["NEW", "PENDING_SUBMISSION", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT"] {
        assert_eq!(o::app_status(ws), "pending", "{}", ws);
    }
    assert_eq!(o::app_status("CANCEL_PENDING"), "cancelling");
    assert_eq!(o::app_status("FILLED"), "filled");
    assert_eq!(o::app_status("POSTED"), "filled");
    assert_eq!(o::app_status("CANCELLED"), "cancelled");
    assert_eq!(o::app_status("DELETED"), "cancelled");
    assert_eq!(o::app_status("EXPIRED"), "expired");
    assert_eq!(o::app_status("REJECTED"), "rejected");
    assert_eq!(o::app_status(""), "");
}

fn ident_sess() -> Value {
    json!({"access_token": "t", "identity_canonical_id": "ident-1"})
}

#[test]
fn test_a_sent_order_is_read_back_by_its_external_id_on_the_TR_branch() {
    let _g = setup();
    let oid = sent_order();
    let asked: Sent = Arc::default();
    let a2 = asked.clone();
    set_gql(move |op, vars| {
        a2.lock().unwrap().push((op.to_string(), vars.clone()));
        match op {
            "FetchSoOrdersExtendedOrder" => Ok(json!({"soOrdersExtendedOrder": {"status": "FILLED", "filledQuantity": 25, "averageFilledPrice": 165.38, "submittedAtUtc": "2026-09-10T13:30:00Z", "expiredAtUtc": null, "rejectionCause": null, "timeInForce": "DAY", "submittedQuantity": 25}})),
            "OrderServiceExtendedOrderFeed" => Ok(json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}}})),
            _ => panic!("{}", op),
        }
    });
    set_session(Some(ident_sess()));
    let r = o::refresh_orders("");
    assert_eq!((n(&r, "read"), n(&r, "added"), n(&r, "failed")), (1.0, 0.0, 0.0), "{}", r);
    {
        let a = asked.lock().unwrap();
        assert_eq!((a[0].0.as_str(), &a[0].1), ("FetchSoOrdersExtendedOrder", &json!({"branchId": "TR", "externalId": oid})));
        assert_eq!(a[1].1["statuses"], json!(o::WS_PENDING.to_vec()));
    }
    let row = get_order(&oid);
    assert_eq!((st(&row, "status"), st(&row, "wsStatus"), n(&row, "filledQty"), n(&row, "avgFill"), st(&row, "submittedAt")),
        ("filled".into(), "FILLED".into(), 25.0, 165.38, "2026-09-10T13:30:00Z".into()));
    asked.lock().unwrap().clear();
    o::refresh_orders("");
    unpatch();
    let ops: Vec<String> = asked.lock().unwrap().iter().map(|a| a.0.clone()).collect();
    assert_eq!(ops, ["OrderServiceExtendedOrderFeed"]);
}

#[test]
fn test_an_order_placed_in_wealthsimples_app_becomes_a_row_from_the_feed() {
    let _g = setup();
    let node = json!({"id": "order-ws-placed", "orderId": "ws-9", "canonicalAccountId": "acct-tfsa", "createdAtUtc": "2026-09-10T01:00:00Z", "status": "SUBMITTED", "side": "BUY", "executionType": "LIMIT",
        "submittedQuantity": 3, "limitPrice": 1.76, "stopPrice": null, "averageFillPrice": null, "securityCurrency": "USD", "securityId": "sec-s-us", "symbol": "QNC", "security": {"id": "sec-s-us", "stock": {"symbol": "QNC", "name": "Quantum Emotion Corp"}}});
    set_gql(move |op, _| match op {
        "OrderServiceExtendedOrderFeed" => Ok(json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": [{"cursor": "c1", "node": node.clone()}], "pageInfo": {"hasNextPage": false, "endCursor": "c1"}}}})),
        "FetchSoOrdersExtendedOrder" => Ok(json!({"soOrdersExtendedOrder": {"status": "SUBMITTED", "timeInForce": "UNTIL_CANCEL", "submittedQuantity": 3, "limitPrice": 1.76, "securityCurrency": "USD"}})),
        _ => panic!("{}", op),
    });
    set_session(Some(ident_sess()));
    let r = o::refresh_orders("");
    assert_eq!((n(&r, "read"), n(&r, "added")), (0.0, 1.0), "{}", r);
    let row = get_order("order-ws-placed");
    assert_eq!((st(&row, "source"), st(&row, "status"), st(&row, "symbol"), st(&row, "account"), st(&row, "side"), st(&row, "type"), n(&row, "quantity"), n(&row, "limitPrice"), st(&row, "wsOrderId")),
        ("wealthsimple".into(), "pending".into(), "QNC".into(), "TFSA".into(), "BUY".into(), "LIMIT".into(), 3.0, 1.76, "ws-9".into()));
    assert!(row["stopLoss"].is_null());
    let r = o::refresh_orders("");
    unpatch();
    assert_eq!((n(&r, "read"), n(&r, "added")), (1.0, 0.0), "{}", r);
    assert_eq!(st(&get_order("order-ws-placed"), "tif"), "UNTIL_CANCEL");
    assert_eq!(list_orders().len(), 1);
}

#[test]
fn test_a_failed_read_is_counted_and_the_others_still_happen() {
    let _g = setup();
    let oid = sent_order();
    set_gql(|op, _| {
        if op == "FetchSoOrdersExtendedOrder" {
            return Err(CallError::Failed("FetchSoOrdersExtendedOrder: boom".into()));
        }
        Ok(json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": [], "pageInfo": {"hasNextPage": false}}}}))
    });
    set_session(Some(ident_sess()));
    let r = o::refresh_orders("");
    unpatch();
    assert_eq!((r["ok"].clone(), n(&r, "failed")), (json!(false), 1.0), "{}", r);
    assert_eq!(st(&get_order(&oid), "status"), "sent", "an unanswered read changes nothing");
}

#[test]
fn test_cancel_needs_the_switch_and_a_live_order() {
    let _g = setup();
    let oid = sent_order();
    set_live(Some(false));
    assert!(st(&o::cancel_order(&oid), "error").contains("Orders are off"));
    set_live(Some(true)); // the default
    assert_eq!(st(&o::cancel_order("nope"), "error"), "No such order.");
    update_order(&oid, json!({"status": "filled"}));
    assert_eq!(st(&o::cancel_order(&oid), "error"), "That order is not open.");
    unpatch();
}

#[test]
fn test_cancel_goes_to_wealthsimple_by_external_id_and_the_row_says_cancelling() {
    let _g = setup();
    let oid = sent_order();
    let sent: Sent = Arc::default();
    let s2 = sent.clone();
    let o2 = oid.clone();
    set_gql(move |op, vars| {
        s2.lock().unwrap().push((op.to_string(), vars.clone()));
        Ok(json!({"orderServiceCancelOrder": {"externalId": o2, "errors": []}}))
    });
    set_live(Some(true));
    set_session(Some(tok()));
    let r = o::cancel_order(&oid);
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!(st(&r, "status"), "cancelling");
    assert_eq!(*sent.lock().unwrap(), vec![("SoOrdersOrderCancel".to_string(), json!({"cancelOrderRequest": {"externalId": oid}}))]);
    let row = get_order(&oid);
    assert_eq!((st(&row, "status"), st(&row, "wsStatus")), ("cancelling".into(), "CANCEL_PENDING".into()));
    update_order(&oid, json!({"status": "pending", "wsStatus": "SUBMITTED"}));
    let o3 = oid.clone();
    set_gql(move |_, _| Ok(json!({"orderServiceCancelOrder": {"externalId": o3, "errors": [{"code": "x", "message": "Too late to cancel"}]}})));
    let r = o::cancel_order(&oid);
    unpatch();
    assert!(st(&r, "error").contains("Too late to cancel"));
    assert_eq!(st(&get_order(&oid), "status"), "pending");
}

#[test]
fn test_the_orders_tab_opening_kicks_a_read_unless_one_is_fresh() {
    // refresh_orders cannot be replaced in Rust; the spawned read runs inline
    // against a fake feed, and what it asked records that it ran.
    let _g = setup();
    let ran: Sent = Arc::default();
    let r2 = ran.clone();
    set_gql(move |op, vars| {
        r2.lock().unwrap().push((op.to_string(), vars.clone()));
        Ok(json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": [], "pageInfo": {"hasNextPage": false}}}}))
    });
    set_session(Some(ident_sess()));
    seam::SPAWN_INLINE.store(true, Ordering::SeqCst);
    app().state.lock().unwrap().connected = false;
    assert!(!o::kick_orders_refresh(), "nothing is read while not connected");
    app().state.lock().unwrap().connected = true;
    *o::REFRESHED_AT.lock().unwrap() = String::new();
    assert_eq!(o::orders_payload(true)["ok"], json!(true));
    let ops: Vec<String> = ran.lock().unwrap().iter().map(|a| a.0.clone()).collect();
    assert_eq!(ops, ["OrderServiceExtendedOrderFeed"], "the list's first request reads everything");
    *o::REFRESHED_AT.lock().unwrap() = now_iso();
    assert!(!o::kick_orders_refresh(), "a read younger than the loop's tick is fresh enough");
    *o::REFRESHED_AT.lock().unwrap() = "2026-01-01T00:00:00Z".into();
    assert!(o::kick_orders_refresh());
    unpatch();
    app().state.lock().unwrap().connected = false;
    *o::REFRESHED_AT.lock().unwrap() = String::new();
}

#[test]
fn test_one_orders_read_after_a_send_does_not_count_as_a_check() {
    let _g = setup();
    let oid = sent_order();
    *o::REFRESHED_AT.lock().unwrap() = String::new();
    set_gql(|_, _| Ok(json!({"soOrdersExtendedOrder": {"status": "SUBMITTED"}})));
    set_session(Some(tok()));
    o::refresh_orders(&oid);
    unpatch();
    assert_eq!(*o::REFRESHED_AT.lock().unwrap(), "");
    assert_eq!(st(&get_order(&oid), "status"), "pending");
}

// ---------------------------------------------------------------------------
// StopFillBooksLocallyTest
// ---------------------------------------------------------------------------

fn long(cid: &str, qty: f64, price: f64) {
    let date = "2026-09-01";
    apply_ws(&[json!({
        "canonicalId": cid, "occurredAt": format!("{}T14:00:00Z", date), "transactionDate": date, "settlementDate": date,
        "accountId": "acct-tfsa", "bookId": "acct-tfsa", "fifoId": "acct-tfsa", "accountType": "TFSA",
        "activityType": "Trade", "activitySubType": "BUY", "symbol": "QNC", "name": "QNC", "currency": "USD",
        "quantity": qty, "unitPrice": price, "commission": 0.0, "netCashAmount": -(qty * price), "category": "trade",
        "source": "wealthsimple",
    })]);
}

fn ws_sell(cid: &str, qty: f64, price: f64, symbol: &str, sub: &str, atype: &str) -> Value {
    let date = "2026-09-10";
    json!({
        "canonicalId": cid, "occurredAt": format!("{}T20:47:00Z", date), "transactionDate": date, "settlementDate": date,
        "accountId": "acct-tfsa", "bookId": "acct-tfsa", "fifoId": "acct-tfsa", "accountType": "TFSA",
        "activityType": atype, "activitySubType": sub, "symbol": symbol, "name": symbol, "currency": "USD",
        "quantity": -qty, "unitPrice": price, "commission": 0.0, "netCashAmount": qty * price, "category": "trade",
        "source": "wealthsimple",
    })
}

fn place_live(oid: &str, symbol: &str, security_id: &str, qty: f64, typ: &str, role: &str) -> String {
    so::insert_order(&conn(), &json!({
        "id": oid, "accountId": "acct-tfsa", "account": "TFSA", "securityId": security_id, "symbol": symbol,
        "currency": "USD", "side": "SELL", "type": typ, "quantity": qty, "stopPrice": 1.60, "tif": "UNTIL_CANCEL",
        "status": "sent", "source": "bagholder", "role": role,
    }), &now_iso()).unwrap();
    oid.to_string()
}

fn fill(oid: &str, filled: f64, avg: f64, submitted: Option<f64>) -> Value {
    let last = "2026-09-10T20:47:00Z";
    let ext = json!({"status": "FILLED", "filledQuantity": filled, "averageFilledPrice": avg,
        "firstFilledAtUtc": last, "lastFilledAtUtc": last, "submittedAtUtc": last, "submittedQuantity": submitted});
    set_gql(move |_, _| Ok(json!({"soOrdersExtendedOrder": ext.clone()})));
    set_session(Some(tok()));
    let r = o::refresh_orders(oid);
    unpatch();
    r
}

fn booked(symbol: &str) -> Vec<Value> {
    activities().into_iter().filter(|a| st(a, "source") == "bagholder-fill" && st(a, "symbol") == symbol).collect()
}

#[test]
fn test_a_filled_order_is_booked_as_one_local_sell_that_closes_the_position() {
    let _g = setup();
    long("ws-buy-1", 5.0, 1.40);
    let oid = place_live("order-stop-1", "QNC", "sec-s-us", 5.0, "STOP", "stop");
    let r = fill(&oid, 5.0, 1.6374, None);
    assert_eq!(n(&r, "read"), 1.0, "{}", r);
    let bk = booked("QNC");
    assert_eq!(bk.len(), 1, "exactly one local activity for the fill");
    let b = &bk[0];
    assert_eq!((st(b, "symbol"), st(b, "accountId"), n(b, "quantity"), n(b, "unitPrice")), ("QNC".into(), "acct-tfsa".into(), -5.0, 1.6374));
    assert_eq!(bagholder_store::activities::trade_side(b), "SELL");
    assert_eq!(st(b, "transactionDate"), "2026-09-10");
    assert!(b.get("canonicalId").map_or(true, |v| v.is_null()), "a local row, not a fabricated Wealthsimple row");
    assert!(!bagholder_store::activities::looks_like_homemade_id(&st(b, "id")));
    let res = bagholder_model::fifo::match_fifo(&activities().iter().map(|a| serde_json::from_value(a.clone()).unwrap()).collect::<Vec<_>>());
    assert!(res.open.is_empty(), "the 5 shares are gone once the fill is on the book");
    assert_eq!(n(&get_order(&oid), "fillBookedQty"), 5.0);
}

#[test]
fn test_the_real_wealthsimple_sell_collapses_with_the_booked_row() {
    let _g = setup();
    long("ws-buy-1", 5.0, 1.40);
    let oid = place_live("order-stop-2", "QNC", "sec-s-us", 5.0, "STOP", "stop");
    fill(&oid, 5.0, 1.6374, None);
    let before = bagholder_store::activities::activity_count(&conn()).unwrap();
    assert_eq!(booked("QNC").len(), 1);
    let result = apply_ws(&[ws_sell("ws-sell-9", 5.0, 1.6374, "QNC", "SELL", "Trade")]);
    assert_eq!((result.linked, result.inserted), (1, 0), "the synced sell links to the booked row, none inserted");
    assert_eq!(bagholder_store::activities::activity_count(&conn()).unwrap(), before);
    let rows: Vec<Value> = activities().into_iter().filter(|a| bagholder_store::activities::trade_side(a) == "SELL" && st(a, "symbol") == "QNC").collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(st(&rows[0], "canonicalId"), "ws-sell-9");
    let res = bagholder_model::fifo::match_fifo(&activities().iter().map(|a| serde_json::from_value(a.clone()).unwrap()).collect::<Vec<_>>());
    assert!(res.open.is_empty());
    assert_eq!(res.closed.iter().map(|t| t.quantity).sum::<f64>(), 5.0);
    apply_ws(&[ws_sell("ws-sell-9", 5.0, 1.6374, "QNC", "SELL", "Trade")]);
    assert_eq!(bagholder_store::activities::activity_count(&conn()).unwrap(), before);
}

#[test]
fn test_re_reading_the_same_fill_books_nothing_more() {
    let _g = setup();
    long("ws-buy-1", 5.0, 1.40);
    let oid = place_live("order-stop-3", "QNC", "sec-s-us", 5.0, "STOP", "stop");
    fill(&oid, 5.0, 1.6374, None);
    assert_eq!(booked("QNC").len(), 1);
    update_order(&oid, json!({"status": "sent"}));
    fill(&oid, 5.0, 1.6374, None);
    assert_eq!(booked("QNC").len(), 1, "the durable fill_booked_qty marker prevents a duplicate");
}

#[test]
fn test_a_partial_fill_reduces_the_position_it_does_not_close_it() {
    let _g = setup();
    long("ws-buy-1", 10.0, 1.40);
    let oid = place_live("order-stop-4", "QNC", "sec-s-us", 10.0, "STOP", "stop");
    fill(&oid, 5.0, 1.6374, Some(10.0));
    let bk = booked("QNC");
    assert_eq!(bk.len(), 1);
    assert_eq!(n(&bk[0], "quantity"), -5.0, "the filled quantity, never the ordered quantity");
    let res = bagholder_model::fifo::match_fifo(&activities().iter().map(|a| serde_json::from_value(a.clone()).unwrap()).collect::<Vec<_>>());
    assert_eq!(res.open.len(), 1);
    assert_eq!(res.open[0].qty, 5.0, "five shares still held");
}

#[test]
fn test_an_option_fill_nets_with_the_hundred_times_multiplier() {
    let _g = setup();
    let sym = "QNC 16JAN26 5.00 CALL";
    assert!(bagholder_model::symbols::is_option_symbol(sym));
    apply_ws(&[json!({
        "canonicalId": "ws-opt-buy", "occurredAt": "2026-09-01T14:00:00Z", "transactionDate": "2026-09-01",
        "settlementDate": "2026-09-01", "accountId": "acct-tfsa", "bookId": "acct-tfsa", "fifoId": "acct-tfsa",
        "accountType": "TFSA", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "symbol": sym,
        "name": sym, "currency": "USD", "quantity": 2, "unitPrice": 1.00, "commission": 0.0,
        "netCashAmount": -200.0, "category": "trade", "source": "wealthsimple",
    })]);
    let oid = place_live("order-opt-1", sym, "sec-o-1", 2.0, "LIMIT", "entry");
    fill(&oid, 2.0, 1.50, None);
    let bk = booked(sym);
    assert_eq!(bk.len(), 1);
    let b = &bk[0];
    assert_eq!((n(b, "quantity"), n(b, "unitPrice")), (-2.0, 1.50));
    assert!((n(b, "netCashAmount") - 300.0).abs() < 1e-7, "the 100x multiplier is in the cash");
    let res = bagholder_model::fifo::match_fifo(&activities().iter().map(|a| serde_json::from_value(a.clone()).unwrap()).collect::<Vec<_>>());
    assert!(res.open.is_empty(), "the two contracts are closed");
    assert!((res.closed.iter().map(|t| t.pnl).sum::<f64>() - 100.0).abs() < 1e-7);
    let before = bagholder_store::activities::activity_count(&conn()).unwrap();
    let result = apply_ws(&[ws_sell("ws-opt-sell", 2.0, 1.50, sym, "SELLTOCLOSE", "OPTIONS_SELL")]);
    assert_eq!((result.linked, result.inserted), (1, 0));
    assert_eq!(bagholder_store::activities::activity_count(&conn()).unwrap(), before);
}

// ---------------------------------------------------------------------------
// _EngineBase
// ---------------------------------------------------------------------------

struct Engine {
    sent: Sent,
    #[allow(dead_code)]
    rejections: Arc<Mutex<Vec<String>>>,
}

fn engine() -> (MutexGuard<'static, ()>, Engine) {
    let g = setup();
    let c = conn();
    bagholder_store::tables::replace_balances(&c, &[json!({"accountId": "acct-margin", "securityId": "sec-s-us", "quantity": 25})]).unwrap();
    bagholder_store::tables::set_meta(&c, "balances_read_at", "").unwrap();
    o::stop_allowed_cache().lock().unwrap().clear();
    o::bracket_said().lock().unwrap().clear();
    let e = Engine { sent: Arc::default(), rejections: Arc::default() };
    (g, e)
}

impl Engine {
    /// The fake Wealthsimple, orders on, a session, no threads.
    fn live(&self) {
        let sent = self.sent.clone();
        let rej = self.rejections.clone();
        set_gql(move |op, vars| {
            let len = {
                let mut s = sent.lock().unwrap();
                s.push((op.to_string(), vars.clone()));
                s.len()
            };
            match op {
                "SoOrdersOrderCreate" => {
                    let mut r = rej.lock().unwrap();
                    if !r.is_empty() {
                        let m = r.remove(0);
                        return Ok(json!({"soOrdersCreateOrder": {"errors": [{"code": "x", "message": m}], "order": null}}));
                    }
                    Ok(json!({"soOrdersCreateOrder": {"errors": [], "order": {"orderId": format!("ws-{}", len), "createdAt": "2026-09-10T13:30:00Z"}}}))
                }
                "SoOrdersOrderCancel" => Ok(json!({"orderServiceCancelOrder": {"externalId": vars["cancelOrderRequest"]["externalId"], "errors": []}})),
                "FetchSecurityMarketData" => Ok(json!({"security": {"id": vars["id"], "allowedOrderSubtypes": ["MARKET", "LIMIT", "STOP", "STOP_LIMIT"], "marginRates": {"clientMarginRate": 0.3}}})),
                "FetchSoOrdersExtendedOrder" => {
                    let row = so::get_order(&conn(), &st(vars, "externalId")).unwrap().unwrap_or(json!({}));
                    let ws = match st(&row, "status").as_str() {
                        "cancelling" => "CANCEL_PENDING",
                        "filled" => "FILLED",
                        "cancelled" => "CANCELLED",
                        "expired" => "EXPIRED",
                        "rejected" => "REJECTED",
                        _ => "SUBMITTED",
                    };
                    let exp = if st(&row, "expiresAt").is_empty() { Value::Null } else { row["expiresAt"].clone() };
                    let g = |k: &str| row.get(k).cloned().unwrap_or(Value::Null);
                    Ok(json!({"soOrdersExtendedOrder": {"status": ws, "filledQuantity": g("filledQty"), "averageFilledPrice": g("avgFill"), "submittedQuantity": g("quantity"),
                        "timeInForce": g("tif"), "expiredAtUtc": exp}}))
                }
                _ => panic!("{}", op),
            }
        });
        set_live(Some(true));
        set_session(Some(tok()));
    }

    fn off(&self) {
        unpatch();
    }

    fn entry(&self, over: Value) -> (String, Value) {
        self.live();
        let r = o::place_order(&ticket(over));
        self.off();
        assert_eq!(r["ok"], json!(true), "{}", r);
        (st(&r, "id"), get_bracket(&st(&r, "bracketId")))
    }

    fn tick(&self, quote: Option<Value>) -> Value {
        self.live();
        let mut m = HashMap::new();
        if let Some(q) = quote {
            m.insert("sec-s-us".to_string(), q);
        }
        let r = o::bracket_tick(Some(m));
        self.off();
        r
    }

    fn ops(&self) -> Vec<String> {
        self.sent.lock().unwrap().iter().map(|x| x.0.clone()).collect()
    }

    fn creates(&self) -> Vec<Value> {
        self.sent.lock().unwrap().iter().filter(|x| x.0 == "SoOrdersOrderCreate").map(|x| x.1["input"].clone()).collect()
    }

    fn cancels(&self) -> Vec<String> {
        self.sent.lock().unwrap().iter().filter(|x| x.0 == "SoOrdersOrderCancel").map(|x| st(&x.1["cancelOrderRequest"], "externalId")).collect()
    }

    fn clear(&self) {
        self.sent.lock().unwrap().clear();
    }
}

fn q(last: f64, bid: Option<f64>, status: &str) -> Option<Value> {
    Some(json!({"last": last, "bid": bid.unwrap_or(last), "ask": last + 0.02, "marketStatus": status}))
}

// ---------------------------------------------------------------------------
// OrdersPanelTest
// ---------------------------------------------------------------------------

#[test]
fn test_open_orders_are_counted_for_the_header_badge() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    assert_eq!(o::open_orders_count(), 1);
    assert_eq!(crate::status::payload()["openOrders"], json!(1));
    update_order(&oid, json!({"status": "filled", "filledQty": 25}));
    e.tick(None);
    assert_eq!(st(&get_bracket(&st(&b, "id")), "status"), "armed");
    assert_eq!(o::open_orders_count(), 1);
    update_bracket(&st(&b, "id"), json!({"status": "done"}));
    assert_eq!(o::open_orders_count(), 0);
}

#[test]
fn test_edit_sends_wealthsimples_modify_with_the_new_price_and_quantity() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    let sent: Sent = Arc::default();
    let s2 = sent.clone();
    let fake = move |op: &str, vars: &Value| {
        s2.lock().unwrap().push((op.to_string(), vars.clone()));
        Ok(json!({"soOrdersModifyOrder": {"errors": []}}))
    };
    set_gql(fake.clone());
    set_live(Some(true));
    set_session(Some(tok()));
    let r = o::modify_order(&oid, Some(&json!(30)), Some(&json!(164.0)));
    unpatch();
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!(*sent.lock().unwrap(), vec![("SoOrdersOrderModify".to_string(), json!({"input": {"externalId": oid, "newLimitPrice": 164.0, "newQuantity": 30.0}}))]);
    let row = get_order(&oid);
    assert_eq!((n(&row, "quantity"), n(&row, "limitPrice")), (30.0, 164.0));
    assert_eq!(n(&get_bracket(&st(&b, "id")), "quantity"), 30.0, "a waiting bracket follows the entry's quantity");
    sent.lock().unwrap().clear();
    set_gql(fake);
    set_live(Some(true));
    set_session(Some(tok()));
    assert_eq!(o::modify_order(&oid, Some(&json!(30)), Some(&json!(165.0)))["ok"], json!(true));
    assert_eq!(sent.lock().unwrap().last().unwrap().1["input"], json!({"externalId": oid, "newLimitPrice": 165.0}));
    assert_eq!(o::modify_order(&oid, Some(&json!(30)), Some(&json!(165.0)))["unchanged"], json!(true));
    unpatch();
    set_live(Some(true)); // the default
    assert!(st(&o::modify_order(&oid, Some(&json!(0)), Some(&json!(165.0))), "error").contains("more than zero"));
    assert_eq!(st(&o::modify_order("nope", Some(&json!(1)), Some(&json!(1))), "error"), "No such order.");
    set_gql(|_, _| Ok(json!({"soOrdersModifyOrder": {"errors": [{"code": "x", "message": "Too late"}]}})));
    set_session(Some(tok()));
    assert!(st(&o::modify_order(&oid, Some(&json!(31)), Some(&json!(165.0))), "error").contains("Too late"));
    unpatch();
    set_live(Some(true));
    assert_eq!(n(&get_order(&oid), "quantity"), 30.0, "a refused change changes nothing");
    update_order(&oid, json!({"status": "filled"}));
    assert!(st(&o::modify_order(&oid, Some(&json!(31)), Some(&json!(165.0))), "error").contains("not open"));
    unpatch();
}

fn adjust(e: &Engine, id: &str, leg: &str, price: Option<Value>, trail: Option<Value>, remove: bool) -> Value {
    e.live();
    let r = o::adjust_bracket(id, leg, price.as_ref(), trail.as_ref(), remove);
    e.off();
    r
}

#[test]
fn test_adjusting_a_resting_stop_cancels_it_and_the_engine_places_the_new_level() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25}));
    e.tick(None);
    let b = get_bracket(&st(&b, "id"));
    let first = st(&b, "slOrderId");
    e.clear();
    let r = adjust(&e, &st(&b, "id"), "sl", Some(json!(160.0)), None, false);
    assert_eq!(r["ok"], json!(true), "{}", r);
    assert_eq!(e.ops(), ["SoOrdersOrderCancel"]);
    let b = get_bracket(&st(&b, "id"));
    assert_eq!((n(&b, "slPrice"), st(&b, "slOrderId"), st(&b, "status")), (160.0, String::new(), "armed".into()));
    update_order(&first, json!({"status": "cancelled"}));
    e.clear();
    e.tick(None);
    let c = e.creates();
    assert_eq!((st(&c[0], "executionType"), n(&c[0], "stopPrice")), ("STOP".into(), 160.0));
}

#[test]
fn test_adjusting_the_target_and_a_trailing_stop() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "pct"}}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25, "avgFill": 165.4}));
    e.tick(None);
    let id = st(&b, "id");
    assert_eq!(adjust(&e, &id, "tp", Some(json!(190.0)), None, false)["ok"], json!(true));
    assert_eq!(adjust(&e, &id, "sl", None, Some(json!(10)), false)["ok"], json!(true));
    let b = get_bracket(&id);
    assert_eq!(n(&b, "tpPrice"), 190.0);
    assert_eq!((n(&b, "slTrail"), n(&b, "slPrice")), (10.0, 148.86), "ten percent under the high of 165.40");
    assert!(st(&adjust(&e, &id, "tp", Some(json!(0)), None, false), "error").contains("required"));
    assert!(st(&adjust(&e, &id, "x", Some(json!(1)), None, false), "error").contains("Which leg"));
    assert_eq!(st(&adjust(&e, "nope", "tp", Some(json!(1)), None, false), "error"), "No such bracket.");
}

#[test]
fn test_a_placed_target_moved_is_cancelled_and_watched_again() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25}));
    e.tick(None);
    let id = st(&b, "id");
    let b = get_bracket(&id);
    update_order(&st(&b, "slOrderId"), json!({"status": "cancelled"}));
    update_bracket(&id, json!({"slKind": "", "slOrderId": ""}));
    e.tick(q(182.0, Some(181.95), "OPEN"));
    let b = get_bracket(&id);
    assert_eq!(st(&b, "status"), "target_placed");
    let tp = st(&b, "tpOrderId");
    e.clear();
    assert_eq!(adjust(&e, &id, "tp", Some(json!(185.0)), None, false)["ok"], json!(true));
    assert_eq!(e.ops(), ["SoOrdersOrderCancel"]);
    let b = get_bracket(&id);
    assert_eq!((st(&b, "status"), n(&b, "tpPrice"), st(&b, "tpOrderId")), ("armed".into(), 185.0, String::new()));
    assert_eq!(st(&get_order(&tp), "status"), "cancelling");
}

#[test]
fn test_removing_a_leg_and_then_the_other_ends_the_bracket() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25}));
    e.tick(None);
    let id = st(&b, "id");
    e.clear();
    assert_eq!(adjust(&e, &id, "sl", None, None, true)["ok"], json!(true));
    assert_eq!(e.ops(), ["SoOrdersOrderCancel"], "the resting stop is cancelled");
    let b = get_bracket(&id);
    assert_eq!((st(&b, "status"), st(&b, "slKind"), st(&b, "slOrderId")), ("armed".into(), String::new(), String::new()));
    assert_eq!(adjust(&e, &id, "tp", None, None, true)["ok"], json!(true));
    let b = get_bracket(&id);
    assert_eq!((st(&b, "status"), st(&b, "outcome")), ("cancelled".into(), "both legs removed".into()));
    assert!(b["tpPrice"].is_null());
}

// ---------------------------------------------------------------------------
// FeedMatchingTest
// ---------------------------------------------------------------------------

#[test]
fn test_the_feed_matches_bagholders_order_by_either_id() {
    let _g = setup();
    let oid = sent_order();
    let node = json!({"id": "order-some-other-id", "orderId": "ws-1", "canonicalAccountId": "acct-margin", "createdAtUtc": "2026-09-10T01:00:00Z", "status": "SUBMITTED", "side": "BUY", "executionType": "LIMIT",
        "submittedQuantity": 25, "limitPrice": 165.4, "securityCurrency": "USD", "securityId": "sec-s-us", "symbol": "QNC", "security": {"id": "sec-s-us", "stock": {"symbol": "QNC", "name": "Quantum Emotion Corp"}}});
    let mut node2 = node.clone();
    node2["id"] = json!(oid);
    set_gql(move |op, _| match op {
        "OrderServiceExtendedOrderFeed" => Ok(json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": [{"cursor": "c1", "node": node.clone()}, {"cursor": "c2", "node": node2.clone()}], "pageInfo": {"hasNextPage": false}}}})),
        "FetchSoOrdersExtendedOrder" => Ok(json!({"soOrdersExtendedOrder": {"status": "SUBMITTED"}})),
        _ => panic!("{}", op),
    });
    set_session(Some(ident_sess()));
    let r = o::refresh_orders("");
    unpatch();
    assert_eq!(n(&r, "added"), 0.0, "{}", r);
    assert_eq!(list_orders().len(), 1);
}

#[test]
fn test_an_option_order_from_the_feed_is_named_by_its_contract() {
    let _g = setup();
    let item = json!({"occurredAt": "2026-08-05T16:12:17.268Z", "canonicalId": "ws-opt-1", "status": "POSTED", "type": "OPTIONS_BUY", "subType": "BUYTOOPEN",
        "assetSymbol": "QNC 20NOV26 3.00 CALL", "assetQuantity": 5, "amount": -150, "accountId": "acct-1", "currency": "CAD", "securityId": "sec-o-1"});
    let mapped = bagholder_ws::mapping::map_activity(&item, None).expect("mapped");
    apply_ws(&[mapped]);
    let node = json!({"id": "order-opt", "orderId": "ws-7", "canonicalAccountId": "acct-tfsa", "createdAtUtc": "2026-08-05T16:16:16Z", "status": "SUBMITTED", "side": "SELL", "executionType": "LIMIT",
        "submittedQuantity": 40, "limitPrice": 0.25, "securityCurrency": "USD", "securityId": "sec-o-1", "symbol": "QNC", "security": {"id": "sec-o-1", "stock": {"symbol": "QNC", "name": "Quantum Emotion Corp"}}});
    set_gql(move |op, _| match op {
        "OrderServiceExtendedOrderFeed" => Ok(json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": [{"cursor": "c1", "node": node.clone()}], "pageInfo": {"hasNextPage": false}}}})),
        "FetchSoOrdersExtendedOrder" => Ok(json!({"soOrdersExtendedOrder": {"status": "SUBMITTED", "timeInForce": "UNTIL_CANCEL"}})),
        _ => panic!("{}", op),
    });
    set_session(Some(ident_sess()));
    o::refresh_orders("");
    unpatch();
    let row = get_order("order-opt");
    assert_eq!(st(&row, "symbol"), "QNC 20NOV26 3.00 CALL", "the book's name for the contract");
    assert_eq!((st(&row, "side"), n(&row, "quantity"), n(&row, "limitPrice"), st(&row, "account")), ("SELL".into(), 40.0, 0.25, "TFSA".into()));
}

// ---------------------------------------------------------------------------
// OrderTickTest
// ---------------------------------------------------------------------------

#[test]
fn test_prices_are_rounded_to_the_tick_before_they_are_sent() {
    let _g = setup();
    assert_eq!(o::order_tick(Some(1.736)), Some(1.74));
    assert_eq!(o::order_tick(Some(0.2537)), Some(0.2537));
    assert_eq!(o::order_tick(Some(0.25371)), Some(0.2537));
    assert_eq!(o::order_tick(None), None);
    let (row, req) = o::order_request(&ticket(json!({"limitPrice": 1.736, "stopLoss": {"kind": "stop", "price": 1.6512}, "takeProfit": {"price": 1.9139}}))).unwrap();
    assert_eq!(n(&req, "limitPrice"), 1.74);
    assert_eq!((n(&row["stopLoss"], "price"), n(&row["takeProfit"], "price")), (1.65, 1.91));
    let (_, req) = o::order_request(&ticket(json!({"type": "STOP_LIMIT", "limitPrice": 0.98765, "stopPrice": 1.005}))).unwrap();
    assert_eq!((n(&req, "limitPrice"), n(&req, "stopPrice")), (0.9877, 1.0));
}

// ---------------------------------------------------------------------------
// NinetyDayRollTest
// ---------------------------------------------------------------------------

fn armed(e: &Engine) -> Value {
    let (oid, b) = e.entry(json!({}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25, "avgFill": 165.38}));
    e.tick(None);
    let b = get_bracket(&st(&b, "id"));
    assert!(!st(&b, "slOrderId").is_empty());
    b
}

fn ending_in(order_id: &str, seconds: i64) {
    let t = stamp_of(now_unix() as i64 + seconds);
    let when = format!("{}.000Z", &t[..19]);
    update_order(order_id, json!({"expiresAt": when}));
}

#[test]
fn test_a_stop_near_its_ninety_days_is_rolled_outside_the_session() {
    let (_g, e) = engine();
    let b = armed(&e);
    let id = st(&b, "id");
    let first = st(&b, "slOrderId");
    ending_in(&first, 5 * 86400);
    e.clear();
    e.tick(q(170.0, None, "OPEN"));
    assert!(e.cancels().is_empty(), "five days left and the market open: it waits for the close");
    assert_eq!(st(&get_bracket(&id), "slOrderId"), first);
    e.tick(q(170.0, None, "CLOSED"));
    assert_eq!(e.cancels(), vec![first.clone()], "closed: the stop is cancelled to be placed again");
    let b = get_bracket(&id);
    assert_eq!((st(&b, "status"), st(&b, "slOrderId"), n(&b, "slPrice")), ("armed".into(), String::new(), 157.13), "the level is kept");
    assert!(e.creates().is_empty(), "nothing new until the cancel is confirmed");
    update_order(&first, json!({"status": "cancelled", "wsStatus": "CANCELLED"}));
    e.clear();
    e.tick(q(170.0, None, "CLOSED"));
    let b = get_bracket(&id);
    assert!(!st(&b, "slOrderId").is_empty() && st(&b, "slOrderId") != first, "a new stop rests at Wealthsimple");
    let c = e.creates();
    assert_eq!((st(&c[0], "executionType"), n(&c[0], "stopPrice"), st(&c[0], "timeInForce")), ("STOP".into(), 157.13, "UNTIL_CANCEL".into()));
    assert_eq!(st(&b, "status"), "armed");
}

#[test]
fn test_in_the_last_two_days_the_roll_does_not_wait_for_the_close() {
    let (_g, e) = engine();
    let b = armed(&e);
    let first = st(&b, "slOrderId");
    ending_in(&first, 86400);
    e.clear();
    e.tick(q(170.0, None, "OPEN"));
    assert_eq!(e.cancels(), vec![first]);
}

#[test]
fn test_a_stop_with_time_left_is_left_alone() {
    let (_g, e) = engine();
    let b = armed(&e);
    ending_in(&st(&b, "slOrderId"), 30 * 86400);
    e.clear();
    e.tick(q(170.0, None, "CLOSED"));
    assert!(e.cancels().is_empty());
    assert_eq!(st(&get_bracket(&st(&b, "id")), "slOrderId"), st(&b, "slOrderId"));
}

#[test]
fn test_the_end_is_ninety_days_from_submission_when_wealthsimple_reports_none() {
    let (_g, e) = engine();
    let b = armed(&e);
    let long_ago = stamp_of(now_unix() as i64 - 86 * 86400);
    update_order(&st(&b, "slOrderId"), json!({"expiresAt": "", "submittedAt": long_ago}));
    e.clear();
    e.tick(q(170.0, None, "CLOSED"));
    assert_eq!(e.cancels(), vec![st(&b, "slOrderId")]);
}

fn latest_stop(b: &Value) -> String {
    let rows: Vec<Value> = list_orders().into_iter().filter(|x| st(x, "parentId") == st(b, "orderId") && st(x, "role") == "stop").collect();
    st(&rows[0], "id")
}

#[test]
fn test_a_trailing_stop_keeps_its_level_and_high_through_the_roll() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "pct"}}));
    let id = st(&b, "id");
    update_order(&oid, json!({"status": "filled", "filledQty": 25, "avgFill": 165.38}));
    e.tick(None);
    e.tick(q(180.0, None, "OPEN"));
    let b = get_bracket(&id);
    update_order(&latest_stop(&b), json!({"status": "cancelled"}));
    e.tick(q(180.0, None, "OPEN"));
    let b = get_bracket(&id);
    let (level, high, cur) = (n(&b, "slPrice"), n(&b, "highWater"), st(&b, "slOrderId"));
    assert_eq!((level, high), (171.0, 180.0));
    ending_in(&cur, 86400);
    e.clear();
    e.tick(q(180.0, None, "CLOSED"));
    update_order(&cur, json!({"status": "cancelled"}));
    e.tick(q(180.0, None, "CLOSED"));
    let b = get_bracket(&id);
    assert_eq!((n(&b, "slPrice"), n(&b, "highWater")), (level, high));
    assert_eq!(n(e.creates().last().unwrap(), "stopPrice"), level);
    assert!(!st(&b, "slOrderId").is_empty() && st(&b, "slOrderId") != cur);
}

#[test]
fn test_a_placed_target_near_its_ninety_days_is_rolled_too() {
    let (_g, e) = engine();
    let b = armed(&e);
    let id = st(&b, "id");
    let stop = st(&b, "slOrderId");
    e.tick(q(182.0, Some(182.0), "OPEN"));
    update_order(&stop, json!({"status": "cancelled"}));
    e.tick(q(182.0, Some(182.0), "OPEN"));
    let b = get_bracket(&id);
    assert_eq!(st(&b, "status"), "target_placed");
    let tp = st(&b, "tpOrderId");
    ending_in(&tp, 86400);
    e.clear();
    e.tick(q(182.0, Some(182.0), "CLOSED"));
    assert_eq!(e.cancels(), vec![tp.clone()]);
    let b = get_bracket(&id);
    assert_eq!((st(&b, "status"), st(&b, "tpOrderId")), ("target_placed".into(), String::new()), "still the target's turn; not re-armed");
    update_order(&tp, json!({"status": "cancelled"}));
    e.clear();
    e.tick(q(182.0, Some(182.0), "CLOSED"));
    let b = get_bracket(&id);
    assert!(!st(&b, "tpOrderId").is_empty() && st(&b, "tpOrderId") != tp);
    let c = e.creates();
    assert_eq!((st(&c[0], "executionType"), n(&c[0], "limitPrice"), st(&c[0], "timeInForce")), ("LIMIT".into(), 181.94, "UNTIL_CANCEL".into()));
    assert_eq!(st(&b, "status"), "target_placed");
}

// ---------------------------------------------------------------------------
// StopExpiryTest
// ---------------------------------------------------------------------------

#[test]
fn test_exits_go_out_good_till_cancelled_whatever_the_entry_was() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    assert_eq!(st(&get_order(&oid), "tif"), "DAY", "the entry keeps its own time in force");
    assert_eq!(st(&b, "tif"), "UNTIL_CANCEL");
    update_order(&oid, json!({"status": "filled", "filledQty": 25}));
    e.tick(None);
    let c = e.creates();
    let last = c.last().unwrap();
    assert_eq!((st(last, "executionType"), st(last, "timeInForce")), ("STOP".into(), "UNTIL_CANCEL".into()));
    assert_eq!(st(&get_order(&st(&get_bracket(&st(&b, "id")), "slOrderId")), "tif"), "UNTIL_CANCEL");
}

#[test]
fn test_a_stop_that_expires_is_placed_again_good_till_cancelled() {
    let (_g, e) = engine();
    let (oid, b) = e.entry(json!({}));
    let id = st(&b, "id");
    update_order(&oid, json!({"status": "filled", "filledQty": 25}));
    e.tick(None);
    let b = get_bracket(&id);
    let first = st(&b, "slOrderId");
    update_order(&first, json!({"status": "expired", "wsStatus": "EXPIRED", "tif": "DAY"}));
    e.clear();
    e.tick(None);
    let b = get_bracket(&id);
    assert_eq!((st(&b, "status"), st(&b, "slKind"), n(&b, "slPrice")), ("armed".into(), "stop".into(), 157.13), "the leg stays");
    let c = e.creates();
    assert_eq!((st(&c[0], "executionType"), n(&c[0], "stopPrice"), st(&c[0], "timeInForce")), ("STOP".into(), 157.13, "UNTIL_CANCEL".into()));
    assert_ne!(st(&b, "slOrderId"), first);
    update_order(&st(&b, "slOrderId"), json!({"status": "cancelled"}));
    e.tick(None);
    assert_eq!(st(&get_bracket(&id), "status"), "done");
}
