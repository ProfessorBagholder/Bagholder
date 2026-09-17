//! One filter object, applied to the trades and the positions alike:
//! `clean_filters`, `trade_matches`, `position_matches`.

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::dates::shift_date;
use crate::value::{field_s, get, num, s as vs};

pub const LIST_KEYS: [&str; 8] = ["account", "symbol", "grade", "tag", "kind", "exchange", "side", "result"];
pub const RANGE_KEYS: [&str; 4] = ["price", "hold", "pnl", "qty"];
pub const BENCHMARK_LABELS: [(&str, &str); 3] = [("SP500", "S&P 500"), ("TSX", "S&P/TSX"), ("TSX60", "TSX 60")];
pub const PRESET_DAYS: [(&str, i64); 7] =
    [("1d", 1), ("1w", 7), ("1m", 30), ("3m", 90), ("6m", 180), ("1y", 365), ("5y", 1826)];

#[derive(Clone, Debug)]
pub struct Range {
    pub op: String,
    pub v: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Filters {
    pub lists: HashMap<String, Vec<String>>,
    pub ranges: HashMap<String, Range>,
    pub preset: String,
    pub years: Vec<String>,
    pub from: String,
    pub to: String,
    pub search: String,
    pub benchmark: String,
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            lists: LIST_KEYS.iter().map(|k| (k.to_string(), vec![])).collect(),
            ranges: RANGE_KEYS
                .iter()
                .map(|k| (k.to_string(), Range { op: ">".into(), v: None }))
                .collect(),
            preset: "all".into(),
            years: vec![],
            from: String::new(),
            to: String::new(),
            search: String::new(),
            benchmark: "SP500".into(),
        }
    }
}

impl Filters {
    pub fn list(&self, k: &str) -> &[String] {
        self.lists.get(k).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn to_json(&self) -> Value {
        let mut lists = serde_json::Map::new();
        for k in LIST_KEYS {
            lists.insert(k.into(), json!(self.list(k)));
        }
        let mut ranges = serde_json::Map::new();
        for k in RANGE_KEYS {
            let r = &self.ranges[k];
            ranges.insert(k.into(), json!({"op": r.op, "v": r.v}));
        }
        json!({
            "lists": lists, "ranges": ranges, "preset": self.preset, "years": self.years,
            "from": self.from, "to": self.to, "search": self.search, "benchmark": self.benchmark,
        })
    }
}

fn is_four_digits(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|c| c.is_ascii_digit())
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit())
}

/// `clean_filters`: whatever the page sent, reduced to the shape the
/// model will act on. Anything unrecognised falls back to the default rather
/// than filtering the book to nothing.
pub fn clean_filters(raw: Option<&Value>) -> Filters {
    let mut f = Filters::default();
    let raw = match raw {
        Some(Value::Object(_)) => raw.unwrap(),
        _ => return f,
    };

    if let Some(Value::Object(lists)) = raw.get("lists") {
        for k in LIST_KEYS {
            if let Some(Value::Array(vals)) = lists.get(k) {
                let cleaned: Vec<String> = vals.iter().map(|v| vs(Some(v))).filter(|s| !s.is_empty()).collect();
                f.lists.insert(k.into(), cleaned);
            }
        }
    }
    if let Some(Value::Object(ranges)) = raw.get("ranges") {
        for k in RANGE_KEYS {
            if let Some(Value::Object(r)) = ranges.get(k) {
                let op = vs(r.get("op"));
                let entry = f.ranges.get_mut(k).unwrap();
                entry.op = if op == ">" || op == "<" { op } else { ">".into() };
                entry.v = match r.get("v") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) if s.is_empty() => None,
                    Some(x) => {
                        let n = num(Some(x), f64::NAN);
                        if n.is_nan() { None } else { Some(n) }
                    }
                };
            }
        }
    }
    let preset = vs(raw.get("preset")).to_lowercase();
    f.preset = if PRESET_DAYS.iter().any(|(k, _)| *k == preset) || preset == "ytd" || preset == "all" {
        preset
    } else {
        "all".into()
    };
    if let Some(Value::Array(years)) = raw.get("years") {
        let mut ys: Vec<String> = years
            .iter()
            .map(|y| vs(Some(y)).chars().take(4).collect::<String>())
            .filter(|y| is_four_digits(y))
            .collect();
        ys.sort();
        ys.dedup();
        f.years = ys;
    }
    for k in ["from", "to"] {
        let v: String = vs(raw.get(k)).chars().take(10).collect();
        let v = if is_iso_date(&v) { v } else { String::new() };
        if k == "from" { f.from = v } else { f.to = v }
    }
    f.search = vs(raw.get("search")).trim().to_string();
    let b = vs(raw.get("benchmark")).trim().to_uppercase();
    f.benchmark = if BENCHMARK_LABELS.iter().any(|(k, _)| *k == b) { b } else { "SP500".into() };
    f
}

/// `date_bounds`: the explicit range, then the preset; `None` when the
/// years list is doing the filtering instead.
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

/// `in_date_scope`.
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

/// `trade_matches`.
pub fn trade_matches(t: &Value, f: &Filters, today: &str) -> bool {
    let s = f.search.to_uppercase();
    if !s.is_empty()
        && !contains_ci(&field_s(t, "symbol"), &s)
        && !contains_ci(&field_s(t, "underlying"), &s)
        && !contains_ci(&field_s(t, "name"), &s)
    {
        return false;
    }
    let account = f.list("account");
    if !account.is_empty() && !account.contains(&field_s(t, "account")) {
        return false;
    }
    let symbol = f.list("symbol");
    if !symbol.is_empty() && !symbol.contains(&field_s(t, "symbol")) && !symbol.contains(&field_s(t, "underlying")) {
        return false;
    }
    let grade_list = f.list("grade");
    if !grade_list.is_empty() {
        let g = field_s(t, "grade");
        let g = if g.is_empty() { "Ungraded".to_string() } else { g };
        if !grade_list.contains(&g) {
            return false;
        }
    }
    let tag_list = f.list("tag");
    if !tag_list.is_empty() {
        let tags: Vec<String> = match t.get("tags").and_then(|v| v.as_array()) {
            Some(a) if !a.is_empty() => a.iter().map(|x| vs(Some(x))).collect(),
            _ => vec!["untagged".into()],
        };
        if !tags.iter().any(|x| tag_list.contains(x)) {
            return false;
        }
    }
    let kind = f.list("kind");
    if !kind.is_empty() && !kind.contains(&field_s(t, "kind")) {
        return false;
    }
    let exchange = f.list("exchange");
    if !exchange.is_empty() && !exchange.contains(&field_s(t, "exchange")) {
        return false;
    }
    let side = f.list("side");
    if !side.is_empty() && !side.contains(&field_s(t, "side")) {
        return false;
    }
    let result = f.list("result");
    if !result.is_empty() {
        let p = num(get(t, "pnlCad"), 0.0);
        let res = if p > 0.0 { "Winners" } else if p < 0.0 { "Losers" } else { "Breakeven" };
        if !result.iter().any(|r| r == res) {
            return false;
        }
    }
    for (key, val) in [
        ("price", num(get(t, "entry"), 0.0)),
        ("hold", num(get(t, "holdDays"), 0.0)),
        ("pnl", num(get(t, "pnlCad"), 0.0)),
        ("qty", num(get(t, "qty"), 0.0)),
    ] {
        let r = &f.ranges[key];
        let bound = match r.v { Some(v) => v, None => continue };
        if r.op == ">" && !(val > bound) {
            return false;
        }
        if r.op == "<" && !(val < bound) {
            return false;
        }
    }
    in_date_scope(f, today, &field_s(t, "exitDate"))
}

/// `position_matches`.
pub fn position_matches(p: &Value, f: &Filters) -> bool {
    let s = f.search.to_uppercase();
    if !s.is_empty() && !contains_ci(&field_s(p, "symbol"), &s) && !contains_ci(&field_s(p, "name"), &s) {
        return false;
    }
    let account = f.list("account");
    if !account.is_empty() && !account.contains(&field_s(p, "account")) {
        return false;
    }
    let symbol = f.list("symbol");
    if !symbol.is_empty() && !symbol.contains(&field_s(p, "symbol")) && !symbol.contains(&field_s(p, "underlying")) {
        return false;
    }
    let kind = f.list("kind");
    if !kind.is_empty() && !kind.contains(&field_s(p, "kind")) {
        return false;
    }
    let exchange = f.list("exchange");
    if !exchange.is_empty() && !exchange.contains(&field_s(p, "exchange")) {
        return false;
    }
    true
}
