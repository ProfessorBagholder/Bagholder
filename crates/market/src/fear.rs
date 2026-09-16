//! The fear and greed indexes as their publishers give them: CNN's for the US
//! stock market, and alternative.me's for crypto.
//!
//! Nothing here is computed from the market. A reading is the publisher's own
//! score, its own rating and, where it publishes them, its own indicators. A
//! score a publisher gives without a rating is named on that same publisher's
//! scale, which is the scale the score was made on.

use serde_json::{json, Value};

use crate::http::{get_text, UA};
use bagholder_model::value::{field_s, get, num};

pub const STOCK_URL: &str = "https://production.dataviz.cnn.io/index/fearandgreed/graphdata";
pub const CRYPTO_URL: &str = "https://api.alternative.me/fng/?limit={}";

/// A year of daily readings, which is what both publish.
pub const DAYS: i64 = 366;

pub const INDEXES: [&str; 2] = ["stocks", "crypto"];
pub const SOURCES: [(&str, &str); 2] = [("stocks", "CNN"), ("crypto", "Alternative.me")];

/// CNN's seven indicators under CNN's own names.
///
/// Its answer carries two forms of the momentum and volatility ones; these are
/// the averages its own page names -- the S&P 500's 125-day and the VIX's
/// 50-day.
const PARTS: [(&str, &str); 7] = [
    ("market_momentum_sp125", "Market momentum"),
    ("stock_price_strength", "Stock price strength"),
    ("stock_price_breadth", "Stock price breadth"),
    ("put_call_options", "Put and call options"),
    ("market_volatility_vix_50", "Market volatility"),
    ("junk_bond_demand", "Junk bond demand"),
    ("safe_haven_demand", "Safe haven demand"),
];

/// The scale both publishers name their own scores on.
const BANDS: [(f64, &str); 4] = [(25.0, "Extreme fear"), (45.0, "Fear"), (56.0, "Neutral"), (76.0, "Greed")];

fn opt(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// `fear.band`: what a score is called on the publishers' own scale.
pub fn band(score: Option<f64>) -> String {
    let n = match score { Some(n) => n, None => return String::new() };
    for (edge, name) in BANDS {
        if n < edge {
            return name.to_string();
        }
    }
    "Extreme greed".into()
}

/// `fear.rating`: the publisher's own word where it gives one, its own scale's
/// word where it gives only a number.
pub fn rating(given: &str, score: Option<f64>) -> String {
    let text = given.trim();
    if text.is_empty() {
        return band(score);
    }
    let mut chars = text.chars();
    let first: String = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    format!("{}{}", first, chars.as_str().to_lowercase())
}

/// `fear._day`: a point's day, from the milliseconds both publishers stamp
/// their history with.
fn day(ms: Option<f64>) -> String {
    let n = match ms { Some(n) => n, None => return String::new() };
    let secs = (n / 1000.0).floor() as i64;
    let (y, m, d) = bagholder_model::dates::from_days(secs.div_euclid(86400));
    bagholder_model::dates::fmt(y, m, d)
}

/// `fear._moment`: CNN stamps the live reading with an offset time; it is kept
/// the way the app writes times.
fn moment(text: &str) -> String {
    let s = text.trim();
    if s.is_empty() {
        return String::new();
    }
    let norm = if s.ends_with('Z') { format!("{}+00:00", &s[..s.len() - 1]) } else { s.to_string() };
    let (d, t) = match norm.split_once('T') { Some(p) => p, None => return String::new() };
    let (y, m, dd) = match bagholder_model::dates::parse_iso(d) { Some(p) => p, None => return String::new() };
    let mut offset = 0i64;
    let mut clock = t;
    if let Some(pos) = t.rfind(['+', '-']) {
        if pos > 0 {
            let sign = if t.as_bytes()[pos] == b'-' { -1 } else { 1 };
            let off = &t[pos + 1..];
            let (oh, om) = off.split_once(':').unwrap_or((off, "0"));
            offset = sign * (oh.parse::<i64>().unwrap_or(0) * 3600 + om.parse::<i64>().unwrap_or(0) * 60);
            clock = &t[..pos];
        }
    }
    let parts: Vec<&str> = clock.split(':').collect();
    let hh: i64 = parts.first().and_then(|x| x.parse().ok()).unwrap_or(0);
    let mm: i64 = parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    let ss: i64 = parts.get(2).and_then(|x| x.split('.').next()?.parse().ok()).unwrap_or(0);
    let unix = bagholder_model::dates::to_days(y, m, dd) * 86400 + hh * 3600 + mm * 60 + ss - offset;
    let days = unix.div_euclid(86400);
    let rem = unix.rem_euclid(86400);
    let (uy, um, ud) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", uy, um, ud, rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn reading(label: &str, score: Option<&Value>) -> Option<Value> {
    let n = opt(score)?;
    Some(json!({"label": label, "score": round1(n), "rating": band(Some(n))}))
}

/// `fear.parse_stocks`: the reading now, the readings it compares itself
/// against, its seven indicators, and a year of daily readings.
pub fn parse_stocks(data: &Value) -> Value {
    let fg = data.get("fear_and_greed").cloned().unwrap_or_else(|| json!({}));
    let score = match opt(get(&fg, "score")) { Some(s) => s, None => return json!({}) };

    let earlier: Vec<Value> = [
        ("Previous close", "previous_close"),
        ("A week ago", "previous_1_week"),
        ("A month ago", "previous_1_month"),
        ("A year ago", "previous_1_year"),
    ]
    .iter()
    .filter_map(|(label, key)| reading(label, get(&fg, key)))
    .collect();

    let mut parts = Vec::new();
    for (key, name) in PARTS {
        let part = data.get(key).cloned().unwrap_or_else(|| json!({}));
        if let Some(value) = opt(get(&part, "score")) {
            parts.push(json!({
                "name": name,
                "score": round1(value),
                "rating": rating(&field_s(&part, "rating"), Some(value)),
            }));
        }
    }

    let mut series: Vec<Value> = Vec::new();
    let points = data
        .get("fear_and_greed_historical")
        .and_then(|h| h.get("data"))
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    for point in points {
        let d = day(opt(get(&point, "x")));
        if let (false, Some(v)) = (d.is_empty(), opt(get(&point, "y"))) {
            series.push(json!({"date": d, "score": round1(v)}));
        }
    }
    series.sort_by_key(|r| field_s(r, "date"));

    json!({
        "index": "stocks",
        "source": "CNN",
        "score": round1(score),
        "rating": rating(&field_s(&fg, "rating"), Some(score)),
        "asOf": moment(&field_s(&fg, "timestamp")),
        "previous": earlier,
        "parts": parts,
        "series": series,
    })
}

/// `fear.parse_crypto`: one reading a day, newest first. What it is compared
/// against is its own earlier days; it publishes no indicators.
pub fn parse_crypto(data: &Value) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    for row in data.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default() {
        let ts = opt(get(&row, "timestamp")).map(|t| t * 1000.0);
        let d = day(ts);
        if let (false, Some(v)) = (d.is_empty(), opt(get(&row, "value"))) {
            rows.push(json!({
                "date": d,
                "score": round1(v),
                "rating": rating(&field_s(&row, "value_classification"), Some(v)),
            }));
        }
    }
    if rows.is_empty() {
        return json!({});
    }
    let now = rows[0].clone();
    let at = |i: usize, label: &str| -> Option<Value> {
        rows.get(i).map(|r| json!({"label": label, "score": r.get("score"), "rating": field_s(r, "rating")}))
    };
    let earlier: Vec<Value> = [(1usize, "Yesterday"), (7, "A week ago"), (30, "A month ago"), (365, "A year ago")]
        .iter()
        .filter_map(|(i, l)| at(*i, l))
        .collect();
    let mut series: Vec<Value> = rows
        .iter()
        .map(|r| json!({"date": field_s(r, "date"), "score": r.get("score")}))
        .collect();
    series.sort_by_key(|r| field_s(r, "date"));

    json!({
        "index": "crypto",
        "source": "Alternative.me",
        "score": now.get("score"),
        "rating": field_s(&now, "rating"),
        "asOf": format!("{}T00:00:00Z", field_s(&now, "date")),
        "previous": earlier,
        "parts": [],
        "series": series,
    })
}

/// `fear.read`: one index as its publisher gives it now, or nothing where it
/// did not answer.
pub fn read(index: &str) -> Value {
    let which = index.trim().to_lowercase();
    let stock_headers = [
        ("User-Agent", UA),
        ("Accept", "application/json"),
        ("Origin", "https://www.cnn.com"),
        ("Referer", "https://www.cnn.com/"),
    ];
    let crypto_headers = [("User-Agent", UA), ("Accept", "application/json")];
    let text = match which.as_str() {
        "stocks" => get_text(STOCK_URL, &stock_headers),
        "crypto" => get_text(&CRYPTO_URL.replace("{}", &DAYS.to_string()), &crypto_headers),
        _ => return json!({}),
    };
    let data: Value = match text.ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(d) => d,
        None => return json!({}),
    };
    match which.as_str() {
        "stocks" => parse_stocks(&data),
        "crypto" => parse_crypto(&data),
        _ => json!({}),
    }
}
