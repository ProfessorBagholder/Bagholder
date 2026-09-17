//! The shared model cases: activity rows in, the figures the spec says they
//! produce out, as JSON every implementation (Rust, Swift, Kotlin) runs through
//! its own model. `make-cases` writes them to `tests/cases`; the model's tests
//! read the rows builders here too.

use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::base::build_base;
use crate::view::build_view;

// --------------------------------------------------------------------------
// numbers that keep Python's int/float distinction
// --------------------------------------------------------------------------

fn is_int(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_i64() || n.is_u64())
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// `a * b`: an int when both are ints, else a float.
pub fn mul(a: &Value, b: &Value) -> Value {
    if is_int(a) && is_int(b) {
        json!(a.as_i64().unwrap() * b.as_i64().unwrap())
    } else {
        json!(f(a) * f(b))
    }
}

/// `-a`.
pub fn neg(a: &Value) -> Value {
    if is_int(a) { json!(-a.as_i64().unwrap()) } else { json!(-f(a)) }
}

/// Python's `round(x, n)` on a float: the exact binary value rounded half-even.
pub fn round_half_even(x: f64, n: usize) -> f64 {
    format!("{:.*}", n, x).parse().unwrap()
}

// --------------------------------------------------------------------------
// activity rows as the sync stores them
// --------------------------------------------------------------------------

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

fn merge(base: &mut Map<String, Value>, extra: Value) {
    if let Value::Object(m) = extra {
        for (k, v) in m {
            base.insert(k, v);
        }
    }
}

/// One activity row with every field the sync writes, overridden by `o`.
pub fn act(o: Value) -> Value {
    let mut base = json!({
        "id": "",
        "accountId": "acct-1",
        "accountType": "Trading",
        "symbol": "LUNR 15JAN27 12.00 CALL",
        "name": "LUNR",
        "currency": "USD",
        "commission": 0,
        "category": "other",
        "activityType": "",
        "activitySubType": "",
        "rawType": "",
        "quantity": 0,
        "unitPrice": 0,
        "netCashAmount": 0,
        "transactionDate": "2026-01-01",
        "occurredAt": "",
        "securityId": "",
    })
    .as_object()
    .cloned()
    .unwrap();
    merge(&mut base, o);
    if base["id"].as_str().map_or(true, |s| s.is_empty()) {
        base.insert("id".into(), json!(format!("act-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))));
    }
    if base["occurredAt"].as_str().map_or(true, |s| s.is_empty()) {
        let day = base["transactionDate"].as_str().unwrap_or("").to_string();
        base.insert("occurredAt".into(), json!(format!("{}T15:00:00+00:00", day)));
    }
    Value::Object(base)
}

pub fn buy_x(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str, extra: Value) -> Value {
    let (qty, px) = (qty.into(), px.into());
    let mut o = json!({
        "id": id, "category": "trade", "activityType": "Trade", "activitySubType": "BUY", "rawType": "DIY_BUY",
        "quantity": qty, "unitPrice": px, "netCashAmount": mul(&neg(&qty), &px), "transactionDate": day,
        "symbol": symbol, "currency": "CAD",
    })
    .as_object()
    .cloned()
    .unwrap();
    merge(&mut o, extra);
    act(Value::Object(o))
}

pub fn buy(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str) -> Value {
    buy_x(id, symbol, qty, px, day, json!({}))
}

pub fn sell_x(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str, extra: Value) -> Value {
    let (qty, px) = (qty.into(), px.into());
    let mut o = json!({
        "id": id, "category": "trade", "activityType": "Trade", "activitySubType": "SELL", "rawType": "DIY_SELL",
        "quantity": neg(&qty), "unitPrice": px, "netCashAmount": mul(&qty, &px), "transactionDate": day,
        "symbol": symbol, "currency": "CAD",
    })
    .as_object()
    .cloned()
    .unwrap();
    merge(&mut o, extra);
    act(Value::Object(o))
}

pub fn sell(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str) -> Value {
    sell_x(id, symbol, qty, px, day, json!({}))
}

pub fn snapshot(
    acts: Vec<Value>,
    securities: Option<Value>,
    nav: Option<Value>,
    nav_by_account: Option<Value>,
    accounts: Option<Value>,
    balances: Option<Value>,
    margin: Option<Value>,
) -> Value {
    json!({
        "activities": acts, "accounts": accounts.unwrap_or(json!([])), "balances": balances.unwrap_or(json!([])),
        "margin": margin.unwrap_or(json!([])), "navHistory": nav.unwrap_or(json!([])),
        "navByAccount": nav_by_account.unwrap_or(json!({})), "syncedAt": "", "tradeGroups": [], "notes": {},
        "securities": securities.unwrap_or(json!([])),
    })
}

pub fn dividend(id: &str, symbol: &str, qty: impl Into<Value>, per: impl Into<Value>, day: &str, account: &str) -> Value {
    let (qty, per) = (qty.into(), per.into());
    let amount = mul(&qty, &per);
    let amount = if is_int(&amount) { amount } else { json!(round_half_even(f(&amount), 2)) };
    act(json!({
        "id": id, "category": "dividend", "activityType": "Dividend", "rawType": "DIVIDEND", "quantity": qty,
        "unitPrice": per, "netCashAmount": amount, "transactionDate": day, "symbol": symbol, "currency": "CAD",
        "accountType": account,
    }))
}

pub fn sto_x(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str, extra: Value) -> Value {
    let (qty, px) = (qty.into(), px.into());
    let mut o = json!({
        "id": id, "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN",
        "rawType": "OPTIONS_SELL", "quantity": neg(&qty), "unitPrice": px,
        "netCashAmount": mul(&mul(&qty, &px), &json!(100)), "transactionDate": day, "symbol": symbol,
    })
    .as_object()
    .cloned()
    .unwrap();
    merge(&mut o, extra);
    act(Value::Object(o))
}

pub fn sto(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str) -> Value {
    sto_x(id, symbol, qty, px, day, json!({}))
}

pub fn btc(id: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str, sub: &str) -> Value {
    let (qty, px) = (qty.into(), px.into());
    act(json!({
        "id": id, "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": sub,
        "rawType": "OPTIONS_BUY", "quantity": qty, "unitPrice": px,
        "netCashAmount": mul(&mul(&neg(&qty), &px), &json!(100)), "transactionDate": day, "symbol": symbol,
    }))
}

/// A Wealthsimple multileg fill as posted: quantity 0, only the cash.
pub fn multileg(id: &str, symbol: &str, cash: impl Into<Value>, day: &str) -> Value {
    act(json!({
        "id": id, "activityType": "OPTIONS_MULTILEG", "activitySubType": "FILLED", "rawType": "OPTIONS_MULTILEG",
        "quantity": 0, "netCashAmount": cash.into(), "transactionDate": day, "symbol": symbol,
    }))
}

pub fn crypto(id: &str, kind: &str, symbol: &str, qty: impl Into<Value>, px: impl Into<Value>, day: &str) -> Value {
    let (qty, px) = (qty.into(), px.into());
    let raw = match kind {
        "buy" => "CRYPTO_BUY",
        "sell" => "CRYPTO_SELL",
        _ => "CRYPTO_STAKING_REWARD",
    };
    act(json!({
        "id": id, "activityType": raw, "activitySubType": if kind != "reward" { "MARKET_ORDER" } else { "other" },
        "rawType": raw, "quantity": qty, "unitPrice": px, "netCashAmount": mul(&qty, &px), "transactionDate": day,
        "symbol": symbol, "currency": "CAD", "accountType": "Crypto",
    }))
}

pub fn crypto_transfer(id: &str, symbol: &str, qty: impl Into<Value>, value: impl Into<Value>, day: &str, out: bool) -> Value {
    let (qty, value) = (qty.into(), value.into());
    act(json!({
        "id": id, "activityType": "CRYPTO_TRANSFER", "activitySubType": if out { "TRANSFER_OUT" } else { "TRANSFER_IN" },
        "rawType": "CRYPTO_TRANSFER", "direction": if out { "DEBIT" } else { "CREDIT" }, "quantity": qty,
        "unitPrice": f(&value) / f(&qty), "netCashAmount": if out { neg(&value) } else { value.clone() },
        "transactionDate": day, "symbol": symbol, "currency": "CAD", "accountType": "Crypto",
    }))
}

fn nav(day: &str, equity: f64, deposits: f64) -> Value {
    json!({"date": day, "equity": equity, "netDeposits": deposits})
}

// --------------------------------------------------------------------------
// the cases
// --------------------------------------------------------------------------

fn nav_rows() -> Vec<Value> {
    vec![
        nav("2023-12-15", 50.0, 50.0), // pre-history: a few dollars parked before the real start
        nav("2024-01-15", 100000.0, 100000.0),
        nav("2024-03-01", 110000.0, 100000.0),
        nav("2024-06-03", 95000.0, 100000.0),
        nav("2024-09-03", 140000.0, 130000.0), // a 30,000 deposit on the day
        nav("2024-12-31", 150000.0, 130000.0),
        nav("2025-03-03", 170000.0, 130000.0),
        nav("2025-06-02", 150000.0, 130000.0),
        nav("2025-12-31", 200000.0, 130000.0),
        nav("2026-03-02", 230000.0, 130000.0),
        nav("2026-06-01", 190000.0, 120000.0), // a 10,000 withdrawal
        nav("2026-09-04", 215000.0, 120000.0),
    ]
}

fn sp500() -> Value {
    json!({"2023-12-29": 4700.0, "2024-06-28": 5400.0, "2024-12-31": 5900.0, "2025-06-30": 6200.0, "2025-12-31": 6800.0, "2026-09-04": 7200.0})
}

fn tsx() -> Value {
    json!({"2023-12-29": 20900.0, "2024-06-28": 21800.0, "2024-12-31": 24700.0, "2025-06-30": 26800.0, "2025-12-31": 28100.0, "2026-09-04": 29500.0})
}

/// A small book across two accounts for the filter cases: shares, an option
/// chain, a loser, a position in each account, one exchange record per share,
/// and a journal on three trades.
fn filter_acts() -> Vec<Value> {
    vec![
        buy_x("b1", "AAA", 100, 10.0, "2026-01-05", json!({"securityId": "sec-aaa"})),
        sell_x("s1", "AAA", 100, 12.0, "2026-01-20", json!({"securityId": "sec-aaa"})),
        buy_x("b2", "AAA", 50, 12.0, "2026-02-02", json!({"securityId": "sec-aaa"})),
        sell_x("s2", "AAA", 50, 11.0, "2026-02-10", json!({"securityId": "sec-aaa"})),
        buy_x("b3", "BBB", 200, 5.0, "2025-11-03", json!({"accountType": "TFSA", "securityId": "sec-bbb"})),
        sell_x("s3", "BBB", 200, 6.0, "2026-03-16", json!({"accountType": "TFSA", "securityId": "sec-bbb"})),
        sto("sto", "ZZZ 21AUG26 10.00 CALL", 2, 3, "2026-04-01"),
        btc("cover", "ZZZ 21AUG26 10.00 CALL", 2, 1, "2026-05-15", "BUYTOCLOSE"),
        buy_x("b4", "CCC", 10, 100.0, "2026-06-01", json!({"currency": "USD"})),
        sell_x("s4", "CCC", 10, 90.0, "2026-06-12", json!({"currency": "USD"})),
        buy_x("b5", "DDD", 300, 2.0, "2026-07-01", json!({"securityId": "sec-ddd"})),
        buy_x("b6", "EEE", 40, 25.0, "2026-08-01", json!({"accountType": "TFSA"})),
        dividend("d1", "DDD", 300, 0.05, "2026-08-15", "Trading"),
    ]
}

fn filter_secs() -> Value {
    json!([{"id": "sec-aaa", "symbol": "AAA", "name": "Triple A Corp", "primaryExchange": "TSX"},
           {"id": "sec-bbb", "symbol": "BBB", "name": "Bee Inc", "primaryExchange": "NASDAQ"},
           {"id": "sec-ddd", "symbol": "DDD", "name": "Dee Fund", "primaryExchange": "TSX"}])
}

fn filter_journal() -> Value {
    json!({"rt:b1": {"grade": "A", "thesis": "Breakout after earnings.", "tags": ["earnings", "breakout"]},
           "rt:b2": {"grade": "F", "thesis": "", "tags": []},
           "rt:sto": {"grade": "B", "thesis": "Covered call on a flat name.", "tags": ["income"]},
           "rt:b5": {"grade": "", "thesis": "Holding for the distribution.", "tags": ["income"]}})
}

fn filter_market() -> Value {
    json!({"fx": {"2026-06-01": 1.37, "2026-06-12": 1.36}, "benchmark": sp500(), "benchmarks": {"TSX": tsx()}})
}

/// One case: what `make_cases.CASES` holds for it.
pub struct Case {
    pub today: &'static str,
    pub activities: Vec<Value>,
    pub securities: Option<Value>,
    pub nav: Option<Value>,
    pub nav_by_account: Option<Value>,
    pub accounts: Option<Value>,
    pub balances: Option<Value>,
    pub margin: Option<Value>,
    pub journal: Option<Value>,
    pub filters: Option<Value>,
    pub market: Value,
}

fn case(today: &'static str, activities: Vec<Value>, market: Value) -> Case {
    Case {
        today, activities, market, securities: None, nav: None, nav_by_account: None, accounts: None,
        balances: None, margin: None, journal: None, filters: None,
    }
}

fn filter_case(filters: Option<Value>, nav_by_account: bool) -> Case {
    let mut c = case("2026-09-07", filter_acts(), filter_market());
    c.securities = Some(filter_secs());
    c.journal = Some(filter_journal());
    c.nav = Some(json!(nav_rows()));
    if nav_by_account {
        c.nav_by_account = Some(json!({"Trading": nav_rows()[..6].to_vec()}));
    }
    c.filters = filters;
    c
}

fn empty_market() -> Value {
    json!({"fx": {}, "benchmark": {}})
}

/// Every case, in the order the generator writes them.
pub fn cases() -> Vec<(&'static str, Case)> {
    let cash_secs = json!([{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}]);
    let mut out: Vec<(&'static str, Case)> = Vec::new();

    // two share round trips in CAD: a win, then a loss, one account
    out.push(("shares_two_round_trips", case("2026-03-01", vec![
        buy("b1", "AAA", 100, 10.0, "2026-01-05"), sell("s1", "AAA", 100, 12.0, "2026-01-20"),
        buy("b2", "AAA", 50, 12.0, "2026-02-02"), sell("s2", "AAA", 50, 11.0, "2026-02-10"),
    ], empty_market())));

    // a short option sold to open and bought to close, USD, contract multiplier 100
    out.push(("option_short_then_cover", case("2026-03-01", vec![
        act(json!({"id": "sto", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOOPEN", "rawType": "OPTIONS_SELL", "quantity": -2, "unitPrice": 3, "netCashAmount": 600, "transactionDate": "2026-01-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
        act(json!({"id": "btc", "category": "trade", "activityType": "OPTIONS_BUY", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 1, "netCashAmount": -200, "transactionDate": "2026-02-01", "symbol": "ZZZ 21AUG26 10.00 CALL"})),
    ], json!({"fx": {"2026-01-01": 1.40, "2026-02-01": 1.35}, "benchmark": {}}))));

    // an open position with two lots, no sale yet
    out.push(("shares_open_position_two_lots", case("2026-03-01", vec![
        buy("b1", "BBB", 100, 5.0, "2026-01-05"), buy("b2", "BBB", 100, 7.0, "2026-02-05"),
    ], empty_market())));

    // the Portfolio tiles: CAD aggregates over every account, cash accounts included, margin used
    // from the negative cash per currency, available margin from Wealthsimple's buying power,
    // and the day's change on a position from its quote
    let mut c = case("2026-02-01", vec![
        buy("b1", "AAA", 10, 10, "2026-01-05"),
        buy_x("b2", "BBB", 5, 20, "2026-01-06", json!({"accountType": "Kids", "accountId": "acct-2", "currency": "USD"})),
    ], json!({"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0}}}));
    c.securities = Some(cash_secs.clone());
    c.accounts = Some(json!([
        {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
        {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 400.0, "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
        {"id": "acct-3", "nickname": "Cash", "currency": "CAD", "netLiquidationValue": 25.0, "unifiedAccountType": "CASH"},
        {"id": "acct-4", "nickname": "Old", "currency": "CAD", "netLiquidationValue": 999.0, "status": "closed", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
        {"id": "acct-5", "nickname": "TFSA", "currency": "CAD", "netLiquidationValue": 0.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
    ]));
    c.balances = Some(json!([
        {"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0},
        {"accountId": "acct-1", "securityId": "sec-c-usd", "quantity": -10.0},
        {"accountId": "acct-2", "securityId": "sec-c-cad", "quantity": 50.0},
    ]));
    c.margin = Some(json!([
        {"accountId": "acct-1", "buyingPower": 700.0, "currency": "CAD", "unavailable": ""},
        {"accountId": "acct-2", "buyingPower": null, "currency": "CAD", "unavailable": "UnavailableSecurities (1 securities)"},
        {"accountId": "acct-5", "buyingPower": 5638.24, "currency": "CAD", "unavailable": ""},
    ]));
    c.journal = Some(json!({"rt:b1": {"grade": "B", "thesis": "hold", "tags": ["core"]}}));
    out.push(("portfolio_tiles_over_all_accounts", c));

    // the same book with one account on: the tiles narrow to it, and the account whose margin is unavailable says so
    let mut c = case("2026-02-01", vec![
        buy("b1", "AAA", 10, 10, "2026-01-05"),
        buy_x("b2", "BBB", 5, 20, "2026-01-06", json!({"accountType": "Kids", "accountId": "acct-2", "currency": "USD"})),
    ], json!({"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"BBB": {"price": 30.0, "priceChange": -1.0, "percentChange": -3.2}}}));
    c.securities = Some(cash_secs.clone());
    c.accounts = Some(json!([
        {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
        {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 400.0, "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
    ]));
    c.balances = Some(json!([{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0}, {"accountId": "acct-2", "securityId": "sec-c-cad", "quantity": 50.0}]));
    c.margin = Some(json!([{"accountId": "acct-2", "buyingPower": null, "currency": "CAD", "unavailable": "UnavailableSecurities (1 securities)"}]));
    c.filters = Some(json!({"lists": {"account": ["Kids"]}}));
    out.push(("portfolio_tiles_one_account", c));

    // a book without a margin account: Cash and Day change stand in for the margin tiles, Last 12 months for the Margin used tile
    let mut c = case("2026-02-01", vec![
        buy("b1", "AAA", 10, 10, "2025-01-05"),
        buy_x("b2", "BBB", 5, 20, "2025-01-06", json!({"accountType": "Kids", "accountId": "acct-2", "currency": "USD"})),
        dividend("d0", "AAA", 10, 1.0, "2024-12-01", "Cashflow"),
        dividend("d1", "AAA", 10, 1.0, "2025-06-01", "Cashflow"),
        dividend("d2", "AAA", 10, 1.5, "2025-12-01", "Cashflow"),
    ], json!({"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0, "priceChange": -1.0, "percentChange": -3.2}}}));
    c.securities = Some(cash_secs.clone());
    c.accounts = Some(json!([
        {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
        {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 500.0, "unifiedAccountType": "SELF_DIRECTED_RESP"},
    ]));
    c.balances = Some(json!([
        {"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": 300.0},
        {"accountId": "acct-2", "securityId": "sec-c-usd", "quantity": 10.0},
    ]));
    c.margin = Some(json!([{"accountId": "acct-1", "buyingPower": 5638.24, "currency": "CAD", "unavailable": ""}]));
    out.push(("portfolio_tiles_without_margin", c));

    // the Margin used tile: the Portfolio figure, with margin interest averaged over the months that carried a charge
    let charge = |id: &str, amount: i64, day: &str, ccy: &str| act(json!({
        "id": id, "activityType": "INTEREST_CHARGE", "activitySubType": "MARGIN_INTEREST", "rawType": "INTEREST_CHARGE",
        "category": "other", "netCashAmount": -amount, "transactionDate": day, "symbol": "", "currency": ccy,
    }));
    let mut c = case("2026-09-07", vec![
        buy("b1", "AAA", 10, 10, "2026-01-05"),
        dividend("d1", "AAA", 10, 0.5, "2026-08-06", "Cashflow"),
        charge("i1", 100, "2026-07-01", "CAD"),
        charge("i2", 20, "2026-08-01", "USD"),
        charge("i3", 10, "2026-08-04", "CAD"),
    ], json!({"fx": {"2026-08-01": 1.5, "2026-09-07": 1.5}, "benchmark": {}}));
    c.securities = Some(cash_secs.clone());
    c.accounts = Some(json!([{"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"}]));
    c.balances = Some(json!([{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0}]));
    out.push(("cashflow_margin_used_tile", c));

    // an income holding with a declared distribution record: rate, projection, ex-div and pay day
    out.push(("cashflow_holding_with_declared_record", case("2026-09-07", vec![
        buy_x("b1", "RDDY", 4000, 11.64, "2026-05-01", json!({"accountType": "Cashflow"})),
        dividend("d1", "RDDY", 4000, 0.15, "2026-08-06", "Cashflow"),
        dividend("d2", "RDDY", 4000, 0.15, "2026-09-04", "Cashflow"),
    ], json!({"fx": {}, "benchmark": {},
              "distributions": {"RDDY": [{"exDate": "2026-07-31", "payDate": "2026-08-06", "amount": 0.15, "currency": "CAD"},
                                         {"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.15, "currency": "CAD"},
                                         {"exDate": "2026-09-30", "payDate": "2026-10-06", "amount": 0.15, "currency": "CAD"}]}}))));

    // a short call covered and re-sold on the same day is a roll: the cover's P&L
    // folds into the far contract's basis and only the far contract is a trade
    out.push(("option_roll_same_day_folds", case("2027-01-01", vec![
        sto("aug-sto", "ZZZ 21AUG26 10.00 CALL", 1, 3, "2026-01-01"),
        btc("aug-cover", "ZZZ 21AUG26 10.00 CALL", 1, 1, "2026-08-15", "BUYTOCLOSE"),
        sto("jan-sto", "ZZZ 15JAN27 12.00 CALL", 1, 2, "2026-08-15"),
        btc("jan-cover", "ZZZ 15JAN27 12.00 CALL", 1, 0.5, "2026-12-01", "BUYTOCLOSE"),
    ], json!({"fx": {"2026-01-01": 1.40, "2026-08-15": 1.38, "2026-12-01": 1.36}, "benchmark": {}}))));

    // a multileg roll posts only the closing leg, with quantity 0: 16 short Jan27
    // calls rolled to Jan28, 6 more sold, all 22 bought back; nothing stays open
    out.push(("option_multileg_roll_carries_the_leg", case("2026-09-01", vec![
        sto("sto", "LUNR 15JAN27 12.00 CALL", 16, 6.2225, "2025-10-01"),
        multileg("ml", "LUNR 15JAN27 12.00 CALL", -2160, "2025-11-14"),
        sto("sto2", "LUNR 21JAN28 12.00 CALL", 6, 6.75, "2025-12-10"),
        btc("btc", "LUNR 21JAN28 12.00 CALL", 22, 13.3, "2026-06-26", "BUYTOOPEN"),
    ], empty_market())));

    // two multileg debits on one contract, a day apart, close 16 shorts (1, then the
    // 15 left) and carry them to the far contract; a second contract on the same
    // underlying is its own trade; 6 more sold on the far contract; all 22 bought
    // back in three fills. One chain trade, and the other contract's trade.
    out.push(("option_two_multilegs_then_chain", case("2026-09-08", vec![
        sto("sto16", "LUNR 15JAN27 12.00 CALL", 16, 6.2225, "2025-07-24"),
        sto("sto2", "LUNR 15JAN27 10.00 CALL", 2, 6.9025, "2025-07-24"),
        btc("btc2", "LUNR 15JAN27 10.00 CALL", 2, 4.55, "2025-11-06", "BUYTOOPEN"),
        multileg("ml1", "LUNR 15JAN27 12.00 CALL", -128, "2025-11-13"),
        multileg("ml2", "LUNR 15JAN27 12.00 CALL", -2025, "2025-11-14"),
        sto("sto6", "LUNR 21JAN28 12.00 CALL", 6, 6.75, "2025-12-10"),
        btc("b7", "LUNR 21JAN28 12.00 CALL", 7, 13.3, "2026-06-26", "BUYTOOPEN"),
        btc("b10", "LUNR 21JAN28 12.00 CALL", 10, 13.3, "2026-06-26", "BUYTOOPEN"),
        btc("b5", "LUNR 21JAN28 12.00 CALL", 5, 13.45, "2026-06-26", "BUYTOOPEN"),
    ], empty_market())));

    // short Dec puts rolled forward: the roll's only posted leg names a contract never
    // opened; the June buy-back of 26 closes the 11 known shorts, the carried leg and
    // the 9 old puts (nearest expiry first), and nothing stays open
    out.push(("option_rolled_chain_buy_back_closes_older_contracts", case("2026-09-08", vec![
        sto("s1", "BBAI 26DEC25 5.50 PUT", 3, 0.12, "2025-12-05"),
        sto("s2", "BBAI 02JAN26 5.50 PUT", 5, 0.2, "2025-12-11"),
        sto("s4", "BBAI 19DEC25 6.00 PUT", 6, 0.2, "2025-12-12"),
        sto("s3", "BBAI 26DEC25 6.00 PUT", 1, 0.4, "2025-12-15"),
        multileg("ml1", "BBAI 19DEC25 6.00 PUT", -18, "2025-12-15"),
        multileg("ml2", "BBAI 18JUN26 5.00 PUT", -1830, "2025-12-18"),
        sto("s5", "BBAI 21JAN28 5.00 PUT", 11, 2.4, "2026-02-27"),
        btc("btc", "BBAI 21JAN28 5.00 PUT", 26, 2.74, "2026-06-29", "BUYTOOPEN"),
    ], empty_market())));

    // two multileg debits with quantity 0 close a short in two pieces (1, then 15)
    out.push(("option_two_multilegs_close_a_short", case("2026-09-08", vec![
        sto("sto", "LUNR 15JAN27 12.00 CALL", 16, 6.2225, "2026-01-10"),
        multileg("ml1", "LUNR 15JAN27 12.00 CALL", -128, "2026-03-01"),
        multileg("ml2", "LUNR 15JAN27 12.00 CALL", -2025, "2026-03-01"),
    ], empty_market())));

    // credit multilegs on a short are a roll, one of them posted as SELLTOCLOSE
    out.push(("option_credit_multilegs_on_a_short", case("2026-09-08", vec![
        sto("sto", "BBAI 21JAN28 10.00 CALL", 3, 1.2, "2026-01-05"),
        act(json!({"id": "cr1", "category": "trade", "activityType": "OPTIONS_SELL", "activitySubType": "SELLTOCLOSE", "rawType": "OPTIONS_MULTILEG", "quantity": 0, "netCashAmount": 14, "transactionDate": "2026-02-01", "symbol": "BBAI 21JAN28 10.00 CALL"})),
        multileg("cr2", "BBAI 21JAN28 10.00 CALL", 56, "2026-02-01"),
    ], empty_market())));

    // a chain that folds twice: the first cover folds into the second contract, which is
    // then itself covered and folds, with that adjusted P&L, into the third
    out.push(("option_roll_chain_folded_twice", case("2026-09-08", vec![
        sto("s1", "QQQ 20MAR26 5.00 PUT", 1, 0.5, "2025-12-01"),
        btc("c1", "QQQ 20MAR26 5.00 PUT", 1, 1.5, "2025-12-15", "BUYTOCLOSE"),
        sto("s2", "QQQ 17APR26 5.00 PUT", 1, 2.0, "2025-12-15"),
        btc("c2", "QQQ 17APR26 5.00 PUT", 1, 3.0, "2025-12-18", "BUYTOCLOSE"),
        sto("s3", "QQQ 15MAY26 5.00 PUT", 1, 4.0, "2025-12-18"),
        btc("c3", "QQQ 15MAY26 5.00 PUT", 1, 1.0, "2026-02-02", "BUYTOCLOSE"),
    ], empty_market())));

    // a credit roll up: two multileg credits on the 10 call move 5 shorts to
    // the 12 call, then the 12 calls are bought back
    out.push(("option_credit_roll_up", case("2026-09-01", vec![
        sto("sto", "BBAI 21JAN28 10.00 CALL", 5, 3.0, "2025-11-12"),
        multileg("cr1", "BBAI 21JAN28 10.00 CALL", 14, "2026-06-09"),
        multileg("cr2", "BBAI 21JAN28 10.00 CALL", 56, "2026-06-17"),
        btc("btc", "BBAI 21JAN28 12.00 CALL", 5, 0.85, "2026-06-26", "BUYTOOPEN"),
    ], empty_market())));

    // a posted short expiry closes the short at zero and keeps the premium
    out.push(("option_short_expiry_posted", case("2027-02-01", vec![
        sto("sto", "ABC 15JAN27 10.00 CALL", 5, 2, "2026-01-10"),
        act(json!({"id": "exp", "activityType": "OPTIONS_SHORT_EXPIRY", "activitySubType": "EXPIRED", "rawType": "OPTIONS_SHORT_EXPIRY", "quantity": 5, "transactionDate": "2027-01-15", "symbol": "ABC 15JAN27 10.00 CALL"})),
    ], empty_market())));

    // Wealthsimple posted no expiry row: a lot still open after its expiry date
    // closes at zero on that date; a contract not yet expired stays open
    out.push(("option_expiry_assumed", case("2026-03-01", vec![
        sto("sto", "BBAI 02JAN26 5.50 PUT", 2, 0.3, "2025-12-05"),
        btc("bto", "ZZZ 17JUL26 10.00 CALL", 1, 1.0, "2026-01-05", "BUYTOOPEN"),
    ], empty_market())));

    // a long option that expired worthless, posted as a long expiry
    out.push(("option_long_expiry_posted", case("2025-09-01", vec![
        btc("bto", "LUNR 22AUG25 8.00 CALL", 2, 0.4, "2025-07-01", "BUYTOOPEN"),
        act(json!({"id": "exp", "category": "option_event", "activityType": "EXPIR", "activitySubType": "BUY", "rawType": "OPTIONS_EXPIRY", "quantity": 2, "transactionDate": "2025-08-22", "symbol": "LUNR 22AUG25 8.00 CALL"})),
    ], empty_market())));

    // an assigned covered call: the option keeps its premium and the shares are
    // sold at the strike; the share leg is derived, Wealthsimple posts only the option
    let mut c = case("2026-09-06", vec![
        buy_x("b1", "ASTS", 300, 25.0, "2025-01-10", json!({"currency": "USD", "securityId": "sec-s-asts"})),
        sto_x("sto", "ASTS 07MAR25 31.00 CALL", 3, 1.5, "2025-02-10", json!({"securityId": "sec-o-asts"})),
        act(json!({"id": "asg", "category": "option_event", "activityType": "ASSIGN", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_ASSIGN", "quantity": 3, "unitPrice": 0, "netCashAmount": 9300, "transactionDate": "2025-03-07", "symbol": "ASTS 07MAR25 31.00 CALL", "securityId": "sec-o-asts"})),
    ], json!({"fx": {"2025-01-10": 1.44, "2025-02-10": 1.43, "2025-03-07": 1.43}, "benchmark": {}}));
    c.securities = Some(json!([{"id": "sec-o-asts", "symbol": "ASTS", "underlyingId": "sec-s-asts"}, {"id": "sec-s-asts", "symbol": "ASTS", "name": "AST SpaceMobile", "primaryExchange": "NASDAQ"}]));
    out.push(("option_assignment_call_delivers_shares", c));

    // an assigned short put buys the shares at the strike: a new open position
    out.push(("option_assignment_put_buys_shares", case("2026-01-01", vec![
        sto("sto", "BBAI 05DEC25 5.00 PUT", 1, 0.5, "2025-11-10"),
        act(json!({"id": "asg", "category": "option_event", "activityType": "ASSIGN", "activitySubType": "BUYTOCLOSE", "rawType": "OPTIONS_ASSIGN", "quantity": 1, "unitPrice": 0, "netCashAmount": -500, "transactionDate": "2025-12-05", "symbol": "BBAI 05DEC25 5.00 PUT"})),
    ], empty_market())));

    // coins sent out of the account leave at cost: no slice, no P&L; the sale that
    // follows closes what is left, first-in first-out (1 ETH at 100, 1 at 120)
    out.push(("crypto_transfer_out_leaves_at_cost", case("2026-03-01", vec![
        crypto("cb", "buy", "ETH", 2, 100, "2026-01-01"),
        crypto_transfer("ti", "ETH", 1, 120, "2026-01-05", false),
        crypto_transfer("to", "ETH", 1, 200, "2026-01-10", true),
        crypto("cs", "sell", "ETH", 2, 150, "2026-02-01"),
    ], empty_market())));

    // crypto bought, a staking reward (a lot at zero cost), then everything sold
    out.push(("crypto_buy_reward_sell", case("2026-03-01", vec![
        crypto("cb", "buy", "ETH", 2, 100, "2026-01-01"),
        crypto("rw", "reward", "ETH", 1, 0, "2026-01-05"),
        crypto("cs", "sell", "ETH", 3, 150, "2026-02-01"),
    ], empty_market())));

    // a monthly payer between ex-date and pay day: the distribution still to be
    // paid is the one shown, its ex-date passed, its pay day not
    out.push(("cashflow_between_ex_date_and_pay_day", case("2026-09-07", vec![
        buy_x("b1", "EASY", 1000, 20.0, "2026-05-01", json!({"accountType": "Cashflow"})),
        dividend("d1", "EASY", 1000, 0.20, "2026-07-08", "Cashflow"),
        dividend("d2", "EASY", 1000, 0.20, "2026-08-08", "Cashflow"),
    ], json!({"fx": {}, "benchmark": {},
              "distributions": {"EASY": [{"exDate": "2026-06-30", "payDate": "2026-07-08", "amount": 0.20, "currency": "CAD"},
                                         {"exDate": "2026-07-31", "payDate": "2026-08-08", "amount": 0.20, "currency": "CAD"},
                                         {"exDate": "2026-08-31", "payDate": "2026-09-08", "amount": 0.21, "currency": "CAD"}]}}))));

    // the equity series with a pre-history balance, a deposit and a withdrawal: yearly
    // returns net of flows, the index over the same spans, annualized, drawdown
    let aaa_trip = || vec![buy("b1", "AAA", 100, 10.0, "2026-01-05"), sell("s1", "AAA", 100, 12.0, "2026-01-20")];
    let nav_market = || json!({"fx": {}, "benchmark": sp500(), "benchmarks": {"TSX": tsx()}});
    let mut c = case("2026-09-07", aaa_trip(), nav_market());
    c.nav = Some(json!(nav_rows()));
    out.push(("nav_yearly_returns_and_drawdown", c));

    // the same series compared against the S&P/TSX instead
    let mut c = case("2026-09-07", aaa_trip(), nav_market());
    c.nav = Some(json!(nav_rows()));
    c.filters = Some(json!({"benchmark": "TSX"}));
    out.push(("nav_against_the_tsx", c));

    // no filter: every trade and position, the journal on the trades, the dashboard cards
    out.push(("dashboard_journal_monthly_by_symbol_queue", filter_case(None, true)));
    // one account and this year: trades scoped by close date, positions by account, the
    // equity series of that account, Cashflow scoped to the account and the dates
    out.push(("filters_account_and_ytd", filter_case(Some(json!({"lists": {"account": ["Trading"]}, "preset": "ytd"})), true)));
    // a symbol filter matches an option by its underlying; Result keeps the winners
    out.push(("filters_symbol_by_underlying_and_result", filter_case(Some(json!({"lists": {"symbol": ["ZZZ", "AAA"], "result": ["Winners"]}})), false)));
    // a year, a tag, a grade and a range together; Cashflow ignores the ones it cannot apply
    out.push(("filters_year_tag_grade_and_range", filter_case(Some(json!({"years": ["2026"], "lists": {"tag": ["earnings"], "grade": ["A", "Ungraded"]}, "ranges": {"hold": {"op": ">", "v": 5}}})), false)));
    // kind and a date range; then exchange from the security record; then free text
    out.push(("filters_kind_and_date_range", filter_case(Some(json!({"lists": {"kind": ["Options", "Shares"]}, "from": "2026-02-01", "to": "2026-05-31"})), false)));
    out.push(("filters_exchange_and_search", filter_case(Some(json!({"lists": {"exchange": ["TSX"]}, "search": "a"})), false)));

    // no declared record: the rate and frequency come from the payments received
    // (quarterly, read from the gaps), the ex-date from the quote, the pay day
    // from the last payment
    out.push(("cashflow_holding_from_payments_only", case("2026-09-07", vec![
        buy_x("b1", "QQQQ", 200, 50.0, "2025-10-01", json!({"accountType": "Cashflow"})),
        dividend("d1", "QQQQ", 200, 0.30, "2025-12-15", "Cashflow"),
        dividend("d2", "QQQQ", 200, 0.30, "2026-03-16", "Cashflow"),
        dividend("d3", "QQQQ", 200, 0.32, "2026-06-15", "Cashflow"),
    ], json!({"fx": {}, "benchmark": {}, "distributions": {}, "quotes": {"QQQQ": {"exDividendDate": "2026-09-15"}}}))));

    out
}

// --------------------------------------------------------------------------
// the expectation: what every implementation must produce for one case
// --------------------------------------------------------------------------

pub const TRADE_KEYS: [&str; 21] = [
    "id", "symbol", "kind", "currency", "side", "qty", "mult", "entry", "exit", "entryDate", "exitDate",
    "holdDays", "pnl", "pnlCad", "pnlPct", "status", "fees", "account", "exchange", "grade", "tags",
];
pub const KPI_KEYS: [&str; 13] = [
    "count", "wins", "losses", "breakeven", "winRate", "realized", "expectancy", "profitFactor", "avgHold",
    "avgWin", "avgLoss", "grossWin", "grossLoss",
];
pub const POSITION_KEYS: [&str; 14] = [
    "id", "symbol", "kind", "currency", "account", "exchange", "qty", "avg", "cost", "held", "alloc", "short",
    "dayChange", "grade",
];
pub const PORTFOLIO_KEYS: [&str; 18] = [
    "marketValue", "costBasis", "unrealized", "unrealizedPct", "positionCount", "accountCount", "nav",
    "navAccounts", "marginUsed", "marginUsedBy", "marginUsedPct", "availableMargin",
    "availableMarginUnavailable", "hasMargin", "cash", "cashPct", "dayChange", "dayChangePct",
];
pub const ALLOCATION_KEYS: [&str; 5] = ["id", "symbol", "account", "value", "share"];
pub const YEAR_KEYS: [&str; 8] = ["year", "r", "days", "from", "to", "flow", "endV", "spR"];
pub const MONTH_KEYS: [&str; 4] = ["key", "label", "value", "count"];
pub const SYMBOL_KEYS: [&str; 6] = ["symbol", "pnl", "n", "legs", "winRate", "avgHold"];
pub const QUEUE_KEYS: [&str; 5] = ["id", "symbol", "date", "pnl", "missing"];
pub const HOLDING_KEYS: [&str; 14] = [
    "symbol", "qty", "per", "freq", "freqVerified", "annual", "yoc", "ytd", "ttm", "all", "nextExDate",
    "nextPayDate", "exPast", "payPast",
];
pub const TILE_KEYS: [&str; 11] = [
    "label", "total", "perMonth", "count", "yield", "projected", "earned", "book", "marginUsed",
    "interestPerMonth", "interestMonths",
];

/// The listed keys, when present.
fn pick(d: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for k in keys {
        if let Some(v) = d.get(*k) {
            out.insert((*k).into(), v.clone());
        }
    }
    Value::Object(out)
}

/// Every float to six decimals; integers stay integers.
pub fn rounded(v: &Value) -> Value {
    match v {
        Value::Number(n) if !(n.is_i64() || n.is_u64()) => json!(round_half_even(n.as_f64().unwrap(), 6)),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), rounded(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(rounded).collect()),
        _ => v.clone(),
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn rows(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

fn with_fills(row: &Value, keys: &[&str]) -> Value {
    let mut fills = rows(&row["fills"]);
    fills.sort_by(|a, b| s(a, "when").cmp(&s(b, "when")));
    let mut picked = pick(row, keys);
    picked["fills"] = Value::Array(fills.iter().map(|f| json!(s(f, "sub"))).collect());
    picked
}

fn each(v: &Value, keys: &[&str]) -> Value {
    Value::Array(rows(v).iter().map(|r| pick(r, keys)).collect())
}

/// What every implementation must produce for one case: the view for the filters.
pub fn expect_from(snap: &Value, market: &Value, today: &str, filters: &Value, journal: Option<&Value>) -> Value {
    let journal = journal.and_then(|j| j.as_object()).cloned().unwrap_or_default();
    let base = build_base(snap, market, &journal, Some(today));
    let view = build_view(&base, Some(filters));

    let mut trades = rows(&view["trades"]);
    trades.sort_by(|a, b| {
        (s(a, "entryDate"), s(a, "exitDate"), s(a, "symbol")).cmp(&(s(b, "entryDate"), s(b, "exitDate"), s(b, "symbol")))
    });
    let mut positions = rows(&view["positions"]);
    positions.sort_by(|a, b| (s(a, "symbol"), s(a, "account")).cmp(&(s(b, "symbol"), s(b, "account"))));
    let cf = &view["cashflow"];
    let mut holdings = rows(&cf["holdings"]);
    holdings.sort_by(|a, b| s(a, "symbol").cmp(&s(b, "symbol")));

    let mut portfolio = pick(&view["portfolio"], &PORTFOLIO_KEYS);
    portfolio["allocation"] = each(&view["portfolio"]["allocation"], &ALLOCATION_KEYS);

    let out = json!({
        "kpi": pick(&view["kpi"], &KPI_KEYS),
        "trades": trades.iter().map(|t| with_fills(t, &TRADE_KEYS)).collect::<Vec<_>>(),
        "positions": positions.iter().map(|p| with_fills(p, &POSITION_KEYS)).collect::<Vec<_>>(),
        "positionsSummary": view["positionsSummary"],
        "portfolio": portfolio,
        "equity": {
            "label": view["equity"]["label"],
            "series": rows(&view["equity"]["series"]).iter().map(|p| json!({"d": p["d"], "v": p["v"]})).collect::<Vec<_>>(),
            "drawdown": view["equity"]["drawdown"],
            "annualized": view["equity"]["annualized"],
        },
        "years": each(&view["years"], &YEAR_KEYS),
        "benchmark": view["benchmark"],
        "monthly": each(&view["monthly"], &MONTH_KEYS),
        "bySymbol": each(&view["bySymbol"], &SYMBOL_KEYS),
        "grades": {
            "buckets": each(&view["grades"]["buckets"], &["grade", "n", "pnl"]),
            "ungraded": view["grades"]["ungraded"],
            "graded": view["grades"]["graded"],
        },
        "queue": each(&view["queue"], &QUEUE_KEYS),
        "options": pick(&view["options"], &["accounts", "symbols", "tags", "exchanges", "kinds", "years"]),
        "cashflowHoldings": holdings.iter().map(|h| pick(h, &HOLDING_KEYS)).collect::<Vec<_>>(),
        "cashflowTiles": each(&cf["tiles"], &TILE_KEYS),
        "cashflowMonths": each(&cf["months"], &MONTH_KEYS),
        "cashflowTotal": cf["total"],
        "cashflowCount": cf["count"],
        "cashflowSkipped": cf["skippedFilters"],
    });
    rounded(&out)
}

pub fn case_snapshot(c: &Case) -> Value {
    snapshot(
        c.activities.clone(), c.securities.clone(), c.nav.clone(), c.nav_by_account.clone(),
        c.accounts.clone(), c.balances.clone(), c.margin.clone(),
    )
}

pub fn expect(c: &Case) -> Value {
    let filters = c.filters.clone().unwrap_or(json!({}));
    expect_from(&case_snapshot(c), &c.market, c.today, &filters, c.journal.as_ref())
}

/// The file a case is written as.
pub fn case_doc(c: &Case) -> Value {
    json!({
        "today": c.today, "snapshot": case_snapshot(c), "market": c.market,
        "filters": c.filters.clone().unwrap_or(json!({})), "journal": c.journal.clone().unwrap_or(json!({})),
        "expect": expect(c),
    })
}

// --------------------------------------------------------------------------
// JSON as the case files are written: two-space indent, sorted keys, ASCII
// --------------------------------------------------------------------------

/// A float the way the files print it: the shortest digits that read back,
/// positional between 1e-4 and 1e16, else an exponent of at least two digits.
pub fn float_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let e_form = format!("{:e}", x);
    let (mant, exp) = e_form.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let (sign, mant) = if let Some(m) = mant.strip_prefix('-') { ("-", m) } else { ("", mant) };
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let n = digits.len() as i32;
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp + 1;
            if n <= int_len {
                format!("{}{}{}.0", sign, digits, "0".repeat((int_len - n) as usize))
            } else {
                format!("{}{}.{}", sign, &digits[..int_len as usize], &digits[int_len as usize..])
            }
        } else {
            format!("{}0.{}{}", sign, "0".repeat((-exp - 1) as usize), digits)
        }
    } else {
        let rest = if n > 1 { format!(".{}", &digits[1..]) } else { String::new() };
        format!("{}{}{}e{}{:02}", sign, &digits[..1], rest, if exp < 0 { "-" } else { "+" }, exp.abs())
    }
}

fn string_text(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(v: &Value, indent: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                out.push_str(&n.to_string());
            } else {
                out.push_str(&float_text(n.as_f64().unwrap()));
            }
        }
        Value::String(s) => string_text(s, out),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&" ".repeat(indent + 2));
                write_value(x, indent + 2, out);
            }
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&" ".repeat(indent + 2));
                string_text(k, out);
                out.push_str(": ");
                write_value(&m[*k], indent + 2, out);
            }
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            out.push('}');
        }
    }
}

/// The text of a case file, trailing newline included.
pub fn to_text(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, 0, &mut out);
    out.push('\n');
    out
}
