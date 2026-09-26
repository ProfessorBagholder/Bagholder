//! The order ticket, reading orders back, fills pulled, the header's badge, the
//! Orders document, changing an order or a bracket, the ninety-day roll and the
//! bracket's failures, on the book's orders and brackets against the misbehaving
//! fake broker (`tests_execution`). Wealthsimple's reads are the app's own seam
//! (`app.orders.seam.gql`); nothing reaches the network and nothing is placed.
//!
//! What another suite already holds is named here instead of repeated:
//! - a ticket with legs makes a waiting bracket: `tests_execution::a_ticket_with_legs_writes_its_bracket_before_the_entry_goes_out`;
//!   an unfilled entry leaves it waiting: core `order::tests::in_flight_is_every_state_the_broker_may_still_act_on` with `bracket::decide`'s `arm`;
//! - a partial fill arms for what filled: core `tests/brackets.rs::a_fill_arms_the_bracket_for_what_filled_and_places_the_stop_at_once`;
//! - an entry that never filled ends the bracket: core `an_entry_that_ends_unfilled_ends_the_bracket`;
//! - the target cancels the stop and then rests: `tests_execution::the_target_reached_cancels_the_stop_and_a_lapsed_session_leaves_the_stop_watched_until_the_target_goes_out`
//!   and core `the_target_waits_for_the_stops_cancel_to_be_confirmed_and_then_rests_with_the_stop_watched`;
//! - nothing fires while the market is closed (a closed market gives no tape): `tests_execution::a_quote_older_than_fifteen_seconds_by_its_own_time_is_not_acted_on`;
//! - the stop filling ends the bracket: `tests_execution::five_thousand_orders_leave_the_oldest_live_bracket_followed`;
//! - the stop level reached while the limit rests swaps it for a market sell: core `the_stop_level_reached_while_the_limit_rests_cancels_it_then_sells_at_market`;
//! - the limit gives way to the stop out of reach: core `a_percent_under_the_target_the_limit_gives_way_to_the_stop_again`;
//! - an ending is closing until its cancel is confirmed: core `an_ended_bracket_is_closing_until_its_exits_cancel_is_confirmed`;
//! - an exit resting with no bracket holding it is swept: `tests_execution::an_exit_resting_with_no_live_bracket_holding_it_is_cancelled`;
//! - a sale from the ticket waits for the stop's cancel, and part of the shares sold: `tests_execution::a_sale_from_the_ticket_goes_out_only_once_the_stops_cancel_is_confirmed`,
//!   `a_sale_whose_stop_cancel_is_not_confirmed_sells_nothing_and_the_stop_rests_again`, `part_of_the_shares_sold_from_the_ticket_leaves_the_stop_on_the_rest`;
//! - a watched stop fires as a market sell: core `a_watched_stop_fires_a_market_sell_on_the_bid_and_its_fill_ends_the_bracket`, `tests_execution::a_market_sell_rejected_after_it_was_taken_puts_the_stop_back`;
//! - a trailing stop follows the high by cancel and replace, and moves at half a percent: core `a_trailing_stop_follows_the_high_by_half_a_percent_or_more_by_cancel_and_new_order`;
//! - the balances never end a bracket with a resting order: core `a_position_closed_elsewhere_ends_only_a_bracket_with_nothing_resting`;
//! - with orders off nothing is placed and no bracket waits: `tests_execution::with_orders_off_a_ticket_with_legs_is_recorded_and_no_bracket_waits`;
//! - exits go out good till cancelled whatever the entry: `tests_execution::a_fill_arms_the_bracket_and_the_stop_goes_to_the_broker_at_once`;
//! - a stop Wealthsimple ends is placed again good till cancelled: `tests_execution::a_partly_filled_exit_that_expires_is_placed_again_for_the_rest_good_till_cancelled`;
//! - a sent order's answer lost, and its read-back: `tests_execution::an_order_whose_answer_is_lost_after_the_broker_took_it_ends_working_by_read_back_never_failed`;
//! - orders survive clearing any other kind: `clear::tests::each_kind_alone_clears_its_own_and_nothing_else`;
//! - the preview's arithmetic at a given rate: `orders::preview::tests::the_value_in_cad_is_at_the_currencys_rate_and_waits_without_one`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bagholder_core::bracket::{Phase, StopLeg, Tape, Trail};
use bagholder_core::order::{Asker, BrokerStatus, OrderEvent, OrderKind, OrderRole, OrderState, Side};
use bagholder_core::Dec;
use bagholder_ws::session::{CallError, Session};
use jiff::{SignedDuration, Timestamp};
use serde_json::{json, Value};

use crate::app::App;
use crate::orders::gate::{self, Found, OrderBroker, Sent};
use crate::orders::{self as o, PageDec, Quoted, Ticket};
use crate::tests_execution::{d, fresh, order, Behaviour, FakeBroker, FakeOrder, World};
use crate::wire::Fig;

fn sess(v: Value) -> Session {
    serde_json::from_value(v).unwrap()
}

fn tok() -> Session {
    sess(json!({"access_token": "t"}))
}

/// A session that names the identity the pending-order feed is read for.
fn ident() -> Session {
    sess(json!({"access_token": "t", "identity_canonical_id": "ident-1"}))
}

fn set_session(app: &App, s: Option<Session>) {
    *app.orders.seam.session.lock().unwrap() = s;
}

fn set_gql<F: Fn(&str, &Value) -> Result<Value, CallError> + Send + Sync + 'static>(app: &App, f: F) {
    *app.orders.seam.gql.lock().unwrap() = Some(Arc::new(f));
}

fn jv(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap()
}

fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(24 * n)
}

fn pd(s: &str) -> PageDec {
    PageDec(Ok(Some(d(s))))
}

fn no() -> PageDec {
    PageDec(Ok(None))
}

/// An app of its own holding the ticket's accounts, with orders live against the fake.
fn ticket_app() -> (tempfile::TempDir, Arc<App>, Arc<FakeBroker>) {
    let (h, app, fake) = fresh();
    crate::tests_common::order_accounts_in(&app);
    (h, app, fake)
}

/// The ticket as the page sends it, with `over` in place of its fields.
fn ticket_json(over: Value) -> Value {
    let mut body = json!({"symbol": "QNC", "securityId": "sec-s-us", "accountId": "acct-margin", "side": "BUY", "type": "LIMIT", "tif": "DAY",
        "quantity": 25, "limitPrice": 165.4, "stopPrice": null, "currency": "USD",
        "stopLoss": {"kind": "stop", "price": 157.13}, "takeProfit": {"price": 181.94}});
    for (k, v) in over.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

fn ticket(over: Value) -> Ticket {
    serde_json::from_value(ticket_json(over)).unwrap()
}

/// A bracket armed on a fill of 10 at 100, on the wall clock (the Orders panel's
/// actions record at the time they are taken).
fn armed_now(stop: Option<StopLeg>, target: Option<&str>) -> World {
    let mut w = World::new();
    w.now = Timestamp::now();
    let entry = w.bracket("b1", "10", stop, target);
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    w.settle();
    assert_ne!(w.phase("b1"), Phase::Waiting, "armed");
    w
}

fn trailing(pct: &str) -> Option<StopLeg> {
    Some(StopLeg { level: d("95"), trail: Some(Trail::Pct(d(pct))), high: None })
}

/// The exits the fake was sent (entries and ticket sales left out).
fn exit_creates(w: &World) -> Vec<String> {
    w.fake.0.lock().unwrap().creates.iter().filter(|c| !c.ends_with("-entry")).cloned().collect()
}

fn cancels(w: &World) -> Vec<String> {
    w.fake.0.lock().unwrap().cancels.clone()
}

fn fake_order<R>(w: &World, id: &str, f: impl FnOnce(&mut FakeOrder) -> R) -> R {
    let mut s = w.fake.0.lock().unwrap();
    f(s.orders.get_mut(id).unwrap_or_else(|| panic!("the fake has no order {id}")))
}

fn feed(edges: Vec<Value>) -> Value {
    let edges: Vec<Value> = edges.into_iter().enumerate().map(|(i, n)| json!({"cursor": format!("c{i}"), "node": n})).collect();
    json!({"identity": {"id": "ident-1", "orderServiceExtendedOrderFeed": {"edges": edges, "pageInfo": {"hasNextPage": false, "endCursor": null}}}})
}

// ---------------------------------------------------------------------------
// the ticket
// ---------------------------------------------------------------------------

#[test]
fn the_ticket_offers_the_open_self_directed_accounts_that_trade_securities() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    let accounts = o::order_accounts(&app).unwrap();
    let ids: Vec<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["acct-tfsa", "acct-margin"], "crypto, managed and closed accounts are not offered; by name (TFSA, Trading)");
    let m: HashMap<&str, &o::OrderAccount> = accounts.iter().map(|a| (a.id.as_str(), a)).collect();
    assert!(m["acct-margin"].margin && !m["acct-tfsa"].margin);
    assert_eq!((m["acct-margin"].margin_account_id.as_str(), m["acct-tfsa"].margin_account_id.as_str()), ("acct-margin", "acct-margin"), "the TFSA backs the margin account");
}

#[test]
fn a_listing_resolves_from_the_book_by_its_id_or_the_symbol_it_is_known_by() {
    let _g = crate::tests_common::guard();
    let a = crate::tests_common::app();
    let resolve = |sym: &str, sid: &str| o::resolve_security(&a, sym, sid).unwrap();
    // a holding of the recorded month, as the book names it
    let f = a.figures.get().unwrap();
    let names = f.names().unwrap();
    let (sid, symbol, currency) = f
        .read(|e| {
            let p = e.figures().positions.iter().find(|p| p.kind == bagholder_core::instrument::InstrumentKind::Security).unwrap();
            let info = &e.inputs().ledger.instruments[&p.instrument];
            (names.security[&p.instrument].clone(), info.current_name().unwrap().symbol.clone(), info.instrument.currency.as_str().to_string())
        })
        .unwrap();
    let by_symbol = resolve(&symbol.to_lowercase(), "").unwrap();
    assert_eq!((by_symbol.id.clone(), by_symbol.symbol.clone(), by_symbol.currency.clone()), (sid.clone(), symbol.clone(), currency), "the book's record of it, whatever the case typed");
    assert_eq!(resolve("", &sid).unwrap().symbol, symbol, "by the broker's id");
    assert!(resolve("NOPE", "").is_none());
    assert_eq!(resolve("NVDA", "sec-nvda").unwrap().id, "sec-nvda", "an id the book does not hold is taken as given");
}

#[test]
fn the_quote_card_is_read_from_wealthsimples_summary() {
    let node = json!({"id": "sec-s-us", "buyable": true, "sellable": true, "wsTradeEligible": true, "securityType": "EQUITY", "currency": "USD", "status": "ACTIVE",
        "stock": {"name": "NVIDIA Corp", "symbol": "NVDA", "primaryExchange": "NASDAQ", "primaryMic": "XNAS"},
        "quoteV2": {"__typename": "EquityQuote", "ask": 165.42, "bid": 165.38, "currency": "USD", "price": 165.40, "previousBaseline": 163.42,
                    "marketStatus": "OPEN", "askSize": 300, "bidSize": 100, "mid": 165.40, "quotedAsOf": "2026-09-10T15:30:00Z"}});
    let q = o::parse_quote(&serde_json::from_value(node).unwrap()).unwrap();
    assert_eq!((q.symbol.as_str(), q.exchange.as_str(), q.currency.as_str(), q.name.as_str()), ("NVDA", "NASDAQ", "USD", "NVIDIA Corp"));
    assert_eq!((q.last, q.bid, q.ask, q.bid_size, q.ask_size, q.mid), (Some(165.40), Some(165.38), Some(165.42), Some(100.0), Some(300.0), Some(165.40)));
    assert!((q.change.unwrap() - 1.98).abs() < 1e-6, "against Wealthsimple's previous baseline");
    assert!((q.change_pct.unwrap() - 1.98 / 163.42).abs() < 1e-9);
    assert_eq!((q.market_status.as_str(), q.quoted_as_of.as_str()), ("OPEN", "2026-09-10T15:30:00Z"));
    assert!(o::parse_quote(&serde_json::from_value(json!({"stock": {}})).unwrap()).is_none(), "no id, no quote");
    let md = o::parse_market_data(&serde_json::from_value(json!({"security": {"allowedOrderSubtypes": ["LIMIT", "FRACTIONAL", "MARKET"], "marginRates": {"clientMarginRate": 30}}})).unwrap());
    assert_eq!(md.order_types, ["MARKET", "LIMIT"], "only the ticket's types, in the ticket's order");
    assert!((md.margin_rate.unwrap() - 0.30).abs() < 1e-9, "a percentage becomes a fraction");
    let bp = o::parse_buying_power(&serde_json::from_value(json!({"account": {"financials": {"current": {"tradingBalanceViewV2": {"buyingPower": {"quantity": 12680.45, "currency": "USD"}, "cash": {"quantity": 3420.18, "currency": "USD"}}}}}})).unwrap());
    assert_eq!((bp.buying_power, bp.cash, bp.currency.as_str()), (Some(12680.45), Some(3420.18), "USD"));
}

fn search_answer() -> Value {
    json!({"securitySearch": {"results": [
        {"id": "sec-s-bbai", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "EQUITY", "wsTradeEligible": true, "stock": {"symbol": "BBAI", "name": "BigBear.ai Holdings Inc", "primaryExchange": "NYSE", "primaryMic": "XNYS"}},
        {"id": "sec-s-baig", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "EXCHANGE_TRADED_FUND", "wsTradeEligible": true, "stock": {"symbol": "BAIG", "name": "2X Long Bbai Daily ETF", "primaryExchange": "NASDAQ", "primaryMic": "XNAS"}},
        {"id": "sec-s-qnc-ca", "buyable": true, "status": "TRADING", "currency": "CAD", "securityType": "EQUITY", "wsTradeEligible": true, "stock": {"symbol": "QNC.TO", "name": "Quantum Emotion Corp", "primaryExchange": "TSX-V", "primaryMic": "XTSX"}},
        {"id": "sec-o-qnc", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "OPTION", "stock": {"symbol": "QNC", "name": "", "primaryExchange": "NYSE"}},
        {"id": "sec-s-never", "buyable": true, "status": "TRADING", "currency": "USD", "securityType": "EQUITY", "wsTradeEligible": true, "stock": {"symbol": "NEVERHELD", "name": "Never Held Inc", "primaryExchange": "NYSE", "primaryMic": "XNYS"}},
    ]}})
}

#[test]
fn listing_search_picks_the_symbol_on_its_exchange() {
    let a = search_answer();
    let pick = |sym: &str, ex: &str| o::parse_listing_search(&serde_json::from_value(a.clone()).unwrap(), sym, ex);
    assert_eq!(pick("BBAI", "NYSE").unwrap().id, "sec-s-bbai");
    assert_eq!(pick("bbai", "nyse").unwrap().currency, "USD");
    let ca = pick("QNC", "TSX-V").unwrap();
    assert_eq!((ca.id.as_str(), ca.symbol.as_str()), ("sec-s-qnc-ca", "QNC.TO"), "a Canadian suffix on either side is ignored");
    assert!(pick("QNC", "NYSE").is_none(), "the NYSE result is an option, not the share");
    assert!(pick("BBAI", "NASDAQ").is_none());
}

fn summary(id: &str, symbol: &str) -> Value {
    json!({"securities": [{"id": id, "buyable": true, "sellable": true, "wsTradeEligible": true, "securityType": "EQUITY", "currency": "USD",
        "stock": {"name": "Quantum Emotion Corp", "symbol": symbol, "primaryExchange": "NYSE"},
        "quoteV2": {"__typename": "EquityQuote", "ask": 3.02, "bid": 3.0, "currency": "USD", "price": 3.01, "previousBaseline": 2.9, "marketStatus": "OPEN", "askSize": 5, "bidSize": 7}}]})
}

fn buying_power(bp: f64, cash: f64) -> Value {
    json!({"account": {"financials": {"current": {"tradingBalanceViewV2": {"buyingPower": {"quantity": bp, "currency": "USD"}, "cash": {"quantity": cash, "currency": "USD"}}}}}})
}

#[test]
fn a_ticket_on_a_symbol_the_book_never_held_asks_wealthsimples_search_once_and_keeps_the_listing() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    let searches: Arc<Mutex<Vec<Value>>> = Arc::default();
    let s2 = searches.clone();
    set_gql(&app, move |op, vars| match op {
        "FetchSecuritySearchResult" => {
            s2.lock().unwrap().push(vars["query"].clone());
            Ok(search_answer())
        }
        "FetchSecuritiesSummary" => {
            assert_eq!(vars["ids"], json!(["sec-s-never"]), "the quote is asked for the id the search gave");
            Ok(summary("sec-s-never", "NEVERHELD"))
        }
        "FetchSecurityMarketData" => Ok(json!({"security": {"id": "sec-s-never", "allowedOrderSubtypes": ["MARKET", "LIMIT"], "marginRates": {"clientMarginRate": 0.5}}})),
        "FetchTradingBalanceBuyingPower" => Ok(buying_power(9000.0, 100.0)),
        _ => panic!("{op}"),
    });
    set_session(&app, Some(tok()));
    let r = jv(&o::ticket_quote(&app, "NEVERHELD", "", "acct-margin", "NYSE"));
    assert_eq!((r["ok"].clone(), r["quote"]["securityId"].clone()), (json!(true), json!("sec-s-never")), "{r}");
    let again = jv(&o::ticket_quote(&app, "NEVERHELD", "", "acct-margin", "NYSE"));
    assert_eq!(again["ok"], json!(true), "{again}");
    assert_eq!(*searches.lock().unwrap(), vec![json!("NEVERHELD")], "Wealthsimple's search is asked once");
    assert_eq!(o::resolve_security(&app, "NEVERHELD", "").unwrap().expect("known while the app runs").id, "sec-s-never");
}

#[test]
fn a_quote_for_a_listing_wealthsimple_lacks_says_so() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    set_session(&app, Some(tok()));
    set_gql(&app, |_, _| Ok(json!({"securitySearch": {"results": []}})));
    let r = jv(&o::ticket_quote(&app, "NEWCO", "", "acct-margin", "NYSE"));
    assert_eq!((r["ok"].clone(), r["error"].clone()), (json!(false), json!("No listing stored for NEWCO.")), "{r}");
    set_gql(&app, |op, _| panic!("no call: {op}"));
    assert_eq!(jv(&o::ticket_quote(&app, "NEWCO", "", "acct-margin", ""))["error"], json!("No listing stored for NEWCO."), "no exchange named: nothing to search on");
    // a listing with no quote at Wealthsimple
    set_gql(&app, |_, _| Ok(json!({"securities": []})));
    assert_eq!(jv(&o::ticket_quote(&app, "QNC", "sec-s-us", "acct-margin", ""))["error"], json!("Wealthsimple has no quote for QNC."));
}

#[test]
fn a_quote_on_a_collateral_account_carries_the_margin_of_the_account_it_backs() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    set_gql(&app, |op, vars| match op {
        "FetchSecuritiesSummary" => Ok(summary("sec-s-us", "QNC")),
        "FetchSecurityMarketData" => Ok(json!({"security": {"id": "sec-s-us", "allowedOrderSubtypes": ["MARKET", "LIMIT"], "marginRates": {"clientMarginRate": 0.5}}})),
        "FetchTradingBalanceBuyingPower" => {
            assert_eq!(vars["accountCanonicalId"], json!("acct-tfsa"), "cash and buying power are the TFSA's own");
            Ok(buying_power(500.0, 500.0))
        }
        _ => panic!("{op}"),
    });
    set_session(&app, Some(tok()));
    let r = jv(&o::ticket_quote(&app, "QNC", "sec-s-us", "acct-tfsa", ""));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!((r["cash"].as_f64(), r["marginAvailable"].as_f64()), (Some(500.0), Some(12680.45)), "the margin of the account the TFSA backs");
    assert_eq!(r["account"]["marginAccountId"], json!("acct-margin"));
}

#[test]
fn the_quote_answers_with_everything_the_ticket_shows() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    set_gql(&app, |op, vars| match op {
        "FetchSecuritiesSummary" => {
            assert_eq!(vars["ids"], json!(["sec-s-us"]));
            Ok(summary("sec-s-us", "QNC"))
        }
        "FetchSecurityMarketData" => Ok(json!({"security": {"id": "sec-s-us", "allowedOrderSubtypes": ["MARKET", "LIMIT", "STOP_LIMIT"], "marginRates": {"clientMarginRate": 0.5}}})),
        "FetchTradingBalanceBuyingPower" => {
            assert_eq!((&vars["accountCanonicalId"], &vars["currency"], &vars["securityId"]), (&json!("acct-margin"), &json!("USD"), &json!("sec-s-us")));
            Ok(buying_power(9000.0, 100.0))
        }
        _ => panic!("{op}"),
    });
    set_session(&app, Some(tok()));
    app.set_orders_live(true);
    let r = jv(&o::ticket_quote(&app, "QNC", "sec-s-us", "acct-margin", ""));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["quote"]["symbol"], json!("QNC"));
    assert_eq!(r["orderTypes"], json!(["MARKET", "LIMIT", "STOP_LIMIT"]));
    assert_eq!((r["marginRate"].as_f64(), r["marginAvailable"].as_f64()), (Some(0.5), Some(12680.45)));
    assert_eq!((r["buyingPower"].as_f64(), r["cash"].as_f64()), (Some(9000.0), Some(100.0)));
    let names: Vec<&str> = r["accounts"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["TFSA", "Trading"], "the accounts the ticket offers, by name");
    assert_eq!((r["account"]["id"].clone(), r["live"].clone()), (json!("acct-margin"), json!(true)));
    set_session(&app, None);
    assert_eq!(jv(&o::ticket_quote(&app, "QNC", "sec-s-us", "acct-margin", ""))["error"], json!("Not connected."));
    set_session(&app, Some(tok()));
    set_gql(&app, |_, _| Err(CallError::NotAuthorized));
    assert_eq!(jv(&o::ticket_quote(&app, "QNC", "sec-s-us", "acct-margin", ""))["error"], json!("Wealthsimple refused the session. Connect Wealthsimple again."));
    assert_eq!(jv(&o::ticket_quote(&app, "NOPE", "", "acct-margin", ""))["error"], json!("No listing stored for NOPE."));
}

/// The request as Wealthsimple's web app sends it, its external id left out.
fn sent_as(req: &Value) -> Value {
    let mut rest = req.clone();
    let id = rest.as_object_mut().unwrap().remove("externalId").expect("an external id");
    assert!(id.as_str().unwrap().starts_with("order-"), "{id}");
    rest
}

#[test]
fn the_ticket_asks_for_the_order_wealthsimples_web_app_sends() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    let t = o::ticket_order(&app, &ticket(json!({}))).unwrap();
    assert_eq!(sent_as(&t.order.request), json!({"canonicalAccountId": "acct-margin", "executionType": "LIMIT", "orderType": "BUY_QUANTITY", "quantity": 25.0, "securityId": "sec-s-us", "timeInForce": "DAY", "limitPrice": 165.4}));
    assert_eq!(t.order.request["externalId"].as_str(), Some(t.order.id.as_str()), "the id the app keeps is the one sent");
    assert_eq!((t.order.side, t.order.kind, t.order.quantity, t.order.limit_price, t.order.stop_price), (Side::Buy, OrderKind::Limit, d("25"), Some(d("165.4")), None));
    assert_eq!((t.order.symbol.as_str(), t.order.currency.as_str(), t.order.broker_account.as_str()), ("QNC", "USD", "acct-margin"));
    assert_eq!((t.stop, t.target), (Some(StopLeg { level: d("157.13"), trail: None, high: None }), Some(d("181.94"))));
    let t = o::ticket_order(&app, &ticket(json!({"type": "MARKET", "tif": "UNTIL_CANCEL"}))).unwrap();
    assert!(t.order.request.get("limitPrice").is_none() && t.order.request.get("stopPrice").is_none());
    assert_eq!(t.order.request["timeInForce"], json!("UNTIL_CANCEL"));
    let t = o::ticket_order(&app, &ticket(json!({"type": "STOP_LIMIT", "stopPrice": 170.0}))).unwrap();
    assert_eq!((&t.order.request["executionType"], t.order.request["stopPrice"].as_f64(), t.order.request["limitPrice"].as_f64()), (&json!("STOP_LIMIT"), Some(170.0), Some(165.4)));
    let t = o::ticket_order(&app, &ticket(json!({"side": "SELL", "type": "STOP", "stopPrice": 150.0}))).unwrap();
    assert_eq!((&t.order.request["executionType"], &t.order.request["orderType"], t.order.request["stopPrice"].as_f64()), (&json!("STOP"), &json!("SELL_QUANTITY"), Some(150.0)));
    assert_eq!((t.stop, t.target), (None, None), "a sell has nothing to protect");
    // a trailing stop starts under the entry's price
    let t = o::ticket_order(&app, &ticket(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "pct"}}))).unwrap();
    assert_eq!(t.stop, Some(StopLeg { level: d("157.13"), trail: Some(Trail::Pct(d("5"))), high: None }));
    let t = o::ticket_order(&app, &ticket(json!({"stopLoss": {"kind": "trail", "trail": "2.5", "trailUnit": "amt"}}))).unwrap();
    assert_eq!(t.stop, Some(StopLeg { level: d("162.90"), trail: Some(Trail::Amount(d("2.5"))), high: None }));
}

#[test]
fn prices_go_out_at_the_orders_tick_two_decimals_from_a_dollar_four_under() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = ticket_app();
    let t = o::ticket_order(&app, &ticket(json!({"limitPrice": 1.736, "stopLoss": {"kind": "stop", "price": 1.6512}, "takeProfit": {"price": 1.9139}}))).unwrap();
    assert_eq!((t.order.limit_price, t.order.request["limitPrice"].as_f64()), (Some(d("1.74")), Some(1.74)));
    assert_eq!((t.stop.unwrap().level, t.target), (d("1.65"), Some(d("1.91"))));
    let t = o::ticket_order(&app, &ticket(json!({"type": "STOP_LIMIT", "limitPrice": 0.98765, "stopPrice": "1.005"}))).unwrap();
    assert_eq!((t.order.request["limitPrice"].as_f64(), t.order.request["stopPrice"].as_f64()), (Some(0.9877), Some(1.01)), "exact decimals, rounded half up");
    let t = o::ticket_order(&app, &ticket(json!({"limitPrice": "0.25371", "stopLoss": null, "takeProfit": null}))).unwrap();
    assert_eq!(t.order.limit_price, Some(d("0.2537")));
}

#[test]
fn a_bad_ticket_is_refused_in_words_and_nothing_is_recorded() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = ticket_app();
    let bad = |over: Value| o::ticket_order(&app, &ticket(over)).err().unwrap_or_else(|| panic!("taken"));
    assert_eq!(bad(json!({"quantity": 0})), "Quantity must be more than zero.");
    assert_eq!(bad(json!({"quantity": null})), "Quantity must be more than zero.");
    assert_eq!(bad(json!({"quantity": "abc"})), "Quantity \"abc\" is not a number.", "text that is not a number");
    assert_eq!(bad(json!({"quantity": true})), "Quantity \"true\" is not a number.");
    assert_eq!(bad(json!({"quantity": {"n": 25}})), "Quantity \"{\\\"n\\\":25}\" is not a number.");
    assert_eq!(bad(json!({"limitPrice": null})), "A limit price is required.");
    assert_eq!(bad(json!({"limitPrice": "1.2.3"})), "The limit price \"1.2.3\" is not a number.");
    assert_eq!(bad(json!({"type": "STOP", "stopPrice": null})), "A stop price is required.");
    assert_eq!(bad(json!({"type": "STOP", "stopPrice": "high"})), "The stop price \"high\" is not a number.");
    for account in ["acct-crypto", "acct-managed", "acct-old", "nope"] {
        assert_eq!(bad(json!({"accountId": account})), "Choose an account.", "{account}");
    }
    assert_eq!(bad(json!({"symbol": "NOPE", "securityId": ""})), "No listing stored for NOPE.");
    assert_eq!(bad(json!({"side": "HOLD"})), "Side must be Buy or Sell.");
    assert_eq!(bad(json!({"type": "TRAILING"})), "Order type must be Market, Limit, Stop or Stop limit.");
    assert_eq!(bad(json!({"tif": "WEEK"})), "Time in force must be Day or Good till cancelled.");
    assert_eq!(bad(json!({"stopLoss": {"kind": "stop", "price": 0}})), "A stop loss price is required.");
    assert_eq!(bad(json!({"stopLoss": {}})), "A stop loss price is required.", "a leg asked with no price");
    assert_eq!(bad(json!({"stopLoss": {"kind": "stop", "price": "x"}})), "The stop loss price \"x\" is not a number.");
    assert_eq!(bad(json!({"stopLoss": {"kind": "bogus", "price": 150}})), "Stop loss type must be Stop or Trailing stop.");
    assert_eq!(bad(json!({"stopLoss": {"kind": "trail"}})), "A trail is required.");
    assert_eq!(bad(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "bps"}})), "A trail is a percent or an amount.");
    assert_eq!(bad(json!({"takeProfit": {"price": null}})), "A take profit price is required.");
    assert_eq!(bad(json!({"takeProfit": {"price": "lots"}})), "The take profit price \"lots\" is not a number.");
    assert_eq!(bad(json!({"securityId": "", "symbol": "QNC", "currency": "ZZZZ"})), "No listing stored for QNC.");
    // numbers typed as text are numbers, whatever the case of the words
    let t: Ticket = serde_json::from_value(json!({"side": "buy", "type": "limit", "quantity": "25", "limitPrice": "165.40", "symbol": "QNC", "securityId": "sec-s-us", "accountId": "acct-margin", "currency": "USD", "stopLoss": null, "takeProfit": null})).unwrap();
    let asked = o::ticket_order(&app, &t).expect("numbers typed as text are numbers");
    assert_eq!((asked.order.quantity, asked.order.limit_price, asked.stop, asked.target), (d("25"), Some(d("165.40")), None, None));
    assert_eq!(asked.order.request["orderType"], json!("BUY_QUANTITY"));
    assert!(serde_json::from_value::<Ticket>(json!("not a ticket")).is_err());
    assert!(serde_json::from_value::<Ticket>(ticket_json(json!({"extra": 1}))).is_err(), "a field the ticket does not have");
    // placed, a refused ticket writes nothing and sends nothing
    app.set_orders_live(true);
    set_session(&app, Some(tok()));
    let book = app.figures.get().unwrap().book().unwrap();
    let r = o::place_ticket(&app, &ticket(json!({"quantity": 0})));
    assert_eq!((r.ok, r.error.as_deref(), r.id.as_deref()), (false, Some("Quantity must be more than zero."), None));
    assert!(book.orders_before(None, 10).unwrap().is_empty() && book.live_brackets().unwrap().is_empty(), "nothing recorded");
    assert!(fake.0.lock().unwrap().creates.is_empty());
}

#[test]
fn orders_are_live_unless_the_dry_setting_is_on() {
    let _g = crate::tests_common::guard();
    // this process always runs with BAGHOLDER_DRY_ORDERS=1 (`tests_common::home`)
    let (_h, app, _fake) = fresh();
    assert_eq!(std::env::var("BAGHOLDER_DRY_ORDERS").as_deref(), Ok("1"));
    assert!(!o::orders_live(&app), "BAGHOLDER_DRY_ORDERS=1: not live");
    assert!(!crate::status::status(&app).orders_live);
}

/// Puts the shared app's seams back however the test ends.
struct SharedSeams(Arc<App>);

impl Drop for SharedSeams {
    fn drop(&mut self) {
        *self.0.orders.gate.broker.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.0.set_orders_live(false);
        *self.0.orders.seam.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.0.orders.seam.gql.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[test]
fn a_ticket_placed_on_the_books_accounts_is_written_then_sent_once_and_answered_with_where_it_stands() {
    let _g = crate::tests_common::guard();
    crate::tests_common::order_accounts_in_book();
    let app = crate::tests_common::app();
    let _restore = SharedSeams(app.clone());
    let fake = FakeBroker::install(&app);
    set_session(&app, Some(tok()));
    let book = app.figures.get().unwrap().book().unwrap();
    // dry: recorded, nothing sent, no bracket waits
    app.set_orders_live(false);
    let r = o::place_ticket(&app, &ticket(json!({})));
    assert_eq!((r.ok, r.status.as_deref(), r.bracket_id.as_deref()), (true, Some("dry"), None), "{r:?}");
    let dry = book.order(r.id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((dry.fold.state, dry.request.side, dry.request.kind, dry.request.quantity), (OrderState::Dry, Side::Buy, OrderKind::Limit, d("25")));
    assert_eq!(dry.request.request["executionType"], json!("LIMIT"), "the request it would have sent is kept");
    assert!(fake.0.lock().unwrap().creates.is_empty(), "nothing left the gate");
    // live: the bracket first, then the entry, sent once under the id it was written with
    app.set_orders_live(true);
    let r = o::place_ticket(&app, &ticket(json!({})));
    assert_eq!((r.ok, r.status.as_deref()), (true, Some("pending")), "{r:?}");
    let id = r.id.clone().unwrap();
    let bid = r.bracket_id.clone().expect("the legs make a bracket");
    assert_eq!(fake.0.lock().unwrap().creates, vec![id.clone()]);
    assert_eq!(fake.0.lock().unwrap().orders[&id].request["externalId"], json!(id));
    let sb = book.bracket(&bid).unwrap().unwrap();
    assert_eq!((sb.bracket.phase, sb.bracket.quantity, sb.bracket.stop.map(|s| s.level), sb.bracket.target), (Phase::Waiting, d("25"), Some(d("157.13")), Some(d("181.94"))));
    assert_eq!(book.order(&id).unwrap().unwrap().fold.broker_id.as_deref(), Some("ws-1"), "Wealthsimple's id kept");
    // refused: in Wealthsimple's words, and its bracket ends with it
    fake.then(Behaviour::Refuse("Insufficient funds", Some("ORDER.insufficient_funds")));
    let r2 = o::place_ticket(&app, &ticket(json!({})));
    assert_eq!((r2.ok, r2.error.as_deref(), r2.status.as_deref()), (false, Some("Wealthsimple rejected the order: Insufficient funds"), Some("rejected")), "{r2:?}");
    assert_eq!(book.bracket(r2.bracket_id.as_deref().unwrap()).unwrap().unwrap().bracket.phase, Phase::Ended);
    // never sent: failed
    fake.then(Behaviour::NotSent);
    let r3 = o::place_ticket(&app, &ticket(json!({"stopLoss": null, "takeProfit": null})));
    assert_eq!((r3.ok, r3.error.as_deref(), r3.status.as_deref(), r3.bracket_id.as_deref()), (false, Some("Order failed: Not connected."), Some("failed"), None), "{r3:?}");
    // an answer that was not Wealthsimple's: sent, not confirmed
    fake.then(Behaviour::LoseAfter);
    let r4 = o::place_ticket(&app, &ticket(json!({"stopLoss": null, "takeProfit": null})));
    assert_eq!((r4.ok, r4.status.as_deref()), (true, Some("unconfirmed")), "{r4:?}");
    // taken, and no id named: not confirmed until it is read back
    fake.then(Behaviour::NoId);
    let r5 = o::place_ticket(&app, &ticket(json!({"stopLoss": null, "takeProfit": null})));
    assert_eq!((r5.ok, r5.status.as_deref()), (true, Some("unconfirmed")), "{r5:?}");
    assert_eq!(gate::read_back(&app, &book, r5.id.as_deref().unwrap(), Timestamp::now()).unwrap().state, OrderState::Pending);
    fake.set(r5.id.as_deref().unwrap(), BrokerStatus::Cancelled, "0");
    gate::read_back(&app, &book, r5.id.as_deref().unwrap(), Timestamp::now()).unwrap();
    for r in [&r, &r2, &r3, &r4, &r5] {
        assert!(book.order(r.id.as_deref().unwrap()).unwrap().is_some(), "every attempt is in the book");
    }
    // the entry cancelled at Wealthsimple ends its waiting bracket; nothing is left live
    fake.set(&id, BrokerStatus::Cancelled, "0");
    fake.set(r4.id.as_deref().unwrap(), BrokerStatus::Cancelled, "0");
    for oid in [&id, r4.id.as_ref().unwrap()] {
        gate::read_back(&app, &book, oid, Timestamp::now()).unwrap();
    }
    o::bracket_tick(&app, Some(HashMap::new()), Timestamp::now()).unwrap();
    assert_eq!(book.bracket(&bid).unwrap().unwrap().bracket.phase, Phase::Ended);
    assert!(book.live_brackets().unwrap().iter().all(|b| b.place.id != bid));
    assert_eq!(book.states_disagreeing().unwrap(), Vec::<String>::new());
}

#[test]
fn a_submit_with_no_session_is_written_with_its_failure_and_touches_no_bracket() {
    let _g = crate::tests_common::guard();
    let mut w = armed_now(World::stop("95"), None);
    crate::tests_common::order_accounts_in(&w.app);
    let before = w.bracket_of("b1");
    // Wealthsimple itself, not the fake: with no session nothing leaves the app
    *w.app.orders.gate.broker.lock().unwrap() = None;
    set_session(&w.app, None);
    for side in ["BUY", "SELL"] {
        let r = o::place_ticket(&w.app, &ticket(json!({"side": side, "securityId": "sec", "stopLoss": null, "takeProfit": null})));
        assert_eq!((r.ok, r.error.as_deref(), r.status.as_deref()), (false, Some("Order failed: Not connected."), Some("failed")), "{side}: {r:?}");
        let row = w.book.order(r.id.as_deref().expect("the answer names the order it wrote")).unwrap().unwrap();
        assert_eq!((row.fold.state, row.fold.why.as_deref()), (OrderState::Failed, Some("Not connected.")), "{side}");
    }
    assert_eq!(w.bracket_of("b1"), before, "the bracket on those shares is untouched");
    w.now = Timestamp::now();
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// reading orders back
// ---------------------------------------------------------------------------

/// Answers the pending-order feed with `edges`, and records every read asked.
fn feed_gql(app: &App, edges: Arc<Mutex<Vec<Value>>>, asked: Arc<Mutex<Vec<(String, Value)>>>) {
    set_gql(app, move |op, vars| {
        asked.lock().unwrap().push((op.to_string(), vars.clone()));
        match op {
            "OrderServiceExtendedOrderFeed" => Ok(feed(edges.lock().unwrap().clone())),
            _ => Err(CallError::Failed(format!("{op}: not in this test"))),
        }
    });
}

#[test]
fn every_order_in_flight_is_read_back_by_its_own_id_and_the_pending_feed_with_it() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    set_session(&w.app, Some(ident()));
    let asked: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    feed_gql(&w.app, Arc::default(), asked.clone());
    let now = w.now;
    for id in ["o1", "o2"] {
        gate::place(&w.app, &w.book, &order(id, None), &Asker::Person, now).unwrap().unwrap();
    }
    w.book.write_order(&order("o-dry", None), true, &Asker::Person, now).unwrap();
    w.fake.set("o1", BrokerStatus::Filled, "10");
    let r = o::refresh_orders(&w.app);
    assert_eq!((r.ok, r.read, r.failed), (true, 2, 0), "{r:?}");
    assert_eq!(w.fake.0.lock().unwrap().reads, ["o1", "o2"], "each by the id it was sent with; a dry one never");
    let feeds: Vec<Value> = asked.lock().unwrap().iter().filter(|(op, _)| op == "OrderServiceExtendedOrderFeed").map(|(_, v)| v.clone()).collect();
    assert_eq!(feeds.len(), 1);
    assert_eq!((&feeds[0]["identityId"], &feeds[0]["statuses"]), (&json!("ident-1"), &json!(o::FEED_STATUSES)));
    assert_eq!(w.book.order("o1").unwrap().unwrap().fold.state, OrderState::Filled);
    assert!(w.app.orders.refreshed_at.lock().unwrap().is_some());
    // what has ended is not read again
    w.fake.0.lock().unwrap().reads.clear();
    o::refresh_orders(&w.app);
    assert_eq!(w.fake.0.lock().unwrap().reads, ["o2"]);
    w.state_is_the_log();
}

#[test]
fn wealthsimple_is_asked_for_an_order_by_its_external_id_on_the_tr_branch() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    // Wealthsimple's own read, not the fake's
    *w.app.orders.gate.broker.lock().unwrap() = None;
    set_session(&w.app, Some(ident()));
    let asked: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let a2 = asked.clone();
    set_gql(&w.app, move |op, vars| {
        a2.lock().unwrap().push((op.to_string(), vars.clone()));
        match op {
            "FetchSoOrdersExtendedOrder" => Ok(json!({"soOrdersExtendedOrder": {"status": "FILLED", "filledQuantity": 25, "averageFilledPrice": 165.38, "submittedAtUtc": "2026-09-10T13:30:00Z", "expiredAtUtc": null, "rejectionCause": null, "timeInForce": "DAY", "submittedQuantity": 25, "limitPrice": "165.40"}})),
            "OrderServiceExtendedOrderFeed" => Ok(feed(vec![])),
            _ => Err(CallError::Failed(format!("{op}: not in this test"))),
        }
    });
    let mut req = order("order-1", None);
    req.quantity = d("25");
    w.book.write_order(&req, false, &Asker::Person, w.now).unwrap();
    w.book.order_event("order-1", &Asker::Person, w.now, &OrderEvent::Accepted { broker_id: "ws-1".into() }).unwrap().unwrap();
    let r = o::refresh_orders(&w.app);
    assert_eq!((r.read, r.failed), (1, 0), "{r:?}");
    let reads: Vec<Value> = asked.lock().unwrap().iter().filter(|(op, _)| op == "FetchSoOrdersExtendedOrder").map(|(_, v)| v.clone()).collect();
    assert_eq!(reads, vec![json!({"branchId": "TR", "externalId": "order-1"})]);
    let o1 = w.book.order("order-1").unwrap().unwrap();
    assert_eq!((o1.fold.state, o1.fold.filled, o1.fold.average, o1.stated_quantity, o1.stated_price), (OrderState::Filled, d("25"), Some(d("165.38")), Some(d("25")), Some(d("165.40"))));
    w.state_is_the_log();
}

#[test]
fn a_failed_read_is_counted_changes_nothing_and_the_others_still_happen() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    set_session(&w.app, Some(ident()));
    feed_gql(&w.app, Arc::default(), Arc::default());
    for id in ["o1", "o2"] {
        gate::place(&w.app, &w.book, &order(id, None), &Asker::Person, w.now).unwrap().unwrap();
    }
    w.fake.0.lock().unwrap().read_fails.push("o1".into());
    w.fake.set("o1", BrokerStatus::Filled, "10");
    w.fake.set("o2", BrokerStatus::Filled, "10");
    let r = o::refresh_orders(&w.app);
    assert_eq!((r.ok, r.read, r.failed), (false, 1, 1), "{r:?}");
    assert_eq!(w.book.order("o1").unwrap().unwrap().fold.state, OrderState::Pending, "an unanswered read changes nothing");
    assert_eq!(w.book.order("o2").unwrap().unwrap().fold.state, OrderState::Filled);
    // the feed failing is counted too
    set_gql(&w.app, |_, _| Err(CallError::Failed("boom".into())));
    let r = o::refresh_orders(&w.app);
    assert_eq!((r.ok, r.read, r.failed), (false, 0, 2), "{r:?}");
    w.state_is_the_log();
}

fn placed_elsewhere(id: &str, broker_id: &str, account: &str) -> Value {
    json!({"id": id, "orderId": broker_id, "canonicalAccountId": account, "createdAtUtc": "2026-09-10T01:00:00Z", "status": "SUBMITTED", "side": "BUY", "executionType": "LIMIT",
        "submittedQuantity": 3, "limitPrice": 1.76, "stopPrice": null, "averageFillPrice": null, "securityCurrency": "USD", "securityId": "sec", "symbol": "QNC",
        "security": {"id": "sec", "stock": {"symbol": "QNC", "name": "Quantum Emotion Corp"}}})
}

#[test]
fn an_order_placed_in_wealthsimples_app_is_shown_from_the_feed_and_read_back_once_when_it_leaves() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    // fills are told, and never reach the system's own notifications from a test
    let channel = crate::notify::test_hooks::CHANNEL.lock().unwrap().replace(String::new());
    crate::notify::set_settings(&w.app.open().unwrap(), &serde_json::from_value(json!({"fills": true})).unwrap()).unwrap();
    set_session(&w.app, Some(ident()));
    let edges: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![placed_elsewhere("order-ws-placed", "ws-9", "acct-tfsa")]));
    feed_gql(&w.app, edges.clone(), Arc::default());
    let r = o::refresh_orders(&w.app);
    assert_eq!((r.ok, r.read), (true, 0), "{r:?}");
    let held = w.app.orders.elsewhere.lock().unwrap().clone();
    assert_eq!(held.len(), 1);
    let e = &held[0];
    assert_eq!((e.id.as_str(), e.broker_id.as_deref(), e.account.as_str(), e.symbol.as_str(), e.side, e.kind, e.quantity, e.limit_price, e.state), ("order-ws-placed", Some("ws-9"), "acct-tfsa", "QNC", Side::Buy, OrderKind::Limit, d("3"), Some(d("1.76")), OrderState::Pending));
    assert!(w.book.order("order-ws-placed").unwrap().is_none(), "Wealthsimple's, never the book's");
    let card = o::orders_doc(&w.app).orders.into_iter().find(|c| c.id == "order-ws-placed").expect("a card");
    assert_eq!((card.tab.as_str(), card.tif.clone(), card.live), ("pending", None, true));
    // still listed: kept, not read back, not doubled
    o::refresh_orders(&w.app);
    assert_eq!(w.app.orders.elsewhere.lock().unwrap().len(), 1);
    assert!(w.fake.0.lock().unwrap().reads.is_empty());
    // gone from the feed: read back once by its id, and its fill told
    w.fake.0.lock().unwrap().orders.insert("order-ws-placed".into(), FakeOrder { request: json!({}), status: BrokerStatus::Filled, filled: d("3"), average: Some(d("1.75")), expires_at: None, price: None, quantity: None, why: None, code: None });
    edges.lock().unwrap().clear();
    o::refresh_orders(&w.app);
    let e = w.app.orders.elsewhere.lock().unwrap()[0].clone();
    assert_eq!((e.state, e.filled, e.average, e.ended_at.is_some()), (OrderState::Filled, d("3"), Some(d("1.75")), true));
    let told = bagholder_store::feeds::list_notifications(&w.app.open().unwrap(), 0, "", false, 100, true).unwrap();
    *crate::notify::test_hooks::CHANNEL.lock().unwrap() = channel;
    assert!(told.iter().any(|n| n.key == "order:order-ws-placed:filled" && n.title == "Order filled · QNC"), "{:?}", told.iter().map(|n| &n.key).collect::<Vec<_>>());
    o::refresh_orders(&w.app);
    assert_eq!(w.fake.0.lock().unwrap().reads, ["order-ws-placed"], "read back once");
    let card = o::orders_doc(&w.app).orders.into_iter().find(|c| c.id == "order-ws-placed").unwrap();
    assert_eq!(card.tab, "filled");
}

#[test]
fn the_apps_own_order_is_never_doubled_from_the_feed_whichever_id_matches() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    set_session(&w.app, Some(ident()));
    gate::place(&w.app, &w.book, &order("order-mine", None), &Asker::Person, w.now).unwrap().unwrap();
    // by Wealthsimple's id, and by the app's own
    let edges = vec![placed_elsewhere("order-some-other-id", "ws-1", "acct"), placed_elsewhere("order-mine", "ws-other", "acct")];
    feed_gql(&w.app, Arc::new(Mutex::new(edges)), Arc::default());
    let r = o::refresh_orders(&w.app);
    assert!(r.ok, "{r:?}");
    assert!(w.app.orders.elsewhere.lock().unwrap().is_empty(), "{:?}", w.app.orders.elsewhere.lock().unwrap());
    assert_eq!(o::orders_doc(&w.app).orders.iter().filter(|c| c.id == "order-mine").count(), 1);
    w.state_is_the_log();
}

#[test]
fn an_order_from_the_feed_is_named_as_the_book_names_its_listing() {
    let _g = crate::tests_common::guard();
    let a = crate::tests_common::app();
    let f = a.figures.get().unwrap();
    let names = f.names().unwrap();
    let (sid, symbol) = f
        .read(|e| {
            let p = e.figures().positions.iter().find(|p| p.kind == bagholder_core::instrument::InstrumentKind::Security).unwrap();
            (names.security[&p.instrument].clone(), e.inputs().ledger.instruments[&p.instrument].current_name().unwrap().symbol.clone())
        })
        .unwrap();
    let e = o::Elsewhere {
        id: "order-from-feed".into(),
        broker_id: Some("ws-77".into()),
        account: "acct-tfsa".into(),
        security: sid,
        symbol: "FEEDS-OWN-WORD".into(),
        currency: "USD".into(),
        side: Side::Sell,
        kind: OrderKind::Limit,
        quantity: d("40"),
        limit_price: Some(d("0.25")),
        stop_price: None,
        state: OrderState::Pending,
        filled: Dec::ZERO,
        average: None,
        created_at: Timestamp::now(),
        ended_at: None,
    };
    a.orders.elsewhere.lock().unwrap().push(e);
    let card = o::orders_doc(&a).orders.into_iter().find(|c| c.id == "order-from-feed");
    a.orders.elsewhere.lock().unwrap().retain(|x| x.id != "order-from-feed");
    let card = card.expect("a card");
    assert_eq!((card.symbol.as_str(), card.side.as_str(), card.quantity.0, card.limit_price.map(|p| p.0)), (symbol.as_str(), "sell", d("40"), Some(d("0.25"))), "the book's name for the listing");
    assert_eq!(card.value.map(|v| match v {
        Fig::Stated(t) => t.0,
        Fig::Waits { gaps } => panic!("{gaps:?}"),
    }), Some(d("10")), "40 at 0.25, of a share each");
}

#[test]
fn opening_the_orders_panel_asks_a_read_only_when_the_last_is_older_than_a_tick() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = fresh();
    let asked = || app.orders.read_asked.swap(false, std::sync::atomic::Ordering::SeqCst);
    o::kick_orders_refresh(&app);
    assert!(asked(), "never read: asked");
    *app.orders.refreshed_at.lock().unwrap() = Some(Timestamp::now());
    o::kick_orders_refresh(&app);
    assert!(!asked(), "a read younger than the loop's tick is fresh enough");
    *app.orders.refreshed_at.lock().unwrap() = Some(Timestamp::now() - SignedDuration::from_secs(o::ORDERS_REFRESH_SEC as i64 + 1));
    o::kick_orders_refresh(&app);
    assert!(asked());
}

/// A broker whose create is overtaken by a read-back of every order in flight, as
/// the orders loop may run while the ticket is sending.
struct ReadWhileSending(Arc<FakeBroker>);

impl OrderBroker for ReadWhileSending {
    fn create(&self, app: &Arc<App>, request: &Value) -> Sent {
        o::refresh_orders(app);
        self.0.create(app, request)
    }
    fn cancel(&self, app: &Arc<App>, id: &str) -> Sent {
        self.0.cancel(app, id)
    }
    fn modify(&self, app: &Arc<App>, id: &str, change: &Value) -> Sent {
        self.0.modify(app, id, change)
    }
    fn read(&self, app: &Arc<App>, id: &str) -> Result<Found, String> {
        self.0.read(app, id)
    }
}

#[test]
fn an_order_this_run_is_still_sending_is_not_read_back_until_its_answer_is_in() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    *w.app.orders.gate.broker.lock().unwrap() = Some(Arc::new(ReadWhileSending(w.fake.clone())));
    let got = gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, w.now).unwrap().unwrap();
    assert_eq!((got.state, got.broker_id.as_deref()), (OrderState::Pending, Some("ws-1")), "taken by Wealthsimple, never failed as unknown to it");
    assert!(!w.fake.0.lock().unwrap().reads.contains(&"o1".to_string()), "not read while it was being sent");
    w.state_is_the_log();
}

#[test]
fn an_order_a_stopped_run_left_sending_is_read_back_and_wealthsimple_decides_it() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    set_session(&w.app, Some(ident()));
    feed_gql(&w.app, Arc::default(), Arc::default());
    // written, and the app stopped before any answer: a bracket waits on one of them
    let place = bagholder_book::orders::BracketPlace { id: "b-held".into(), broker: "wealthsimple".into(), broker_account: "acct".into(), broker_security: "sec".into(), symbol: "SHOP".into(), currency: bagholder_core::Currency::parse("USD").unwrap() };
    w.book.write_bracket(&place, &bagholder_core::bracket::BracketEvent::Created { quantity: d("10"), stop: World::stop("95"), target: None }, &Asker::Person, w.now).unwrap();
    let mut held = order("o-held", Some(("b-held", OrderRole::Entry)));
    held.request = json!({"externalId": "o-held"});
    for o in [held, order("o-unknown", None), order("o-unread", None)] {
        w.book.write_order(&o, false, &Asker::Person, w.now).unwrap();
    }
    assert_eq!(o::open_orders_count(&w.app, None), 3, "an order being sent is a Pending card");
    w.fake.0.lock().unwrap().orders.insert("o-held".into(), FakeOrder { request: json!({"externalId": "o-held"}), status: BrokerStatus::Open, filled: Dec::ZERO, average: None, expires_at: None, price: None, quantity: None, why: None, code: None });
    w.fake.0.lock().unwrap().read_fails.push("o-unread".into());
    let r = o::refresh_orders(&w.app);
    assert_eq!((r.read, r.failed), (2, 1), "{r:?}");
    assert_eq!(w.book.order("o-held").unwrap().unwrap().fold.state, OrderState::Pending, "as Wealthsimple has it");
    let unknown = w.book.order("o-unknown").unwrap().unwrap();
    assert_eq!((unknown.fold.state, unknown.fold.why.as_deref()), (OrderState::Failed, Some("the broker has no record of it")));
    assert_eq!(w.book.order("o-unread").unwrap().unwrap().fold.state, OrderState::Sending, "unread: decided on a later read");
    w.tick();
    assert_eq!(w.phase("b-held"), Phase::Waiting, "the entry works at Wealthsimple: its bracket waits on it");
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// fills reach the book
// ---------------------------------------------------------------------------

#[test]
fn a_filled_quantity_asks_for_wealthsimples_own_row_once_per_rise() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    let pulled = || w.app.pull_asked.swap(false, std::sync::atomic::Ordering::SeqCst);
    gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, w.now).unwrap().unwrap();
    pulled();
    w.fake.set("o1", BrokerStatus::Open, "4");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert!(pulled(), "part filled: pulled as Wealthsimple records it");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert!(!pulled(), "the same fill read again asks nothing more");
    w.fake.set("o1", BrokerStatus::Filled, "10");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert!(pulled(), "what filled beyond is asked for");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert!(!pulled());
    // no trade of the app's own making: the book's fills are Wealthsimple's rows
    assert_eq!(w.book.order("o1").unwrap().unwrap().fold.filled, d("10"), "the filled quantity kept on the order");
    w.state_is_the_log();
}

#[test]
fn a_fill_is_pulled_for_until_the_book_holds_wealthsimples_row_soon_at_first_then_less_often() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    crate::tests_common::order_accounts_in(&w.app);
    let conn = w.book.connections().unwrap().into_iter().find(|c| c.broker == bagholder_core::Broker::named("wealthsimple")).unwrap().id;
    gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, w.now).unwrap().unwrap();
    assert_eq!(o::own_fills_booked(&w.app).unwrap(), Vec::<String>::new(), "nothing filled yet");
    w.fake.set("o1", BrokerStatus::Filled, "10");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert_eq!(o::own_fills_booked(&w.app).unwrap(), ["ws-1"], "by Wealthsimple's id for the order");
    w.app.fill_waits.lock().unwrap().clear();
    let now: Timestamp = "2026-09-25T15:00:00Z".parse().unwrap();
    let last = now - SignedDuration::from_secs(3);
    // Wealthsimple has not listed the fill: the next pull is due soon after the last
    assert_eq!(crate::broker_reads::fill_pull_due(&w.app, &w.book, conn, Some(last), now).unwrap(), Some(last + SignedDuration::from_secs(10)));
    // still missing an hour on: less often, still pulled for
    let later = now + SignedDuration::from_secs(3600);
    assert_eq!(crate::broker_reads::fill_pull_due(&w.app, &w.book, conn, Some(later), later).unwrap(), Some(later + crate::broker_reads::fill_pull_step(SignedDuration::from_secs(3600))));
    // its row arrives, keyed by the order's Wealthsimple id: nothing more is due
    w.book.store(&bagholder_wealthsimple::mapping::WealthsimpleMapping, &bagholder_book::records::Incoming { connection: Some(conn), source_key: "ws-1", payload: r#"{"unread": "a row of the test's"}"#, refs: vec![] }, later).unwrap();
    assert_eq!(crate::broker_reads::fill_pull_due(&w.app, &w.book, conn, Some(later), later).unwrap(), None);
    assert!(w.app.fill_waits.lock().unwrap().is_empty(), "a fill whose row arrived stops waiting");
    w.state_is_the_log();
}

#[test]
fn a_missing_fill_is_pulled_for_every_ten_seconds_then_thirty_two_minutes_fifteen_and_hourly() {
    let step = |s: i64| crate::broker_reads::fill_pull_step(SignedDuration::from_secs(s)).as_secs();
    let waits: Vec<i64> = [0, 59, 60, 299, 300, 1799, 1800, 3 * 3600 - 1, 3 * 3600, 30 * 86400].iter().map(|s| step(*s)).collect();
    assert_eq!(waits, [10, 10, 30, 30, 120, 120, 900, 900, 3600, 3600]);
}

// ---------------------------------------------------------------------------
// the badge and the Orders document
// ---------------------------------------------------------------------------

fn place_in(w: &World, id: &str, account: &str) {
    let mut o = order(id, None);
    o.broker_account = account.into();
    gate::place(&w.app, &w.book, &o, &Asker::Person, w.now).unwrap().unwrap();
}

fn elsewhere(id: &str, account: &str, state: OrderState) -> o::Elsewhere {
    o::Elsewhere {
        id: id.into(),
        broker_id: None,
        account: account.into(),
        security: "sec".into(),
        symbol: "SHOP".into(),
        currency: "USD".into(),
        side: Side::Buy,
        kind: OrderKind::Limit,
        quantity: d("1"),
        limit_price: Some(d("1")),
        stop_price: None,
        state,
        filled: Dec::ZERO,
        average: None,
        created_at: "2026-09-28T13:00:00Z".parse().unwrap(),
        ended_at: None,
    }
}

#[test]
fn the_badge_counts_the_pending_cards_of_the_accounts_in_the_pages_scope() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    crate::tests_common::order_accounts_in(&w.app);
    // the margin account: an armed bracket (its stop rests: never a card), a plain
    // entry, a waiting bracket's entry, a dry order
    let entry = w.bracket_in("acct-margin", "b-armed", "10", World::stop("95"), None);
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    w.settle();
    assert_eq!(w.phase("b-armed"), Phase::Guarding);
    assert!(w.exit("b-armed").is_some_and(|x| x.fold.state == OrderState::Pending), "its stop rests");
    place_in(&w, "o-margin", "acct-margin");
    w.bracket_in("acct-margin", "b-waiting", "5", World::stop("90"), None);
    let mut dry = order("o-dry", None);
    dry.broker_account = "acct-margin".into();
    w.book.write_order(&dry, true, &Asker::Person, w.now).unwrap();
    // the TFSA: an entry, one being sent, and Wealthsimple's own (one ended)
    place_in(&w, "o-tfsa", "acct-tfsa");
    let mut sending = order("o-sending", None);
    sending.broker_account = "acct-tfsa".into();
    w.book.write_order(&sending, false, &Asker::Person, w.now).unwrap();
    w.app.orders.elsewhere.lock().unwrap().extend([elsewhere("ws-pending", "acct-tfsa", OrderState::Pending), elsewhere("ws-filled", "acct-tfsa", OrderState::Filled)]);
    let ws = bagholder_core::Broker::named("wealthsimple");
    let id = |broker: &str| w.book.account_by_ref(&bagholder_core::account::AccountRef::new(ws.clone(), broker)).unwrap().unwrap().to_string();
    let badge = |accounts: &[&str]| {
        let lists = if accounts.is_empty() { json!({}) } else { json!({"account": accounts.iter().map(|b| id(b)).collect::<Vec<_>>()}) };
        let feed = crate::events::Feed::open(w.app.clone(), Some(json!({"lists": lists}).to_string()));
        feed.status_for(&crate::status::status).open_orders
    };
    assert_eq!(badge(&[]), 6, "no account named: every account's cards");
    assert_eq!(badge(&["acct-margin"]), 3, "the armed bracket, the entry, the waiting bracket's entry");
    assert_eq!(badge(&["acct-tfsa"]), 3, "the entry, the one being sent, Wealthsimple's pending one");
    assert_eq!(badge(&["acct-margin", "acct-tfsa"]), 6);
    assert_eq!(badge(&["acct-crypto"]), 0, "an account with no card counts none");
    assert_eq!(crate::status::status(&w.app).open_orders, 6, "the status alone knows no page's scope");
    // and the panel's Pending cards are the same ones
    let doc = o::orders_doc(&w.app);
    let pending = doc.orders.iter().filter(|c| c.tab == "pending").count() + doc.brackets.iter().filter(|b| b.tab == "pending").count();
    assert_eq!(pending, 6, "{:?}", doc.orders.iter().map(|c| (&c.id, &c.tab)).collect::<Vec<_>>());
    assert!(doc.orders.iter().all(|c| Some(c.id.clone()) != w.exit("b-armed").map(|x| x.request.id)), "a bracket's exit is never a card");
    w.later(5);
    w.state_is_the_log();
}

fn stated(v: &Fig<crate::wire::Dec>) -> Dec {
    match v {
        Fig::Stated(t) => t.0,
        Fig::Waits { gaps } => panic!("waits on {gaps:?}"),
    }
}

fn value_of(doc: &o::OrdersDoc, id: &str) -> (Option<Fig<crate::wire::Dec>>, bool) {
    let c = doc.orders.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no card {id}"));
    (c.value.clone(), c.approx)
}

#[test]
fn an_orders_value_is_its_quantity_at_its_price_times_the_contract_size_and_its_fill_once_filled() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    {
        let mut units = w.app.orders.units.lock().unwrap();
        units.insert("sec".into(), Dec::ONE);
        units.insert("opt".into(), d("100"));
    }
    let now = w.now;
    let place = |o: bagholder_book::orders::OrderRequest| {
        gate::place(&w.app, &w.book, &o, &Asker::Person, now).unwrap().unwrap();
    };
    place(order("o-limit", None));
    let market = |id: &str| {
        let mut o = order(id, None);
        o.kind = OrderKind::Market;
        o.limit_price = None;
        o
    };
    place(market("o-market"));
    place(market("o-market-part"));
    w.fake.set("o-market-part", BrokerStatus::Open, "4");
    let mut stop = order("o-stop", None);
    stop.kind = OrderKind::Stop;
    stop.limit_price = None;
    stop.stop_price = Some(d("90"));
    place(stop);
    let mut filled = order("o-filled", None);
    filled.limit_price = Some(d("105"));
    place(filled);
    w.fake.set("o-filled", BrokerStatus::Filled, "10");
    let mut option = order("o-option", None);
    option.broker_security = "opt".into();
    option.quantity = d("2");
    option.limit_price = Some(d("1.5"));
    place(option);
    let mut unknown = order("o-unknown", None);
    unknown.broker_security = "unk".into();
    place(unknown);
    w.book.write_order(&order("o-dry", None), true, &Asker::Person, now).unwrap();
    place(order("o-cancelled", None));
    w.fake.set("o-cancelled", BrokerStatus::Cancelled, "0");
    for id in ["o-market-part", "o-filled", "o-cancelled"] {
        gate::read_back(&w.app, &w.book, id, now).unwrap();
    }
    let doc = o::orders_doc(&w.app);
    let (v, approx) = value_of(&doc, "o-limit");
    assert_eq!((stated(&v.unwrap()), approx), (d("1000"), false), "10 at the limit of 100");
    assert_eq!(value_of(&doc, "o-market"), (None, false), "a market order not filled has no price to value it at");
    let (v, approx) = value_of(&doc, "o-market-part");
    assert_eq!((stated(&v.unwrap()), approx), (d("1000"), true), "10 at the fill so far: approximate");
    assert_eq!(stated(&value_of(&doc, "o-stop").0.unwrap()), d("900"), "a stop order at its stop");
    let (v, approx) = value_of(&doc, "o-filled");
    assert_eq!((stated(&v.unwrap()), approx), (d("1000"), false), "what it filled for, 10 at 100, not the limit of 105");
    assert_eq!(stated(&value_of(&doc, "o-option").0.unwrap()), d("300"), "2 contracts of 100 at 1.50");
    assert_eq!(value_of(&doc, "o-unknown").0, Some(Fig::Waits { gaps: vec!["multiplier-unstated".into()] }), "a size no one has stated waits");
    let tab = |id: &str| doc.orders.iter().find(|c| c.id == id).unwrap().tab.clone();
    assert_eq!((tab("o-limit"), tab("o-filled"), tab("o-dry"), tab("o-cancelled")), ("pending".into(), "filled".into(), "cancelled".into(), "cancelled".into()));
    let card = doc.orders.iter().find(|c| c.id == "o-stop").unwrap();
    assert_eq!((card.live, card.editable), (true, false), "a stop order is live and cannot be changed");
    let card = doc.orders.iter().find(|c| c.id == "o-limit").unwrap();
    assert_eq!((card.live, card.editable, card.side.as_str(), card.kind.as_str(), card.tif.as_deref()), (true, true, "buy", "limit", Some("day")));
    w.state_is_the_log();
}

fn legs_of(legs: &[o::Leg]) -> Vec<(String, Dec, Dec, String, Dec)> {
    legs.iter().map(|l| (l.key.clone(), l.quantity.0, l.level.0, l.note.clone(), stated(&l.amount))).collect()
}

#[test]
fn a_waiting_brackets_legs_are_on_its_entrys_card_and_an_armed_bracket_is_a_card_of_its_own() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    w.app.orders.units.lock().unwrap().insert("sec".into(), Dec::ONE);
    let entry = w.bracket("b1", "10", World::stop("95"), Some("110"));
    let doc = o::orders_doc(&w.app);
    assert!(doc.brackets.is_empty(), "a bracket not armed has no card");
    let card = doc.orders.iter().find(|c| c.id == entry).unwrap();
    assert_eq!(legs_of(&card.legs), vec![("sl".into(), d("10"), d("95"), String::new(), d("950")), ("tp".into(), d("10"), d("110"), String::new(), d("1100"))]);
    // armed: its own card, what was paid under it, its legs; the entry's card loses them
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    w.settle();
    let stop = w.exit("b1").unwrap().request.id;
    let doc = o::orders_doc(&w.app);
    let b = doc.brackets.iter().find(|b| b.id == "b1").expect("its card");
    assert_eq!((b.tab.as_str(), b.live, stated(&b.value), b.stop_level.map(|l| l.0), b.target.map(|t| t.0)), ("pending", true, d("1000"), Some(d("95")), Some(d("110"))));
    assert_eq!(legs_of(&b.legs), vec![("sl".into(), d("10"), d("95"), String::new(), d("950")), ("tp".into(), d("10"), d("110"), String::new(), d("1100"))], "no word while a leg rests or is watched");
    let card = doc.orders.iter().find(|c| c.id == entry).unwrap();
    assert!(card.legs.is_empty() && card.tab == "filled");
    assert!(doc.orders.iter().all(|c| c.id != stop), "the stop is never a card");
    // the target reached: the stop's cancel is out, the target waits to be placed
    w.quote("110");
    w.later(5);
    w.tick();
    let doc = o::orders_doc(&w.app);
    let notes: Vec<String> = doc.brackets[0].legs.iter().map(|l| l.note.clone()).collect();
    assert_eq!(notes, ["Cancelling", "Placing"]);
    // the stop filled before its cancel: the bracket ended stopped, its card on Filled
    w.fake.set(&stop, BrokerStatus::Filled, "10");
    w.later(5);
    w.settle();
    let doc = o::orders_doc(&w.app);
    let b = &doc.brackets[0];
    assert_eq!((b.tab.as_str(), b.live, b.end_word.clone()), ("filled", false, None));
    assert_eq!(b.legs[0].filled.as_ref().map(|f| (f.quantity.0, f.average.map(|a| a.0))), Some((d("10"), Some(d("100")))), "the exit's fill on its leg");
    assert_eq!(b.legs[1].note, "Cancelled", "the leg that did not exit");
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// the person's cancel and edit
// ---------------------------------------------------------------------------

#[test]
fn cancel_goes_to_wealthsimple_by_the_orders_id_and_it_is_cancelling_until_read_back() {
    let _g = crate::tests_common::guard();
    let mut w = armed_now(World::stop("95"), None);
    w.now = Timestamp::now();
    for id in ["o1", "o2", "o3"] {
        gate::place(&w.app, &w.book, &order(id, None), &Asker::Person, w.now).unwrap().unwrap();
    }
    w.app.set_orders_live(false);
    assert_eq!(o::cancel_order(&w.app, "o1").error.as_deref(), Some(o::ORDERS_OFF));
    w.app.set_orders_live(true);
    assert_eq!(o::cancel_order(&w.app, "nope").error.as_deref(), Some("No such order."));
    let stop = w.exit("b1").unwrap().request.id;
    assert_eq!(o::cancel_order(&w.app, &stop).error.as_deref(), Some("That order is a bracket's; cancel the bracket."));
    let r = o::cancel_order(&w.app, "o1");
    assert_eq!((r.ok, r.status.as_deref()), (true, Some("cancelling")), "{r:?}");
    assert_eq!(cancels(&w), ["o1"]);
    assert_eq!(w.book.order("o1").unwrap().unwrap().fold.state, OrderState::Cancelling);
    assert_eq!(o::cancel_order(&w.app, "o1").error.as_deref(), Some("Its cancel is already sent."));
    w.fake.set("o1", BrokerStatus::Cancelled, "0");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert_eq!(w.book.order("o1").unwrap().unwrap().fold.state, OrderState::Cancelled);
    // refused: the order still works, and Wealthsimple's reason is the answer
    w.fake.then(Behaviour::Refuse("Too late to cancel", None));
    assert_eq!(o::cancel_order(&w.app, "o2").error.as_deref(), Some("Wealthsimple refused the cancel: Too late to cancel"));
    assert_eq!(w.book.order("o2").unwrap().unwrap().fold.state, OrderState::Pending);
    w.fake.set("o3", BrokerStatus::Filled, "10");
    gate::read_back(&w.app, &w.book, "o3", w.now).unwrap();
    assert_eq!(o::cancel_order(&w.app, "o3").error.as_deref(), Some("That order is not open."));
    // one placed in Wealthsimple's app
    w.app.orders.elsewhere.lock().unwrap().push(elsewhere("ws-own", "acct", OrderState::Pending));
    assert_eq!(o::cancel_order(&w.app, "ws-own").status.as_deref(), Some("cancelling"));
    assert_eq!(cancels(&w).last().map(String::as_str), Some("ws-own"));
    assert_eq!(w.app.orders.elsewhere.lock().unwrap()[0].state, OrderState::Cancelling);
    w.state_is_the_log();
}

#[test]
fn edit_sends_wealthsimples_modify_with_only_what_differs() {
    let _g = crate::tests_common::guard();
    let mut w = armed_now(World::stop("95"), None);
    w.now = Timestamp::now();
    gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, w.now).unwrap().unwrap();
    let modifies = |w: &World| w.fake.0.lock().unwrap().modifies.clone();
    let r = o::modify_order(&w.app, "o1", &pd("30"), &pd("164"));
    assert!(r.ok, "{r:?}");
    assert_eq!(modifies(&w), vec![("o1".to_string(), json!({"newLimitPrice": 164.0, "newQuantity": 30.0}))]);
    assert!(o::modify_order(&w.app, "o1", &pd("30"), &pd("165")).ok);
    assert_eq!(modifies(&w).last().unwrap().1, json!({"newLimitPrice": 165.0}), "only what differs from the order as it rests");
    let same = o::modify_order(&w.app, "o1", &pd("30"), &pd("165"));
    assert_eq!((same.ok, same.unchanged), (true, Some(true)));
    assert_eq!(modifies(&w).len(), 2, "nothing sent for no change");
    // a price at the order's tick
    assert!(o::modify_order(&w.app, "o1", &no(), &pd("165.004")).unchanged == Some(true), "165.004 is 165.00 at the tick");
    // refused: in Wealthsimple's words, and it changes nothing
    w.fake.then(Behaviour::Refuse("Too late", None));
    assert_eq!(o::modify_order(&w.app, "o1", &pd("31"), &pd("165")).error.as_deref(), Some("Wealthsimple refused the change: Too late"));
    let again = o::modify_order(&w.app, "o1", &pd("31"), &pd("165"));
    assert!(again.ok && again.unchanged.is_none(), "the refused change is asked again: {again:?}");
    assert_eq!(modifies(&w).last().unwrap().1, json!({"newQuantity": 31.0}));
    // words for what cannot be sent
    assert_eq!(o::modify_order(&w.app, "o1", &pd("0"), &pd("165")).error.as_deref(), Some("Shares must be more than zero."));
    assert_eq!(o::modify_order(&w.app, "o1", &pd("30"), &pd("-1")).error.as_deref(), Some("A limit price must be more than zero."));
    assert_eq!(o::modify_order(&w.app, "o1", &PageDec(Err("abc".into())), &pd("165")).error.as_deref(), Some("Shares \"abc\" is not a number."));
    assert_eq!(o::modify_order(&w.app, "o1", &pd("30"), &PageDec(Err("true".into()))).error.as_deref(), Some("The limit price \"true\" is not a number."));
    assert_eq!(o::modify_order(&w.app, "nope", &pd("1"), &pd("1")).error.as_deref(), Some("No such order."));
    let stop = w.exit("b1").unwrap().request.id;
    assert_eq!(o::modify_order(&w.app, &stop, &pd("1"), &no()).error.as_deref(), Some("That order is a bracket's; change the bracket."));
    let mut s = order("o-stop", None);
    s.kind = OrderKind::Stop;
    s.limit_price = None;
    s.stop_price = Some(d("90"));
    gate::place(&w.app, &w.book, &s, &Asker::Person, w.now).unwrap().unwrap();
    assert_eq!(o::modify_order(&w.app, "o-stop", &pd("5"), &no()).error.as_deref(), Some("A stop order cannot be changed; cancel it and place another."));
    // a market order has no limit to change: only its size goes
    let mut m = order("o-market", None);
    m.kind = OrderKind::Market;
    m.limit_price = None;
    gate::place(&w.app, &w.book, &m, &Asker::Person, w.now).unwrap().unwrap();
    assert!(o::modify_order(&w.app, "o-market", &pd("12"), &pd("99")).ok);
    assert_eq!(modifies(&w).last().unwrap(), &("o-market".to_string(), json!({"newQuantity": 12.0})));
    let n = modifies(&w).len();
    w.app.set_orders_live(false);
    assert_eq!(o::modify_order(&w.app, "o1", &pd("40"), &no()).error.as_deref(), Some(o::ORDERS_OFF));
    w.app.set_orders_live(true);
    assert_eq!(modifies(&w).len(), n, "nothing sent with orders off");
    w.fake.set("o1", BrokerStatus::Filled, "31");
    gate::read_back(&w.app, &w.book, "o1", w.now).unwrap();
    assert_eq!(o::modify_order(&w.app, "o1", &pd("40"), &no()).error.as_deref(), Some("That order is not open."));
    // one placed in Wealthsimple's app
    w.app.orders.elsewhere.lock().unwrap().push(elsewhere("ws-own", "acct", OrderState::Pending));
    assert!(o::modify_order(&w.app, "ws-own", &pd("2"), &pd("1")).ok);
    assert_eq!(modifies(&w).last().unwrap(), &("ws-own".to_string(), json!({"newQuantity": 2.0})));
    assert_eq!(w.app.orders.elsewhere.lock().unwrap()[0].quantity, d("2"));
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// changing and cancelling a bracket
// ---------------------------------------------------------------------------

#[test]
fn moving_a_resting_stop_cancels_it_and_the_engine_places_the_new_level() {
    let _g = crate::tests_common::guard();
    let mut w = armed_now(World::stop("95"), Some("110"));
    let first = w.exit("b1").unwrap().request.id;
    let before = exit_creates(&w).len();
    let r = o::adjust_bracket(&w.app, "b1", "sl", &pd("90.004"), &no(), false);
    assert!(r.ok, "{r:?}");
    assert_eq!(cancels(&w), [first.clone()], "the resting stop is cancelled first");
    assert_eq!(exit_creates(&w).len(), before, "nothing new until the cancel is confirmed");
    assert_eq!((w.bracket_of("b1").stop.map(|s| s.level), w.phase("b1")), (Some(d("90")), Phase::Guarding), "the new level, at the tick");
    w.fake.set(&first, BrokerStatus::Cancelled, "0");
    w.now = Timestamp::now();
    w.settle();
    let working = w.working_exits();
    assert_eq!(working.len(), 1, "{working:?}");
    assert_eq!((working[0].1.request["executionType"].as_str(), working[0].1.request["stopPrice"].as_f64()), (Some("STOP"), Some(90.0)));
    let askers: Vec<String> = w.book.bracket_log("b1").unwrap().into_iter().filter(|l| l.event.kind() == "adjusted").map(|l| l.asker.to_text()).collect();
    assert_eq!(askers, ["person"], "the person's change, in the log as theirs");
    w.state_is_the_log();
}

#[test]
fn a_trailing_stop_takes_a_new_trail_under_its_high_and_the_target_moves() {
    let _g = crate::tests_common::guard();
    let w = armed_now(trailing("5"), Some("110"));
    assert_eq!(w.bracket_of("b1").stop, Some(StopLeg { level: d("95"), trail: Some(Trail::Pct(d("5"))), high: Some(d("100")) }), "five percent under the fill");
    assert!(o::adjust_bracket(&w.app, "b1", "tp", &pd("120"), &no(), false).ok);
    assert!(o::adjust_bracket(&w.app, "b1", "sl", &no(), &pd("10"), false).ok);
    let b = w.bracket_of("b1");
    assert_eq!(b.target, Some(d("120")));
    assert_eq!(b.stop, Some(StopLeg { level: d("90"), trail: Some(Trail::Pct(d("10"))), high: Some(d("100")) }), "ten percent under the high of 100");
    assert_eq!(o::adjust_bracket(&w.app, "b1", "tp", &pd("0"), &no(), false).error.as_deref(), Some("A limit price is required."));
    assert_eq!(o::adjust_bracket(&w.app, "b1", "sl", &pd("90"), &no(), false).error.as_deref(), Some("A trail is required."));
    assert_eq!(o::adjust_bracket(&w.app, "b1", "sl", &no(), &PageDec(Err("far".into())), false).error.as_deref(), Some("The trail \"far\" is not a number."));
    assert_eq!(o::adjust_bracket(&w.app, "b1", "x", &pd("1"), &no(), false).error.as_deref(), Some("Which leg?"));
    assert_eq!(o::adjust_bracket(&w.app, "nope", "tp", &pd("1"), &no(), false).error.as_deref(), Some("No such bracket."));
    w.state_is_the_log();
}

/// A bracket armed on the wall clock whose target's limit sell rests.
fn target_resting(stop: Option<StopLeg>) -> (World, String) {
    let mut w = armed_now(stop, Some("110"));
    let stop_id = w.exit("b1").unwrap().request.id;
    w.quote("110");
    w.now = Timestamp::now();
    w.tick();
    w.fake.set(&stop_id, BrokerStatus::Cancelled, "0");
    w.now = Timestamp::now();
    w.settle();
    assert_eq!(w.phase("b1"), Phase::Target);
    let target = w.exit("b1").unwrap();
    assert_eq!((target.request.kind, target.request.limit_price), (OrderKind::Limit, Some(d("110"))));
    (w, target.request.id)
}

#[test]
fn a_placed_target_moved_is_cancelled_and_the_target_watched_again() {
    let _g = crate::tests_common::guard();
    let (mut w, target) = target_resting(World::stop("95"));
    assert!(o::adjust_bracket(&w.app, "b1", "tp", &pd("115"), &no(), false).ok);
    assert_eq!(cancels(&w).last(), Some(&target));
    assert_eq!(w.book.order(&target).unwrap().unwrap().fold.state, OrderState::Cancelling);
    assert_eq!(w.bracket_of("b1").target, Some(d("115")));
    w.fake.set(&target, BrokerStatus::Cancelled, "0");
    w.now = Timestamp::now();
    w.settle();
    // 110 is out of the new target's reach: the stop rests again and 115 is watched
    assert_eq!(w.phase("b1"), Phase::Guarding);
    let working = w.working_exits();
    assert_eq!(working.len(), 1, "{working:?}");
    assert_eq!((working[0].1.request["executionType"].as_str(), working[0].1.request["stopPrice"].as_f64()), (Some("STOP"), Some(95.0)));
    w.state_is_the_log();
}

#[test]
fn removing_one_leg_then_the_other_ends_the_bracket_once_its_stops_cancel_is_confirmed() {
    let _g = crate::tests_common::guard();
    let mut w = armed_now(World::stop("95"), Some("110"));
    let stop = w.exit("b1").unwrap().request.id;
    assert!(o::adjust_bracket(&w.app, "b1", "sl", &no(), &no(), true).ok);
    assert_eq!(cancels(&w), [stop.clone()], "the resting stop is cancelled");
    let b = w.bracket_of("b1");
    assert_eq!((b.phase, b.stop, b.target), (Phase::Guarding, None, Some(d("110"))));
    assert!(o::adjust_bracket(&w.app, "b1", "tp", &no(), &no(), true).ok);
    let b = w.bracket_of("b1");
    assert_eq!((b.phase, b.outcome.as_deref()), (Phase::Closing, Some("both legs removed")), "closing while the stop's cancel is not confirmed");
    w.fake.set(&stop, BrokerStatus::Cancelled, "0");
    w.now = Timestamp::now();
    w.settle();
    assert_eq!(w.phase("b1"), Phase::Ended);
    let card = o::orders_doc(&w.app).brackets.into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!((card.tab.as_str(), card.end_word.as_deref()), ("cancelled", Some("Cancelled")));
    w.state_is_the_log();
}

#[test]
fn with_orders_off_a_brackets_changes_answer_orders_are_off_and_change_nothing() {
    let _g = crate::tests_common::guard();
    let w = armed_now(trailing("5"), Some("110"));
    let before = w.bracket_of("b1");
    let sent = (exit_creates(&w).len(), cancels(&w).len());
    w.app.set_orders_live(false);
    let tries = [
        ("stop moved", o::adjust_bracket(&w.app, "b1", "sl", &pd("150"), &no(), false)),
        ("trail changed", o::adjust_bracket(&w.app, "b1", "sl", &no(), &pd("8"), false)),
        ("target moved", o::adjust_bracket(&w.app, "b1", "tp", &pd("190"), &no(), false)),
        ("stop removed", o::adjust_bracket(&w.app, "b1", "sl", &no(), &no(), true)),
        ("target removed", o::adjust_bracket(&w.app, "b1", "tp", &no(), &no(), true)),
        ("bracket cancelled", o::cancel_bracket(&w.app, "b1")),
    ];
    for (what, r) in tries {
        assert_eq!((r.ok, r.error.as_deref()), (false, Some(o::ORDERS_OFF)), "{what}");
    }
    assert_eq!((exit_creates(&w).len(), cancels(&w).len()), sent, "nothing reached Wealthsimple");
    assert_eq!(w.bracket_of("b1"), before, "the bracket is as it was");
    w.state_is_the_log();
}

#[test]
fn cancelling_a_bracket_cancels_its_resting_exit_and_it_is_closing_until_confirmed_then_ended() {
    let _g = crate::tests_common::guard();
    let mut w = armed_now(World::stop("95"), Some("110"));
    let stop = w.exit("b1").unwrap().request.id;
    // Wealthsimple refuses the first cancel: sent again on the next check
    w.fake.then(Behaviour::Refuse("Wealthsimple is busy", None));
    let r = o::cancel_bracket(&w.app, "b1");
    assert!(r.ok, "{r:?}");
    let b = w.bracket_of("b1");
    assert_eq!((b.phase, b.outcome.as_deref()), (Phase::Closing, Some("cancelled by the user")));
    assert_eq!(w.book.order(&stop).unwrap().unwrap().fold.state, OrderState::Pending, "the stop still rests: the cancel was refused");
    w.now = Timestamp::now();
    w.tick();
    assert_eq!(cancels(&w), [stop.clone(), stop.clone()], "the cancel is sent again");
    assert_eq!(w.phase("b1"), Phase::Closing);
    w.fake.set(&stop, BrokerStatus::Cancelled, "0");
    w.now = Timestamp::now();
    w.settle();
    assert_eq!(w.phase("b1"), Phase::Ended, "ended once Wealthsimple confirms the cancel");
    assert_eq!(w.book.order("b1-entry").unwrap().unwrap().fold.state, OrderState::Filled, "the entry is not touched");
    assert_eq!(o::cancel_bracket(&w.app, "b1").error.as_deref(), Some("That bracket is not live."));
    assert_eq!(o::cancel_bracket(&w.app, "nope").error.as_deref(), Some("No such bracket."));
    let card = o::orders_doc(&w.app).brackets.into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!((card.tab.as_str(), card.end_word.as_deref(), card.legs.iter().map(|l| l.note.clone()).collect::<Vec<_>>()), ("cancelled", Some("Cancelled"), vec!["Off".to_string(), "Off".to_string()]));
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// the engine at the server
// ---------------------------------------------------------------------------

#[test]
fn a_brackets_exit_is_the_order_wealthsimples_web_app_sends() {
    let _g = crate::tests_common::guard();
    let (w, _) = World::armed(World::stop("95"), Some("110"));
    let working = w.working_exits();
    assert_eq!(working.len(), 1);
    assert_eq!(sent_as(&working[0].1.request), json!({"canonicalAccountId": "acct", "executionType": "STOP", "orderType": "SELL_QUANTITY", "quantity": 10.0, "securityId": "sec", "timeInForce": "UNTIL_CANCEL", "stopPrice": 95.0}));
    w.state_is_the_log();
}

fn tape(last: &str, bid: &str) -> HashMap<String, Quoted> {
    HashMap::from([("sec".to_string(), Quoted { open: true, tape: Some(Tape { last: d(last), bid: Some(d(bid)) }), problem: None })])
}

#[test]
fn the_bid_not_the_last_decides_the_target() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), Some("110"));
    w.later(5);
    o::bracket_tick(&w.app, Some(tape("112", "109.50")), w.now).unwrap();
    assert_eq!((w.phase(&b), cancels(&w).len()), (Phase::Guarding, 0), "the last past the target, the bid short of it");
    w.later(5);
    o::bracket_tick(&w.app, Some(tape("109", "110")), w.now).unwrap();
    assert_eq!((w.phase(&b), cancels(&w).len()), (Phase::ToTarget, 1));
    w.state_is_the_log();
}

#[test]
fn a_trailing_level_keeps_following_the_high_while_the_limit_sell_rests() {
    let _g = crate::tests_common::guard();
    let (mut w, target) = target_resting(trailing("5"));
    let sent = (exit_creates(&w).len(), cancels(&w).len());
    w.quote("115");
    w.now = Timestamp::now();
    w.tick();
    let stop = w.bracket_of("b1").stop.unwrap();
    assert_eq!((stop.level, stop.high), (d("109.25"), Some(d("115"))), "five percent under the new high, with no order to move");
    assert_eq!((exit_creates(&w).len(), cancels(&w).len()), sent, "the limit sell stays");
    assert_eq!(w.exit("b1").unwrap().request.id, target);
    w.state_is_the_log();
}

#[test]
fn a_stop_or_quantity_changed_by_hand_at_wealthsimple_is_adopted_and_nothing_is_placed_again() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap().request.id;
    let sent = (exit_creates(&w).len(), cancels(&w).len());
    fake_order(&w, &stop, |o| o.price = Some(d("90")));
    w.later(5);
    w.tick();
    assert_eq!(w.bracket_of(&b).stop.map(|s| s.level), Some(d("90")), "the bracket follows the level set by hand");
    fake_order(&w, &stop, |o| o.quantity = Some(d("8")));
    w.later(5);
    w.tick();
    let after = w.bracket_of(&b);
    assert_eq!((after.quantity, after.phase), (d("8"), Phase::Guarding), "it follows the quantity set by hand and keeps guarding it");
    w.later(5);
    w.settle();
    assert_eq!((exit_creates(&w).len(), cancels(&w).len()), sent, "nothing is cancelled or placed again for it");
    assert_eq!(w.exit(&b).unwrap().request.id, stop);
    w.state_is_the_log();
}

#[test]
fn a_target_price_changed_by_hand_at_wealthsimple_is_adopted() {
    let _g = crate::tests_common::guard();
    let (mut w, target) = target_resting(World::stop("95"));
    let sent = (exit_creates(&w).len(), cancels(&w).len());
    // (still within a percent of the bid, so the limit keeps resting)
    fake_order(&w, &target, |o| o.price = Some(d("110.50")));
    w.now = Timestamp::now();
    w.settle();
    assert_eq!(w.bracket_of("b1").target, Some(d("110.50")));
    assert_eq!((exit_creates(&w).len(), cancels(&w).len()), sent);
    w.state_is_the_log();
}

#[test]
fn a_stop_cancelled_by_hand_at_wealthsimple_ends_the_bracket() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), Some("110"));
    let stop = w.exit(&b).unwrap().request.id;
    let creates = exit_creates(&w).len();
    w.fake.set(&stop, BrokerStatus::Cancelled, "0");
    w.later(5);
    w.tick();
    let after = w.bracket_of(&b);
    assert_eq!((after.phase, after.outcome.as_deref()), (Phase::Ended, Some("stop cancelled at Wealthsimple by hand")));
    assert!(cancels(&w).is_empty(), "nothing else of the bracket's rested, so nothing to cancel");
    w.quote("115");
    w.later(5);
    w.settle();
    assert_eq!(exit_creates(&w).len(), creates, "the target never fires");
    w.state_is_the_log();
}

#[test]
fn an_exit_refused_for_shares_that_are_not_there_ends_the_bracket_with_the_reason() {
    let _g = crate::tests_common::guard();
    for code in ["BALANCE_INSUFFICIENT_SHARES", "NOT_ENOUGH_SHARES", "balance_insufficient_shares"] {
        let mut w = World::new();
        let entry = w.bracket("b1", "10", World::stop("95"), Some("110"));
        w.quote("100");
        w.fake.set(&entry, BrokerStatus::Filled, "10");
        w.fake.then(Behaviour::Refuse("You do not have enough shares", Some(code)));
        w.settle();
        let b = w.bracket_of("b1");
        assert_eq!((b.phase, b.outcome.as_deref(), b.why.as_deref()), (Phase::Ended, Some("the shares are not there"), Some("You do not have enough shares")), "{code}");
        let creates = exit_creates(&w).len();
        w.later(3600);
        w.settle();
        assert_eq!(exit_creates(&w).len(), creates, "{code}: nothing is tried again");
        w.state_is_the_log();
    }
    // taken, then read back rejected for the same reason
    let (mut w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap().request.id;
    fake_order(&w, &stop, |o| {
        o.status = BrokerStatus::Rejected;
        o.why = Some("You do not have enough shares".into());
        o.code = Some("NOT_ENOUGH_SHARES".into());
    });
    w.later(5);
    w.settle();
    let after = w.bracket_of(&b);
    assert_eq!((after.phase, after.outcome.as_deref(), after.why.as_deref()), (Phase::Ended, Some("the shares are not there"), Some("You do not have enough shares")));
    let creates = exit_creates(&w).len();
    w.later(3600);
    w.settle();
    assert_eq!(exit_creates(&w).len(), creates, "nothing is tried again");
    w.state_is_the_log();
}

#[test]
fn a_rejected_exit_is_tried_again_after_one_five_fifteen_minutes_then_hourly() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    w.app.orders.units.lock().unwrap().insert("sec".into(), Dec::ONE);
    let entry = w.bracket("b1", "10", World::stop("95"), None);
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    for _ in 0..5 {
        w.fake.then(Behaviour::Refuse("Market closed", None));
    }
    w.tick();
    let b = w.bracket_of("b1");
    assert_eq!((b.phase, b.attempts, b.why.as_deref(), exit_creates(&w).len()), (Phase::Guarding, 1, Some("Market closed"), 1));
    let card = o::orders_doc(&w.app).brackets.into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(card.legs[0].note, "Retrying · Market closed");
    let mut tried = 1;
    for rest in [60, 300, 900, 3600, 3600] {
        w.later(rest - 1);
        w.tick();
        assert_eq!(exit_creates(&w).len(), tried, "not before {rest} seconds");
        w.later(1);
        w.tick();
        tried += 1;
        assert_eq!(exit_creates(&w).len(), tried, "tried again after {rest} seconds");
    }
    let b = w.bracket_of("b1");
    assert_eq!((b.phase, b.attempts, b.why.as_deref()), (Phase::Guarding, 0, None), "the sixth went out");
    assert_eq!(w.working_exits().len(), 1);
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// the ninety-day roll
// ---------------------------------------------------------------------------

/// An armed bracket whose resting exit Wealthsimple says lapses `left` from now.
fn lapsing(stop: Option<StopLeg>, left: SignedDuration) -> (World, String) {
    let (w, b) = World::armed(stop, None);
    let x = w.exit(&b).unwrap().request.id;
    let at = w.now + left;
    fake_order(&w, &x, |o| o.expires_at = Some(at));
    (w, x)
}

#[test]
fn a_stop_within_a_week_of_its_end_is_placed_again_while_the_market_is_closed_not_while_it_trades() {
    let _g = crate::tests_common::guard();
    let (mut w, first) = lapsing(World::stop("95"), days(5));
    w.later(5);
    w.tick();
    assert!(cancels(&w).is_empty(), "five days left and the market open: it waits for the close");
    assert_eq!(w.book.order(&first).unwrap().unwrap().expires_at, Some(w.now - SignedDuration::from_secs(5) + days(5)), "the end Wealthsimple states is kept");
    w.open = false;
    w.later(5);
    w.tick();
    assert_eq!(cancels(&w), [first.clone()], "closed: cancelled to be placed again");
    let creates = exit_creates(&w).len();
    w.later(5);
    w.tick();
    assert_eq!(exit_creates(&w).len(), creates, "nothing new until the cancel is confirmed");
    w.fake.set(&first, BrokerStatus::Cancelled, "0");
    w.later(5);
    w.settle();
    let again = w.exit("b1").unwrap();
    assert_ne!(again.request.id, first);
    assert_eq!((again.request.kind, again.request.stop_price, again.request.request["timeInForce"].as_str()), (OrderKind::Stop, Some(d("95")), Some("UNTIL_CANCEL")), "the same level, good till cancelled");
    assert_eq!(w.phase("b1"), Phase::Guarding, "the bracket stands");
    w.state_is_the_log();
}

#[test]
fn a_stop_within_two_days_of_its_end_is_placed_again_whatever_the_market_and_one_with_time_left_is_left_alone() {
    let _g = crate::tests_common::guard();
    let (mut w, first) = lapsing(World::stop("95"), SignedDuration::from_hours(36));
    w.later(5);
    w.tick();
    assert_eq!(cancels(&w), [first], "the market open, a day and a half left");
    let (mut w, first) = lapsing(World::stop("95"), days(30));
    w.open = false;
    w.later(5);
    w.settle();
    assert!(cancels(&w).is_empty(), "thirty days left");
    assert_eq!(w.exit("b1").unwrap().request.id, first);
    w.state_is_the_log();
}

#[test]
fn a_stop_whose_end_wealthsimple_does_not_state_ends_ninety_days_from_when_it_was_sent() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap();
    assert_eq!(stop.expires_at, None, "no end stated");
    w.open = false;
    w.now = stop.created_at + days(82);
    w.tick();
    assert!(cancels(&w).is_empty(), "eight days left");
    w.now = stop.created_at + days(84);
    w.tick();
    assert_eq!(cancels(&w), [stop.request.id], "six days left, the market closed");
    w.state_is_the_log();
}

#[test]
fn a_trailing_stop_keeps_its_level_and_its_high_through_the_roll() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(trailing("5"), None);
    let first = w.exit(&b).unwrap().request.id;
    w.quote("110");
    w.later(5);
    w.tick();
    w.fake.set(&first, BrokerStatus::Cancelled, "0");
    w.later(5);
    w.settle();
    let cur = w.exit(&b).unwrap();
    assert_eq!(cur.request.stop_price, Some(d("104.50")));
    let (level, high) = { let s = w.bracket_of(&b).stop.unwrap(); (s.level, s.high) };
    assert_eq!((level, high), (d("104.50"), Some(d("110"))));
    let at = w.now + days(1);
    fake_order(&w, &cur.request.id, |o| o.expires_at = Some(at));
    w.later(5);
    w.tick();
    assert_eq!(cancels(&w).last(), Some(&cur.request.id), "rolled");
    w.fake.set(&cur.request.id, BrokerStatus::Cancelled, "0");
    w.later(5);
    w.settle();
    let s = w.bracket_of(&b).stop.unwrap();
    assert_eq!((s.level, s.high), (level, high), "the level and the high kept");
    let again = w.exit(&b).unwrap();
    assert!(again.request.id != cur.request.id && again.request.stop_price == Some(level));
    w.state_is_the_log();
}

#[test]
fn a_resting_target_is_rolled_too_and_stays_the_targets() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(None, Some("110"));
    w.quote("110");
    w.later(5);
    w.settle();
    assert_eq!(w.phase(&b), Phase::Target);
    let target = w.exit(&b).unwrap().request.id;
    let at = w.now + days(1);
    fake_order(&w, &target, |o| o.expires_at = Some(at));
    w.later(5);
    w.tick();
    assert_eq!(cancels(&w), [target.clone()]);
    assert_eq!(w.phase(&b), Phase::Target, "still the target's turn");
    w.fake.set(&target, BrokerStatus::Cancelled, "0");
    w.later(5);
    w.settle();
    let again = w.exit(&b).unwrap();
    assert_ne!(again.request.id, target);
    assert_eq!((again.request.kind, again.request.limit_price, again.request.request["timeInForce"].as_str()), (OrderKind::Limit, Some(d("110")), Some("UNTIL_CANCEL")));
    assert_eq!(w.phase(&b), Phase::Target);
    w.state_is_the_log();
}

// ---------------------------------------------------------------------------
// when the checks run
// ---------------------------------------------------------------------------

#[test]
fn the_order_and_bracket_checks_run_while_a_sync_does() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = fresh();
    let set = |connected: bool, syncing: bool| {
        let mut st = app.state.lock().unwrap();
        st.connected = connected;
        st.syncing = syncing;
    };
    for syncing in [false, true] {
        set(true, syncing);
        assert!(o::orders_can_run(&app), "connected, syncing {syncing}: the checks run");
        set(false, syncing);
        assert!(!o::orders_can_run(&app), "not connected: nothing to check with");
    }
}

// ---------------------------------------------------------------------------
// the review's value in CAD
// ---------------------------------------------------------------------------

/// SPEC §4, Review: Position size is the order's cost in CAD at today's rate, the
/// figures' own (the Bank of Canada's) for the quote's currency, whatever the
/// currency; one the figures hold no rate for waits, never taken 1:1.
#[test]
fn the_reviews_value_in_cad_is_at_the_figures_rate_for_any_currency() {
    use crate::orders::preview::{preview_for, PreviewRequest, QuoteInput};
    let _g = crate::tests_common::guard();
    let a = crate::tests_common::app();
    let f = a.figures.get().unwrap();
    let (mut rated, mut waiting) = (0, 0);
    for code in ["CAD", "USD", "EUR", "GBP", "JPY", "AUD", "CHF", "HKD", "MXN", "ZZZ"] {
        let currency = bagholder_core::Currency::parse(code).unwrap();
        let expected = f.read(|e| bagholder_engine::fx::rate(&e.inputs().facts.rates, &e.inputs().clock, currency, e.inputs().clock.today)).unwrap();
        let r = PreviewRequest {
            side: "BUY".into(),
            kind: "LIMIT".into(),
            quantity: Some("10".into()),
            limit: Some("100".into()),
            quote: QuoteInput { last: Some("100".into()), currency: code.into(), ..Default::default() },
            nav: Some("10000".into()),
            ..Default::default()
        };
        let p = preview_for(&a, &r).unwrap();
        match expected {
            Ok(rate) => {
                rated += 1;
                assert_eq!(p.cad, Some(Fig::Stated(crate::wire::Dec(d("1000").checked_mul(rate).unwrap()))), "{code}");
                if code == "CAD" {
                    assert_eq!(rate, Dec::ONE);
                }
            }
            Err(g) => {
                waiting += 1;
                let gaps: Vec<String> = g.words().into_iter().map(String::from).collect();
                assert!(!gaps.is_empty());
                assert_eq!(p.cad, Some(Fig::Waits { gaps: gaps.clone() }), "{code}: waits, never 1:1");
                assert_eq!(p.position_share, Some(Fig::Waits { gaps }), "{code}");
            }
        }
    }
    assert!(rated >= 1 && waiting >= 1, "both kinds walked: {rated} rated, {waiting} waiting");
}

