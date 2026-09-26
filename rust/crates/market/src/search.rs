//! Symbol search: the ⌘K box asks the exchanges' own directories, never
//! Wealthsimple, for a text the book has no symbol for -- Nasdaq's autocomplete
//! for US listings, TSX's company directory for the TSX and the TSX-V, the
//! three asked together -- remembered for the process.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bagholder_model::wire::SymbolMatch;

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

/// US shares and ETFs on the exchanges
/// Wealthsimple trades.
pub fn parse_nasdaq_search(data: &Value) -> Vec<SymbolMatch> {
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
        out.push(SymbolMatch { symbol: sym, name, exchange: ex.to_string(), currency: "USD".into(), ..SymbolMatch::default() });
    }
    out
}

/// That exchange's listings, one per issuer.
pub fn parse_tsx_search(data: &Value, exchange: &str) -> Vec<SymbolMatch> {
    let rows = match data { Value::Object(_) => data.get("results").and_then(|d| d.as_array()).cloned().unwrap_or_default(), _ => vec![] };
    let mut out = Vec::new();
    for r in rows {
        let sym = if r.is_object() { s(r.get("symbol")).to_uppercase() } else { String::new() };
        if !sym.is_empty() {
            out.push(SymbolMatch { symbol: sym, name: s(r.get("name")), exchange: exchange.to_string(), currency: "CAD".into(), ..SymbolMatch::default() });
        }
    }
    out
}

/// Exact symbols first, then symbols starting with the
/// text, then the rest, each group in the order the sources gave; duplicates
/// dropped; at most twelve.
pub fn rank_search(text: &str, rows: Vec<SymbolMatch>) -> Vec<SymbolMatch> {
    let key = bagholder_model::textrules::trim_space(text).to_uppercase();
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out: Vec<SymbolMatch> = Vec::new();
    for r in rows {
        let k = (r.symbol.clone(), r.exchange.clone());
        if seen.contains(&k) {
            continue;
        }
        seen.push(k);
        out.push(r);
    }
    // an instrument found by an alias ranks as the exact match it is
    let rank = |r: &SymbolMatch| -> f64 {
        match r.rank {
            Some(v) => v,
            None => {
                if r.symbol == key { 0.0 } else if r.symbol.starts_with(&key) { 1.0 } else { 2.0 }
            }
        }
    };
    out.sort_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(SEARCH_MAX);
    out
}

fn cache() -> &'static Mutex<HashMap<String, Vec<SymbolMatch>>> {
    static C: OnceLock<Mutex<HashMap<String, Vec<SymbolMatch>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The listings the directories find for the text,
/// remembered for the process. A source that fails leaves the others' answer;
/// nothing is remembered when one failed. `Err` is every source's own failure,
/// joined into one message.
pub fn symbol_search(pool: &std::sync::Arc<bagholder_store::pool::Pool>, text: &str) -> Result<Vec<SymbolMatch>, String> {
    let text = bagholder_model::textrules::trim_space(text).to_string();
    if text.is_empty() {
        return Ok(vec![]);
    }
    let key = text.to_uppercase();
    if let Some(hit) = cache().lock().unwrap().get(&key) {
        return Ok(hit.clone());
    }
    let (text, venues) = crate::parse::yahoo_split(&text);
    let q = crate::quotes::percent_encode(&text);
    let jobs: Vec<(&str, String)> = vec![
        ("nasdaq", NASDAQ_SEARCH_URL.replace("{}", &q)),
        ("tsx", TSX_SEARCH_URL.replacen("{}", "tsx", 1).replacen("{}", &q, 1)),
        ("tsxv", TSX_SEARCH_URL.replacen("{}", "tsxv", 1).replacen("{}", &q, 1)),
    ];
    let results: Vec<(String, Result<Vec<SymbolMatch>, String>)> = std::thread::scope(|sc| {
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
    let mut answers: HashMap<String, Vec<SymbolMatch>> = HashMap::new();
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
        return Err(format!("Search failed: {}", errors.join("; ")));
    }
    let mut found: Vec<SymbolMatch> = jobs.iter().flat_map(|(n, _)| answers.get(*n).cloned().unwrap_or_default()).collect();
    if found.is_empty() && text.chars().count() <= 6 && !text.contains(' ') {
        // the directories carry the TSX and Nasdaq registries alone, so a CSE
        // or Cboe Canada listing is in none of them: TMX is asked what it
        // knows the ticker as
        if let Ok(conn) = pool.get() {
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
        let kept: Vec<SymbolMatch> = rows.iter().filter(|r| v.contains(&r.exchange.to_uppercase().as_str())).cloned().collect();
        if !kept.is_empty() {
            rows = kept;
        }
    }
    if errors.is_empty() {
        cache().lock().unwrap().insert(key, rows.clone());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn m(symbol: &str, name: &str, exchange: &str, currency: &str) -> SymbolMatch {
        SymbolMatch { symbol: symbol.into(), name: name.into(), exchange: exchange.into(), currency: currency.into(), ..SymbolMatch::default() }
    }

    #[test]
    fn test_nasdaq_search_keeps_shares_and_etfs_and_drops_derivatives() {
        let data = json!({"data": [
            {"symbol": "AAPL", "name": "Apple Inc. Common Stock", "exchange": "NASDAQ-GS", "asset": "STOCKS"},
            {"symbol": "SPY", "name": "SPDR S&P 500 ETF Trust", "exchange": "NASDAQ", "asset": "ETF"},
            {"symbol": "AAPL.WS", "name": "Apple Warrants", "exchange": "NASDAQ-GS", "asset": "STOCKS"},
            {"symbol": "XYZ", "name": "Some Bond", "exchange": "NASDAQ-GS", "asset": "BOND"},
            {"symbol": "NOEX", "name": "No Exchange Match", "exchange": "OTC", "asset": "STOCKS"},
        ]});
        let out = parse_nasdaq_search(&data);
        assert_eq!(out, vec![m("AAPL", "Apple Inc.", "NASDAQ", "USD"), m("SPY", "SPDR S&P 500 ETF Trust", "NASDAQ", "USD")]);
    }

    #[test]
    fn test_tsx_search_reads_each_result_under_the_exchange_given() {
        let data = json!({"results": [{"symbol": "shop", "name": "Shopify Inc."}, {"symbol": ""}]});
        assert_eq!(parse_tsx_search(&data, "TSX"), vec![m("SHOP", "Shopify Inc.", "TSX", "CAD")]);
    }

    #[test]
    fn test_rank_search_orders_exact_then_prefix_then_the_rest_and_drops_duplicates() {
        let rows = vec![
            m("SHOPIFY", "not exact", "TSX", ""),
            m("SHOP", "Shopify Inc.", "TSX", ""),
            m("SHOP", "duplicate, dropped", "TSX", ""),
            SymbolMatch { rank: Some(0.0), ..m("AAPL", "Apple Inc.", "NASDAQ", "") },
        ];
        let out = rank_search("SHOP", rows);
        assert_eq!(out.iter().map(|r| (r.symbol.clone(), r.exchange.clone())).collect::<Vec<_>>(),
                   vec![("SHOP".into(), "TSX".into()), ("AAPL".into(), "NASDAQ".into()), ("SHOPIFY".into(), "TSX".into())],
                   "an exact symbol match and an explicit rank of the same weight keep their given order; a prefix match ranks after both");
    }

    #[test]
    fn test_rank_search_keeps_at_most_the_max() {
        let rows: Vec<SymbolMatch> = (0..SEARCH_MAX + 5).map(|i| m(&format!("S{}", i), "", "TSX", "")).collect();
        assert_eq!(rank_search("S", rows).len(), SEARCH_MAX);
    }
}
