//! Daily bars for the chart.
//!
//! An instrument's bars can come from more than one place, and which place has
//! them depends on the instrument. The candidates are asked in order and the
//! first whose bars reach back to the start of the span wins; a source with a
//! late start never beats one that has the earlier days. The winner is
//! remembered for the symbol, so the next span asks it first.

use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::http::{post_json, TMX_HEADERS};
use crate::parse::{parse_coinbase_candles, parse_tmx_history};
use crate::quotes::{yahoo_forms, tmx_quote_symbol};
use crate::tmx::{tmx_lookup, TMX_URL};
use bagholder_model::value::{field_s, get, num};

/// `market.COVERAGE_SLACK_DAYS`: a source covers a span when its first bar is
/// within this of the span's start.
pub const COVERAGE_SLACK_DAYS: i64 = 7;

/// `market.HISTORY_STALE_HOURS`.
pub const HISTORY_STALE_HOURS: f64 = 20.0;

const TMX_HISTORY_QUERY: &str = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }";

pub const COINBASE_CANDLES_URL: &str = "https://api.exchange.coinbase.com/products/{}/candles?granularity={}&start={}&end={}";
pub const COINBASE_PRODUCT_URL: &str = "https://api.exchange.coinbase.com/products/{}";
pub const YAHOO_CHART_RANGE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval={}";

/// `market.history_candidates`: where an instrument's bars can come from, in
/// order of preference.
///
/// Shares and ETFs: TMX Money under the venue's form, then Yahoo under each
/// venue suffix of the currency. Crypto: the Coinbase market and the Yahoo
/// pair in the position's own currency, then the USD market and pair, which
/// are converted at the Bank of Canada rate. Nothing for an option contract
/// itself -- the chart maps it to its underlying first.
pub fn history_candidates(rec: &Value) -> Vec<(String, String)> {
    let kind = { let k = field_s(rec, "kind"); if k.is_empty() { "Shares".to_string() } else { k } };
    let sym = bagholder_model::venues::tmx_symbol(&field_s(rec, "symbol"));
    let ccy = { let c = field_s(rec, "currency"); if c.is_empty() { "CAD".to_string() } else { c.trim().to_uppercase() } };
    if sym.is_empty() {
        return vec![];
    }
    if kind == "Crypto" {
        let mut out = vec![
            ("coinbase".to_string(), format!("{}-{}", sym, ccy)),
            ("yahoo".to_string(), format!("{}-{}", sym, ccy)),
        ];
        if ccy != "USD" {
            out.push(("coinbase".to_string(), format!("{}-USD", sym)));
            out.push(("yahoo".to_string(), format!("{}-USD", sym)));
        }
        return out;
    }
    if kind != "Shares" {
        return vec![];
    }
    let mut out = Vec::new();
    if let Some(k) = tmx_quote_symbol(&field_s(rec, "symbol"), &field_s(rec, "exchange"), &ccy) {
        out.push(("tmx".to_string(), k));
    }
    out.extend(yahoo_forms(rec).into_iter().map(|f| ("yahoo".to_string(), f)));
    out
}

fn bars_meta_key(rec: &Value) -> String {
    format!("bars_source:{}", bagholder_model::venues::tmx_symbol(&field_s(rec, "symbol")))
}

/// `market.ordered_candidates`: the candidates with the remembered winner
/// first.
pub fn ordered_candidates(conn: &rusqlite::Connection, rec: &Value) -> Vec<(String, String)> {
    let cands = history_candidates(rec);
    if cands.is_empty() {
        return cands;
    }
    let v = bagholder_store::tables::get_meta(conn, &bars_meta_key(rec), "").unwrap_or_default();
    if let Some((s, k)) = v.split_once('|') {
        let win = (s.to_string(), k.to_string());
        if cands.contains(&win) {
            let mut out = vec![win.clone()];
            out.extend(cands.into_iter().filter(|c| *c != win));
            return out;
        }
    }
    cands
}

fn remember_winner(conn: &rusqlite::Connection, rec: &Value, source: &str, key: &str) {
    let _ = bagholder_store::tables::set_meta(conn, &bars_meta_key(rec), &format!("{}|{}", source, key));
}

/// `market.bar_currency`: the currency a candidate's bars are quoted in -- a
/// crypto pair's quote currency, otherwise the listing's own.
pub fn bar_currency(source_kind: &str, key: &str, rec: &Value) -> String {
    let _ = source_kind;
    if field_s(rec, "kind") == "Crypto" {
        if let Some((_, q)) = key.split_once('-') {
            return q.to_string();
        }
    }
    let c = field_s(rec, "currency");
    if c.is_empty() { "CAD".into() } else { c.to_uppercase() }
}

/// `market._rate_on_or_before`: the published rate for a day, or the most
/// recent one within a week.
fn rate_on_or_before(fx: &BTreeMap<String, f64>, day: &str, days: i64) -> Option<f64> {
    for i in 0..days {
        let d = bagholder_model::dates::shift_date(day, -i);
        if let Some(r) = fx.get(&d) {
            if *r > 0.0 {
                return Some(*r);
            }
        }
    }
    None
}

/// `market.in_position_currency`: bars in the position's currency.
///
/// USD bars become CAD at the Bank of Canada rate of the bar's own day. A bar
/// whose day has no published rate within a week is dropped, never guessed;
/// anything else cannot be converted and yields nothing at all.
pub fn in_position_currency(bars: &[Value], quoted_in: &str, currency: &str) -> Vec<Value> {
    let quote = quoted_in.to_uppercase();
    let ccy = { let c = currency.trim(); if c.is_empty() { "CAD".to_string() } else { c.to_uppercase() } };
    if quote == ccy {
        return bars.to_vec();
    }
    if !(quote == "USD" && ccy == "CAD") {
        return vec![];
    }
    vec![]
}

/// The same, with the rates the caller has already read.
pub fn in_position_currency_with(bars: &[Value], quoted_in: &str, currency: &str, fx: &BTreeMap<String, f64>) -> Vec<Value> {
    let quote = quoted_in.to_uppercase();
    let ccy = { let c = currency.trim(); if c.is_empty() { "CAD".to_string() } else { c.to_uppercase() } };
    if quote == ccy {
        return bars.to_vec();
    }
    if !(quote == "USD" && ccy == "CAD") {
        return vec![];
    }
    let mut out = Vec::new();
    for b in bars {
        let day = {
            let d = field_s(b, "date");
            if !d.is_empty() { d } else { day_of_epoch(num(get(b, "time"), 0.0) as i64) }
        };
        let rate = match rate_on_or_before(fx, &day, 7) { Some(r) => r, None => continue };
        let mut m = b.as_object().cloned().unwrap_or_default();
        for k in ["open", "high", "low", "close"] {
            if let Some(v) = m.get(k).and_then(|x| x.as_f64()) {
                m.insert(k.into(), json!(v * rate));
            }
        }
        out.push(Value::Object(m));
    }
    out
}

fn day_of_epoch(ts: i64) -> String {
    let (y, m, d) = bagholder_model::dates::from_days(ts.div_euclid(86400));
    bagholder_model::dates::fmt(y, m, d)
}

fn epoch_of_day(day: &str) -> i64 {
    match bagholder_model::dates::parse_iso(day) {
        Some((y, m, d)) => bagholder_model::dates::to_days(y, m, d) * 86400,
        None => 0,
    }
}

/// `market._whole_bars`: only bars with an open, a high and a low -- the chart
/// draws candlesticks or nothing.
fn whole_bars(bars: Vec<Value>) -> Vec<Value> {
    bars.into_iter()
        .filter(|b| {
            ["open", "high", "low"].iter().all(|k| b.get(*k).map(|v| !v.is_null()).unwrap_or(false))
        })
        .collect()
}

/// `market.coinbase_market`: the Coinbase Exchange market for a pair, or
/// nothing when it does not trade there. Remembered; a miss for a day.
pub fn coinbase_market(conn: &rusqlite::Connection, pair: &str, today: &str) -> String {
    let pair = pair.trim().to_uppercase();
    if !pair.contains('-') {
        return String::new();
    }
    let meta_key = format!("coinbase_product:{}", pair);
    let v = bagholder_store::tables::get_meta(conn, &meta_key, "").unwrap_or_default();
    if let Some(rest) = v.strip_prefix('@') {
        return rest.to_string();
    }
    if let Some(when) = v.strip_prefix("none@") {
        if when > bagholder_model::dates::shift_date(today, -1).as_str() {
            return String::new();
        }
    }
    let url = COINBASE_PRODUCT_URL.replace("{}", &pair);
    let ok = match crate::http::get_text(&url, &[("User-Agent", crate::http::UA), ("Accept", "application/json")]) {
        Ok(t) => serde_json::from_str::<Value>(&t).ok().map(|d| !field_s(&d, "id").is_empty()).unwrap_or(false),
        Err(_) => false,
    };
    if ok {
        let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("@{}", pair));
        return pair;
    }
    let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("none@{}", today));
    String::new()
}

/// `market.fetch_coinbase_candles`.
pub fn fetch_coinbase_candles(product: &str, granularity: i64, start_ts: i64, end_ts: i64) -> Vec<Value> {
    let url = COINBASE_CANDLES_URL
        .replacen("{}", product, 1)
        .replacen("{}", &granularity.to_string(), 1)
        .replacen("{}", &iso_instant(start_ts), 1)
        .replacen("{}", &iso_instant(end_ts), 1);
    match crate::http::get_text(&url, &[("User-Agent", crate::http::UA), ("Accept", "application/json")]) {
        Ok(t) => parse_coinbase_candles(&t),
        Err(_) => vec![],
    }
}

fn iso_instant(ts: i64) -> String {
    let days = ts.div_euclid(86400);
    let rem = ts.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// `market.fetch_yahoo`: a symbol Yahoo says it does not carry is remembered
/// for the day and not asked again.
pub fn fetch_yahoo(conn: &rusqlite::Connection, symbol: &str, start_ts: i64, end_ts: i64, interval: &str, today: &str) -> Vec<Value> {
    let miss_key = format!("yahoo_miss:{}", symbol);
    if bagholder_store::tables::get_meta(conn, &miss_key, "").unwrap_or_default() == today {
        return vec![];
    }
    let url = YAHOO_CHART_RANGE_URL
        .replacen("{}", symbol, 1)
        .replacen("{}", &start_ts.to_string(), 1)
        .replacen("{}", &end_ts.to_string(), 1)
        .replacen("{}", interval, 1);
    match crate::quotes::yahoo_get_public(&url) {
        Ok(text) => crate::quotes::parse_yahoo_chart(&text),
        Err(code) => {
            if code == Some(404) {
                let _ = bagholder_store::tables::set_meta(conn, &miss_key, today);
            }
            vec![]
        }
    }
}

/// `market.fetch_daily_from`: one candidate's daily bars over a span, oldest
/// first.
pub fn fetch_daily_from(
    conn: &rusqlite::Connection,
    source: &str,
    key: &str,
    rec: &Value,
    start: &str,
    end: &str,
    today: &str,
    fx: &BTreeMap<String, f64>,
) -> Vec<Value> {
    let start_ts = epoch_of_day(start);
    let end_ts = epoch_of_day(end) + 86400;
    match source {
        "tmx" => {
            let daily = |form: &str| -> Option<Value> {
                let payload = json!({
                    "operationName": "getTimeSeriesData",
                    "variables": {"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end},
                    "query": TMX_HISTORY_QUERY,
                });
                let data = post_json(TMX_URL, &payload, &TMX_HEADERS).ok()?;
                let bars = parse_tmx_history(&data);
                if bars.is_empty() { None } else { Some(Value::Array(bars)) }
            };
            let got = tmx_lookup(conn, key, today, daily).0;
            whole_bars(got.and_then(|v| v.as_array().cloned()).unwrap_or_default())
        }
        "coinbase" => {
            if coinbase_market(conn, key, today).is_empty() {
                return vec![];
            }
            let days: Vec<Value> = fetch_coinbase_candles(key, 86400, start_ts, end_ts)
                .into_iter()
                .map(|b| {
                    let mut m = b.as_object().cloned().unwrap_or_default();
                    m.insert("date".into(), json!(day_of_epoch(num(m.get("time"), 0.0) as i64)));
                    Value::Object(m)
                })
                .collect();
            in_position_currency_with(&days, &bar_currency(source, key, rec), &field_s(rec, "currency"), fx)
        }
        "yahoo" => {
            let days: Vec<Value> = fetch_yahoo(conn, key, start_ts, end_ts, "1d", today)
                .into_iter()
                .map(|b| {
                    json!({
                        "date": field_s(&b, "day"),
                        "open": b.get("open").cloned().unwrap_or(Value::Null),
                        "high": b.get("high").cloned().unwrap_or(Value::Null),
                        "low": b.get("low").cloned().unwrap_or(Value::Null),
                        "close": b.get("close").cloned().unwrap_or(Value::Null),
                        "volume": b.get("volume").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect();
            in_position_currency_with(&whole_bars(days), &bar_currency(source, key, rec), &field_s(rec, "currency"), fx)
        }
        _ => vec![],
    }
}

/// `market._pick_covering`: the first candidate whose bars reach back to the
/// span's start, else the one reaching furthest back.
fn pick_covering(
    conn: &rusqlite::Connection,
    rec: &Value,
    answers: &[(String, String, Vec<Value>)],
    span_start: &str,
) -> (Vec<Value>, String) {
    let limit = bagholder_model::dates::shift_date(span_start, COVERAGE_SLACK_DAYS);
    let mut best: Option<(String, String, String, Vec<Value>)> = None;
    for (source, key, bars) in answers {
        if bars.is_empty() {
            continue;
        }
        let first = field_s(&bars[0], "date");
        if first <= limit {
            remember_winner(conn, rec, source, key);
            return (bars.clone(), source.clone());
        }
        if best.as_ref().map(|(f, _, _, _)| first < *f).unwrap_or(true) {
            best = Some((first, source.clone(), key.clone(), bars.clone()));
        }
    }
    match best {
        Some((_, source, key, bars)) => {
            remember_winner(conn, rec, &source, &key);
            (bars, source)
        }
        None => (vec![], String::new()),
    }
}

/// `market.fetch_history`: the chain, stopping as soon as one candidate covers
/// the span.
pub fn fetch_history(
    conn: &rusqlite::Connection,
    rec: &Value,
    start: &str,
    end: &str,
    today: &str,
    fx: &BTreeMap<String, f64>,
) -> (Vec<Value>, String) {
    let limit = bagholder_model::dates::shift_date(start, COVERAGE_SLACK_DAYS);
    let mut answers: Vec<(String, String, Vec<Value>)> = Vec::new();
    for (source, key) in ordered_candidates(conn, rec) {
        let bars = fetch_daily_from(conn, &source, &key, rec, start, end, today, fx);
        let covered = bars.first().map(|b| field_s(b, "date") <= limit).unwrap_or(false);
        answers.push((source, key, bars));
        if covered {
            break;
        }
    }
    pick_covering(conn, rec, &answers, start)
}

/// `market.ensure_history`: the stored bars for a span, fetching when it was
/// never fetched or the copy is stale and the span reaches the present.
pub fn ensure_history(
    conn: &rusqlite::Connection,
    rec: &Value,
    start: &str,
    end: &str,
    today: &str,
    now_unix: f64,
    now_stamp: &str,
) -> rusqlite::Result<Vec<Value>> {
    let sym = bagholder_model::venues::tmx_symbol(&field_s(rec, "symbol"));
    let start: String = start.chars().take(10).collect();
    let end: String = end.chars().take(10).collect();
    if sym.is_empty() || start.len() != 10 || end.len() != 10 {
        return Ok(vec![]);
    }
    let last = bagholder_store::market::history_fetch(conn, &sym)?;
    let last_start = field_s(&last, "start");
    let covered = !last.is_null() && !last_start.is_empty() && last_start <= start;
    let fresh = match crate::quotes::instant_secs_public(&field_s(&last, "fetchedAt")) {
        Some(then) => (now_unix - then) < HISTORY_STALE_HOURS * 3600.0,
        None => false,
    };
    let needs_recent = end >= bagholder_model::dates::shift_date(today, -3);

    if !covered || (needs_recent && !fresh) {
        let fetch_from = if !covered {
            start.clone()
        } else if last_start < start {
            last_start.clone()
        } else {
            start.clone()
        };
        let fx = read_fx(conn)?;
        let (bars, source) = fetch_history(conn, rec, &fetch_from, today, today, &fx);
        if !bars.is_empty() {
            bagholder_store::market::upsert_price_history(conn, &sym, &bars, &source)?;
            // the stamp says what is covered: when the bars begin well after
            // the day asked for, only from their first day, so an earlier span
            // asks again
            let got_from = field_s(&bars[0], "date");
            let slack = bagholder_model::dates::shift_date(&fetch_from, COVERAGE_SLACK_DAYS);
            let covered_from = if got_from <= slack { fetch_from } else { got_from };
            bagholder_store::market::mark_history_fetched(conn, &sym, &covered_from, now_stamp)?;
        }
    }
    bagholder_store::market::price_history(conn, &sym, &start, &end)
}

fn read_fx(conn: &rusqlite::Connection) -> rusqlite::Result<BTreeMap<String, f64>> {
    let m = bagholder_store::tables::fx_rates(conn, bagholder_store::tables::FX_PAIR)?;
    Ok(m.into_iter().filter_map(|(k, v)| v.as_f64().map(|f| (k, f))).collect())
}

/// `market.aggregate_daily`: weekly (Monday start) or monthly bars from daily
/// ones -- the period's first open, highest high, lowest low, last close and
/// summed volume.
pub fn aggregate_daily(bars: &[Value], tf: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut cur_key = String::new();
    for b in bars {
        let date = field_s(b, "date");
        let (y, m, d) = match bagholder_model::dates::parse_iso(&date) { Some(p) => p, None => continue };
        let key = if tf == "1w" {
            let days = bagholder_model::dates::to_days(y, m, d);
            // Monday is 0: 1970-01-01 was a Thursday
            let weekday = ((days + 3) % 7 + 7) % 7;
            let (wy, wm, wd) = bagholder_model::dates::from_days(days - weekday);
            bagholder_model::dates::fmt(wy, wm, wd)
        } else {
            bagholder_model::dates::fmt(y, m, 1)
        };
        if out.is_empty() || cur_key != key {
            cur_key = key.clone();
            out.push(json!({
                "date": key,
                "open": b.get("open").cloned().unwrap_or(Value::Null),
                "high": b.get("high").cloned().unwrap_or(Value::Null),
                "low": b.get("low").cloned().unwrap_or(Value::Null),
                "close": b.get("close").cloned().unwrap_or(Value::Null),
                "volume": b.get("volume").cloned().unwrap_or(Value::Null),
            }));
            continue;
        }
        let cur = out.last_mut().unwrap();
        cur["close"] = b.get("close").cloned().unwrap_or(Value::Null);
        if let Some(h) = b.get("high").and_then(|v| v.as_f64()) {
            let now = cur.get("high").and_then(|v| v.as_f64());
            cur["high"] = json!(match now { Some(x) => x.max(h), None => h });
        }
        if let Some(l) = b.get("low").and_then(|v| v.as_f64()) {
            let now = cur.get("low").and_then(|v| v.as_f64());
            cur["low"] = json!(match now { Some(x) => x.min(l), None => l });
        }
        if let Some(v) = b.get("volume").and_then(|x| x.as_f64()) {
            let now = cur.get("volume").and_then(|x| x.as_f64()).unwrap_or(0.0);
            cur["volume"] = json!(now + v);
        }
    }
    out
}
