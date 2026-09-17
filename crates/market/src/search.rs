//! Symbol search: the ⌘K box asks the exchanges' own directories, never
//! Wealthsimple, for a text the book has no symbol for -- Nasdaq's autocomplete
//! for US listings, TSX's company directory for the TSX and the TSX-V, the
//! three asked together -- remembered for the process.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bagholder_model::value::field_s;

pub const NASDAQ_SEARCH_URL: &str = "https://api.nasdaq.com/api/autocomplete/slookup/10?search={}";
pub const TSX_SEARCH_URL: &str = "https://www.tsx.com/json/company-directory/search/{}/{}";
/// Nasdaq's exchange code, as the book names the exchange.
const NASDAQ_EXCHANGES: [(&str, &str); 8] = [
    ("NYSE", "NYSE"), ("AMEX", "NYSE"), ("PSE", "NYSE"), ("NASDAQ-GS", "NASDAQ"), ("NASDAQ-GM", "NASDAQ"),
    ("NASDAQ-CM", "NASDAQ"), ("NASDAQ", "NASDAQ"), ("BAT", "BATS"),
];
const NASDAQ_ASSETS: [&str; 2] = ["STOCKS", "ETF"];
/// Warrants, units and rights listed beside a share.
const NASDAQ_DERIVATIVE_SUFFIXES: [&str; 5] = ["WS", "W", "U", "RT", "R"];
const NASDAQ_NAME_TAILS: [&str; 5] = [" Common Stock", " Common Shares", " Ordinary Shares", " Class A Common Stock", " Class A Ordinary Shares"];
pub const SEARCH_MAX: usize = 12;

fn headers() -> [(&'static str, &'static str); 3] {
    [("User-Agent", crate::http::UA), ("Accept", "application/json, text/plain, */*"), ("Accept-Language", "en-CA,en;q=0.9")]
}

fn s(v: Option<&Value>) -> String {
    bagholder_model::value::s(v.filter(|x| !x.is_null()))
}

/// `bagholder.parse_nasdaq_search`: US shares and ETFs on the exchanges
/// Wealthsimple trades.
pub fn parse_nasdaq_search(data: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let rows = match data { Value::Object(_) => data.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default(), _ => vec![] };
    for q in rows {
        if !q.is_object() || !NASDAQ_ASSETS.contains(&s(q.get("asset")).to_uppercase().as_str()) {
            continue;
        }
        let ex = NASDAQ_EXCHANGES.iter().find(|(k, _)| *k == s(q.get("exchange")).to_uppercase()).map(|(_, v)| *v);
        let sym = s(q.get("symbol")).to_uppercase();
        let tail = sym.rsplit('.').next().unwrap_or("").to_string();
        let ex = match ex { Some(e) if !sym.is_empty() && !NASDAQ_DERIVATIVE_SUFFIXES.contains(&tail.as_str()) => e, _ => continue };
        let mut name = s(q.get("name"));
        for t in NASDAQ_NAME_TAILS {
            if name.ends_with(t) {
                name = name[..name.len() - t.len()].trim_end_matches([' ', ',']).to_string();
                break;
            }
        }
        out.push(json!({"symbol": sym, "name": name, "exchange": ex, "currency": "USD"}));
    }
    out
}

/// `bagholder.parse_tsx_search`: that exchange's listings, one per issuer.
pub fn parse_tsx_search(data: &Value, exchange: &str) -> Vec<Value> {
    let rows = match data { Value::Object(_) => data.get("results").and_then(|d| d.as_array()).cloned().unwrap_or_default(), _ => vec![] };
    let mut out = Vec::new();
    for r in rows {
        let sym = if r.is_object() { s(r.get("symbol")).to_uppercase() } else { String::new() };
        if !sym.is_empty() {
            out.push(json!({"symbol": sym, "name": s(r.get("name")), "exchange": exchange, "currency": "CAD"}));
        }
    }
    out
}

/// `bagholder.rank_search`: exact symbols first, then symbols starting with the
/// text, then the rest, each group in the order the sources gave; duplicates
/// dropped; at most twelve.
pub fn rank_search(text: &str, rows: Vec<Value>) -> Vec<Value> {
    let key = bagholder_model::pytext::py_strip(text).to_uppercase();
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for r in rows {
        let k = (field_s(&r, "symbol"), field_s(&r, "exchange"));
        if seen.contains(&k) {
            continue;
        }
        seen.push(k);
        out.push(r);
    }
    // an instrument found by an alias ranks as the exact match it is
    let rank = |r: &Value| -> f64 {
        match r.get("rank") {
            Some(v) if !v.is_null() => v.as_f64().unwrap_or(0.0),
            _ => {
                let sym = field_s(r, "symbol");
                if sym == key { 0.0 } else if sym.starts_with(&key) { 1.0 } else { 2.0 }
            }
        }
    };
    out.sort_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(SEARCH_MAX);
    out
}

fn cache() -> &'static Mutex<HashMap<String, Vec<Value>>> {
    static C: OnceLock<Mutex<HashMap<String, Vec<Value>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `bagholder.symbol_search`: the listings the directories find for the text,
/// remembered for the process. A source that fails leaves the others' answer;
/// nothing is remembered when one failed.
pub fn symbol_search(conn_path: &std::path::Path, text: &str) -> Value {
    let text = bagholder_model::pytext::py_strip(text).to_string();
    if text.is_empty() {
        return json!({"ok": true, "matches": []});
    }
    let key = text.to_uppercase();
    if let Some(hit) = cache().lock().unwrap().get(&key) {
        return json!({"ok": true, "matches": hit});
    }
    let (text, venues) = crate::parse::yahoo_split(&text);
    let q = crate::quotes::percent_encode(&text);
    let jobs: Vec<(&str, String)> = vec![
        ("nasdaq", NASDAQ_SEARCH_URL.replace("{}", &q)),
        ("tsx", TSX_SEARCH_URL.replacen("{}", "tsx", 1).replacen("{}", &q, 1)),
        ("tsxv", TSX_SEARCH_URL.replacen("{}", "tsxv", 1).replacen("{}", &q, 1)),
    ];
    let results: Vec<(String, Result<Vec<Value>, String>)> = std::thread::scope(|sc| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(name, url)| {
                let name = name.to_string();
                let url = url.clone();
                sc.spawn(move || {
                    let got = crate::http::get_text(&url, &headers())
                        .map_err(|e| e.to_string())
                        .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                        .map(|d| match name.as_str() {
                            "nasdaq" => parse_nasdaq_search(&d),
                            "tsx" => parse_tsx_search(&d, "TSX"),
                            _ => parse_tsx_search(&d, "TSX-V"),
                        });
                    (name, got)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| (String::new(), Err("failed".into())))).collect()
    });
    let mut answers: HashMap<String, Vec<Value>> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();
    for (name, r) in results {
        match r {
            Ok(rows) => {
                answers.insert(name, rows);
            }
            Err(e) => errors.push(e),
        }
    }
    if answers.is_empty() {
        return json!({"ok": false, "error": format!("Search failed: {}", errors.join("; ")), "matches": []});
    }
    let mut found: Vec<Value> = jobs.iter().flat_map(|(n, _)| answers.get(*n).cloned().unwrap_or_default()).collect();
    if found.is_empty() && text.chars().count() <= 6 && !text.contains(' ') {
        // the directories carry the TSX and Nasdaq registries alone, so a CSE
        // or Cboe Canada listing is in none of them: TMX is asked what it
        // knows the ticker as
        if let Ok(conn) = rusqlite::Connection::open(conn_path) {
            let (today, _, _) = crate::clock_now();
            if let Some(hit) = crate::tmx::tmx_listing(&conn, &text, &today) {
                found = vec![hit];
            }
        }
    }
    let mut all = bagholder_model::instruments::search(&text);
    all.extend(found);
    let mut rows = rank_search(&text, all);
    if let Some(v) = venues {
        let kept: Vec<Value> = rows.iter().filter(|r| v.contains(&field_s(r, "exchange").to_uppercase().as_str())).cloned().collect();
        if !kept.is_empty() {
            rows = kept;
        }
    }
    if errors.is_empty() {
        cache().lock().unwrap().insert(key, rows.clone());
    }
    json!({"ok": true, "matches": rows})
}
