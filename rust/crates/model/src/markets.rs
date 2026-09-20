//! The Markets tab: `markets_view` and everything it reads.
//!
//! The tile row, the watchlist with its quotes, the heatmap of what the book
//! holds, and the news items each tagged with the listings they were read for.

use serde_json::{json, Map, Value};

use crate::base::Base;
use crate::exposure::{norm_sector, UNCLASSIFIED};
use crate::fx::to_cad;
use crate::instruments;
use crate::venues::{tmx_symbol, watch_exposure_key, SHARE_KEY};
use crate::value::{field_s, get, num};

/// `MARKET`: the feed whose items belong to the market rather than to a
/// listing.
const MARKET_FEED: (&str, &str) = ("*", "MARKET");

/// The Markets tab's tile row when the user has never changed it.
const DEFAULT_TILES: [(&str, &str); 6] = [
    ("SPX", "INDEX"), ("NDX", "INDEX"), ("DJI", "INDEX"), ("VIX", "INDEX"), ("GC", "COMEX"), ("BTCUSD", "FX"),
];
const TILES_MAX: usize = 12;

fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

/// `watch_quote_key`: where a watched listing's quote is kept -- its
/// symbol and venue, so a listing the book also holds elsewhere keeps its own.
pub fn watch_quote_key(symbol: &str, exchange: &str) -> String {
    format!("{}@{}", symbol.trim().to_uppercase(), exchange.trim().to_uppercase())
}

/// `tile_list`: the saved set, else the default; only what the directory
/// knows, twelve at most.
fn tile_list(base: &Base) -> Vec<&'static instruments::Instrument> {
    let saved = (*base.tiles).as_ref().and_then(|v| v.as_array());
    let rows: Vec<(String, String)> = match saved {
        Some(rows) => rows
            .iter()
            .map(|r| (field_s(r, "symbol"), field_s(r, "exchange")))
            .collect(),
        None => DEFAULT_TILES.iter().map(|(s, e)| (s.to_string(), e.to_string())).collect(),
    };
    let mut out: Vec<&'static instruments::Instrument> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (sym, ex) in rows {
        if let Some(inst) = instruments::find(&sym, &ex) {
            if !seen.contains(&inst.symbol) {
                seen.push(inst.symbol);
                out.push(inst);
            }
        }
    }
    out.truncate(TILES_MAX);
    out
}

/// `tile_symbols`: the tile row keyed as a watched instrument is.
pub fn tile_symbols(base: &Base) -> Vec<Value> {
    tile_list(base)
        .into_iter()
        .map(|i| {
            json!({
                "symbol": i.symbol, "exchange": i.exchange, "currency": i.currency, "kind": "Instrument",
                "quoteKey": watch_quote_key(i.symbol, i.exchange), "yahoo": i.yahoo,
            })
        })
        .collect()
}

/// `tile_decimals`: the instrument's own price scale.
fn tile_decimals(inst: &instruments::Instrument) -> i64 {
    if inst.symbol == "BTCUSD" {
        return 0;
    }
    match inst.kind {
        "Rate" => 3,
        "Currency" => 4,
        _ => 2,
    }
}

/// `tile_rows`.
pub fn tile_rows(base: &Base) -> Vec<Value> {
    let mut out = Vec::new();
    for inst in tile_list(base) {
        let q = base.quotes.get(&watch_quote_key(inst.symbol, inst.exchange)).cloned().unwrap_or(Value::Null);
        let price = opt_num(get(&q, "price"));
        let move_ = opt_num(get(&q, "priceChange"));
        let mut row = json!({
            "symbol": inst.symbol, "exchange": inst.exchange, "label": instruments::label(inst.symbol),
            "name": inst.name, "kind": inst.kind,
            "last": price, "change": move_, "percentChange": opt_num(get(&q, "percentChange")),
            "decimals": tile_decimals(inst),
        });
        // A contract quoted as 100 minus the rate carries that rate beside its
        // published price: the price is what the exchange gives, the rate is
        // the contract's own definition of it, and a day that moves the price
        // down has moved the rate it prices up.
        if let Some(rate) = instruments::implied_rate(inst.symbol, price) {
            let m = row.as_object_mut().unwrap();
            m.insert("rate".into(), json!(rate));
            m.insert(
                "rateChange".into(),
                match move_ { Some(v) => json!(((-v) * 1e4).round() / 1e4), None => Value::Null },
            );
        }
        out.push(row);
    }
    out
}

/// `watch_symbols`: every watched listing, with what a quote source
/// needs to price it.
pub fn watch_symbols(base: &Base) -> Vec<Value> {
    let mut out = Vec::new();
    for w in base.watchlist.iter() {
        let sym = field_s(w, "symbol");
        let ex = field_s(w, "exchange");
        let inst = instruments::find(&sym, &ex);
        // a watched coin is the USD pair, whatever currency the book holds it in
        let crypto = ex.to_uppercase() == "CRYPTO";
        let currency = if crypto { "USD".to_string() } else { field_s(w, "currency") };
        let kind = if inst.is_some() { "Instrument" } else if crypto { "Crypto" } else { "Shares" };
        let mut rec = json!({
            "symbol": sym, "exchange": ex, "currency": currency, "kind": kind,
            "quoteKey": watch_quote_key(&sym, &ex),
        });
        if let Some(i) = inst {
            rec.as_object_mut().unwrap().insert("yahoo".into(), json!(i.yahoo));
        }
        out.push(rec);
    }
    out
}

/// `quote_symbols`: the watched listings, then the tile row's
/// instruments not already among them.
pub fn quote_symbols(base: &Base) -> Vec<Value> {
    let mut out = watch_symbols(base);
    let mut keys: Vec<String> = out.iter().map(|r| field_s(r, "quoteKey")).collect();
    for rec in tile_symbols(base) {
        let k = field_s(&rec, "quoteKey");
        if !keys.contains(&k) {
            keys.push(k);
            out.push(rec);
        }
    }
    out
}

/// `dominant_sector`: the sector a record gives most weight to.
pub fn dominant_sector(rec: Option<&Value>) -> String {
    let sectors = match rec.and_then(|r| r.get("sectors")).and_then(|s| s.as_object()) {
        Some(s) => s,
        None => return UNCLASSIFIED.to_string(),
    };
    let mut best = String::new();
    let mut w = 0.0_f64;
    for (name, weight) in sectors {
        let name = { let n = norm_sector(name); if n.is_empty() { name.clone() } else { n } };
        let weight = num(Some(weight), 0.0);
        if weight > w {
            best = name;
            w = weight;
        }
    }
    if best.is_empty() { UNCLASSIFIED.to_string() } else { best }
}

/// A listing is one listing whether the book names it `QNC.TO` or the
/// watchlist `QNC`: the bare ticker and the venue.
fn lk(symbol: &str, exchange: &str) -> (String, String) {
    (tmx_symbol(symbol), exchange.trim().to_uppercase())
}

/// `watch_rows`.
pub fn watch_rows(base: &Base, positions: &[Value]) -> Vec<Value> {
    let mut held: Vec<((String, String), &Value)> = Vec::new();
    for p in positions {
        let key = (field_s(p, "symbol"), field_s(p, "exchange").to_uppercase());
        if !held.iter().any(|(k, _)| *k == key) {
            held.push((key, p));
        }
    }
    let mut out = Vec::new();
    for w in base.watchlist.iter() {
        let sym = field_s(w, "symbol");
        let ex = field_s(w, "exchange");
        let q = base.quotes.get(&watch_quote_key(&sym, &ex)).cloned().unwrap_or(Value::Null);
        let key = (sym.clone(), ex.to_uppercase());
        let pos = held.iter().find(|(k, _)| *k == key).map(|(_, p)| *p);
        let inst = instruments::find(&sym, &ex);
        let crypto = ex.to_uppercase() == "CRYPTO";
        let rec = if inst.is_some() || crypto {
            None
        } else {
            base.exposures.get(&watch_exposure_key(&sym, &ex, &field_s(w, "currency")))
        };
        let exchange = match inst {
            Some(i) => i.exchange.to_string(),
            None if crypto => "Crypto".to_string(),
            None => ex.clone(),
        };
        let sector = match inst {
            Some(i) => instruments::kind_label(i.kind),
            None if crypto => "Digital assets".to_string(),
            None => match rec { Some(r) => dominant_sector(Some(r)), None => UNCLASSIFIED.to_string() },
        };
        let kind = match inst {
            Some(i) => i.kind.to_string(),
            None if crypto => "Crypto".to_string(),
            None => "Shares".to_string(),
        };
        out.push(json!({
            "symbol": sym,
            "exchange": exchange,
            "name": field_s(w, "name"),
            "currency": if crypto { "USD".to_string() } else { field_s(w, "currency") },
            "last": opt_num(get(&q, "price")),
            "priceChange": opt_num(get(&q, "priceChange")),
            "percentChange": opt_num(get(&q, "percentChange")),
            "sector": sector,
            "kind": kind,
            "positionId": pos.map(|p| json!(field_s(p, "id"))).unwrap_or(Value::Null),
        }));
    }
    out
}

/// `heatmap_items`: one tile per symbol held, its market value in CAD
/// summed over the accounts holding it.
pub fn heatmap_items(positions: &[Value], exposures: &Map<String, Value>, cad: &dyn Fn(f64, &str) -> f64) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut keys: Vec<(String, String)> = Vec::new();
    for p in positions {
        let v = cad(num(get(p, "mv"), 0.0), &field_s(p, "currency"));
        if !(v > 0.0) {
            continue;
        }
        let kind = field_s(p, "kind");
        let sector = if kind == "Crypto" {
            "Digital assets".to_string()
        } else if kind == "Options" {
            // a contract counts under its underlying's record
            let under = field_s(p, "underlying").to_uppercase();
            let us = format!("{}{}::US", SHARE_KEY, under);
            let ca = format!("{}{}:", SHARE_KEY, under);
            let (first, second) = if field_s(p, "currency").to_uppercase() == "USD" { (us, ca) } else { (ca, us) };
            dominant_sector(exposures.get(&first).or_else(|| exposures.get(&second)))
        } else {
            dominant_sector(exposures.get(&field_s(p, "securityId")))
        };
        let key = (field_s(p, "symbol"), field_s(p, "exchange").to_uppercase());
        if let Some(i) = keys.iter().position(|k| *k == key) {
            let cur = num(get(&out[i], "value"), 0.0);
            out[i].as_object_mut().unwrap().insert("value".into(), json!(cur + v));
            continue;
        }
        keys.push(key);
        out.push(json!({
            "id": field_s(p, "id"), "symbol": field_s(p, "symbol"), "exchange": field_s(p, "exchange"),
            "value": v, "percentChange": p.get("percentChange").cloned().unwrap_or(Value::Null),
            "sector": sector,
        }));
    }
    out
}

// --------------------------------------------------------------------------
// news
// --------------------------------------------------------------------------

/// `news_text_key`: a headline as one story -- letters and digits only,
/// one case, one space between words.
pub fn news_text_key(headline: &str) -> String {
    let lower = headline.to_lowercase();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.join(" ")
}

const FRENCH_WORDS: [&str; 20] = [
    "annonce", "annoncent", "ses", "du", "des", "une", "pour", "avec", "sur", "résultats", "clôture",
    "croissance", "les", "et", "au", "aux", "dans", "son", "sa", "le",
];
const FRENCH_LETTERS: [char; 14] = ['à', 'â', 'ç', 'é', 'è', 'ê', 'ë', 'î', 'ï', 'ô', 'û', 'ù', 'ü', 'ÿ'];

/// `looks_french`: accented letters or French function words, two or
/// more.
pub fn looks_french(headline: &str) -> bool {
    let t = headline.to_lowercase();
    let accents = t.chars().filter(|c| FRENCH_LETTERS.contains(c) || *c == 'œ').count();
    if accents >= 2 {
        return true;
    }
    // whole words, bounded by \b
    let words: Vec<&str> = t.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
    let hits = words.iter().filter(|w| FRENCH_WORDS.contains(w) || **w == "la").count();
    hits >= 2
}

/// Minutes since the epoch for an ISO instant, or nothing when it cannot be
/// read.
fn when_minutes(iso: &str) -> Option<f64> {
    let s = iso.replace('Z', "+00:00");
    let (d, t) = s.split_once('T')?;
    let (y, m, day) = crate::dates::parse_iso(d)?;
    let hhmm: Vec<&str> = t.split(|c| c == ':' || c == '+' || c == '-').collect();
    let hh: f64 = hhmm.first()?.parse().ok()?;
    let mm: f64 = hhmm.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let ss: f64 = hhmm.get(2).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    // the offset, when the instant carries one
    let mut offset_minutes = 0.0;
    if let Some(pos) = t.rfind(['+', '-']) {
        if pos > 0 {
            let sign = if t.as_bytes()[pos] == b'-' { -1.0 } else { 1.0 };
            let off = &t[pos + 1..];
            let (oh, om) = off.split_once(':').unwrap_or((off, "0"));
            offset_minutes = sign * (oh.parse::<f64>().unwrap_or(0.0) * 60.0 + om.parse::<f64>().unwrap_or(0.0));
        }
    }
    let days = crate::dates::to_days(y, m, day) as f64;
    Some(days * 1440.0 + hh * 60.0 + mm + ss / 60.0 - offset_minutes)
}

/// `drop_translations`: a release posted in French beside its English
/// original -- the same wire, a listing in common, within three hours -- is one
/// story. The English row stays.
fn drop_translations(rows: Vec<Value>) -> Vec<Value> {
    let keys = |r: &Value| -> Vec<(String, String)> {
        r.get("tags")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().map(|t| lk(&field_s(t, "symbol"), &field_s(t, "exchange"))).collect())
            .unwrap_or_default()
    };
    let french: Vec<bool> = rows.iter().map(|r| looks_french(&field_s(r, "headline"))).collect();
    let whens: Vec<Option<f64>> = rows.iter().map(|r| when_minutes(&field_s(r, "publishedAt"))).collect();

    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if french[i] {
            let kr = keys(r);
            let tr = whens[i];
            let twin = rows.iter().enumerate().any(|(j, o)| {
                j != i
                    && !french[j]
                    && field_s(o, "source") == field_s(r, "source")
                    && keys(o).iter().any(|k| kr.contains(k))
                    && tr.is_some()
                    && whens[j].is_some()
                    && (whens[j].unwrap() - tr.unwrap()).abs() <= 180.0
            });
            if twin {
                continue;
            }
        }
        out.push(r.clone());
    }
    out
}

/// `news_rows`: every item kept, newest first, each tagged with the
/// listings it was read for. An item two listings share is one row with two
/// tags.
pub fn news_rows(base: &Base, positions: &[Value], watch: &[Value]) -> Vec<Value> {
    let mut held: Vec<((String, String), &Value)> = Vec::new();
    for p in positions {
        let key = lk(&field_s(p, "symbol"), &field_s(p, "exchange"));
        if !held.iter().any(|(k, _)| *k == key) {
            held.push((key, p));
        }
    }
    let watched: Vec<((String, String), &Value)> = watch
        .iter()
        .map(|w| (lk(&field_s(w, "symbol"), &field_s(w, "exchange")), w))
        .collect();

    let mut items: Vec<&Value> = base.news.iter().collect();
    items.sort_by(|a, b| field_s(b, "publishedAt").cmp(&field_s(a, "publishedAt")));

    let mut rows: Vec<Value> = Vec::new();
    // one story is one row: the same wire id, or the same headline under
    // another id (a release carried by several wires, a story republished per
    // symbol, an update)
    let mut by_id: Vec<(String, usize)> = Vec::new();
    let mut by_text: Vec<(String, usize)> = Vec::new();

    for n in items {
        // the market feed's items carry no tag: they are the market's
        let is_market = (field_s(n, "symbol"), field_s(n, "exchange").to_uppercase())
            == (MARKET_FEED.0.to_string(), MARKET_FEED.1.to_string());
        let key = lk(&field_s(n, "symbol"), &field_s(n, "exchange"));
        let p = held.iter().find(|(k, _)| *k == key).map(|(_, p)| *p);
        let w = watched.iter().find(|(k, _)| *k == key).map(|(_, w)| *w);
        let tag = if is_market {
            Value::Null
        } else {
            json!({
                "symbol": key.0, "exchange": field_s(n, "exchange"),
                "held": p.is_some(), "watched": w.is_some(),
                "percentChange": p.map(|p| p.get("percentChange").cloned().unwrap_or(Value::Null))
                    .or_else(|| w.map(|w| w.get("percentChange").cloned().unwrap_or(Value::Null)))
                    .unwrap_or(Value::Null),
                "positionId": p.map(|p| json!(field_s(p, "id"))).unwrap_or(Value::Null),
            })
        };
        let id = field_s(n, "id");
        let text = news_text_key(&field_s(n, "headline"));

        let existing = by_id
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, i)| *i)
            .or_else(|| if text.is_empty() { None } else { by_text.iter().find(|(k, _)| *k == text).map(|(_, i)| *i) });

        if let Some(i) = existing {
            if is_market {
                rows[i].as_object_mut().unwrap().insert("market".into(), json!(true));
            } else {
                let already = rows[i]["tags"]
                    .as_array()
                    .map(|a| a.iter().any(|t| lk(&field_s(t, "symbol"), &field_s(t, "exchange")) == key))
                    .unwrap_or(false);
                if !already {
                    rows[i]["tags"].as_array_mut().unwrap().push(tag);
                }
            }
            // the same text on a wire and in a publisher's column is the release
            if field_s(n, "kind") == "release" {
                rows[i].as_object_mut().unwrap().insert("kind".into(), json!("release"));
            }
            if !by_id.iter().any(|(k, _)| *k == id) {
                by_id.push((id, i));
            }
            continue;
        }

        let kind = { let k = field_s(n, "kind"); if k.is_empty() { "story".to_string() } else { k } };
        rows.push(json!({
            "id": id, "headline": field_s(n, "headline"), "source": field_s(n, "wire"),
            "url": field_s(n, "url"), "publishedAt": field_s(n, "publishedAt"),
            "market": is_market,
            "tags": if is_market { json!([]) } else { json!([tag]) },
            "kind": kind,
        }));
        let i = rows.len() - 1;
        by_id.push((field_s(&rows[i], "id"), i));
        if !text.is_empty() && !by_text.iter().any(|(k, _)| *k == text) {
            by_text.push((text, i));
        }
    }
    rows.sort_by(|a, b| field_s(b, "publishedAt").cmp(&field_s(a, "publishedAt")));
    drop_translations(rows)
}

/// `markets_view`.
pub fn markets_view(base: &Base, positions: &[Value]) -> Value {
    let today = base.today.clone();
    let cad = |amount: f64, currency: &str| to_cad(&base.fx, amount, currency, &today);
    let watch = watch_rows(base, positions);

    let mut universes = Map::new();
    for (k, rows) in base.universes.iter() {
        let arr: Vec<Value> = rows
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let sector = { let s = field_s(r, "sector"); if s.is_empty() { UNCLASSIFIED.to_string() } else { s } };
                        json!({
                            "id": Value::Null, "symbol": field_s(r, "symbol"), "name": field_s(r, "name"),
                            "value": num(get(r, "value"), 0.0),
                            "percentChange": r.get("percentChange").cloned().unwrap_or(Value::Null),
                            "sector": sector, "country": field_s(r, "country"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        universes.insert(k.clone(), Value::Array(arr));
    }

    let directory: Vec<Value> = instruments::INSTRUMENTS
        .iter()
        .map(|r| {
            json!({
                "symbol": r.symbol, "label": instruments::label(r.symbol), "name": r.name,
                "exchange": r.exchange, "kind": r.kind, "aliases": r.aliases,
            })
        })
        .collect();

    json!({
        "holdings": heatmap_items(positions, &base.exposures, &cad),
        "watchlist": watch,
        "news": news_rows(base, positions, &watch),
        "universes": universes,
        "tiles": tile_rows(base),
        "instruments": directory,
    })
}
