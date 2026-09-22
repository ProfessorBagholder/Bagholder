//! Reading what each public source sends back.
//!
//! Every one of these is pure: text or JSON in, rows out. A feed that answers
//! with something unreadable yields nothing rather than raising, because one
//! bad response must not empty a stored series.

use serde_json::{json, Value};
use std::collections::BTreeMap;

use bagholder_model::value::{get, num};
use bagholder_store::bars::{DayBar, Ohlcv, TimeBar};

/// A number that is absent rather than zero, a numeric string read as
/// leniently as a number.
fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => bagholder_model::textrules::parse_float(s),
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub type Series = BTreeMap<String, f64>;

pub fn n(v: Option<&Value>, default: f64) -> f64 {
    num(v, default)
}

fn is_iso(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-'
}

/// `{"observations":[{"d":"2024-01-02","FXUSDCAD":{"v":"1.3316"}}]}`.
pub fn parse_boc_json(text: &str) -> Series {
    let mut out = Series::new();
    let data: Value = match serde_json::from_str(text) { Ok(v) => v, Err(_) => return out };
    let obs = match data.get("observations").and_then(|v| v.as_array()) { Some(a) => a, None => return out };
    for ob in obs {
        if !ob.is_object() {
            continue;
        }
        let d: String = bagholder_model::value::field_s(ob, "d").chars().take(10).collect();
        let cell = ob.get("FXUSDCAD").cloned().unwrap_or(Value::Null);
        let v = match get(&cell, "v") {
            Some(Value::String(s)) => match s.parse::<f64>() { Ok(f) => f, Err(_) => continue },
            Some(x) => {
                let f = num(Some(x), f64::NAN);
                if f.is_nan() { continue } else { f }
            }
            None => continue,
        };
        if is_iso(&d) && v > 0.0 {
            out.insert(d, v);
        }
    }
    out
}

/// `observation_date,SP500` rows. A `.` marks a
/// holiday and is skipped.
pub fn parse_fred_csv(text: &str) -> Series {
    let mut out = Series::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 2 {
            continue;
        }
        let d = parts[0].trim();
        let raw = parts[1].trim();
        if !is_iso(d) || raw.is_empty() || raw == "." {
            continue;
        }
        if let Ok(px) = raw.parse::<f64>() {
            if px > 0.0 {
                out.insert(d.to_string(), px);
            }
        }
    }
    out
}

/// `Date,Open,High,Low,Close,Volume`.
pub fn parse_stooq_csv(text: &str) -> Series {
    let mut out = Series::new();
    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 5 {
            continue;
        }
        let d = parts[0].trim();
        let px = match parts[4].trim().parse::<f64>() { Ok(p) => p, Err(_) => continue };
        if is_iso(d) && px > 0.0 {
            out.insert(d.to_string(), px);
        }
    }
    out
}

/// Rows into daily bars, oldest first. A row without a positive close was
/// never a bar -- it could never be stored -- and is dropped here rather
/// than carried further.
fn bars_from(rows: &[Value], date_key: &str) -> Vec<DayBar> {
    let mut out: Vec<DayBar> = Vec::new();
    for r in rows {
        if !r.is_object() {
            continue;
        }
        let d: String = bagholder_model::value::field_s(r, date_key).chars().take(10).collect();
        let close = match opt_num(r.get("close")) { Some(c) if c > 0.0 => c, _ => continue };
        if d.chars().count() == 10 {
            out.push(DayBar {
                date: d,
                px: Ohlcv { open: opt_num(r.get("open")), high: opt_num(r.get("high")), low: opt_num(r.get("low")), close, volume: opt_num(r.get("volume")) },
            });
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date));
    out
}

pub fn parse_tmx_history(data: &Value) -> Vec<DayBar> {
    let rows = data
        .get("data")
        .and_then(|d| d.get("getTimeSeriesData"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    bars_from(&rows, "dateTime")
}

pub fn parse_cboe_ca_history(text: &str) -> Vec<DayBar> {
    let data: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    let rows = data.get("data").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    bars_from(&rows, "date")
}

const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

fn valid_root(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 10
        && b[0].is_ascii_uppercase()
        && b[1..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'.')
}

/// `QNC 20NOV26 3.00 CALL` -> `QNC261120C00003000`, the
/// code Cboe keys its chains by.
pub fn occ_code(symbol: &str) -> String {
    let u = symbol.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase();
    let parts: Vec<&str> = u.split(' ').collect();

    // the compact form: `ROOT 261120C00003000`
    if parts.len() == 2 && valid_root(parts[0]) {
        let t = parts[1].as_bytes();
        if t.len() == 15
            && t[..6].iter().all(|c| c.is_ascii_digit())
            && (t[6] == b'C' || t[6] == b'P')
            && t[7..].iter().all(|c| c.is_ascii_digit())
        {
            return format!("{}{}", parts[0], parts[1]);
        }
    }
    // the wordy form: `ROOT 20NOV26 3.00 CALL`
    if parts.len() != 4 || !valid_root(parts[0]) {
        return String::new();
    }
    let date = parts[1].as_bytes();
    let digits = date.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 2 || date.len() != digits + 5 {
        return String::new();
    }
    let day: u32 = parts[1][..digits].parse().unwrap_or(0);
    let mon = &parts[1][digits..digits + 3];
    let yy = &parts[1][digits + 3..];
    if !yy.bytes().all(|c| c.is_ascii_digit()) {
        return String::new();
    }
    let month = match MONTHS.iter().position(|m| *m == mon) { Some(i) => i + 1, None => return String::new() };
    let strike: f64 = match parts[2].parse() { Ok(s) => s, Err(_) => return String::new() };
    let right = match parts[3] {
        "CALL" | "C" => 'C',
        "PUT" | "P" => 'P',
        _ => return String::new(),
    };
    format!("{}{}{:02}{:02}{}{:08}", parts[0], yy, month, day, right, (strike * 1000.0).round() as i64)
}

/// `YES.V` as Yahoo writes it -- the bare ticker and the
/// venues the suffix names.
pub fn yahoo_split(text: &str) -> (String, Option<&'static [&'static str]>) {
    const VENUES: [(&str, &[&str]); 4] = [
        (".TO", &["TSX"]),
        (".V", &["TSX-V", "TSXV"]),
        (".CN", &["CSE"]),
        (".NE", &["CBOE CANADA", "NEO"]),
    ];
    let s = text.trim().to_uppercase();
    for (suffix, venues) in VENUES {
        if s.ends_with(suffix) && s.len() > suffix.len() {
            return (s[..s.len() - suffix.len()].to_string(), Some(venues));
        }
    }
    (s, None)
}

/// `{"data":{"amount":"..."}}`.
pub fn parse_coinbase(text: &str) -> Option<f64> {
    let data: Value = serde_json::from_str(text).ok()?;
    let amount = data.get("data").and_then(|d| get(d, "amount"))?;
    let v = match amount {
        Value::String(s) => s.parse::<f64>().ok()?,
        other => {
            let f = num(Some(other), f64::NAN);
            if f.is_nan() { return None } else { f }
        }
    };
    if v > 0.0 { Some(v) } else { None }
}

/// A Coinbase spot price, with the currency the pair names when the feed
/// does not say.
pub fn parse_coinbase_rec(text: &str, pair: &str) -> Option<Value> {
    let data: Value = serde_json::from_str(if text.is_empty() { "{}" } else { text }).ok()?;
    let d = data.get("data").cloned().unwrap_or(json!({}));
    let px = opt(get(&d, "amount"))?;
    if px <= 0.0 {
        return None;
    }
    let ccy = {
        let c = bagholder_model::value::field_s(&d, "currency");
        if c.is_empty() { pair.rsplit('-').next().unwrap_or("").to_string() } else { c }
    };
    Some(json!({"price": px, "currency": ccy}))
}

/// A number that is absent rather than zero.
pub fn opt(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(x) => {
            let f = num(Some(x), f64::NAN);
            if f.is_nan() { None } else { Some(f) }
        }
    }
}

/// Outside a session `last` is 0, so the
/// previous close stands in.
pub fn parse_cboe_ca_quote(text: &str) -> Option<Value> {
    let data: Value = serde_json::from_str(if text.is_empty() { "{}" } else { text }).ok()?;
    let d = data.get("data").cloned().unwrap_or(json!({}));
    let last = opt(get(&d, "last"));
    let prev = opt(get(&d, "prev_close"));
    let px = match last { Some(l) if l > 0.0 => Some(l), _ => prev };
    let px = px?;
    if px <= 0.0 {
        return None;
    }
    Some(json!({
        "price": px,
        "priceChange": opt(get(&d, "change")),
        "percentChange": opt(get(&d, "change_pct")),
        "prevClose": prev,
        "currency": "CAD",
        "name": bagholder_model::value::field_s(&d, "company_name"),
    }))
}

/// OCC code -> the row for one underlying's
/// delayed chain.
pub fn parse_cboe_options(text: &str) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    let data: Value = match serde_json::from_str(if text.is_empty() { "{}" } else { text }) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let d = data.get("data").cloned().unwrap_or(json!({}));
    if let Some(options) = d.get("options").and_then(|v| v.as_array()) {
        for o in options {
            if o.is_object() {
                out.insert(bagholder_model::value::field_s(o, "option"), o.clone());
            }
        }
    }
    out
}

/// One contract's price per share -- the bid/ask
/// midpoint while both are quoted, else the last trade, else the previous
/// close.
pub fn option_mark(row: &Value) -> Option<Value> {
    if !row.is_object() {
        return None;
    }
    let bid = num(get(row, "bid"), 0.0);
    let ask = num(get(row, "ask"), 0.0);
    let prev = opt(get(row, "prev_day_close"));
    let px = if bid > 0.0 && ask > 0.0 {
        (bid + ask) / 2.0
    } else {
        // a zero last trade falls through as a missing one does
        match opt(get(row, "last_trade_price")) {
            Some(v) if v != 0.0 => v,
            _ => prev.unwrap_or(0.0),
        }
    };
    if px <= 0.0 {
        return None;
    }
    Some(json!({
        "price": px,
        "prevClose": prev,
        "priceChange": prev.map(|p| px - p),
        "percentChange": prev.filter(|p| *p != 0.0).map(|p| (px / p - 1.0) * 100.0),
        "currency": "USD",
    }))
}

/// `[time, low, high, open, close, volume]`
/// rows, oldest first.
pub fn parse_coinbase_candles(text: &str) -> Vec<TimeBar> {
    let rows: Vec<Value> = serde_json::from_str(if text.is_empty() { "[]" } else { text }).unwrap_or_default();
    let mut out: BTreeMap<i64, TimeBar> = BTreeMap::new();
    for r in rows {
        let a = match r.as_array() { Some(a) if a.len() >= 6 => a.clone(), _ => continue };
        let t = match a[0].as_f64() { Some(t) => t as i64, None => continue };
        let vals: Option<Vec<f64>> = a[1..6].iter().map(|x| x.as_f64()).collect();
        let v = match vals { Some(v) => v, None => continue };
        let (lo, hi, op, cl, vol) = (v[0], v[1], v[2], v[3], v[4]);
        if cl > 0.0 {
            out.insert(t, TimeBar { time: t, px: Ohlcv { open: Some(op), high: Some(hi), low: Some(lo), close: cl, volume: Some(vol) } });
        }
    }
    out.into_values().collect()
}
