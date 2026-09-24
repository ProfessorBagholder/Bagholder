//! The derived model: FIFO matching, round trips, FX, the view and its filters,
//! cash flow and returns.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

use bagholder_model::base::{build_base, Base};
use bagholder_model::cases::{act, buy, buy_x, sell, sell_x};
use bagholder_model::dates::option_expiry;
use bagholder_model::fifo::{match_fifo, match_fifo_in_place, Matched};
use bagholder_model::filters::clean_filters;
use bagholder_model::fx::{apply_fx, rate_on, to_cad, Fx, FX_FALLBACK};
use bagholder_model::nav::{year_return, NavRow, Point};
use bagholder_model::activity::{Direction, Flag, Kind, RawActivity};
use bagholder_model::book::Book;
use bagholder_model::input::{journal_from, Quote};
use bagholder_model::normalize::normalize_all;
use bagholder_model::securities::Securities;
use bagholder_model::stats::payments_per_year;
use bagholder_model::symbols::is_option_symbol;
use bagholder_model::symbols_of::migrate_legacy_notes;
use bagholder_model::trades::{build_trades, group_id_for_keys, slice_member_key};
use bagholder_model::view::{view_of, Detail};

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

/// `assertAlmostEqual` to `places` decimals.
fn near_p(a: f64, b: f64, places: i32) {
    let d = a - b;
    let r = (d * 10f64.powi(places)).round();
    assert!(r == 0.0, "{} != {} within {} places", a, b, places);
}

fn near(a: f64, b: f64) {
    near_p(a, b, 7)
}

fn n(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {}", v))
}

fn st(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {}", v))
}

fn arr(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap_or_else(|| panic!("not a list: {}", v))
}


// The tests below assert on what the page is sent, so they read the model's rows
// as that JSON: these stand in front of the typed functions and hand it over.

fn sent<T: serde::Serialize>(rows: &[T]) -> Vec<Value> {
    rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect()
}

fn build_view(base: &Base, filters: Option<&Value>) -> Value {
    let cleaned = bagholder_model::filters::clean_filters(filters);
    bagholder_model::view::build_view(base, Some(&cleaned)).to_value()
}

/// The view as sent: no row's legs and fills, or only the open one's.
fn slim(base: &Base, open: Option<&str>) -> Value {
    view_of(base, None, Detail::Only(open)).to_value()
}

fn trade_detail(base: &Base, id: &str) -> Option<Value> {
    bagholder_model::view::trade_detail(base, id).map(|d| serde_json::to_value(d).unwrap())
}

fn equity_series(days: &[Value]) -> Vec<Point> {
    bagholder_model::nav::equity_series(&days.iter().map(|d| serde_json::from_value::<NavRow>(d.clone()).unwrap()).collect::<Vec<_>>())
}

fn drawdown(series: &[Point]) -> Value {
    serde_json::to_value(bagholder_model::nav::drawdown(series)).unwrap()
}

fn yearly_returns(series: &[Point], bench: &BTreeMap<String, f64>, today: &str) -> Vec<Value> {
    sent(&bagholder_model::nav::yearly_returns(series, bench, today))
}

fn held_symbols(base: &Base) -> Vec<Value> {
    sent(&bagholder_model::symbols_of::held_symbols(base))
}

fn payer_symbols(base: &Base) -> Vec<Value> {
    sent(&bagholder_model::symbols_of::payer_symbols(base))
}

fn intraday_archive_symbols(base: &Base) -> Vec<Value> {
    sent(&bagholder_model::symbols_of::intraday_archive_symbols(base))
}

fn snap(acts: Vec<Value>) -> Value {
    json!({"activities": acts, "accounts": [], "balances": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": []})
}

fn base_of(snapshot: &Value, market: Value, journal: Value, today: &str) -> Base {
    build_base(snapshot, &market, journal.as_object().unwrap(), Some(today))
}

fn empty_market() -> Value {
    json!({"fx": {}, "benchmark": {}})
}

/// Stored rows, read as the model reads them.
fn raws(acts: &[Value]) -> Vec<RawActivity> {
    acts.iter().map(|a| serde_json::from_value(a.clone()).unwrap()).collect()
}

fn fifo(acts: Vec<Value>) -> Matched {
    match_fifo(&raws(&acts))
}

fn pnl_sum(m: &Matched) -> f64 {
    m.closed.iter().map(|t| t.pnl).sum()
}

fn opt(o: Value) -> Value {
    act(o)
}

fn by_key<'a>(rows: &'a [Value], key: &str) -> HashMap<String, &'a Value> {
    rows.iter().map(|r| (st(&r[key]).to_string(), r)).collect()
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// --------------------------------------------------------------------------
// FifoPortTest: scenarios ported one-for-one from the ledger.html engine tests
// --------------------------------------------------------------------------

#[test]
fn test_multileg_zero_qty_closes_short() {
    let r = fifo(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -16, "unitPrice": 6.2225, "netCashAmount": 9956, "transactionDate": "2026-01-10"})),
        opt(json!({"id": "ml1", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": -128, "transactionDate": "2026-03-01"})),
        opt(json!({"id": "ml2", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": -2025, "transactionDate": "2026-03-01"})),
    ]);
    assert!(r.open.is_empty());
    let mut real: Vec<_> = r.closed.iter().filter(|t| !t.flags.iter().any(|f| *f == Flag::RolledOut)).collect();
    assert_eq!(real.len(), 2);
    real.sort_by(|a, b| a.quantity.partial_cmp(&b.quantity).unwrap());
    assert_eq!(real[0].quantity, 1.0);
    near(real[0].exit_price, 1.28);
    assert_eq!(real[1].quantity, 15.0);
    near(real[1].exit_price, 1.35);
    assert!(r.closed.iter().all(|t| t.open_direction == Direction::Short));
    let want = (6.2225 - 1.28) * 1.0 * 100.0 + (6.2225 - 1.35) * 15.0 * 100.0;
    near(pnl_sum(&r), want);
    assert!(real.iter().all(|t| t.rt.as_deref() == Some("rt:sto")));
}

#[test]
fn test_roll_carries_the_unposted_leg_to_the_next_buy_back() {
    // STO 16 Jan27 calls; roll to Jan28 (only the closing leg is posted);
    // STO 6 more Jan28; buy back all 22. Nothing stays open.
    let r = fifo(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -16, "unitPrice": 6.2225, "netCashAmount": 9956, "transactionDate": "2025-10-01", "symbol": "LUNR 15JAN27 12.00 CALL"})),
        opt(json!({"id": "ml", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": -2160, "transactionDate": "2025-11-14", "symbol": "LUNR 15JAN27 12.00 CALL"})),
        opt(json!({"id": "sto2", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -6, "unitPrice": 6.75, "netCashAmount": 4050, "transactionDate": "2025-12-10", "symbol": "LUNR 21JAN28 12.00 CALL"})),
        opt(json!({"id": "btc", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 22, "unitPrice": 13.3, "netCashAmount": -29260, "transactionDate": "2026-06-26", "symbol": "LUNR 21JAN28 12.00 CALL"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    near(pnl_sum(&r), (9956 - 2160 + 4050 - 29260) as f64);
    let rolled_in: Vec<_> = r.closed.iter().filter(|t| t.flags.iter().any(|f| *f == Flag::RolledIn)).collect();
    near(rolled_in.iter().map(|t| t.quantity).sum(), 16.0);
    assert!(rolled_in.iter().all(|t| t.symbol == "LUNR 21JAN28 12.00 CALL"));
    // everything the buy-back closed is one position, so one trade row
    let jan28: HashSet<_> = r.closed.iter().filter(|t| t.symbol == "LUNR 21JAN28 12.00 CALL").map(|t| t.rt.clone()).collect();
    assert_eq!(jan28.len(), 1);
}

#[test]
fn test_credit_roll_up_moves_shorts_to_the_new_strike() {
    let r = fifo(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -5, "unitPrice": 3.0, "netCashAmount": 1500, "transactionDate": "2025-11-12", "symbol": "BBAI 21JAN28 10.00 CALL"})),
        opt(json!({"id": "cr1", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": 14, "transactionDate": "2026-06-09", "symbol": "BBAI 21JAN28 10.00 CALL"})),
        opt(json!({"id": "cr2", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": 56, "transactionDate": "2026-06-17", "symbol": "BBAI 21JAN28 10.00 CALL"})),
        opt(json!({"id": "btc", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 5, "unitPrice": 0.85, "netCashAmount": -425, "transactionDate": "2026-06-26", "symbol": "BBAI 21JAN28 12.00 CALL"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    near(pnl_sum(&r), (1500 + 14 + 56 - 425) as f64);
}

#[test]
fn test_buy_back_closes_older_contracts_of_a_rolled_chain() {
    let s = |id: &str, q: i64, px: f64, cash: i64, day: &str, sym: &str| opt(json!({"id": id, "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -q, "unitPrice": px, "netCashAmount": cash, "transactionDate": day, "symbol": sym}));
    let ml = |id: &str, cash: i64, day: &str, sym: &str| opt(json!({"id": id, "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": cash, "transactionDate": day, "symbol": sym}));
    let r = fifo(vec![
        s("s1", 3, 0.12, 36, "2025-12-05", "BBAI 26DEC25 5.50 PUT"),
        s("s2", 5, 0.2, 100, "2025-12-11", "BBAI 02JAN26 5.50 PUT"),
        s("s3", 1, 0.4, 40, "2025-12-15", "BBAI 26DEC25 6.00 PUT"),
        s("s4", 6, 0.2, 120, "2025-12-12", "BBAI 19DEC25 6.00 PUT"),
        ml("ml1", -18, "2025-12-15", "BBAI 19DEC25 6.00 PUT"),
        ml("ml2", -1830, "2025-12-18", "BBAI 18JUN26 5.00 PUT"),
        s("s5", 11, 2.4, 2640, "2026-02-27", "BBAI 21JAN28 5.00 PUT"),
        opt(json!({"id": "btc", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 26, "unitPrice": 2.74, "netCashAmount": -7124, "transactionDate": "2026-06-29", "symbol": "BBAI 21JAN28 5.00 PUT"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    near(pnl_sum(&r), (36 + 100 + 40 + 120 - 18 - 1830 + 2640 - 7124) as f64);
    assert!(r.closed.iter().filter(|t| t.exit_date == "2026-06-29").all(|t| t.symbol == "BBAI 21JAN28 5.00 PUT"));
}

#[test]
fn test_plain_option_buys_without_a_roll_stay_long() {
    let r = fifo(vec![opt(json!({"id": "bto", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 10, "unitPrice": 1.27, "netCashAmount": -1270, "transactionDate": "2026-06-15", "symbol": "QNC 20NOV26 3.00 CALL"}))]);
    assert_eq!(r.open.len(), 1);
    assert_eq!(r.open[0].direction, Direction::Long);
}

#[test]
fn test_short_expiry_closes_short() {
    let r = fifo(vec![
        opt(json!({"id": "sto2", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -5, "unitPrice": 2, "netCashAmount": 1000, "transactionDate": "2026-01-10", "symbol": "ABC 15JAN27 10.00 CALL"})),
        opt(json!({"id": "exp", "activityType": "OPTIONS_SHORT_EXPIRY", "activitySubType": "EXPIRED", "rawType": "OPTIONS_SHORT_EXPIRY", "quantity": 5, "transactionDate": "2027-01-15", "symbol": "ABC 15JAN27 10.00 CALL"})),
    ]);
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].exit_price, 0.0);
    assert_eq!(r.closed[0].quantity, 5.0);
    near(r.closed[0].pnl, 1000.0);
}

#[test]
fn test_shares_round_trip() {
    let r = fifo(vec![buy("b", "AAA", 10, 12, "2026-01-10"), sell("s", "AAA", 10, 15, "2026-02-10")]);
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].quantity, 10.0);
    near(r.closed[0].pnl, 30.0);
    assert_eq!(r.closed[0].hold_days, 31);
    assert!(!is_option_symbol("AAA"));
    assert!(is_option_symbol("LUNR 15JAN27 12.00 CALL"));
}

#[test]
fn test_credit_multilegs_on_a_short_are_a_roll() {
    let r = fifo(vec![
        opt(json!({"id": "bbai-sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -3, "unitPrice": 1.2, "netCashAmount": 360, "transactionDate": "2026-01-05", "symbol": "BBAI 21JAN28 10.00 CALL"})),
        opt(json!({"id": "bbai-cr1", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOCLOSE", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": 14, "transactionDate": "2026-02-01", "symbol": "BBAI 21JAN28 10.00 CALL"})),
        opt(json!({"id": "bbai-cr2", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": 56, "transactionDate": "2026-02-01", "symbol": "BBAI 21JAN28 10.00 CALL"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    near(pnl_sum(&r), (360 + 14 + 56) as f64);
}

#[test]
fn test_long_expiry_and_same_day_expiry() {
    let r = fifo(vec![
        opt(json!({"id": "lunr-bto", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 0.4, "netCashAmount": -80, "transactionDate": "2025-07-01", "symbol": "LUNR 22AUG25 8.00 CALL"})),
        opt(json!({"id": "lunr-exp", "category": "option_event", "activityType": "EXPIR", "activitySubType": "BUY", "rawType": "OPTIONS_EXPIRY", "quantity": 2, "transactionDate": "2025-08-22", "symbol": "LUNR 22AUG25 8.00 CALL"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].open_direction, Direction::Long);
    near(r.closed[0].pnl, -80.0);
    let r = fifo(vec![
        opt(json!({"id": "spy-bto", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 1, "unitPrice": 1.1, "netCashAmount": -110, "transactionDate": "2025-07-17", "symbol": "SPY 17JUL25 624.00 PUT"})),
        opt(json!({"id": "spy-exp", "activityType": "OPTIONS_EXPIRY", "activitySubType": "EXPIRED", "rawType": "OPTIONS_EXPIRY", "quantity": 1, "transactionDate": "2025-07-17", "symbol": "SPY 17JUL25 624.00 PUT"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
}

#[test]
fn test_debit_multileg_opens_long_and_sto_opens_short() {
    let r = fifo(vec![opt(json!({"id": "put-ml", "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": -90, "transactionDate": "2026-01-30", "symbol": "BBAI 30JAN26 6.00 PUT"}))]);
    assert!(r.unmatched.is_empty());
    assert_eq!(r.open.len(), 1);
    assert_eq!(r.open[0].direction, Direction::Long);
    let r = fifo(vec![opt(json!({"id": "sto-only", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -4, "unitPrice": 2, "netCashAmount": 800, "transactionDate": "2026-01-01", "symbol": "XYZ 15JAN27 5.00 CALL"}))]);
    assert!(r.unmatched.is_empty());
    assert_eq!(r.open.len(), 1);
    assert_eq!(r.open[0].direction, Direction::Short);
    assert_eq!(r.open[0].qty, 4.0);
}

#[test]
fn test_assignment_keeps_premium() {
    let r = fifo(vec![
        opt(json!({"id": "asts-sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -1, "unitPrice": 4.7475, "netCashAmount": 474.75, "transactionDate": "2025-01-15", "symbol": "ASTS 07MAR25 31.00 CALL"})),
        opt(json!({"id": "asts-asg", "category": "option_event", "activityType": "ASSIGN", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_ASSIGN", "quantity": 1, "unitPrice": 31, "netCashAmount": -3100, "transactionDate": "2025-03-07", "symbol": "ASTS 07MAR25 31.00 CALL"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].exit_price, 0.0);
    near(r.closed[0].pnl, 474.75);
}

#[test]
fn test_same_day_roll_folds_into_far_contract() {
    let r = fifo(vec![
        opt(json!({"id": "aug-sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -1, "unitPrice": 3, "netCashAmount": 300, "transactionDate": "2026-01-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
        opt(json!({"id": "aug-cover", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_BUY", "quantity": 1, "unitPrice": 1, "netCashAmount": -100, "transactionDate": "2026-08-15", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
        opt(json!({"id": "jan-sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -1, "unitPrice": 2, "netCashAmount": 200, "transactionDate": "2026-08-15", "symbol": "ZZZ 15JAN27 12.00 CALL"})),
        opt(json!({"id": "jan-cover", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_BUY", "quantity": 1, "unitPrice": 0.5, "netCashAmount": -50, "transactionDate": "2026-12-01", "symbol": "ZZZ 15JAN27 12.00 CALL"})),
    ]);
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].symbol, "ZZZ 15JAN27 12.00 CALL");
    near(r.closed[0].entry_price, 4.0);
    near(r.closed[0].pnl, 350.0);
    assert!(r.closed[0].flags.contains(&Flag::Rolled));
}

#[test]
fn test_stkdis_name_change_nets_to_zero() {
    let r = fifo(vec![
        buy("b", "OLD", 100, 2, "2026-01-01"),
        opt(json!({"id": "out", "category": "trade", "activityType": "STKDIS", "activitySubType": "SELL", "rawType": "CORPORATE_ACTION", "quantity": -100, "transactionDate": "2026-02-01", "symbol": "OLD", "currency": "CAD"})),
        opt(json!({"id": "in", "category": "trade", "activityType": "STKDIS", "activitySubType": "BUY", "rawType": "CORPORATE_ACTION", "quantity": 100, "transactionDate": "2026-02-01", "symbol": "NEW", "currency": "CAD"})),
        sell("s", "NEW", 100, 3, "2026-03-01"),
    ]);
    // Parity with ledger.html: the +N leg opens NEW at $0 and the sell
    // closes it; the OLD lot is only reused when NEW runs out of lots.
    assert!(r.unmatched.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].symbol, "NEW");
    near(r.closed[0].pnl, 300.0);
    assert_eq!(r.open.iter().map(|l| l.symbol.as_str()).collect::<Vec<_>>(), vec!["OLD"]);
    let r = fifo(vec![
        buy("b", "OLD", 100, 2, "2026-01-01"),
        opt(json!({"id": "out", "category": "trade", "activityType": "STKDIS", "activitySubType": "SELL", "rawType": "CODE_CHANGE", "quantity": -100, "transactionDate": "2026-02-01", "symbol": "OLD", "currency": "CAD"})),
        sell("s", "NEW", 100, 3, "2026-03-01"),
    ]);
    assert!(r.unmatched.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].symbol, "NEW");
    near(r.closed[0].pnl, 100.0);
    assert!(r.open.is_empty());
}

// --------------------------------------------------------------------------
// SplitTest
// --------------------------------------------------------------------------

/// Pins a known mistake of the old app (`docs/old-app-mistakes.md`: split
/// ratios inferred from fill prices); goes with the old model at the switch.
#[test]
fn test_known_wrong_split_ratio_read_from_fill_prices() {
    let acts = vec![
        buy("b1", "MSTY", 100, 7.0, "2025-12-01"),
        buy("b2", "MSTY", 75, 6.9, "2025-12-05"),
        opt(json!({"id": "ca", "category": "trade", "activityType": "STKDIS", "activitySubType": "BUY", "rawType": "CORPORATE_ACTION", "quantity": 0, "transactionDate": "2025-12-08", "symbol": "MSTY", "currency": "CAD"})),
        buy("b3", "MSTY", 4, 34.0, "2025-12-11"),
        sell("s1", "MSTY", 39, 31.0, "2026-01-16"),
    ];
    let r = match_fifo(&raws(&acts));
    assert!(r.unmatched.is_empty());
    assert!(r.open.is_empty());
    near(r.closed.iter().map(|t| t.quantity).sum(), 39.0);
    let first = r.closed.iter().min_by(|a, b| a.entry_date.cmp(&b.entry_date)).unwrap();
    near(first.entry_price, 35.0);
    near(pnl_sum(&r), 39.0 * 31.0 - (100.0 * 7.0 + 75.0 * 6.9 + 4.0 * 34.0));
}

#[test]
fn test_forward_split_and_no_marker_without_prices() {
    let acts = vec![
        buy_x("b1", "NVDA", 10, 1000.0, "2024-05-01", json!({"currency": "USD"})),
        opt(json!({"id": "ca", "category": "trade", "activityType": "STKDIS", "activitySubType": "BUY", "rawType": "CORPORATE_ACTION", "quantity": 0, "transactionDate": "2024-06-10", "symbol": "NVDA", "currency": "CAD"})),
        buy_x("b2", "NVDA", 5, 98.0, "2024-06-12", json!({"currency": "USD"})),
    ];
    let r = match_fifo(&raws(&acts));
    near(r.open.iter().map(|l| l.qty).sum(), 105.0);
    let big = r.open.iter().max_by(|a, b| a.qty.partial_cmp(&b.qty).unwrap()).unwrap();
    near(big.price, 100.0);
    assert!(big.flags.contains(&Flag::Split(10)));
    let r = match_fifo(&raws(&[
        buy("b1", "AAA", 10, 10.0, "2024-05-01"),
        opt(json!({"id": "ca", "category": "trade", "activityType": "STKDIS", "activitySubType": "BUY", "rawType": "CORPORATE_ACTION", "quantity": 0, "transactionDate": "2024-06-10", "symbol": "AAA", "currency": "CAD"})),
    ]));
    assert_eq!(r.open[0].qty, 10.0);
}

// --------------------------------------------------------------------------
// RoundTripTest
// --------------------------------------------------------------------------

/// The trades of some rows, as the page is sent them.
fn trades_of(acts: Vec<Value>, groups: Vec<Value>, journal: Value) -> Vec<Value> {
    let mut rows = normalize_all(&raws(&acts));
    let matched = match_fifo_in_place(&mut rows);
    let book = Book::of(rows, matched, Securities::default(), acts.len());
    let mut closed = book.fifo.closed.clone();
    apply_fx(&mut closed, &Fx::new());
    let groups = groups.iter().map(|g| serde_json::from_value(g.clone()).unwrap()).collect::<Vec<_>>();
    build_trades(&closed, &groups, &book, &journal_from(journal.as_object().unwrap())).iter().map(|t| serde_json::to_value(t).unwrap()).collect()
}

fn trades_plain(acts: Vec<Value>) -> Vec<Value> {
    trades_of(acts, vec![], json!({}))
}

#[test]
fn test_flat_to_flat_twice_is_two_trades() {
    let trades = trades_plain(vec![
        buy("b1", "AAA", 100, 10, "2026-01-01"),
        sell("s1", "AAA", 100, 12, "2026-01-10"),
        buy("b2", "AAA", 50, 11, "2026-02-01"),
        sell("s2", "AAA", 50, 9, "2026-02-10"),
    ]);
    assert_eq!(trades.len(), 2);
    let ids: HashSet<&str> = trades.iter().map(|t| st(&t["id"])).collect();
    assert_eq!(ids, HashSet::from(["rt:b1", "rt:b2"]));
    assert!(trades.iter().all(|t| t["status"] == "closed"));
    let pnl = by_key(&trades, "id");
    near(n(&pnl["rt:b1"]["pnl"]), 200.0);
    near(n(&pnl["rt:b2"]["pnl"]), -100.0);
}

#[test]
fn test_scale_in_and_out_is_one_trade_with_legs() {
    let trades = trades_plain(vec![
        buy("b1", "AAA", 100, 10, "2026-01-01"),
        sell("s1", "AAA", 50, 12, "2026-01-10"),
        buy("b2", "AAA", 100, 11, "2026-01-15"),
        sell("s2", "AAA", 150, 13, "2026-02-01"),
    ]);
    assert_eq!(trades.len(), 1);
    let t = &trades[0];
    assert_eq!(t["id"], "rt:b1");
    assert_eq!(t["status"], "closed");
    assert_eq!(n(&t["qty"]), 200.0);
    assert_eq!(n(&t["legCount"]), 3.0);
    assert_eq!(t["entryDate"], "2026-01-01");
    assert_eq!(t["exitDate"], "2026-02-01");
    near(n(&t["pnl"]), (50 * 2 + 50 * 3 + 100 * 2) as f64);
    assert_eq!(arr(&t["fills"]).len(), 4);
    assert_eq!(n(&t["opened"]["fills"]), 2.0);
    assert_eq!(n(&t["closed"]["fills"]), 2.0);
    assert_eq!(t["side"], "SELL");
}

#[test]
fn test_partial_exit_is_a_closed_trade_with_stable_id() {
    let mut acts = vec![buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 40, 12, "2026-01-10")];
    let trades = trades_plain(acts.clone());
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["status"], "closed");
    assert_eq!(trades[0]["id"], "rt:b1");
    assert_eq!(n(&trades[0]["qty"]), 40.0);
    acts.push(sell("s2", "AAA", 60, 15, "2026-02-01"));
    let trades = trades_plain(acts);
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["status"], "closed");
    assert_eq!(trades[0]["id"], "rt:b1");
    assert_eq!(n(&trades[0]["qty"]), 100.0);
}

#[test]
fn test_saved_group_overrides_round_trip() {
    let acts = vec![
        buy("b1", "AAA", 100, 10, "2026-01-01"),
        sell("s1", "AAA", 100, 12, "2026-01-10"),
        buy("b2", "AAA", 50, 11, "2026-02-01"),
        sell("s2", "AAA", 50, 9, "2026-02-10"),
    ];
    let key1 = format!("b1|s1|{:.8}", 100.0);
    let key2 = format!("b2|s2|{:.8}", 50.0);
    let trades = trades_of(acts, vec![json!({"id": "g_manual", "locked": true, "members": [key1, key2]})], json!({}));
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["id"], "g_manual");
    assert_eq!(trades[0]["locked"], true);
    assert_eq!(n(&trades[0]["legCount"]), 2.0);
}

#[test]
fn test_position_notes_carry_over_to_the_closed_trade() {
    let mut snapshot = snap(vec![buy("b1", "AAA", 100, 10, "2026-01-01")]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-02-01");
    let pid = st(&sent(&base.positions)[0]["id"]).to_string();
    assert_eq!(pid, "rt:b1");
    let journal = json!({pid.clone(): {"thesis": "holding for the catalyst", "tags": ["core"], "grade": ""}});
    let base = base_of(&snapshot, empty_market(), journal.clone(), "2026-02-01");
    assert_eq!(sent(&base.positions)[0]["thesis"], "holding for the catalyst");
    snapshot["activities"].as_array_mut().unwrap().push(sell("s1", "AAA", 100, 12, "2026-03-01"));
    let base = base_of(&snapshot, empty_market(), journal, "2026-04-01");
    assert!(base.positions.is_empty());
    assert_eq!(sent(&base.trades)[0]["id"], "rt:b1");
    assert_eq!(sent(&base.trades)[0]["thesis"], "holding for the catalyst");
    assert_eq!(sent(&base.trades)[0]["tags"], json!(["core"]));
}

#[test]
fn test_journal_attaches_to_trade() {
    let trades = trades_of(
        vec![buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")],
        vec![],
        json!({"rt:b1": {"thesis": "breakout", "tags": ["momo"], "grade": "A"}}),
    );
    assert_eq!(trades[0]["grade"], "A");
    assert_eq!(trades[0]["tags"], json!(["momo"]));
    assert_eq!(trades[0]["thesis"], "breakout");
}

fn subs(t: &Value) -> HashMap<String, String> {
    arr(&t["fills"]).iter().map(|f| (st(&f["id"]).to_string(), st(&f["sub"]).to_string())).collect()
}

#[test]
fn test_fill_labels_reflect_what_the_fill_did() {
    let trades = trades_plain(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -2, "unitPrice": 3, "netCashAmount": 600, "transactionDate": "2026-01-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
        opt(json!({"id": "buy", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 1, "netCashAmount": -200, "transactionDate": "2026-02-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
    ]);
    assert_eq!(subs(&trades[0]), HashMap::from([("sto".into(), "SELL TO OPEN".into()), ("buy".into(), "BUY TO CLOSE".into())]));
    // Shares are bought and sold; the open/close order types are option language.
    let trades = trades_plain(vec![buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")]);
    assert_eq!(subs(&trades[0]), HashMap::from([("b1".into(), "BUY".into()), ("s1".into(), "SELL".into())]));
}

#[test]
fn test_short_round_trip_is_cover() {
    let trades = trades_plain(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -2, "unitPrice": 3, "netCashAmount": 600, "transactionDate": "2026-01-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
        opt(json!({"id": "btc", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 1, "netCashAmount": -200, "transactionDate": "2026-02-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
    ]);
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["side"], "COVER");
    assert_eq!(trades[0]["kind"], "Options");
    assert_eq!(n(&trades[0]["mult"]), 100.0);
    near(n(&trades[0]["pnl"]), 400.0);
    near(n(&trades[0]["pnlPct"]), 400.0 / 600.0);
}

// --------------------------------------------------------------------------
// ExpiryTest, AssignmentTest
// --------------------------------------------------------------------------

#[test]
fn test_option_expiry_parse() {
    assert_eq!(option_expiry("LUNR 29AUG25 11.50 CALL"), "2025-08-29");
    assert_eq!(option_expiry("BBAI 02JAN26 5.50 PUT"), "2026-01-02");
    assert_eq!(option_expiry("AAPL"), "");
}

#[test]
fn test_open_option_past_expiry_is_closed_at_zero() {
    let snapshot = snap(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -2, "unitPrice": 0.3, "netCashAmount": 60, "transactionDate": "2025-12-05", "symbol": "BBAI 02JAN26 5.50 PUT"})),
        opt(json!({"id": "bto", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOOPEN", "rawType": "OPTIONS_BUY", "quantity": 1, "unitPrice": 1.0, "netCashAmount": -100, "transactionDate": "2026-01-05", "symbol": "ZZZ 17JUL26 10.00 CALL"})),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-03-01");
    assert_eq!(base.book.fifo.open.len(), 1);
    assert_eq!(base.book.fifo.open[0].symbol, "ZZZ 17JUL26 10.00 CALL");
    assert_eq!(base.trades.len(), 1);
    let t = &sent(&base.trades)[0];
    assert_eq!(t["exitDate"], "2026-01-02");
    assert_eq!(n(&t["exit"]), 0.0);
    near(n(&t["pnl"]), 60.0);
    assert!(arr(&t["flags"]).contains(&json!("assumed-expiry")));
    assert_eq!(t["status"], "closed");
}

#[test]
fn test_assigned_call_delivers_the_shares() {
    let mut snapshot = snap(vec![
        buy_x("b1", "ASTS", 300, 25.0, "2025-01-10", json!({"currency": "USD", "securityId": "sec-s-asts"})),
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -3, "unitPrice": 1.5, "netCashAmount": 450, "transactionDate": "2025-02-10", "symbol": "ASTS 07MAR25 31.00 CALL", "securityId": "sec-o-asts"})),
        opt(json!({"id": "asg", "category": "option_event", "activityType": "ASSIGN", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_ASSIGN", "quantity": 3, "unitPrice": 0, "netCashAmount": 9300, "transactionDate": "2025-03-07", "symbol": "ASTS 07MAR25 31.00 CALL", "securityId": "sec-o-asts"})),
    ]);
    snapshot["securities"] = json!([{"id": "sec-o-asts", "symbol": "ASTS", "underlyingId": "sec-s-asts"}, {"id": "sec-s-asts", "symbol": "ASTS", "name": "AST SpaceMobile", "primaryExchange": "NASDAQ"}]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-06");
    assert!(base.book.fifo.open.is_empty());
    let trades = sent(&base.trades);
    let by_sym = by_key(&trades, "symbol");
    let shares = by_sym["ASTS"];
    assert_eq!(n(&shares["qty"]), 300.0);
    assert_eq!(n(&shares["exit"]), 31.0);
    assert_eq!(shares["exitDate"], "2025-03-07");
    near(n(&shares["pnl"]), (31.0 - 25.0) * 300.0);
    assert!(arr(&shares["flags"]).contains(&json!("assignment")));
    assert_eq!(shares["name"], "AST SpaceMobile");
    near(n(&by_sym["ASTS 07MAR25 31.00 CALL"]["pnl"]), 450.0);
}

#[test]
fn test_assigned_put_buys_the_shares() {
    let snapshot = snap(vec![
        opt(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -1, "unitPrice": 0.5, "netCashAmount": 50, "transactionDate": "2025-11-10", "symbol": "BBAI 05DEC25 5.00 PUT"})),
        opt(json!({"id": "asg", "category": "option_event", "activityType": "ASSIGN", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_ASSIGN", "quantity": 1, "unitPrice": 0, "netCashAmount": -500, "transactionDate": "2025-12-05", "symbol": "BBAI 05DEC25 5.00 PUT"})),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-01-01");
    let lots: Vec<(String, f64, f64)> = base.book.fifo.open.iter().map(|l| (l.symbol.clone(), l.qty, l.price)).collect();
    assert_eq!(lots, vec![("BBAI".to_string(), 100.0, 5.0)]);
    assert!(base.book.fifo.open[0].flags.contains(&Flag::Assignment));
}

// --------------------------------------------------------------------------
// CryptoTest
// --------------------------------------------------------------------------

fn eth_transfer(id: &str, qty: f64, value: f64, day: &str, out: bool) -> Value {
    opt(json!({"id": id, "activityType": "CRYPTO_TRANSFER", "activitySubType": if out { "TRANSFER_OUT" } else { "TRANSFER_IN" },
        "rawType": "CRYPTO_TRANSFER", "direction": if out { "DEBIT" } else { "CREDIT" }, "quantity": qty, "unitPrice": value / qty,
        "netCashAmount": if out { -value } else { value }, "transactionDate": day, "symbol": "ETH", "currency": "CAD", "accountType": "Ponzi"}))
}

#[test]
fn test_a_transfer_out_leaves_at_cost_with_no_pnl() {
    let acts = vec![
        opt(json!({"id": "cb", "activityType": "CRYPTO_BUY", "activitySubType": "MARKET_ORDER", "rawType": "CRYPTO_BUY", "quantity": 2, "unitPrice": 100, "netCashAmount": 200, "transactionDate": "2026-01-01", "symbol": "ETH", "currency": "CAD", "accountType": "Ponzi"})),
        eth_transfer("ti", 1.0, 120.0, "2026-01-05", false),
        eth_transfer("to", 1.0, 200.0, "2026-01-10", true), // would be +100 as a sale
        opt(json!({"id": "cs", "activityType": "CRYPTO_SELL", "activitySubType": "MARKET_ORDER", "rawType": "CRYPTO_SELL", "quantity": 2, "unitPrice": 150, "netCashAmount": 300, "transactionDate": "2026-02-01", "symbol": "ETH", "currency": "CAD", "accountType": "Ponzi"})),
    ];
    let m = match_fifo(&raws(&acts));
    assert!(m.unmatched.is_empty());
    assert!(m.open.is_empty());
    let got: Vec<(f64, f64, f64)> = m.closed.iter().map(|s| ((s.pnl * 1e6).round() / 1e6, s.quantity, s.entry_price)).collect();
    assert_eq!(got, vec![(50.0, 1.0, 100.0), (30.0, 1.0, 120.0)], "the coin sent out came off the first lot at cost; the sale closed one at 100 and one at 120");
    let trades = trades_plain(acts.clone());
    assert_eq!(trades.len(), 2, "the deposited coin is its own unscoreable trade, not merged");
    let is_dep = |t: &Value| t.get("flags").and_then(|v| v.as_array()).map(|a| a.iter().any(|f| f == "basis-unknown")).unwrap_or(false);
    let bought = trades.iter().find(|t| !is_dep(t)).unwrap();
    let deposited = trades.iter().find(|t| is_dep(t)).unwrap();
    assert_eq!(((n(&bought["pnl"]) * 1e6).round() / 1e6, n(&bought["qty"])), (50.0, 1.0), "coin bought here scores 50");
    assert_eq!(((n(&deposited["pnl"]) * 1e6).round() / 1e6, n(&deposited["qty"])), (30.0, 1.0), "deposited coin's sale is flagged, not scored");
    assert!(!trades.iter().any(|t| arr(&t["fills"]).iter().any(|f| f["id"] == "to")), "the transfer out is not a fill of the trade");
    // nothing held: nothing to take off, nothing unmatched, no trade
    let m = match_fifo(&raws(&[eth_transfer("to2", 1.0, 200.0, "2026-01-10", true)]));
    assert!(m.closed.is_empty() && m.open.is_empty() && m.unmatched.is_empty());
}

#[test]
fn test_crypto_buy_sell_and_reward() {
    let acts = vec![
        opt(json!({"id": "cb", "activityType": "CRYPTO_BUY", "activitySubType": "MARKET_ORDER", "rawType": "CRYPTO_BUY", "quantity": 2, "unitPrice": 100, "netCashAmount": 200, "transactionDate": "2026-01-01", "symbol": "ETH", "currency": "CAD", "accountType": "Ponzi"})),
        opt(json!({"id": "rw", "activityType": "CRYPTO_STAKING_REWARD", "activitySubType": "other", "rawType": "CRYPTO_STAKING_REWARD", "quantity": 1, "unitPrice": 0, "netCashAmount": 0, "transactionDate": "2026-01-05", "symbol": "ETH", "currency": "CAD", "accountType": "Ponzi"})),
        opt(json!({"id": "cs", "activityType": "CRYPTO_SELL", "activitySubType": "MARKET_ORDER", "rawType": "CRYPTO_SELL", "quantity": 3, "unitPrice": 150, "netCashAmount": 450, "transactionDate": "2026-02-01", "symbol": "ETH", "currency": "CAD", "accountType": "Ponzi"})),
    ];
    let norm = normalize_all(&raws(&acts));
    assert_eq!(norm[0].kind, Kind::Crypto);
    assert!(norm[0].net_cash_amount < 0.0);
    assert!(norm[1].flags.contains(&Flag::Reward));
    let m = match_fifo(&raws(&acts));
    assert!(m.unmatched.is_empty());
    assert!(m.open.is_empty());
    assert_eq!(m.closed.len(), 2);
    near(pnl_sum(&m), (150.0 - 100.0) * 2.0 + 150.0);
    assert!(m.closed.iter().all(|t| t.kind == Kind::Crypto));
}

#[test]
fn test_crypto_dust_sell_is_not_unmatched() {
    let acts = vec![
        opt(json!({"id": "cb", "activityType": "CRYPTO_BUY", "rawType": "CRYPTO_BUY", "quantity": 1.0, "unitPrice": 100, "netCashAmount": 100, "transactionDate": "2026-01-01", "symbol": "DOGE", "currency": "CAD"})),
        opt(json!({"id": "cs", "activityType": "CRYPTO_SELL", "rawType": "CRYPTO_SELL", "quantity": 1.0000004, "unitPrice": 120, "netCashAmount": 120, "transactionDate": "2026-02-01", "symbol": "DOGE", "currency": "CAD"})),
    ];
    let m = match_fifo(&raws(&acts));
    assert!(m.unmatched.is_empty());
    assert_eq!(m.closed.len(), 1);
}

#[test]
fn test_pending_distribution_notice_is_not_a_lot() {
    let acts = vec![
        buy("b", "RDDY", 100, 9, "2026-01-01"),
        opt(json!({"id": "stk", "category": "trade", "activityType": "STKDIS", "activitySubType": "BUY", "rawType": "DIVIDEND", "quantity": 100, "unitPrice": 0, "netCashAmount": 0, "transactionDate": "2026-02-01", "symbol": "RDDY", "currency": "CAD"})),
    ];
    let m = match_fifo(&raws(&acts));
    assert_eq!(m.open.len(), 1);
    assert_eq!(m.open[0].qty, 100.0);
    assert_eq!(m.open[0].price, 9.0);
}

// --------------------------------------------------------------------------
// FxTest
// --------------------------------------------------------------------------

fn fx_of(pairs: &[(&str, f64)]) -> Fx {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

#[test]
fn test_usd_pnl_uses_rates_on_fill_dates() {
    let fx = fx_of(&[("2026-01-05", 1.40), ("2026-02-05", 1.30)]);
    let mut m = fifo(vec![
        buy_x("b", "LUNR", 100, 10, "2026-01-05", json!({"currency": "USD"})),
        sell_x("s", "LUNR", 100, 12, "2026-02-05", json!({"currency": "USD"})),
    ]);
    apply_fx(&mut m.closed, &fx);
    let t = &m.closed[0];
    near(t.pnl, 200.0);
    near(t.pnl_cad, 1200.0 * 1.30 - 1000.0 * 1.40);
}

#[test]
fn test_rate_walks_back_over_weekends_and_falls_back() {
    let fx = fx_of(&[("2026-01-02", 1.40)]);
    assert_eq!(rate_on(&fx, "2026-01-04"), 1.40);
    assert_eq!(rate_on(&fx, "2025-06-01"), FX_FALLBACK);
    assert_eq!(to_cad(&fx, 100.0, "CAD", "2026-01-04"), 100.0);
}

// --------------------------------------------------------------------------
// ViewTest
// --------------------------------------------------------------------------

fn view_base() -> Base {
    let t = json!({"accountType": "Trading"});
    let snapshot = json!({
        "activities": [
            buy_x("b1", "AAA", 100, 10, "2025-03-01", t.clone()),
            sell_x("s1", "AAA", 100, 12, "2025-03-10", t.clone()),
            buy_x("b2", "BBB", 10, 100, "2026-01-05", t.clone()),
            sell_x("s2", "BBB", 10, 90, "2026-01-20", t.clone()),
            buy_x("b3", "CCC", 10, 5, "2026-02-01", json!({"accountType": "Retirement"})),
            sell_x("s3", "CCC", 10, 6, "2026-02-15", json!({"accountType": "Retirement"})),
            buy_x("b4", "DDD", 10, 5, "2026-03-01", t.clone()),
            buy_x("b5", "LUNR", 10, 10, "2026-03-01", json!({"accountType": "Trading", "currency": "USD"})),
            sell_x("s5", "LUNR", 10, 11, "2026-03-05", json!({"accountType": "Trading", "currency": "USD"})),
        ],
        "accounts": [
            {"id": "acct-1", "nickname": "Trading", "unifiedAccountType": "TFSA", "currency": "CAD"},
            {"id": "acct-2", "nickname": "Retirement", "unifiedAccountType": "RRSP", "currency": "CAD"},
        ],
        "balances": [],
        "navHistory": [
            {"date": "2024-12-31", "equity": 1000, "netDeposits": 1000},
            {"date": "2025-06-30", "equity": 1500, "netDeposits": 1200},
            {"date": "2025-12-31", "equity": 1600, "netDeposits": 1200},
            {"date": "2026-03-31", "equity": 1400, "netDeposits": 1200},
        ],
        "navByAccount": {"Trading": [{"date": "2025-12-31", "equity": 800, "netDeposits": 500}, {"date": "2026-03-31", "equity": 700, "netDeposits": 500}]},
        "syncedAt": "2026-04-01T00:00:00Z",
        "tradeGroups": [],
        "notes": {},
        "securities": [],
    });
    let market = json!({"fx": {"2026-03-01": 1.4, "2026-03-05": 1.3}, "benchmark": {"2024-12-31": 100, "2025-12-31": 120, "2026-03-31": 126}});
    let journal = json!({"rt:b1": {"thesis": "yes", "tags": ["x"], "grade": "A"}, "rt:b2": {"thesis": "", "tags": [], "grade": "F"}});
    base_of(&snapshot, market, journal, "2026-04-01")
}

fn symbols(v: &Value) -> Vec<String> {
    arr(&v["trades"]).iter().map(|t| st(&t["symbol"]).to_string()).collect()
}

fn symbol_set(v: &Value) -> HashSet<String> {
    symbols(v).into_iter().collect()
}

fn set(v: &[&str]) -> HashSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_one_list_feeds_every_tile() {
    let base = view_base();
    let v = build_view(&base, None);
    let k = &v["kpi"];
    assert_eq!(n(&k["count"]), 4.0);
    assert_eq!(arr(&v["trades"]).len(), 4);
    near(n(&k["realized"]), arr(&v["trades"]).iter().map(|t| n(&t["pnlCad"])).sum());
    near(arr(&v["monthly"]).iter().map(|m| n(&m["value"])).sum(), n(&k["realized"]));
    near(arr(&v["bySymbol"]).iter().map(|r| n(&r["pnl"])).sum(), n(&k["realized"]));
    let g = &v["grades"];
    assert_eq!(arr(&g["buckets"]).iter().map(|b| n(&b["n"])).sum::<f64>() + n(&g["ungraded"]), n(&k["count"]));
    assert_eq!(arr(&v["bySymbol"]).iter().map(|r| n(&r["n"])).sum::<f64>(), n(&k["count"]));
    assert_eq!(n(&k["wins"]) + n(&k["losses"]) + n(&k["breakeven"]), n(&k["count"]));
    assert_eq!(arr(&v["queue"]).len(), 3);
    let usd = arr(&v["trades"]).iter().find(|t| t["symbol"] == "LUNR").unwrap();
    near(n(&usd["pnl"]), 10.0);
    near(n(&usd["pnlCad"]), 110.0 * 1.3 - 100.0 * 1.4);
}

#[test]
fn test_positions_and_options() {
    let base = view_base();
    let v = build_view(&base, None);
    assert_eq!(arr(&v["positions"]).len(), 1);
    let p = &v["positions"][0];
    assert_eq!(p["symbol"], "DDD");
    assert_eq!(p["priceSource"], "fill");
    assert_eq!(n(&p["alloc"]), 1.0);
    assert_eq!(n(&p["held"]), 31.0);
    assert_eq!(v["options"]["accounts"], json!(["Retirement", "Trading"]));
    assert_eq!(v["options"]["kinds"], json!(["Shares"]));
    assert_eq!(v["options"]["tags"], json!(["x"]));
    let listings = v["options"]["listings"].as_object().unwrap();
    let mut keys: Vec<&String> = listings.keys().collect();
    keys.sort();
    assert_eq!(json!(keys), v["options"]["symbols"], "one listing per symbol the ⌘K list can show");
    assert_eq!(listings["DDD"]["exchange"], p["exchange"]);
    assert_eq!(listings["DDD"]["kind"], "Shares");
    assert!(listings["DDD"].get("name").is_some());
}

#[test]
fn test_account_filter_narrows_everything() {
    let base = view_base();
    let v = build_view(&base, Some(&json!({"lists": {"account": ["Retirement"]}})));
    assert_eq!(n(&v["kpi"]["count"]), 1.0);
    assert_eq!(v["trades"][0]["symbol"], "CCC");
    assert_eq!(v["positions"], json!([]));
    assert_eq!(v["equity"]["label"], "All accounts");
    let v = build_view(&base, Some(&json!({"lists": {"account": ["Trading"]}})));
    assert_eq!(v["equity"]["label"], "Trading");
    assert_eq!(arr(&v["positions"]).len(), 1);
}

#[test]
fn test_date_filters() {
    let base = view_base();
    assert_eq!(symbols(&build_view(&base, Some(&json!({"years": ["2025"]})))), strs(&["AAA"]));
    assert_eq!(symbol_set(&build_view(&base, Some(&json!({"preset": "ytd"})))), set(&["BBB", "CCC", "LUNR"]));
    assert_eq!(symbols(&build_view(&base, Some(&json!({"from": "2026-02-01", "to": "2026-02-28"})))), strs(&["CCC"]));
    assert_eq!(symbols(&build_view(&base, Some(&json!({"preset": "1m"})))), strs(&["LUNR"]));
}

#[test]
fn test_list_and_range_filters() {
    let base = view_base();
    assert_eq!(symbols(&build_view(&base, Some(&json!({"lists": {"grade": ["A"]}})))), strs(&["AAA"]));
    assert_eq!(symbol_set(&build_view(&base, Some(&json!({"lists": {"grade": ["Ungraded"]}})))), set(&["CCC", "LUNR"]));
    assert_eq!(symbols(&build_view(&base, Some(&json!({"lists": {"result": ["Losers"]}})))), strs(&["BBB"]));
    assert_eq!(symbols(&build_view(&base, Some(&json!({"ranges": {"price": {"op": ">", "v": 50}}})))), strs(&["BBB"]));
    assert_eq!(symbols(&build_view(&base, Some(&json!({"search": "aa"})))), strs(&["AAA"]));
    assert_eq!(n(&build_view(&base, Some(&json!({"lists": {"tag": ["untagged"]}})))["kpi"]["count"]), 3.0);
}

#[test]
fn test_returns_and_drawdown() {
    let base = view_base();
    let v = build_view(&base, None);
    let years = by_key(arr(&v["years"]), "year");
    // 2025: two steps, (1500-1000-200)/1000 then (1600-1500)/1500
    near(n(&years["2025"]["r"]), (1.0 + 0.3) * (1.0 + 100.0 / 1500.0) - 1.0);
    near(n(&years["2025"]["spR"]), 0.2);
    near(n(&years["2025"]["flow"]), 200.0);
    near(n(&years["2026"]["r"]), (1400.0 - 1600.0) / 1600.0);
    near(n(&years["2026"]["spR"]), 0.05);
    let dd = &v["equity"]["drawdown"];
    near(n(&dd["pct"]), (1400.0 - 1600.0) / 1600.0);
    assert_eq!(dd["at"], "2026-03-31");
    assert!(!v["equity"]["annualized"]["rate"].is_null());
}

#[test]
fn test_drawdown_ignores_withdrawals_and_deposits() {
    let series = equity_series(arr(&json!([
        {"date": "2026-01-01", "equity": 100000, "netDeposits": 100000},
        {"date": "2026-01-02", "equity": 101000, "netDeposits": 100000},
        {"date": "2026-01-03", "equity": 21000, "netDeposits": 20000},
        {"date": "2026-01-04", "equity": 21210, "netDeposits": 20000},
        {"date": "2026-01-05", "equity": 41210, "netDeposits": 40000},
        {"date": "2026-01-06", "equity": 37089, "netDeposits": 40000},
    ])));
    let dd = drawdown(&series);
    near_p(n(&dd["pct"]), -0.1, 4);
    assert_eq!(dd["at"], "2026-01-06");
    assert_eq!(dd["peakAt"], "2026-01-05");
    assert!((n(&dd["abs"]) - -4121.0).abs() <= 1.0);
}

#[test]
fn test_negligible_years_are_skipped() {
    let series = equity_series(arr(&json!([
        {"date": "2020-12-22", "equity": 0, "netDeposits": 0},
        {"date": "2020-12-23", "equity": 100, "netDeposits": 100},
        {"date": "2020-12-31", "equity": 101, "netDeposits": 100},
        {"date": "2023-06-30", "equity": 45000, "netDeposits": 40000},
        {"date": "2023-12-31", "equity": 50000, "netDeposits": 40000},
        {"date": "2024-12-31", "equity": 60000, "netDeposits": 40000},
    ])));
    let years = yearly_returns(&series, &BTreeMap::new(), "2025-01-01");
    assert_eq!(years.iter().map(|y| st(&y["year"])).collect::<Vec<_>>(), vec!["2023", "2024"]);
    assert_eq!(years[0]["from"], "2023-06-30", "2023 is measured from the first funded point, not from the $101 of 2020");
    near_p(n(&years[0]["r"]), 50000.0 / 45000.0 - 1.0, 6);
}

#[test]
fn test_year_starts_where_the_account_was_really_funded() {
    let series = equity_series(arr(&json!([
        {"date": "2023-09-06", "equity": 0, "netDeposits": 15},
        {"date": "2023-09-13", "equity": 1666, "netDeposits": 1682},
        {"date": "2023-09-20", "equity": 111771, "netDeposits": 112806},
        {"date": "2023-10-04", "equity": 116832, "netDeposits": 119270},
        {"date": "2023-12-27", "equity": 135232, "netDeposits": 125411},
        {"date": "2024-06-30", "equity": 174611, "netDeposits": 125411},
        {"date": "2024-12-31", "equity": 170000, "netDeposits": 125411},
    ])));
    let yr = year_return(&series, "2023", "2025-01-01").unwrap();
    assert_eq!(yr.from, "2023-09-20");
    assert!((yr.r - (116832.0 - 111771.0 - 6464.0) / 111771.0).abs() <= 0.2);
    assert!(yr.r > 0.05);
    assert!(yr.r < 0.15);
    let bench: BTreeMap<String, f64> = [("2022-12-30", 3800.0), ("2023-09-19", 4400.0), ("2023-12-29", 4770.0), ("2024-12-31", 5880.0)]
        .iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let years = yearly_returns(&series, &bench, "2025-01-01");
    let by = by_key(&years, "year");
    near_p(n(&by["2023"]["spR"]), 4770.0 / 4400.0 - 1.0, 6);
    near_p(n(&by["2024"]["spR"]), 5880.0 / 4770.0 - 1.0, 6);
}

#[test]
fn test_yearly_returns_compare_against_the_chosen_index() {
    let mut market = json!({"fx": {}, "benchmark": {"2023-12-29": 100.0, "2024-12-31": 110.0}, "benchmarks": {"SP500": {"2023-12-29": 100.0, "2024-12-31": 110.0}, "TSX": {"2023-12-29": 200.0, "2024-12-31": 250.0}}});
    let mut snapshot = snap(vec![]);
    snapshot["navHistory"] = json!([{"date": "2023-12-31", "equity": 100000, "netDeposits": 100000}, {"date": "2024-12-31", "equity": 120000, "netDeposits": 100000}]);
    let base = base_of(&snapshot, market.clone(), json!({}), "2025-01-01");
    assert_eq!(clean_filters(Some(&json!({"benchmark": "tsx"}))).benchmark, "TSX");
    assert_eq!(clean_filters(Some(&json!({"benchmark": "tsx60"}))).benchmark, "TSX60");
    assert_eq!(clean_filters(Some(&json!({"benchmark": "nope"}))).benchmark, "SP500");
    let last = |v: &Value, k: &str| n(&arr(&v["years"]).last().unwrap()[k]);
    let v = build_view(&base, Some(&json!({})));
    assert_eq!(v["benchmark"], json!({"key": "SP500", "label": "S&P 500"}));
    near_p(last(&v, "spR"), 0.10, 6);
    let v = build_view(&base, Some(&json!({"benchmark": "TSX"})));
    assert_eq!(v["benchmark"], json!({"key": "TSX", "label": "S&P/TSX"}));
    near_p(last(&v, "spR"), 0.25, 6);
    market["benchmarks"]["TSX60"] = json!({"2023-12-29": 100.0, "2024-12-31": 115.0});
    let base = base_of(&snapshot, market, json!({}), "2025-01-01");
    let v = build_view(&base, Some(&json!({"benchmark": "TSX60"})));
    assert_eq!(v["benchmark"], json!({"key": "TSX60", "label": "TSX 60"}));
    near_p(last(&v, "spR"), 0.15, 6);
    near_p(last(&v, "r"), 0.20, 6);
}

fn div_row(id: &str, sym: &str, qty: f64, per: f64, cash: f64, day: &str, account: &str) -> Value {
    opt(json!({"id": id, "category": "dividend", "activityType": "Dividend", "rawType": "DIVIDEND", "quantity": qty, "unitPrice": per, "netCashAmount": cash, "transactionDate": day, "symbol": sym, "currency": "CAD", "accountType": account}))
}

fn cf_buy(id: &str, sym: &str, qty: i64, px: impl Into<Value>, day: &str) -> Value {
    buy_x(id, sym, qty, px, day, json!({"accountType": "Cashflow"}))
}

fn next_dates(base: &Base) -> HashMap<String, (String, String, bool, bool)> {
    let v = build_view(base, Some(&json!({})));
    arr(&v["cashflow"]["holdings"]).iter().map(|h| (
        st(&h["symbol"]).to_string(),
        (st(&h["nextExDate"]).to_string(), st(&h["nextPayDate"]).to_string(), h["exPast"].as_bool().unwrap(), h["payPast"].as_bool().unwrap()),
    )).collect()
}

fn dates(ex: &str, pay: &str, exp: bool, payp: bool) -> (String, String, bool, bool) {
    (ex.into(), pay.into(), exp, payp)
}

#[test]
fn test_ex_div_and_pay_day_next_declared_else_last_known() {
    let snapshot = snap(vec![
        cf_buy("b1", "RDDY", 100, 5, "2026-05-01"),
        div_row("d1", "RDDY", 100.0, 0.2, 20.0, "2026-08-06", "Cashflow"),
        cf_buy("b2", "HHIS", 100, 5, "2026-05-01"),
        div_row("d2", "HHIS", 100.0, 0.2, 20.0, "2026-08-06", "Cashflow"),
        cf_buy("b3", "HBIX", 100, 5, "2026-05-01"),
        div_row("d3", "HBIX", 100.0, 0.2, 20.0, "2026-08-06", "Cashflow"),
        cf_buy("b4", "EASY", 100, 20, "2026-05-01"),
        div_row("d4", "EASY", 100.0, 0.31, 31.0, "2026-08-21", "Cashflow"),
    ]);
    let market = json!({"fx": {}, "benchmark": {},
        "distributions": {
            "RDDY": [{"exDate": "2026-09-30", "payDate": "2026-10-06", "amount": 0.15, "currency": "CAD"}, {"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.15, "currency": "CAD"}],
            "HHIS": [{"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.27, "currency": "CAD"}],
            // EASY pays twice a month: gone ex on the 31st, paid on the 8th, ex again on the 15th.
            "EASY": [{"exDate": "2026-08-31", "payDate": "2026-09-08", "amount": 0.255, "currency": "CAD"}, {"exDate": "2026-09-15", "payDate": "2026-09-22", "amount": 0.255, "currency": "CAD"}],
        },
        "quotes": {"HHIS": {"price": 11.0, "exDividendDate": "2026-09-29"}, "HBIX": {"price": 6.7, "exDividendDate": "2026-08-29"}}});
    let by = next_dates(&base_of(&snapshot, market.clone(), json!({}), "2026-09-07"));
    assert_eq!(by["RDDY"], dates("2026-09-30", "2026-10-06", false, false), "the declared record's next distribution, with its pay date");
    assert_eq!(by["EASY"], dates("2026-08-31", "2026-09-08", true, false), "gone ex but not yet paid: that distribution, not the one after it");
    assert_eq!(by["HHIS"], dates("2026-08-31", "2026-09-04", true, true), "nothing left to pay: the last known one, both dates passed");
    assert_eq!(by["HBIX"], dates("2026-08-29", "2026-08-06", true, true), "no record: the quote's last ex-date and the last payment received");
    let by = next_dates(&base_of(&snapshot, market.clone(), json!({}), "2026-09-08"));
    assert_eq!(by["EASY"], dates("2026-08-31", "2026-09-08", true, false), "pay day itself still counts as ahead");
    let by = next_dates(&base_of(&snapshot, market, json!({}), "2026-09-09"));
    assert_eq!(by["EASY"], dates("2026-09-15", "2026-09-22", false, false), "once paid, the next one");
}

#[test]
fn test_monthly_distributions_run_to_the_current_month() {
    let snapshot = snap(vec![
        cf_buy("b1", "RDDY", 100, 5, "2026-05-01"),
        div_row("d1", "RDDY", 100.0, 0.2, 20.0, "2026-06-06", "Cashflow"),
        div_row("d2", "RDDY", 100.0, 0.2, 20.0, "2026-07-06", "Cashflow"),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-07");
    let v = build_view(&base, Some(&json!({})));
    let months: Vec<(String, f64)> = arr(&v["cashflow"]["months"]).iter().map(|m| (st(&m["key"]).to_string(), n(&m["count"]))).collect();
    assert_eq!(months, vec![("2026-06".into(), 1.0), ("2026-07".into(), 1.0), ("2026-08".into(), 0.0), ("2026-09".into(), 0.0)], "empty bars up to the current month");
    let tiles: Vec<&Value> = arr(&v["cashflow"]["tiles"]).iter().filter(|t| t.get("perMonth").is_some()).collect();
    let ytd = tiles.iter().find(|t| t["label"] == "2026 YTD").unwrap();
    assert_eq!(n(&ytd["perMonth"]), 20.0, "the monthly average counts paying months only");
    let v = build_view(&base, Some(&json!({"to": "2026-08-15"})));
    let keys: Vec<&str> = arr(&v["cashflow"]["months"]).iter().map(|m| st(&m["key"])).collect();
    assert_eq!(keys, vec!["2026-06", "2026-07", "2026-08"], "a date filter ends the chart at its bound");
    let base25 = base_of(&snapshot, empty_market(), json!({}), "2027-03-01");
    let v = build_view(&base25, Some(&json!({"years": ["2026"]})));
    assert_eq!(arr(&v["cashflow"]["months"]).last().unwrap()["key"], "2026-12", "a year filter ends the chart at December");
}

#[test]
fn test_cashflow_tiles_roll_over_with_the_calendar() {
    let snapshot = snap(vec![
        cf_buy("b1", "RDDY", 100, 5, "2025-06-01"),
        div_row("d1", "RDDY", 100.0, 0.2, 20.0, "2025-07-06", "Cashflow"),
        div_row("d2", "RDDY", 100.0, 0.2, 20.0, "2026-07-06", "Cashflow"),
    ]);
    let tiles = |today: &str| build_view(&base_of(&snapshot, empty_market(), json!({}), today), Some(&json!({})))["cashflow"]["tiles"].clone();
    let labels = |today: &str| arr(&tiles(today)).iter().map(|t| st(&t["label"]).to_string()).collect::<Vec<_>>();
    assert_eq!(labels("2026-09-07"), strs(&["2024", "2025", "2026 YTD", "All time", "Last 12 months", "Yield on cost"]), "no margin account: Last 12 months stands in");
    assert_eq!(labels("2027-01-01"), strs(&["2025", "2026", "2027 YTD", "All time", "Last 12 months", "Yield on cost"]));
    let t = tiles("2027-01-01");
    let totals: HashMap<String, f64> = arr(&t).iter().filter(|t| t.get("total").is_some()).map(|t| (st(&t["label"]).to_string(), n(&t["total"]).round())).collect();
    assert_eq!((totals["2026"], totals["2027 YTD"], totals["All time"]), (20.0, 0.0, 40.0));
}

#[test]
fn test_filters_are_cleaned() {
    let f = serde_json::to_value(clean_filters(Some(&json!({"lists": {"account": ["A", 3, ""]}, "ranges": {"hold": {"op": "<", "v": "7"}}, "preset": "bogus", "years": [2025, "abcd"], "from": "2026-1-1", "to": "2026-02-01"})))).unwrap();
    assert_eq!(f["lists"]["account"], json!(["A", "3"]));
    assert_eq!(f["ranges"]["hold"], json!({"op": "<", "v": 7.0}));
    assert_eq!(f["preset"], "all");
    assert_eq!(f["years"], json!(["2025"]));
    assert_eq!(f["from"], "");
    assert_eq!(f["to"], "2026-02-01");
}

// --------------------------------------------------------------------------
// CashflowTest
// --------------------------------------------------------------------------

fn tiles_by_label(v: &Value) -> HashMap<String, Value> {
    arr(&v["cashflow"]["tiles"]).iter().map(|t| (st(&t["label"]).to_string(), t.clone())).collect()
}

#[test]
fn test_without_a_margin_account_cash_day_change_and_last_twelve_months_stand_in() {
    let dv = |id: &str, per: f64, cash: f64, day: &str| opt(json!({"id": id, "category": "dividend", "activityType": "Dividend", "activitySubType": "dividend", "rawType": "DIVIDEND", "quantity": 10, "unitPrice": per, "netCashAmount": cash, "transactionDate": day, "symbol": "AAA", "currency": "CAD", "accountType": "Trading"}));
    let mut snapshot = json!({
        "activities": [
            buy_x("b1", "AAA", 10, 10, "2025-01-05", json!({"accountType": "Trading"})),
            buy_x("b2", "BBB", 5, 20, "2025-01-06", json!({"accountType": "Kids", "accountId": "acct-2", "currency": "USD"})),
            dv("d0", 1.0, 10.0, "2024-12-01"), dv("d1", 1.0, 10.0, "2025-06-01"), dv("d2", 1.5, 15.0, "2025-12-01"),
        ],
        "accounts": [
            {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
            {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 500.0, "unifiedAccountType": "SELF_DIRECTED_RESP"},
        ],
        "balances": [{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": 300.0}, {"accountId": "acct-2", "securityId": "sec-c-usd", "quantity": 10.0}],
        "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {},
        "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
    });
    let market = json!({"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0, "priceChange": -1.0, "percentChange": -3.2}}});
    let base = base_of(&snapshot, market.clone(), json!({}), "2026-02-01");
    let v = build_view(&base, None);
    let pf = &v["portfolio"];
    assert_eq!(pf["hasMargin"], false);
    near(n(&pf["cash"]), 300.0 + 10.0 * 1.5);
    near(n(&pf["cashPct"]), 315.0 / 2000.0);
    // AAA +$5 CAD, BBB −$5 USD = −$7.5 CAD; over the previous close of both, $120 + $225 − (−$2.5)
    near(n(&pf["dayChange"]), 5.0 - 7.5);
    near(n(&pf["dayChangePct"]), -2.5 / (120.0 + 225.0 + 2.5));
    let tiles = tiles_by_label(&v);
    assert!(!tiles.contains_key("Margin used"));
    near(n(&tiles["Last 12 months"]["total"]), 25.0);
    near(n(&tiles["Last 12 months"]["perMonth"]), 12.5);
    assert_eq!(n(&tiles["Last 12 months"]["count"]), 2.0);
    // one account on, the other a margin account: the margin tiles come back for that scope
    snapshot["accounts"][1]["unifiedAccountType"] = json!("SELF_DIRECTED_NON_REGISTERED_MARGIN");
    let base = base_of(&snapshot, market, json!({}), "2026-02-01");
    assert_eq!(build_view(&base, None)["portfolio"]["hasMargin"], true);
    assert_eq!(build_view(&base, Some(&json!({"lists": {"account": ["Trading"]}})))["portfolio"]["hasMargin"], false);
}

#[test]
fn test_margin_used_tile_averages_interest_charges_over_charged_months() {
    let charge = |i: i64, day: &str, amount: f64, ccy: &str| opt(json!({
        "id": format!("i{}", i), "activityType": "INTEREST_CHARGE", "activitySubType": "MARGIN_INTEREST", "rawType": "INTEREST_CHARGE",
        "category": "other", "netCashAmount": -amount, "transactionDate": day, "symbol": "", "currency": ccy, "accountType": "Trading"}));
    let snapshot = json!({
        "activities": [
            buy_x("b1", "AAA", 10, 10, "2026-01-05", json!({"accountType": "Trading"})),
            charge(1, "2026-07-01", 100.0, "CAD"), charge(2, "2026-08-01", 20.0, "USD"), charge(3, "2026-08-04", 10.0, "CAD"),
        ],
        "accounts": [{"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"}],
        "balances": [{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0}],
        "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": [], "notes": {},
        "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
    });
    let base = base_of(&snapshot, json!({"fx": {"2026-08-01": 1.5}, "benchmark": {}}), json!({}), "2026-09-06");
    let v = build_view(&base, None);
    let tiles = arr(&v["cashflow"]["tiles"]);
    let labels: Vec<&str> = tiles.iter().map(|t| st(&t["label"])).collect();
    assert_eq!(labels[labels.len() - 3..].to_vec(), vec!["All time", "Margin used", "Yield on cost"]);
    let tile = &tiles[tiles.len() - 2];
    near(n(&tile["marginUsed"]), n(&v["portfolio"]["marginUsed"]));
    near(n(&tile["marginUsed"]), 300.0);
    // $100 + $20 × 1.5 + $10 over the two months that carried a charge
    assert_eq!(n(&tile["interestMonths"]), 2.0);
    near(n(&tile["interestPerMonth"]), (100.0 + 30.0 + 10.0) / 2.0);
    // a scope without a margin account has no Margin used tile: Last 12 months stands in
    let v = build_view(&base, Some(&json!({"lists": {"account": ["Cashflow"]}})));
    let labels: Vec<&str> = arr(&v["cashflow"]["tiles"]).iter().map(|t| st(&t["label"])).collect();
    assert_eq!(labels[labels.len() - 2..].to_vec(), vec!["Last 12 months", "Yield on cost"]);
}

#[test]
fn test_yield_on_cost_from_declared_rate() {
    let dv = |i: i64, day: &str, qty: f64, per: f64| opt(json!({"id": format!("d{}", i), "category": "dividend", "activityType": "Dividend", "activitySubType": "dividend", "rawType": "DIVIDEND", "quantity": qty, "unitPrice": per, "netCashAmount": qty * per, "transactionDate": day, "symbol": "RDDY", "currency": "CAD", "accountType": "Cashflow"}));
    let snapshot = snap(vec![
        cf_buy("b1", "RDDY", 20000, 7.13, "2026-01-05"),
        dv(1, "2026-07-06", 19000.0, 0.2),
        dv(2, "2026-08-06", 19000.0, 0.2),
        opt(json!({"id": "int", "category": "interest", "activityType": "Interest", "rawType": "INTEREST", "netCashAmount": 4.5, "transactionDate": "2026-08-01", "symbol": "", "accountType": "Cash"})),
        opt(json!({"id": "wht", "activityType": "WITHHOLDING_TAX", "rawType": "WITHHOLDING_TAX", "netCashAmount": -40, "transactionDate": "2026-08-07", "symbol": "", "accountType": "Cashflow"})),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-06");
    assert_eq!(base.cashflow.len(), 4);
    let v = build_view(&base, None);
    let cf = &v["cashflow"];
    assert_eq!(n(&cf["count"]), 2.0);
    assert_eq!(arr(&cf["rows"]).iter().map(|r| st(&r["kind"])).collect::<Vec<_>>(), vec!["Dividend", "Dividend"]);
    assert_eq!(arr(&cf["other"]).iter().map(|r| st(&r["kind"]).to_string()).collect::<HashSet<_>>(), set(&["Interest", "Withholding tax"]));
    near(n(&cf["total"]), 7600.0);
    assert_eq!(arr(&cf["months"]).iter().map(|m| st(&m["key"])).collect::<Vec<_>>(), vec!["2026-07", "2026-08", "2026-09"], "runs to the current month");
    let h = &cf["holdings"][0];
    assert_eq!(h["symbol"], "RDDY");
    assert_eq!(n(&h["freq"]), 12.0);
    near(n(&h["yoc"]), 2.4 / 7.13);
    near(n(&h["yob"]), 0.2 * 20000.0);
    near(n(&h["ytd"]), 7600.0);
    let tiles = tiles_by_label(&v);
    near(n(&tiles["2026 YTD"]["total"]), 7600.0);
    near(n(&tiles["2026 YTD"]["perMonth"]), 3800.0);
    near(n(&tiles["Yield on cost"]["yield"]), (2.4 * 20000.0) / (20000.0 * 7.13));
    near(n(&tiles["Yield on cost"]["projected"]), 2.4 * 20000.0 / 12.0);
    let v = build_view(&base, Some(&json!({"lists": {"grade": ["A"]}})));
    assert!(arr(&v["cashflow"]["skippedFilters"]).contains(&json!("grade")));
    assert_eq!(n(&v["cashflow"]["count"]), 2.0);
}

// --------------------------------------------------------------------------
// PaymentFrequencyTest
// --------------------------------------------------------------------------

#[test]
fn test_frequency_is_verified_from_dates() {
    let p = |d: &[&str]| payments_per_year(&strs(d));
    assert_eq!(p(&["2026-07-06", "2026-08-06"]), Some(12));
    assert_eq!(p(&["2026-08-06", "2026-07-06", "2026-06-05", "2026-05-06"]), Some(12));
    assert_eq!(p(&["2025-01-07", "2026-01-07"]), Some(1));
    assert_eq!(p(&["2025-03-20", "2025-06-20", "2025-09-22", "2025-12-19"]), Some(4));
    assert_eq!(p(&["2026-01-02", "2026-01-09", "2026-01-16"]), Some(52));
    // a monthly payer that switched to weekly is read from its recent payments
    assert_eq!(p(&["2026-01-06", "2026-02-06", "2026-03-06", "2026-04-06", "2026-05-06", "2026-05-13", "2026-05-20", "2026-05-27"]), Some(52));
    assert_eq!(p(&["2026-08-06"]), None);
    assert_eq!(p(&["2026-08-06", "2026-08-06"]), None);
}

#[test]
fn test_frequency_uses_payment_rows_without_per_unit_values() {
    let dv = |i: i64, day: &str, qty: f64, per: f64, amount: f64| opt(json!({"id": format!("v{}", i), "category": "dividend", "activityType": "Dividend", "activitySubType": "dividend", "rawType": "DIVIDEND", "quantity": qty, "unitPrice": per, "netCashAmount": amount, "transactionDate": day, "symbol": "VEQT", "currency": "CAD", "accountType": "Kids"}));
    let snapshot = snap(vec![
        buy_x("b1", "VEQT", 300, 49.76, "2024-06-01", json!({"accountType": "Kids"})),
        dv(1, "2025-01-07", 0.0, 0.0, 91.56),
        dv(2, "2026-01-07", 300.0, 0.76, 228.0),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-06");
    let h = &build_view(&base, None)["cashflow"]["holdings"][0];
    assert_eq!(n(&h["freq"]), 1.0);
    assert_eq!(h["freqVerified"], true);
}

#[test]
fn test_single_payment_shows_no_yield_and_annual_payer_is_not_x12() {
    let dv = |i: i64, sym: &str, day: &str, qty: f64, per: f64| opt(json!({"id": format!("d{}{}", sym, i), "category": "dividend", "activityType": "Dividend", "activitySubType": "dividend", "rawType": "DIVIDEND", "quantity": qty, "unitPrice": per, "netCashAmount": qty * per, "transactionDate": day, "symbol": sym, "currency": "CAD", "accountType": "Kids"}));
    let snapshot = snap(vec![
        buy_x("b1", "VEQT", 300, 49.76, "2024-06-01", json!({"accountType": "Kids"})),
        dv(1, "VEQT", "2025-01-07", 300.0, 0.76),
        dv(2, "VEQT", "2026-01-07", 300.0, 0.76),
        buy_x("b2", "NEWM", 1000, 10.0, "2026-07-01", json!({"accountType": "Kids"})),
        dv(1, "NEWM", "2026-08-06", 1000.0, 0.1),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-06");
    let v = build_view(&base, None);
    let h = by_key(arr(&v["cashflow"]["holdings"]), "symbol");
    assert_eq!(n(&h["VEQT"]["freq"]), 1.0);
    assert_eq!(h["VEQT"]["freqVerified"], true);
    near(n(&h["VEQT"]["yoc"]), 0.76 / 49.76);
    assert_eq!(n(&h["NEWM"]["freq"]), 12.0);
    assert_eq!(h["NEWM"]["freqVerified"], false);
    near(n(&h["NEWM"]["yoc"]), 0.1 * 12.0 / 10.0);
    assert_eq!(n(&h["NEWM"]["per"]), 0.1);
}

// --------------------------------------------------------------------------
// QuoteTest (the model's side of it)
// --------------------------------------------------------------------------

#[test]
fn test_positions_use_the_quote_when_present() {
    let snapshot = snap(vec![
        buy_x("b1", "VEQT", 100, 49.76, "2026-01-05", json!({"accountType": "Kids"})),
        buy_x("b2", "HBIX", 100, 7.0, "2026-01-05", json!({"accountType": "Kids"})),
    ]);
    let quotes = json!({"VEQT": {"price": 62.4, "priceChange": 0.08, "percentChange": 0.128, "fetchedAt": "2026-09-06T14:00:00Z"}});
    let base = base_of(&snapshot, json!({"fx": {}, "benchmark": {}, "quotes": quotes}), json!({}), "2026-09-06");
    let positions = sent(&base.positions);
    let p = by_key(&positions, "symbol");
    assert_eq!(n(&p["VEQT"]["last"]), 62.4);
    assert_eq!(p["VEQT"]["priceSource"], "quote");
    near(n(&p["VEQT"]["unreal"]), (62.4 - 49.76) * 100.0);
    assert_eq!(n(&p["VEQT"]["priceChange"]), 0.08);
    assert_eq!(p["HBIX"]["priceSource"], "fill");
    assert_eq!(n(&p["HBIX"]["last"]), 7.0);
    assert_eq!(held_symbols(&base), vec![json!({"symbol": "VEQT", "exchange": "", "currency": "CAD", "kind": "Shares"}), json!({"symbol": "HBIX", "exchange": "", "currency": "CAD", "kind": "Shares"})]);
}

#[test]
fn test_positions_price_crypto_and_options_from_quotes() {
    let snapshot = snap(vec![
        opt(json!({"id": "c1", "category": "trade", "activityType": "BUY", "rawType": "CRYPTO_BUY", "quantity": 0.5, "unitPrice": 100000, "netCashAmount": -50000, "transactionDate": "2026-01-05", "symbol": "BTC", "currency": "CAD", "accountType": "Crypto", "securityId": "sec-z-btc"})),
        opt(json!({"id": "o1", "category": "trade", "activityType": "BUY", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 0.10, "netCashAmount": -20, "transactionDate": "2026-02-05", "symbol": "QNC 20NOV26 3.00 CALL", "currency": "USD", "accountType": "TFSA", "securityId": "sec-o-1"})),
    ]);
    let quotes = json!({"BTC": {"price": 120000.0}, "QNC 20NOV26 3.00 CALL": {"price": 0.15}});
    let base = base_of(&snapshot, json!({"fx": {}, "benchmark": {}, "quotes": quotes}), json!({}), "2026-09-06");
    let positions = sent(&base.positions);
    let by = by_key(&positions, "symbol");
    let row = |p: &Value| (st(&p["kind"]).to_string(), st(&p["priceSource"]).to_string(), n(&p["last"]), n(&p["mv"]));
    assert_eq!(row(by["BTC"]), ("Crypto".into(), "quote".into(), 120000.0, 60000.0));
    assert_eq!(row(by["QNC 20NOV26 3.00 CALL"]), ("Options".into(), "quote".into(), 0.15, 30.0));
    let mut held: Vec<(String, String)> = held_symbols(&base).iter().map(|h| (st(&h["symbol"]).to_string(), st(&h["kind"]).to_string())).collect();
    held.sort();
    assert_eq!(held, vec![("BTC".into(), "Crypto".into()), ("QNC 20NOV26 3.00 CALL".into(), "Options".into())]);
}

#[test]
fn test_a_coins_price_never_prices_a_share_with_the_same_symbol() {
    // a share or warrant called BTC beside the coin BTC: one quote row per symbol, and only the coin may take Coinbase's price
    let snapshot = snap(vec![
        opt(json!({"id": "c1", "category": "trade", "activityType": "BUY", "rawType": "CRYPTO_BUY", "quantity": 0.5, "unitPrice": 100000, "netCashAmount": -50000, "transactionDate": "2026-01-05", "symbol": "BTC", "currency": "CAD", "accountType": "Crypto", "securityId": "sec-z-btc-1"})),
        opt(json!({"id": "s1", "category": "trade", "activityType": "BUY", "rawType": "DIY_BUY", "quantity": 4653, "unitPrice": 1.75, "netCashAmount": -8142.75, "transactionDate": "2026-02-05", "symbol": "BTC", "currency": "CAD", "accountType": "TFSA", "securityId": "sec-s-btc-warrant"})),
    ]);
    let quotes = json!({"BTC": {"price": 109998.0, "source": "coinbase"}});
    let base = base_of(&snapshot, json!({"fx": {}, "benchmark": {}, "quotes": quotes}), json!({}), "2026-09-06");
    let find = |kind: &str| sent(&base.positions).into_iter().find(|p| p["symbol"] == "BTC" && p["kind"] == kind).unwrap();
    assert_eq!((st(&find("Crypto")["priceSource"]), n(&find("Crypto")["last"])), ("quote", 109998.0));
    let share = find("Shares");
    assert_eq!((st(&share["priceSource"]), n(&share["last"]), (n(&share["mv"]) * 100.0).round() / 100.0), ("fill", 1.75, 8142.75), "the share keeps its fill price rather than the coin's");
    let quote = |v: Value| serde_json::from_value::<Quote>(v).unwrap();
    assert!(quote(json!({"price": 1.0})).fits(Kind::Shares), "a quote with no source stated is the kind's own");
    assert!(!quote(json!({"price": 1.0, "source": "tmx"})).fits(Kind::Crypto));
    assert!(!quote(json!({"price": 1.0, "source": "coinbase"})).fits(Kind::Shares));
}

// --------------------------------------------------------------------------
// DeclaredDistributionsTest (the model's side of it)
// --------------------------------------------------------------------------

#[test]
fn test_declared_record_beats_own_history_and_tracks_schedule_change() {
    let snapshot = snap(vec![
        cf_buy("b1", "CCHI", 4000, 11.64, "2026-08-25"),
        opt(json!({"id": "c1", "category": "dividend", "activityType": "Dividend", "activitySubType": "dividend", "rawType": "DIVIDEND", "quantity": 4000, "unitPrice": 0.135, "netCashAmount": 4000.0 * 0.135, "transactionDate": "2026-09-04", "symbol": "CCHI", "currency": "CAD", "accountType": "Cashflow"})),
    ]);
    let public = json!({"CCHI": [
        {"exDate": "2026-09-15", "payDate": "2026-09-21", "amount": 0.135, "currency": "CAD"},
        {"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.135, "currency": "CAD"},
        {"exDate": "2026-08-14", "payDate": "2026-08-20", "amount": 0.135, "currency": "CAD"},
        {"exDate": "2026-07-31", "payDate": "2026-08-10", "amount": 0.27, "currency": "CAD"},
        {"exDate": "2026-06-30", "payDate": "2026-07-08", "amount": 0.27, "currency": "CAD"},
        {"exDate": "2026-05-29", "payDate": "2026-06-05", "amount": 0.27, "currency": "CAD"},
    ]});
    let quotes = json!({"CCHI": {"price": 10.95, "dividendAmount": 0.135, "dividendFrequency": "", "exDividendDate": "2026-09-15", "fetchedAt": "2026-09-06T00:00:00Z"}});
    let base = base_of(&snapshot, json!({"fx": {}, "benchmark": {}, "distributions": public, "quotes": quotes}), json!({}), "2026-09-06");
    let v = build_view(&base, None);
    let h = &v["cashflow"]["holdings"][0];
    assert_eq!(n(&h["per"]), 0.135);
    assert_eq!(n(&h["freq"]), 24.0);
    assert_eq!(h["freqVerified"], true);
    assert_eq!(h["rateSource"], "declared");
    near(n(&h["yoc"]), 0.135 * 24.0 / 11.64);
    assert_eq!(n(&h["last"]), 10.95);
    assert_eq!(h["priceSource"], "close");
    near(n(&h["currentYield"]), 0.135 * 24.0 / 10.95);
    // without the public record it falls back to the single own payment
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-06");
    let v = build_view(&base, None);
    let h = &v["cashflow"]["holdings"][0];
    assert_eq!(h["rateSource"], "payments");
    assert_eq!(h["freqVerified"], false);
    assert_eq!(h["priceSource"], "fill");
}

#[test]
fn test_payer_symbols_are_held_dividend_payers() {
    let snapshot = snap(vec![
        cf_buy("b1", "RDDY", 100, 7, "2026-01-05"),
        div_row("d1", "RDDY", 100.0, 0.2, 20.0, "2026-02-06", "Cashflow"),
        cf_buy("b2", "TD", 10, 80, "2025-01-05"),
        div_row("d2", "TD", 10.0, 1.0, 10.0, "2025-02-06", "Cashflow"),
        sell_x("s2", "TD", 10, 90, "2025-03-01", json!({"accountType": "Cashflow"})),
        cf_buy("b3", "AAA", 10, 5, "2026-01-05"),
    ]);
    let base = base_of(&snapshot, empty_market(), json!({}), "2026-09-06");
    assert_eq!(payer_symbols(&base), vec![json!({"symbol": "RDDY", "exchange": "", "currency": "CAD"})]);
}

// --------------------------------------------------------------------------
// LegacyNotesTest
// --------------------------------------------------------------------------

#[test]
fn test_group_id_matches_ledger_html() {
    // ledger.html: FNV-1a over "\n".join(sorted keys), "g_" + hex + "_" + n
    assert_eq!(group_id_for_keys(&strs(&["b|s|100.00000000"])), group_id_for_keys(&strs(&["b|s|100.00000000"])));
    assert!(group_id_for_keys(&strs(&["a", "b"])).ends_with("_2"));
    assert_eq!(group_id_for_keys(&strs(&["a", "b"])), group_id_for_keys(&strs(&["b", "a"])));
}

#[test]
fn test_legacy_note_lands_on_round_trip() {
    let m = match_fifo(&raws(&[buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")]));
    let key = slice_member_key(&m.closed[0]);
    let legacy_id = group_id_for_keys(&[key]);
    let notes = json!({legacy_id: {"thesis": "why", "tag": "a, b", "grade": "C"}});
    let journal = migrate_legacy_notes(&m.closed, &[], notes.as_object().unwrap());
    assert_eq!(Value::Object(journal), json!({"rt:b1": {"thesis": "why", "tags": ["a", "b"], "grade": "C"}}));
}

// --------------------------------------------------------------------------
// StoreTablesTest (the model's side of it)
// --------------------------------------------------------------------------

#[test]
fn test_portfolio_tiles_sum_wealthsimple_figures_over_the_accounts_in_scope() {
    let acts = vec![
        buy("b1", "AAA", 10, 10, "2026-01-05"),
        buy_x("b2", "BBB", 5, 20, "2026-01-06", json!({"accountType": "Kids", "accountId": "acct-2", "currency": "USD"})),
    ];
    let snap_full = json!({
        "activities": acts,
        "accounts": [
            {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
            {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 400.0, "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
            {"id": "acct-3", "nickname": "Cash", "currency": "CAD", "netLiquidationValue": 25.0, "unifiedAccountType": "CASH"},
            {"id": "acct-4", "nickname": "Old", "currency": "CAD", "netLiquidationValue": 999.0, "status": "closed", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
            {"id": "acct-5", "nickname": "TFSA", "currency": "CAD", "netLiquidationValue": 0.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
        ],
        "balances": [
            {"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0},
            {"accountId": "acct-1", "securityId": "sec-c-usd", "quantity": -10.0},
            {"accountId": "acct-2", "securityId": "sec-c-cad", "quantity": 50.0},
            {"accountId": "acct-1", "securityId": "sec-s-aaa", "quantity": 10.0},
            {"accountId": "acct-4", "securityId": "sec-c-cad", "quantity": -5000.0},
        ],
        "securities": [
            {"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"},
            {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"},
            {"id": "sec-s-aaa", "symbol": "AAA", "currency": "CAD"},
        ],
        "margin": [
            {"accountId": "acct-1", "buyingPower": 700.0, "currency": "CAD", "unavailable": ""},
            {"accountId": "acct-2", "buyingPower": null, "currency": "CAD", "unavailable": "UnavailableSecurities (1 securities)"},
            {"accountId": "acct-5", "buyingPower": 5638.24, "currency": "CAD", "unavailable": ""},
        ],
    });
    let market = json!({"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0}}});
    let base = base_of(&snap_full, market.clone(), json!({"rt:b1": {"grade": "B", "thesis": "hold", "tags": ["core"]}}), "2026-02-01");
    let v = build_view(&base, Some(&json!({})));
    let pf = &v["portfolio"];
    near(n(&pf["marketValue"]), 120.0 + 150.0 * 1.5);
    near(n(&pf["costBasis"]), 100.0 + 100.0 * 1.5);
    near(n(&pf["unrealized"]), 20.0 + 50.0 * 1.5);
    assert_eq!((n(&pf["positionCount"]), n(&pf["accountCount"])), (2.0, 2.0));
    near(n(&pf["nav"]), 1500.0 + 400.0 + 25.0);
    near(n(&pf["marginUsed"]), 300.0 + 10.0 * 1.5);
    assert_eq!(pf["marginUsedBy"], json!({"CAD": 300.0, "USD": 10.0}), "the closed account's cash is not margin used");
    near(n(&pf["availableMargin"]), 700.0);
    assert_eq!(pf["availableMarginUnavailable"], json!(["Kids"]));
    let aaa = arr(&v["positions"]).iter().find(|p| p["symbol"] == "AAA").unwrap();
    near(n(&aaa["dayChange"]), 10.0 * 0.5);
    assert_eq!(aaa["grade"], "B");
    assert_eq!(arr(&aaa["fills"]).iter().map(|f| st(&f["side"])).collect::<Vec<_>>(), vec!["BUY"]);
    let bbb = arr(&v["positions"]).iter().find(|p| p["symbol"] == "BBB").unwrap();
    assert!(bbb["dayChange"].is_null(), "no change on the quote, no day change");
    let kids = build_view(&base, Some(&json!({"lists": {"account": ["Kids"]}})))["portfolio"].clone();
    near(n(&kids["nav"]), 400.0);
    near(n(&kids["marginUsed"]), 0.0);
    assert!(kids["availableMargin"].is_null());
    assert_eq!(kids["availableMarginUnavailable"], json!(["Kids"]));
    near(n(&kids["marketValue"]), 150.0 * 1.5);
    let empty_base = base_of(&json!({"activities": snap_full["activities"]}), market, json!({}), "2026-02-01");
    let empty = build_view(&empty_base, Some(&json!({})))["portfolio"].clone();
    assert!(empty["nav"].is_null());
    assert!(empty["availableMargin"].is_null());
    assert_eq!(n(&empty["marginUsed"]), 0.0);
}

// --------------------------------------------------------------------------
// MarketParseTest (the model's side of it)
// --------------------------------------------------------------------------

#[test]
fn test_intraday_archive_covers_recent_trades_and_holdings() {
    let t = json!({"accountType": "TFSA"});
    let acts = vec![
        buy_x("b1", "OLD", 10, 5, "2024-01-05", t.clone()), sell_x("s1", "OLD", 10, 6, "2024-02-05", t.clone()),
        buy_x("b2", "NEW", 10, 5, "2026-03-01", t.clone()), sell_x("s2", "NEW", 10, 6, "2026-04-01", t.clone()),
        buy_x("b3", "NEW", 10, 5, "2026-06-01", t.clone()), sell_x("s3", "NEW", 10, 6, "2026-07-01", t.clone()),
        buy_x("b4", "HELD", 10, 5, "2025-05-01", t.clone()),
    ];
    let base = base_of(&snap(acts.clone()), empty_market(), json!({}), "2026-09-07");
    let recs = intraday_archive_symbols(&base);
    let by = by_key(&recs, "symbol");
    assert!(!by.contains_key("OLD"), "closed long before the window");
    assert_eq!(by["NEW"]["start"], "2026-03-01", "earliest entry within the window");
    assert_eq!(by["HELD"]["start"], "2025-09-07", "an old holding is wanted from the window start");
    let mut opt_acts = acts;
    opt_acts.push(opt(json!({"id": "o1", "category": "trade", "activityType": "BUY", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 0.10, "netCashAmount": -20, "transactionDate": "2026-05-05", "symbol": "LUNR 15JAN27 10.00 CALL", "currency": "USD", "accountType": "TFSA", "securityId": "sec-o-1"})));
    let base = base_of(&snap(opt_acts), empty_market(), json!({}), "2026-09-07");
    let recs = intraday_archive_symbols(&base);
    let by = by_key(&recs, "symbol");
    assert!(by.contains_key("LUNR"), "an option position is archived as its underlying");
    assert_eq!((st(&by["LUNR"]["kind"]), st(&by["LUNR"]["start"])), ("Shares", "2026-05-05"));
}

// --------------------------------------------------------------------------
// DetailTest: legs and fills travel only for the trade or holding open on the page
// --------------------------------------------------------------------------

fn detail_base() -> Base {
    let csv = json!({"source": "csv"});
    let snapshot = snap(vec![
        buy_x("b1", "AAA", 10, 1, "2026-01-01", csv.clone()),
        sell_x("s1", "AAA", 10, 2, "2026-01-05", csv.clone()),
        buy_x("b2", "BBB", 5, 3, "2026-01-02", csv),
    ]);
    base_of(&snapshot, json!({"fx": {"2099-01-01": 1.0}, "benchmark": {"2099-01-01": 1.0}}), json!({}), "2026-09-16")
}

#[test]
fn test_the_view_carries_legs_and_fills_for_the_open_trade_only() {
    let base = detail_base();
    let trade = sent(&base.trades)[0].clone();
    let holding = sent(&base.positions)[0].clone();
    assert_eq!((arr(&trade["fills"]).len(), arr(&holding["fills"]).len()), (2, 1), "the base model keeps every fill");
    let v = slim(&base, None);
    assert!(v["trades"][0].get("legs").is_none() && v["trades"][0].get("fills").is_none());
    assert!(v["positions"][0].get("legs").is_none() && v["positions"][0].get("fills").is_none());
    assert_eq!(n(&v["trades"][0]["legCount"]), 1.0, "the counts stay on the row");
    let v = slim(&base, Some(st(&trade["id"])));
    assert_eq!(arr(&v["trades"][0]["fills"]).len(), 2);
    assert!(v["positions"][0].get("fills").is_none());
    let v = slim(&base, Some(st(&holding["id"])));
    assert_eq!(arr(&v["positions"][0]["fills"]).len(), 1);
    assert!(v["trades"][0].get("fills").is_none());
    assert_eq!(arr(&sent(&base.trades)[0]["fills"]).len(), 2, "slimming the view never touches the base model");
}

#[test]
fn test_trade_detail_finds_a_trade_or_a_holding_by_id() {
    let base = detail_base();
    let d = trade_detail(&base, st(&sent(&base.trades)[0]["id"])).unwrap();
    let ids = |d: &Value| arr(&d["fills"]).iter().map(|f| st(&f["id"]).to_string()).collect::<Vec<_>>();
    assert_eq!((d["id"].clone(), arr(&d["legs"]).len(), ids(&d)), (sent(&base.trades)[0]["id"].clone(), 1, strs(&["s1", "b1"])));
    let d = trade_detail(&base, st(&sent(&base.positions)[0]["id"])).unwrap();
    assert_eq!((d["legs"].clone(), ids(&d)), (json!([]), strs(&["b2"])));
    assert!(trade_detail(&base, "nope").is_none());
}
