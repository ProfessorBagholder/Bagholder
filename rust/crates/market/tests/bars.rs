//! The bar pipeline, pinned end to end: each source's parser, the daily, session
//! and clock aggregations, the conversion into the position's currency, and the
//! store's round trip. The answers are held in `golden/bars.json`, numbers
//! compared as numbers (an integer volume and the same volume as a float are one
//! answer), so a change of representation must leave every bar as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test bars`,
//! and read the diff.

use bagholder_market::{history, parse, quotes};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Every number as a float, so the comparison is of values, not representations.
fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

fn of<T: serde::Serialize>(t: T) -> Value {
    norm(serde_json::to_value(t).unwrap())
}

fn tmx_daily() -> Value {
    json!({"data": {"getTimeSeriesData": [
        {"dateTime": "2026-03-04 00:00:00", "open": 10.5, "high": 11, "low": 10.25, "close": 10.75, "volume": 1200},
        {"dateTime": "2026-03-02 00:00:00", "open": 10, "high": 10.5, "low": 9.75, "close": 10.25, "volume": 900},
        {"dateTime": "2026-03-03", "open": "10.25", "high": 10.8, "low": 10.1, "close": 10.5, "volume": null},
        {"dateTime": "2026-03-09", "open": 10.75, "high": 12, "low": 10.5, "close": 11.5, "volume": 3000},
        {"dateTime": "2026-03-10", "open": 11.5, "high": 11.6, "low": 11.1, "close": 11.2, "volume": 500},
        {"dateTime": "2026-04-01", "open": 11.2, "high": 11.4, "low": 10.9, "close": 11.0, "volume": 800},
        {"dateTime": "2026-04-02", "open": null, "high": null, "low": null, "close": 11.1, "volume": 10},
        {"dateTime": "bad", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
        "not a row"
    ]}})
}

fn cboe_daily() -> String {
    json!({"data": [
        {"date": "2026-05-06", "open": 4.1, "high": 4.3, "low": 4.0, "close": 4.2, "volume": 700},
        {"date": "2026-05-05", "open": 4.0, "high": 4.2, "low": 3.9, "close": 4.1, "volume": 500}
    ]}).to_string()
}

fn coinbase_hourly() -> String {
    // [time, low, high, open, close, volume], newest first as Coinbase sends them
    json!([
        [1772013600, 99.0, 104.0, 100.0, 103.0, 2.5],
        [1772010000, 98.0, 101.0, 99.0, 100.0, 1.5],
        [1772006400, 97.0, 100.0, 98.0, 99.0, 3.0],
        [1772002800, 96.0, 99.0, 97.0, 98.0, 0.0],
        [1772002800, 96.0, 99.0, 97.0, 98.0, 0.0],
        [1771999200, 95.0, 98.0, 96.0, 0.0, 1.0],
        [1771999200]
    ]).to_string()
}

fn yahoo_hourly() -> String {
    // New York across the March change to daylight time (2026-03-08), one bar
    // with no close and one with no high
    json!({"chart": {"result": [{
        "meta": {"exchangeTimezoneName": "America/New_York", "gmtoffset": -14400},
        "timestamp": [1772807400, 1772811000, 1772814600, 1773063000, 1773066600, 1773070200, 1773073800],
        "indicators": {"quote": [{
            "open":   [50.0, 50.5, 51.0, 52.0, 52.5, 53.0, 53.5],
            "high":   [50.8, 51.2, 51.5, null, 53.1, 53.6, 54.0],
            "low":    [49.9, 50.3, 50.8, 51.8, 52.2, 52.9, 53.2],
            "close":  [50.5, 51.0, 51.3, 52.4, 53.0, null, 53.8],
            "volume": [1000, 800, 600, 1500, 900, 700, 650]
        }]}
    }]}}).to_string()
}

fn yahoo_daily() -> String {
    json!({"chart": {"result": [{
        "meta": {"exchangeTimezoneName": "America/New_York", "gmtoffset": -14400},
        "timestamp": [1772721000, 1772807400, 1772893800],
        "indicators": {"quote": [{
            "open": [20.0, 20.5, 21.0], "high": [20.9, 21.2, 21.4], "low": [19.8, 20.1, 20.6],
            "close": [20.6, 21.0, 21.2], "volume": [10000, 12000, 9000]
        }]}
    }]}}).to_string()
}

fn tmx_minutes() -> Value {
    json!({"data": {"intraday": [
        {"dateTime": "2026-09-02T09:31:00-04:00", "open": 5.0, "high": 5.1, "low": 4.95, "close": 5.05, "volume": 100},
        {"dateTime": "2026-09-02T09:30:00-04:00", "open": 4.9, "high": 5.0, "low": 4.9, "close": 5.0, "volume": 300},
        {"dateTime": "2026-09-02T10:29:00-04:00", "open": 5.05, "high": 5.3, "low": 5.0, "close": 5.2, "volume": null},
        {"dateTime": "2026-09-02T10:30:00-04:00", "open": null, "high": null, "low": null, "close": 5.25, "volume": 50},
        {"dateTime": "2026-09-02T13:45:00-04:00", "open": 5.3, "high": 5.4, "low": 5.1, "close": 5.15, "volume": 0},
        {"dateTime": "2026-09-02T09:15:00-04:00", "open": 4.8, "high": 4.85, "low": 4.8, "close": 4.85, "volume": 20},
        {"dateTime": "2026-09-03T09:30:00-04:00", "open": 5.2, "high": 5.2, "low": 5.2, "close": 0, "volume": 5},
        {"dateTime": "2026-09-03T09:45:00-04:00", "open": 5.2, "high": 5.35, "low": 5.15, "close": 5.3, "volume": 75}
    ]}})
}

fn fx() -> BTreeMap<String, f64> {
    [("2026-02-24", 1.36), ("2026-02-25", 1.37), ("2026-03-02", 1.38), ("2026-03-05", 1.4), ("2026-03-06", 1.41), ("2026-03-09", 1.42), ("2026-09-01", 1.39)]
        .into_iter()
        .map(|(d, r)| (d.to_string(), r))
        .collect()
}

fn answers() -> Value {
    let daily = parse::parse_tmx_history(&tmx_daily());
    let candles = parse::parse_coinbase_candles(&coinbase_hourly());
    let y_hourly = quotes::parse_yahoo_chart(&yahoo_hourly());
    let y_daily = quotes::parse_yahoo_chart(&yahoo_daily());
    let minutes = history::parse_tmx_minutes(&tmx_minutes());
    let fx = fx();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    let stored_days = bagholder_store::market::upsert_price_history(&conn, "RDDY", &daily, "tmx").unwrap();
    let stored_hours = bagholder_store::market::upsert_price_bars(&conn, "RDDY", "1h", &history::aggregate_session(&minutes, 60), "tmx").unwrap();

    json!({
        "tmxDaily": of(&daily),
        "cboeDaily": of(parse::parse_cboe_ca_history(&cboe_daily())),
        "coinbaseCandles": of(&candles),
        "yahooHourly": of(&y_hourly),
        "yahooDaily": of(&y_daily),
        "tmxMinutes": of(&minutes),
        "weekly": of(history::aggregate_daily(&daily, "1w")),
        "monthly": of(history::aggregate_daily(&daily, "1M")),
        "session1h": of(history::aggregate_session(&minutes, 60)),
        "session4h": of(history::aggregate_session(&minutes, 240)),
        "yahooSession1h": of(history::aggregate_session(&y_hourly, 60)),
        "clock4h": of(history::aggregate_hourly(&candles, 14400)),
        "candlesInCad": of(history::in_position_currency_with(&candles, "USD", "CAD", &fx)),
        "daysInCad": of(history::in_position_currency_with(&daily, "usd", "", &fx)),
        "yahooInCad": of(history::in_position_currency_with(&y_hourly, "USD", "CAD", &fx)),
        "sameCurrency": of(history::in_position_currency_with(&daily, "CAD", "cad", &fx)),
        "noRate": of(history::in_position_currency_with(&daily, "EUR", "CAD", &fx)),
        "stored": {
            "days": stored_days,
            "hours": stored_hours,
            "history": of(bagholder_store::market::price_history(&conn, "RDDY", "2026-03-03", "2026-04-30").unwrap()),
            "bars": of(bagholder_store::market::price_bars(&conn, "RDDY", "1h", 0, 1 << 40).unwrap()),
        },
    })
}

#[test]
fn test_every_bar_is_what_it_was() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/bars.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/bars.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
