//! The fear and greed indexes as their publishers give them: CNN's for the US
//! stock market, and alternative.me's for crypto.
//!
//! Nothing here is computed from the market. A reading is the publisher's own
//! score, its own rating and, where it publishes them, its own indicators. A
//! score a publisher gives without a rating is named on that same publisher's
//! scale, which is the scale the score was made on.

use serde::Deserialize;
use serde_json::Value;

use crate::http::{get_text, UA};
use bagholder_model::lenient;
use bagholder_store::feeds::{Gauge, GaugePart, GaugePoint, GaugeReading};

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

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// What a score is called on the publishers' own scale.
pub fn band(score: Option<f64>) -> String {
    let n = match score { Some(n) => n, None => return String::new() };
    for (edge, name) in BANDS {
        if n < edge {
            return name.to_string();
        }
    }
    "Extreme greed".into()
}

/// The publisher's own word where it gives one, its own scale's
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

/// A point's day, from the milliseconds both publishers stamp
/// their history with.
fn day(ms: Option<f64>) -> String {
    let n = match ms { Some(n) => n, None => return String::new() };
    let secs = (n / 1000.0).floor() as i64;
    let (y, m, d) = bagholder_model::dates::from_days(secs.div_euclid(86400));
    bagholder_model::dates::fmt(y, m, d)
}

/// CNN stamps the live reading with an offset time; it is kept
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

/// CNN's answer: the reading now with the readings it compares itself against,
/// its history, and its indicators.
#[derive(Default, Deserialize)]
#[serde(default)]
struct CnnAnswer {
    fear_and_greed: CnnNow,
    fear_and_greed_historical: CnnHistory,
    market_momentum_sp125: CnnPart,
    stock_price_strength: CnnPart,
    stock_price_breadth: CnnPart,
    put_call_options: CnnPart,
    market_volatility_vix_50: CnnPart,
    junk_bond_demand: CnnPart,
    safe_haven_demand: CnnPart,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CnnNow {
    #[serde(deserialize_with = "lenient::maybe_number")]
    score: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    rating: String,
    #[serde(deserialize_with = "lenient::text")]
    timestamp: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    previous_close: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    previous_1_week: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    previous_1_month: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    previous_1_year: Option<f64>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CnnHistory {
    data: Value,
}

/// One point of a publisher's history: milliseconds and a score.
#[derive(Default, Deserialize)]
#[serde(default)]
struct CnnPoint {
    #[serde(deserialize_with = "lenient::maybe_number")]
    x: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    y: Option<f64>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CnnPart {
    #[serde(deserialize_with = "lenient::maybe_number")]
    score: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    rating: String,
}

impl CnnAnswer {
    /// The indicators in the order CNN's own page lists them, under its names.
    fn parts(&self) -> [(&CnnPart, &'static str); 7] {
        [
            (&self.market_momentum_sp125, PARTS[0].1),
            (&self.stock_price_strength, PARTS[1].1),
            (&self.stock_price_breadth, PARTS[2].1),
            (&self.put_call_options, PARTS[3].1),
            (&self.market_volatility_vix_50, PARTS[4].1),
            (&self.junk_bond_demand, PARTS[5].1),
            (&self.safe_haven_demand, PARTS[6].1),
        ]
    }
}

/// Alternative.me's answer: one reading a day, newest first.
#[derive(Default, Deserialize)]
#[serde(default)]
struct CryptoAnswer {
    data: Value,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CryptoDay {
    #[serde(deserialize_with = "lenient::maybe_number")]
    value: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    value_classification: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    timestamp: Option<f64>,
}

fn reading(label: &str, score: Option<f64>) -> Option<GaugeReading> {
    let n = score?;
    Some(GaugeReading { label: label.to_string(), score: round1(n), rating: band(Some(n)) })
}

/// The reading now, the readings it compares itself
/// against, its seven indicators, and a year of daily readings.
pub fn parse_stocks(data: &Value) -> Option<Gauge> {
    let answer: CnnAnswer = CnnAnswer::deserialize(data).unwrap_or_default();
    let fg = &answer.fear_and_greed;
    let score = fg.score?;
    let previous: Vec<GaugeReading> = [
        ("Previous close", fg.previous_close),
        ("A week ago", fg.previous_1_week),
        ("A month ago", fg.previous_1_month),
        ("A year ago", fg.previous_1_year),
    ]
    .into_iter()
    .filter_map(|(label, n)| reading(label, n))
    .collect();
    let parts: Vec<GaugePart> = answer
        .parts()
        .into_iter()
        .filter_map(|(part, name)| {
            let value = part.score?;
            Some(GaugePart { name: name.to_string(), score: round1(value), rating: rating(&part.rating, Some(value)) })
        })
        .collect();
    let mut series: Vec<GaugePoint> = lenient::rows::<CnnPoint>(&answer.fear_and_greed_historical.data)
        .into_iter()
        .filter_map(|p| {
            let date = day(p.x);
            match (date.is_empty(), p.y) {
                (false, Some(v)) => Some(GaugePoint { date, score: round1(v) }),
                _ => None,
            }
        })
        .collect();
    series.sort_by(|a, b| a.date.cmp(&b.date));
    Some(Gauge {
        index: "stocks".into(),
        source: "CNN".into(),
        score: round1(score),
        rating: rating(&fg.rating, Some(score)),
        as_of: moment(&fg.timestamp),
        previous,
        parts,
        series,
    })
}

/// One reading a day, newest first. What it is compared
/// against is its own earlier days; it publishes no indicators.
pub fn parse_crypto(data: &Value) -> Option<Gauge> {
    let answer: CryptoAnswer = CryptoAnswer::deserialize(data).unwrap_or_default();
    // (date, score, rating), newest first as published
    let days: Vec<(String, f64, String)> = lenient::rows::<CryptoDay>(&answer.data)
        .into_iter()
        .filter_map(|r| {
            let date = day(r.timestamp.map(|t| t * 1000.0));
            match (date.is_empty(), r.value) {
                (false, Some(v)) => Some((date, round1(v), rating(&r.value_classification, Some(v)))),
                _ => None,
            }
        })
        .collect();
    let now = days.first()?;
    let previous: Vec<GaugeReading> = [(1usize, "Yesterday"), (7, "A week ago"), (30, "A month ago"), (365, "A year ago")]
        .into_iter()
        .filter_map(|(i, label)| days.get(i).map(|(_, score, rating)| GaugeReading { label: label.to_string(), score: *score, rating: rating.clone() }))
        .collect();
    let mut series: Vec<GaugePoint> = days.iter().map(|(date, score, _)| GaugePoint { date: date.clone(), score: *score }).collect();
    series.sort_by(|a, b| a.date.cmp(&b.date));
    Some(Gauge {
        index: "crypto".into(),
        source: "Alternative.me".into(),
        score: now.1,
        rating: now.2.clone(),
        as_of: format!("{}T00:00:00Z", now.0),
        previous,
        parts: vec![],
        series,
    })
}

/// One index as its publisher gives it now, or nothing where it
/// did not answer.
pub fn read(index: &str) -> Option<Gauge> {
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
        _ => return None,
    };
    let data: Value = serde_json::from_str(&text.ok()?).ok()?;
    match which.as_str() {
        "stocks" => parse_stocks(&data),
        "crypto" => parse_crypto(&data),
        _ => None,
    }
}
