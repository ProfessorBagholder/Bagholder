//! Reading what each public source sends back.
//!
//! Every one of these is pure: text or JSON in, rows out. A feed that answers
//! with something unreadable yields nothing rather than raising, because one
//! bad response must not empty a stored series.

use serde_json::{json, Value};
use std::collections::BTreeMap;

use bagholder_model::value::{get, num};
use crate::stored::bars::{DayBar, Ohlcv, TimeBar};

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

/// `[time, low, high, open, close, volume]`
/// rows, oldest first.
/// An answer that is not a list of rows does not read, and is the source's failure.
pub fn parse_coinbase_candles(text: &str) -> Result<Vec<TimeBar>, serde_json::Error> {
    let rows: Vec<Value> = serde_json::from_str(if text.is_empty() { "[]" } else { text })?;
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
    Ok(out.into_values().collect())
}
