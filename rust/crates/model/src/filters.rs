//! One filter object, applied to the trades and the positions alike:
//! `clean_filters`, `trade_matches`, `position_matches`.

use serde::Serialize;
use serde_json::Value;

use crate::dates::shift_date;
use crate::lenient::opt_num;
use crate::value::s as text;
use crate::wire::{Position, Trade};

pub const BENCHMARK_LABELS: [(&str, &str); 3] = [("SP500", "S&P 500"), ("TSX", "S&P/TSX"), ("TSX60", "TSX 60")];
pub const PRESET_DAYS: [(&str, i64); 7] = [("1d", 1), ("1w", 7), ("1m", 30), ("3m", 90), ("6m", 180), ("1y", 365), ("5y", 1826)];

/// Which side of its bound a range keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ts_rs::TS)]
pub enum Op {
    #[serde(rename = ">")]
    Above,
    #[serde(rename = "<")]
    Below,
}

#[derive(Clone, Debug, Serialize, ts_rs::TS)]
pub struct Range {
    pub op: Op,
    /// Nothing: the range is off.
    pub v: Option<f64>,
}

impl Default for Range {
    fn default() -> Range {
        Range { op: Op::Above, v: None }
    }
}

impl Range {
    fn keeps(&self, value: f64) -> bool {
        match (self.v, self.op) {
            (None, _) => true,
            (Some(bound), Op::Above) => value > bound,
            (Some(bound), Op::Below) => value < bound,
        }
    }
}

/// The values each list filter keeps; empty keeps everything.
#[derive(Clone, Debug, Default, Serialize, ts_rs::TS)]
pub struct Lists {
    pub account: Vec<String>,
    pub symbol: Vec<String>,
    pub grade: Vec<String>,
    pub tag: Vec<String>,
    pub kind: Vec<String>,
    pub exchange: Vec<String>,
    pub side: Vec<String>,
    pub result: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, ts_rs::TS)]
pub struct Ranges {
    pub price: Range,
    pub hold: Range,
    pub pnl: Range,
    pub qty: Range,
}

/// The filters as the model acts on them, and as it echoes them to the page.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
pub struct Filters {
    pub lists: Lists,
    pub ranges: Ranges,
    pub preset: String,
    pub years: Vec<String>,
    pub from: String,
    pub to: String,
    pub search: String,
    pub benchmark: String,
}

impl Default for Filters {
    fn default() -> Self {
        Filters { lists: Lists::default(), ranges: Ranges::default(), preset: "all".into(), years: vec![], from: String::new(), to: String::new(), search: String::new(), benchmark: "SP500".into() }
    }
}

impl Filters {
    /// The filters as text, the same for the same filters: what a view is remembered under.
    pub fn key(&self) -> String {
        serde_json::to_string(self).expect("filters are plain data")
    }

    /// The filters in force that the cashflow does not read, lists then ranges.
    pub fn unread_by_cashflow(&self) -> Vec<&'static str> {
        let l = &self.lists;
        let r = &self.ranges;
        [("grade", !l.grade.is_empty()), ("tag", !l.tag.is_empty()), ("kind", !l.kind.is_empty()), ("exchange", !l.exchange.is_empty()), ("side", !l.side.is_empty()), ("result", !l.result.is_empty()),
         ("price", r.price.v.is_some()), ("hold", r.hold.v.is_some()), ("pnl", r.pnl.v.is_some()), ("qty", r.qty.v.is_some())]
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name)
            .collect()
    }
}

fn is_four_digits(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|c| c.is_ascii_digit())
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b[..4].iter().all(|c| c.is_ascii_digit()) && b[5..7].iter().all(|c| c.is_ascii_digit()) && b[8..10].iter().all(|c| c.is_ascii_digit())
}

/// Whatever the page sent, reduced to the shape the model will act on. Anything
/// unrecognised falls back to the default rather than filtering the book to
/// nothing.
pub fn clean_filters(raw: Option<&Value>) -> Filters {
    let mut f = Filters::default();
    let raw = match raw {
        Some(Value::Object(_)) => raw.unwrap(),
        _ => return f,
    };

    if let Some(Value::Object(lists)) = raw.get("lists") {
        let list = |k: &str| -> Option<Vec<String>> {
            match lists.get(k) {
                Some(Value::Array(vals)) => Some(vals.iter().map(|v| text(Some(v))).filter(|s| !s.is_empty()).collect()),
                _ => None,
            }
        };
        let l = &mut f.lists;
        for (k, slot) in [("account", &mut l.account), ("symbol", &mut l.symbol), ("grade", &mut l.grade), ("tag", &mut l.tag), ("kind", &mut l.kind), ("exchange", &mut l.exchange), ("side", &mut l.side), ("result", &mut l.result)] {
            if let Some(values) = list(k) {
                *slot = values;
            }
        }
    }
    if let Some(Value::Object(ranges)) = raw.get("ranges") {
        let r = &mut f.ranges;
        for (k, slot) in [("price", &mut r.price), ("hold", &mut r.hold), ("pnl", &mut r.pnl), ("qty", &mut r.qty)] {
            if let Some(Value::Object(given)) = ranges.get(k) {
                slot.op = if text(given.get("op")) == "<" { Op::Below } else { Op::Above };
                slot.v = given.get("v").and_then(opt_num);
            }
        }
    }
    let preset = text(raw.get("preset")).to_lowercase();
    f.preset = if PRESET_DAYS.iter().any(|(k, _)| *k == preset) || preset == "ytd" || preset == "all" { preset } else { "all".into() };
    if let Some(Value::Array(years)) = raw.get("years") {
        let mut ys: Vec<String> = years.iter().map(|y| text(Some(y)).chars().take(4).collect::<String>()).filter(|y| is_four_digits(y)).collect();
        ys.sort();
        ys.dedup();
        f.years = ys;
    }
    let day = |k: &str| -> String {
        let v: String = text(raw.get(k)).chars().take(10).collect();
        if is_iso_date(&v) { v } else { String::new() }
    };
    f.from = day("from");
    f.to = day("to");
    f.search = text(raw.get("search")).trim().to_string();
    let b = text(raw.get("benchmark")).trim().to_uppercase();
    f.benchmark = if BENCHMARK_LABELS.iter().any(|(k, _)| *k == b) { b } else { "SP500".into() };
    f
}

/// The explicit range, then the preset; `None` when the years list is doing the
/// filtering instead.
pub fn date_bounds(f: &Filters, today: &str) -> Option<(String, String)> {
    if !f.from.is_empty() || !f.to.is_empty() {
        let from = if f.from.is_empty() { "0000-01-01".to_string() } else { f.from.clone() };
        let to = if f.to.is_empty() { "9999-12-31".to_string() } else { f.to.clone() };
        return Some((from, to));
    }
    if !f.years.is_empty() {
        return None;
    }
    if f.preset == "ytd" {
        return Some((format!("{}-01-01", &today[..4.min(today.len())]), today.to_string()));
    }
    if let Some((_, days)) = PRESET_DAYS.iter().find(|(k, _)| *k == f.preset) {
        return Some((shift_date(today, -days), today.to_string()));
    }
    None
}

pub fn in_date_scope(f: &Filters, today: &str, day: &str) -> bool {
    if let Some((lo, hi)) = date_bounds(f, today) {
        return lo.as_str() <= day && day <= hi.as_str();
    }
    if !f.years.is_empty() {
        let y: String = day.chars().take(4).collect();
        return f.years.contains(&y);
    }
    true
}

fn contains_ci(haystack: &str, needle_upper: &str) -> bool {
    haystack.to_uppercase().contains(needle_upper)
}

/// Whether a list filter keeps a value: an empty list keeps everything.
fn keeps(list: &[String], value: &str) -> bool {
    list.is_empty() || list.iter().any(|v| v == value)
}

pub fn trade_matches(t: &Trade, f: &Filters, today: &str) -> bool {
    let s = f.search.to_uppercase();
    if !s.is_empty() && !contains_ci(&t.symbol, &s) && !contains_ci(&t.underlying, &s) && !contains_ci(&t.name, &s) {
        return false;
    }
    let l = &f.lists;
    if !keeps(&l.account, &t.account) || !(keeps(&l.symbol, &t.symbol) || keeps(&l.symbol, &t.underlying)) {
        return false;
    }
    if !keeps(&l.grade, if t.grade.is_empty() { "Ungraded" } else { &t.grade }) {
        return false;
    }
    if !l.tag.is_empty() {
        let tagged = if t.tags.is_empty() { l.tag.iter().any(|x| x == "untagged") } else { t.tags.iter().any(|x| l.tag.contains(x)) };
        if !tagged {
            return false;
        }
    }
    if !keeps(&l.kind, t.kind.as_str()) || !keeps(&l.exchange, &t.exchange) || !keeps(&l.side, t.side.as_str()) {
        return false;
    }
    let result = if t.pnl_cad > 0.0 { "Winners" } else if t.pnl_cad < 0.0 { "Losers" } else { "Breakeven" };
    if !keeps(&l.result, result) {
        return false;
    }
    let r = &f.ranges;
    if !r.price.keeps(t.entry) || !r.hold.keeps(t.hold_days as f64) || !r.pnl.keeps(t.pnl_cad) || !r.qty.keeps(t.qty) {
        return false;
    }
    in_date_scope(f, today, &t.exit_date)
}

pub fn position_matches(p: &Position, f: &Filters) -> bool {
    let s = f.search.to_uppercase();
    if !s.is_empty() && !contains_ci(&p.symbol, &s) && !contains_ci(&p.name, &s) {
        return false;
    }
    let l = &f.lists;
    keeps(&l.account, &p.account) && (keeps(&l.symbol, &p.symbol) || keeps(&l.symbol, &p.underlying)) && keeps(&l.kind, p.kind.as_str()) && keeps(&l.exchange, &p.exchange)
}
