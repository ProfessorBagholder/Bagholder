//! The wire contract: what the model hands the page, whole, for every shared
//! case and for a few scenarios the cases do not reach (quotes, splits, a stock
//! distribution, saved groups). `tests/cases` pins the figures, rounded and
//! picked key by key; this pins the payload itself -- every key, in order, every
//! value -- so the model's internals can be replaced (typed structs for JSON
//! values, layered caches for one) and any difference the page could see fails
//! here. (It found its first fault at once: the view carried the wall clock, which
//! no page read and which made every build of it differ; that is gone.) Strings, keys, key order, array order and lengths are exact. Numbers
//! are equal to within what a different machine's `powf` can move the last bit.
//!
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-model
//! --test wire`, and review the diff of `tests/wire`.

use serde_json::{json, Value};
use std::path::PathBuf;

use bagholder_model::base::build_base;
use bagholder_model::cases::{act, buy, buy_x, sell};
use bagholder_model::view::{build_view, trade_detail, view_of, Detail};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests")
}

/// Everything the page can be sent for one book: the view as built, the view as
/// sent (slim), the view as sent with one trade's detail, and that trade's own
/// detail payload.
fn payload(snapshot: &Value, market: &Value, journal: &Value, today: &str, filters: &Value) -> Value {
    let journal = journal.as_object().cloned().unwrap_or_default();
    let base = build_base(snapshot, market, &journal, Some(today));
    let view = build_view(&base, Some(filters));
    let first = view.trades.first().map(|t| t.id.clone());
    json!({
        "view": view.to_value(),
        "wire": view_of(&base, Some(filters), Detail::Only(None)).to_value(),
        "wireWithDetail": first.as_deref().map(|id| view_of(&base, Some(filters), Detail::Only(Some(id))).to_value()),
        "tradeDetail": first.as_deref().and_then(|id| trade_detail(&base, id)),
    })
}

/// Where two payloads first differ, or nothing.
fn differ(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            let close = x == y || (x - y).abs() <= 1e-12_f64.max(1e-9 * x.abs().max(y.abs()));
            if close { None } else { Some(format!("{}: {} != {}", path, x, y)) }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{}: {} items != {}", path, x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| differ(&format!("{}[{}]", path, i), p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            let (kx, ky): (Vec<&String>, Vec<&String>) = (x.keys().collect(), y.keys().collect());
            if kx != ky {
                return Some(format!("{}: keys {:?} != {:?}", path, kx, ky));
            }
            x.iter().find_map(|(k, v)| differ(&format!("{}.{}", path, k), v, &y[k]))
        }
        _ => if a == b { None } else { Some(format!("{}: {} != {}", path, a, b)) },
    }
}

fn check(name: &str, got: &Value) {
    let path = root().join("wire").join(format!("{}.json", name));
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(got).unwrap() + "\n").unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no wire snapshot for {}: run with BAGHOLDER_BLESS=1 and review it", name));
    let want: Value = serde_json::from_str(&text).unwrap();
    if let Some(d) = differ(name, got, &want) {
        panic!("the payload changed -- {}", d);
    }
}

#[test]
fn test_every_shared_case_keeps_its_payload() {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root().join("cases")).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
    paths.sort();
    assert!(!paths.is_empty(), "no cases found");
    for p in paths {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let got = payload(&doc["snapshot"], &doc["market"], doc.get("journal").unwrap_or(&json!({})), doc["today"].as_str().unwrap(), &doc["filters"]);
        check(&format!("case_{}", p.file_stem().unwrap().to_string_lossy()), &got);
    }
}

fn snap(acts: Vec<Value>, groups: Value) -> Value {
    json!({"activities": acts, "accounts": [], "balances": [], "margin": [], "navHistory": [], "navByAccount": {}, "syncedAt": "", "tradeGroups": groups, "notes": {}, "securities": []})
}

fn market(quotes: Value) -> Value {
    json!({"fx": {"2026-01-05": 1.37, "2026-03-02": 1.41}, "benchmark": {}, "distributions": {}, "quotes": quotes})
}

/// Open positions marked at quotes: a share, one left at its fill, a coin and a
/// share sharing the symbol BTC, and an option contract.
#[test]
fn test_quotes_keep_their_payload() {
    let s = snap(vec![
        buy_x("b1", "VEQT", 100, 49.76, "2026-01-05", json!({"accountType": "Kids", "currency": "CAD"})),
        buy_x("b2", "HBIX", 100, 7.0, "2026-01-05", json!({"accountType": "Kids", "currency": "CAD"})),
        act(json!({"id": "c1", "category": "trade", "activityType": "BUY", "rawType": "CRYPTO_BUY", "quantity": 0.5, "unitPrice": 100000, "netCashAmount": -50000, "transactionDate": "2026-01-05", "symbol": "BTC", "currency": "CAD", "accountType": "Crypto", "securityId": "sec-z-btc-1"})),
        act(json!({"id": "s1", "category": "trade", "activityType": "BUY", "rawType": "DIY_BUY", "quantity": 4653, "unitPrice": 1.75, "netCashAmount": -8142.75, "transactionDate": "2026-02-05", "symbol": "BTC", "currency": "CAD", "accountType": "TFSA", "securityId": "sec-s-btc-warrant"})),
        act(json!({"id": "o1", "category": "trade", "activityType": "BUY", "rawType": "OPTIONS_BUY", "quantity": 2, "unitPrice": 0.10, "netCashAmount": -20, "transactionDate": "2026-02-05", "symbol": "QNC 20NOV26 3.00 CALL", "currency": "USD", "accountType": "TFSA", "securityId": "sec-o-1"})),
    ], json!([]));
    let quotes = json!({
        "VEQT": {"price": 62.4, "priceChange": 0.08, "percentChange": 0.128, "fetchedAt": "2026-09-06T14:00:00Z"},
        "BTC": {"price": 109998.0, "source": "coinbase"},
        "QNC 20NOV26 3.00 CALL": {"price": 0.15},
    });
    check("scenario_quotes", &payload(&s, &market(quotes), &json!({}), "2026-09-06", &json!({})));
}

/// A reverse split that rescales open lots, then a sale; and a forward split
/// left open.
#[test]
fn test_splits_keep_their_payload() {
    let marker = |id: &str, sym: &str, day: &str| act(json!({"id": id, "category": "trade", "activityType": "STKDIS", "activitySubType": "BUY", "rawType": "CORPORATE_ACTION", "quantity": 0, "transactionDate": day, "symbol": sym, "currency": "CAD"}));
    let s = snap(vec![
        buy_x("b1", "MSTY", 100, 7.0, "2025-12-01", json!({"currency": "CAD"})),
        buy_x("b2", "MSTY", 75, 6.9, "2025-12-05", json!({"currency": "CAD"})),
        marker("ca1", "MSTY", "2025-12-08"),
        buy_x("b3", "MSTY", 4, 34.0, "2025-12-11", json!({"currency": "CAD"})),
        act(json!({"id": "s1", "category": "trade", "activityType": "SELL", "rawType": "DIY_SELL", "quantity": -39, "unitPrice": 31.0, "netCashAmount": 1209.0, "transactionDate": "2026-01-16", "symbol": "MSTY", "currency": "CAD", "name": "MSTY"})),
        buy_x("n1", "NVDA", 10, 1000.0, "2024-05-01", json!({"currency": "USD"})),
        marker("ca2", "NVDA", "2024-06-10"),
        buy_x("n2", "NVDA", 5, 98.0, "2024-06-12", json!({"currency": "USD"})),
    ], json!([]));
    check("scenario_splits", &payload(&s, &market(json!({})), &json!({}), "2026-03-02", &json!({})));
}

/// A stock distribution that renames a holding (both legs, and the one-leg
/// code change), each then sold.
#[test]
fn test_a_stock_distribution_keeps_its_payload() {
    let leg = |id: &str, sub: &str, raw: &str, qty: i64, sym: &str| act(json!({"id": id, "category": "trade", "activityType": "STKDIS", "activitySubType": sub, "rawType": raw, "quantity": qty, "transactionDate": "2026-02-01", "symbol": sym, "currency": "CAD", "name": sym}));
    let s = snap(vec![
        buy_x("b", "OLD", 100, 2, "2026-01-01", json!({"currency": "CAD", "name": "OLD"})),
        leg("out", "SELL", "CORPORATE_ACTION", -100, "OLD"),
        leg("in", "BUY", "CORPORATE_ACTION", 100, "NEW"),
        act(json!({"id": "s", "category": "trade", "activityType": "SELL", "rawType": "DIY_SELL", "quantity": -100, "unitPrice": 3, "netCashAmount": 300, "transactionDate": "2026-03-01", "symbol": "NEW", "currency": "CAD", "name": "NEW"})),
        buy_x("b2", "WAS", 50, 4, "2026-01-02", json!({"currency": "CAD", "name": "WAS"})),
        leg("out2", "SELL", "CODE_CHANGE", -50, "WAS"),
        act(json!({"id": "s2", "category": "trade", "activityType": "SELL", "rawType": "DIY_SELL", "quantity": -50, "unitPrice": 5, "netCashAmount": 250, "transactionDate": "2026-03-01", "symbol": "NOW", "currency": "CAD", "name": "NOW"})),
    ], json!([]));
    check("scenario_stock_distribution", &payload(&s, &market(json!({})), &json!({}), "2026-03-02", &json!({})));
}

/// Two round trips joined by a saved, locked group, with a journal entry on it
/// and a filter in force.
#[test]
fn test_saved_groups_keep_their_payload() {
    let s = snap(vec![
        buy("b1", "AAA", 100, 10, "2026-01-01"),
        sell("s1", "AAA", 100, 12, "2026-01-10"),
        buy("b2", "AAA", 50, 11, "2026-02-01"),
        sell("s2", "AAA", 50, 9, "2026-02-10"),
        buy("b3", "BBB", 10, 5, "2026-02-03"),
        sell("s3", "BBB", 10, 6, "2026-02-20"),
    ], json!([{"id": "g_manual", "locked": true, "members": [format!("b1|s1|{:.8}", 100.0), format!("b2|s2|{:.8}", 50.0)]}]));
    let journal = json!({"g_manual": {"thesis": "two swings, one idea", "tags": ["swing"], "grade": "B"}});
    check("scenario_saved_groups", &payload(&s, &market(json!({})), &journal, "2026-03-02", &json!({})));
    check("scenario_saved_groups_filtered", &payload(&s, &market(json!({})), &journal, "2026-03-02", &json!({"lists": {"symbol": ["AAA"]}})));
}

/// The Markets tab and the corners of the Portfolio no case reaches: a watchlist
/// (a held listing, an index, a coin, a plain listing), news (a story two listings
/// share, a release carried twice, the market feed, a French twin), a universe,
/// a tile row with the two rate contracts (one with a move, one without),
/// exposures by security and by share key, an option looked through to its
/// underlying, balances that give `wsQty`, cash and margin drawn, buying power
/// known and unknown, and a sale the book cannot match.
#[test]
fn test_markets_and_the_corners_keep_their_payload() {
    let mut s = snap(vec![
        buy_x("b1", "ENB", 100, 50.0, "2026-01-05", json!({"accountType": "Margin", "accountId": "acct-m", "currency": "CAD", "securityId": "sec-enb", "name": "Enbridge"})),
        buy_x("b2", "ASTS", 10, 30.0, "2026-01-06", json!({"accountType": "Margin", "accountId": "acct-m", "currency": "USD", "securityId": "sec-asts", "name": "AST SpaceMobile"})),
        act(json!({"id": "o1", "category": "trade", "activityType": "BUY", "rawType": "OPTIONS_BUY", "quantity": 1, "unitPrice": 2.0, "netCashAmount": -200, "transactionDate": "2026-02-05", "symbol": "ASTS 15JAN27 40.00 CALL", "currency": "USD", "accountType": "Margin", "accountId": "acct-m", "securityId": "sec-o-asts"})),
        act(json!({"id": "c1", "category": "trade", "activityType": "BUY", "rawType": "CRYPTO_BUY", "quantity": 0.1, "unitPrice": 100000, "netCashAmount": -10000, "transactionDate": "2026-01-05", "symbol": "BTC", "currency": "CAD", "accountType": "Crypto", "accountId": "acct-c"})),
        act(json!({"id": "x1", "category": "trade", "activityType": "SELL", "rawType": "DIY_SELL", "quantity": -25, "unitPrice": 4.0, "netCashAmount": 100, "transactionDate": "2026-02-10", "symbol": "GONE", "currency": "CAD", "accountType": "Margin", "accountId": "acct-m", "description": "Sold before the history starts"})),
    ], json!([]));
    let o = s.as_object_mut().unwrap();
    o.insert("accounts".into(), json!([
        {"id": "acct-m", "nickname": "Margin", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "currency": "CAD", "status": "open", "type": "margin", "netLiquidationValue": 9000.5},
        {"id": "acct-m2", "nickname": "Margin  US", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "currency": "USD", "status": "open", "type": "margin", "netLiquidationValue": null},
        {"id": "acct-c", "nickname": "Crypto", "unifiedAccountType": "SELF_DIRECTED_CRYPTO", "currency": "CAD", "status": "open", "type": "crypto", "netLiquidationValue": "10500"},
        {"id": "acct-x", "nickname": "Old", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "CLOSED", "type": "tfsa", "netLiquidationValue": 0},
    ]));
    o.insert("balances".into(), json!([
        {"accountId": "acct-m", "custodianAccountId": null, "securityId": "sec-enb", "quantity": 100},
        {"accountId": "acct-m", "custodianAccountId": null, "securityId": "sec-c-cad", "quantity": -2500.255},
        {"accountId": "acct-m", "custodianAccountId": null, "securityId": "sec-c-usd", "quantity": 120.5},
        {"accountId": "acct-c", "custodianAccountId": null, "securityId": "sec-c-cad", "quantity": 40},
    ]));
    o.insert("margin".into(), json!([
        {"accountId": "acct-m", "buyingPower": 4200.75, "currency": "CAD", "unavailable": "", "fetchedAt": ""},
        {"accountId": "acct-m2", "buyingPower": null, "currency": "USD", "unavailable": "not offered", "fetchedAt": ""},
        {"accountId": "acct-c", "buyingPower": 40, "currency": "CAD", "unavailable": "", "fetchedAt": ""},
    ]));
    o.insert("securities".into(), json!([
        {"id": "sec-enb", "symbol": "ENB", "name": "Enbridge Inc.", "primaryExchange": "TSX", "primaryMic": "XTSX", "currency": "CAD", "underlyingId": null},
        {"id": "sec-asts", "symbol": "ASTS", "name": "AST SpaceMobile, Inc.", "primaryExchange": "", "primaryMic": "XNAS", "currency": "USD", "underlyingId": null},
        {"id": "sec-o-asts", "symbol": "ASTS 15JAN27 40.00 CALL", "name": "", "primaryExchange": "", "primaryMic": "", "currency": "USD", "underlyingId": "sec-asts"},
        {"id": "sec-c-cad", "symbol": "CAD", "name": "Canadian dollar", "primaryExchange": "", "primaryMic": "", "currency": "CAD", "underlyingId": null},
        {"id": "sec-c-usd", "symbol": "USD", "name": "US dollar", "primaryExchange": "", "primaryMic": "", "currency": "", "underlyingId": null},
    ]));
    o.insert("exposures".into(), json!({
        "sec-enb": {"sectors": {"energy": 0.9, "Utilities": 0.1}, "countries": {"Canada": 1.0}},
        "share:ASTS::US": {"sectors": {"Communication Services": 1}, "countries": {"United States": "1.0"}},
        "share:QNC:": {"sectors": {"Technology": 0.6, "Industrials": 0.6}, "countries": {"Canada": 1}},
    }));
    o.insert("watchlist".into(), json!([
        {"symbol": "ENB", "exchange": "TSX", "name": "Enbridge Inc.", "currency": "CAD", "securityId": "sec-enb", "addedAt": "2026-01-01T00:00:00Z"},
        {"symbol": "SPX", "exchange": "INDEX", "name": "S&P 500", "currency": "USD", "securityId": "", "addedAt": ""},
        {"symbol": "ETH", "exchange": "Crypto", "name": "Ethereum", "currency": "CAD", "securityId": "", "addedAt": ""},
        {"symbol": "QNC", "exchange": "TSX-V", "name": "Quantum eMotion", "currency": "CAD", "securityId": "", "addedAt": ""},
    ]));
    o.insert("news".into(), json!([
        {"id": "n1", "symbol": "ENB", "exchange": "TSX", "source": "tmx", "headline": "Enbridge Announces Quarterly Results", "wire": "CNW", "url": "https://example.test/n1", "publishedAt": "2026-03-01T13:00:00Z", "fetchedAt": "", "kind": "story", "summary": ""},
        {"id": "n1", "symbol": "QNC", "exchange": "TSX-V", "source": "tmx", "headline": "Enbridge Announces Quarterly Results", "wire": "CNW", "url": "https://example.test/n1", "publishedAt": "2026-03-01T13:00:00Z", "fetchedAt": "", "kind": "story", "summary": ""},
        {"id": "n2", "symbol": "ENB", "exchange": "TSX", "source": "sa", "headline": "Enbridge announces quarterly results!", "wire": "Newswire", "url": "https://example.test/n2", "publishedAt": "2026-03-01T13:05:00Z", "fetchedAt": "", "kind": "release", "summary": ""},
        {"id": "n3", "symbol": "ENB", "exchange": "TSX", "source": "tmx", "headline": "Enbridge annonce ses résultats du trimestre", "wire": "CNW", "url": "https://example.test/n3", "publishedAt": "2026-03-01T14:00:00Z", "fetchedAt": "", "kind": "", "summary": ""},
        {"id": "m1", "symbol": "*", "exchange": "market", "source": "nasdaq", "headline": "Stocks close higher", "wire": "Nasdaq", "url": "https://example.test/m1", "publishedAt": "2026-03-01T21:00:00Z", "fetchedAt": "", "kind": "story", "summary": ""},
        {"id": "n4", "symbol": "ZZZ", "exchange": "NYSE", "source": "yahoo", "headline": "A listing nobody follows any more", "wire": "Yahoo", "url": "", "publishedAt": "2026-02-27T10:00:00Z", "fetchedAt": "", "kind": "story", "summary": ""},
    ]));
    o.insert("universes".into(), json!({
        "ca": [
            {"symbol": "RY", "name": "Royal Bank of Canada", "value": 250000000000.0, "percentChange": 0.42, "sector": "Financial Services", "country": "Canada", "fetchedAt": ""},
            {"symbol": "SHOP", "name": "Shopify", "value": null, "percentChange": null, "sector": "", "country": "Canada", "fetchedAt": ""},
        ],
        "us": [],
    }));
    o.insert("tiles".into(), json!([{"symbol": "ZQ", "exchange": "CBOT"}, {"symbol": "SR3", "exchange": "CME"}, {"symbol": "SPX", "exchange": "INDEX"}, {"symbol": "NOPE", "exchange": "X"}]));
    let quotes = json!({
        "ENB": {"price": 55.5, "priceChange": -0.25, "percentChange": -0.448, "source": "tmx"},
        "ENB@TSX": {"price": 55.5, "priceChange": -0.25, "percentChange": -0.448},
        "SPX@INDEX": {"price": 7712.5, "priceChange": 5.25, "percentChange": 0.068},
        "QNC@TSX-V": {"price": "1.75", "priceChange": null, "percentChange": null},
        "ZQ@CBOT": {"price": 96.3725, "priceChange": -0.0125, "percentChange": -0.013},
        "SR3@CME": {"price": 96.5},
        "BTC": {"price": 120000.0, "source": "coinbase"},
    });
    let journal = json!({"rt:b1": {"thesis": "pipes", "tags": ["income", "core"], "grade": "A"}});
    check("scenario_markets", &payload(&s, &market(quotes), &journal, "2026-03-02", &json!({})));
    check("scenario_markets_one_account", &payload(&s, &market(json!({})), &journal, "2026-03-02", &json!({"lists": {"account": ["Margin"]}, "ranges": {"pnl": {"op": "<", "v": "5"}}, "years": [2026, "2025x"], "search": " en "})));
}
