//! Reads one shared case on stdin ({snapshot, market, today, filters,
//! journal}) and writes the same projection `tests/make_cases.expect_from`
//! writes, so a case file can be checked against this model directly.

use serde_json::{json, Map, Value};
use std::io::Read;

const TRADE_KEYS: [&str; 21] = [
    "id", "symbol", "kind", "currency", "side", "qty", "mult", "entry", "exit", "entryDate", "exitDate",
    "holdDays", "pnl", "pnlCad", "pnlPct", "status", "fees", "account", "exchange", "grade", "tags",
];
const KPI_KEYS: [&str; 13] = [
    "count", "wins", "losses", "breakeven", "winRate", "realized", "expectancy", "profitFactor", "avgHold",
    "avgWin", "avgLoss", "grossWin", "grossLoss",
];
const POSITION_KEYS: [&str; 14] = [
    "id", "symbol", "kind", "currency", "account", "exchange", "qty", "avg", "cost", "held", "alloc", "short",
    "dayChange", "grade",
];
const PORTFOLIO_KEYS: [&str; 18] = [
    "marketValue", "costBasis", "unrealized", "unrealizedPct", "positionCount", "accountCount", "nav",
    "navAccounts", "marginUsed", "marginUsedBy", "marginUsedPct", "availableMargin",
    "availableMarginUnavailable", "hasMargin", "cash", "cashPct", "dayChange", "dayChangePct",
];
const ALLOCATION_KEYS: [&str; 5] = ["id", "symbol", "account", "value", "share"];
const YEAR_KEYS: [&str; 8] = ["year", "r", "days", "from", "to", "flow", "endV", "spR"];
const MONTH_KEYS: [&str; 4] = ["key", "label", "value", "count"];
const SYMBOL_KEYS: [&str; 6] = ["symbol", "pnl", "n", "legs", "winRate", "avgHold"];
const QUEUE_KEYS: [&str; 5] = ["id", "symbol", "date", "pnl", "missing"];
const HOLDING_KEYS: [&str; 14] = [
    "symbol", "qty", "per", "freq", "freqVerified", "annual", "yoc", "ytd", "ttm", "all", "nextExDate",
    "nextPayDate", "exPast", "payPast",
];
const TILE_KEYS: [&str; 11] = [
    "label", "total", "perMonth", "count", "yield", "projected", "earned", "book", "marginUsed",
    "interestPerMonth", "interestMonths",
];

/// `make_cases.pick`: the listed keys, in the listed order, when present.
fn pick(d: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for k in keys {
        if let Some(v) = d.get(*k) {
            out.insert((*k).into(), v.clone());
        }
    }
    Value::Object(out)
}

/// `make_cases.rounded`: every float to six decimals.
fn rounded(v: &Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() => {
                json!((f * 1e6).round() / 1e6)
            }
            _ => v.clone(),
        },
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), rounded(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(rounded).collect()),
        _ => v.clone(),
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// The fill sub-labels in `when` order, which is what a case records.
fn fill_subs(row: &Value) -> Value {
    let mut fills: Vec<Value> = row.get("fills").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    fills.sort_by(|a, b| s(a, "when").cmp(&s(b, "when")));
    Value::Array(fills.iter().map(|f| json!(s(f, "sub"))).collect())
}

fn with_fills(row: &Value, keys: &[&str]) -> Value {
    let mut picked = pick(row, keys);
    picked["fills"] = fill_subs(row);
    picked
}

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();

    let snapshot = doc.get("snapshot").cloned().unwrap_or(Value::Null);
    let market = doc.get("market").cloned().unwrap_or(Value::Null);
    let today = doc.get("today").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let journal = doc.get("journal").and_then(|v| v.as_object()).cloned().unwrap_or_default();
    let filters = doc.get("filters").cloned();

    let base = bagholder_model::base::build_base(&snapshot, &market, &journal, Some(&today));
    let view = bagholder_model::view::build_view(&base, filters.as_ref());

    let mut trades: Vec<Value> = view["trades"].as_array().cloned().unwrap_or_default();
    trades.sort_by(|a, b| {
        (s(a, "entryDate"), s(a, "exitDate"), s(a, "symbol"))
            .cmp(&(s(b, "entryDate"), s(b, "exitDate"), s(b, "symbol")))
    });
    let mut positions: Vec<Value> = view["positions"].as_array().cloned().unwrap_or_default();
    positions.sort_by(|a, b| (s(a, "symbol"), s(a, "account")).cmp(&(s(b, "symbol"), s(b, "account"))));

    let cf = &view["cashflow"];
    let mut holdings: Vec<Value> = cf["holdings"].as_array().cloned().unwrap_or_default();
    holdings.sort_by(|a, b| s(a, "symbol").cmp(&s(b, "symbol")));

    let mut portfolio = pick(&view["portfolio"], &PORTFOLIO_KEYS);
    portfolio["allocation"] = Value::Array(
        view["portfolio"]["allocation"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|a| pick(a, &ALLOCATION_KEYS))
            .collect(),
    );

    let out = json!({
        "kpi": pick(&view["kpi"], &KPI_KEYS),
        "trades": trades.iter().map(|t| with_fills(t, &TRADE_KEYS)).collect::<Vec<_>>(),
        "positions": positions.iter().map(|p| with_fills(p, &POSITION_KEYS)).collect::<Vec<_>>(),
        "positionsSummary": view["positionsSummary"],
        "portfolio": portfolio,
        "equity": {
            "label": view["equity"]["label"],
            "series": view["equity"]["series"].as_array().cloned().unwrap_or_default()
                .iter().map(|p| json!({"d": p["d"], "v": p["v"]})).collect::<Vec<_>>(),
            "drawdown": view["equity"]["drawdown"],
            "annualized": view["equity"]["annualized"],
        },
        "years": view["years"].as_array().cloned().unwrap_or_default().iter().map(|y| pick(y, &YEAR_KEYS)).collect::<Vec<_>>(),
        "benchmark": view["benchmark"],
        "monthly": view["monthly"].as_array().cloned().unwrap_or_default().iter().map(|m| pick(m, &MONTH_KEYS)).collect::<Vec<_>>(),
        "bySymbol": view["bySymbol"].as_array().cloned().unwrap_or_default().iter().map(|r| pick(r, &SYMBOL_KEYS)).collect::<Vec<_>>(),
        "grades": {
            "buckets": view["grades"]["buckets"].as_array().cloned().unwrap_or_default()
                .iter().map(|b| pick(b, &["grade", "n", "pnl"])).collect::<Vec<_>>(),
            "ungraded": view["grades"]["ungraded"],
            "graded": view["grades"]["graded"],
        },
        "queue": view["queue"].as_array().cloned().unwrap_or_default().iter().map(|q| pick(q, &QUEUE_KEYS)).collect::<Vec<_>>(),
        "options": {
            "accounts": view["options"]["accounts"],
            "symbols": view["options"]["symbols"],
            "tags": view["options"]["tags"],
            "exchanges": view["options"]["exchanges"],
            "kinds": view["options"]["kinds"],
            "years": view["options"]["years"],
        },
        "cashflowHoldings": holdings.iter().map(|h| pick(h, &HOLDING_KEYS)).collect::<Vec<_>>(),
        "cashflowTiles": cf["tiles"].as_array().cloned().unwrap_or_default().iter().map(|t| pick(t, &TILE_KEYS)).collect::<Vec<_>>(),
        "cashflowMonths": cf["months"].as_array().cloned().unwrap_or_default().iter().map(|m| pick(m, &MONTH_KEYS)).collect::<Vec<_>>(),
        "cashflowTotal": cf["total"],
        "cashflowCount": cf["count"],
        "cashflowSkipped": cf["skippedFilters"],
    });
    println!("{}", serde_json::to_string(&rounded(&out)).unwrap());
}
