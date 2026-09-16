//! News for the symbols the book holds and watches, from two public
//! per-symbol sources: TMX Money's for Canadian listings and Nasdaq's for US
//! ones.
//!
//! Each item is tagged with the symbol it was read for. Nothing is guessed
//! from a headline.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::http::{get_text, post_json};
use bagholder_model::value::field_s;

const TMX_NEWS_QUERY: &str = "query getNewsForSymbol($symbol: String!, $page: Int!, $limit: Int!, $locale: String!) { news: getNewsForSymbol(symbol: $symbol, page: $page, limit: $limit, locale: $locale) { headline datetime source newsid summary } }";

pub const TMX_NEWS_URL: &str = "https://money.tmx.com/en/quote/{}/news/{}";
pub const NASDAQ_NEWS_URL: &str = "https://api.nasdaq.com/api/news/topic/articlebysymbol?q={}|STOCKS&offset=0&limit={}";
pub const NASDAQ_LATEST_URL: &str = "https://api.nasdaq.com/api/news/topic/latestnews?offset=0&limit={}";
/// A US listing's own releases, beside its news.
pub const NASDAQ_PRESS_URL: &str = "https://api.nasdaq.com/api/news/topic/press_release?q=symbol:{}|assetclass:stocks&offset=0&limit={}";

/// In a source's name: GlobeNewswire, Business Wire, PR Newswire, ACCESS
/// Newswire, TheNewsWire, Canada Newswire, TMX Newsfile, Marketwired.
pub const WIRE_MARKS: [&str; 4] = ["wire", "newsfile", "cision", "cnw"];

pub const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";

pub const PER_SYMBOL: i64 = 12;
/// The market-wide feed is a listing of its own: Nasdaq's latest news,
/// whatever it names.
pub const MARKET: (&str, &str, &str) = ("*", "MARKET", "");
pub const PER_MARKET: i64 = 50;
pub const FRESH_MINUTES: f64 = 15.0;
/// Items kept in the database, newest first.
pub const KEEP: i64 = 400;

/// `news.NASDAQ_HEADERS`.
pub fn nasdaq_headers() -> [(&'static str, &'static str); 4] {
    [
        ("User-Agent", UA),
        ("Accept", "application/json, text/plain, */*"),
        ("Origin", "https://www.nasdaq.com"),
        ("Referer", "https://www.nasdaq.com/"),
    ]
}

/// `news.TMX_HEADERS`: TMX Money's own page, not the quote client's.
pub fn tmx_headers() -> [(&'static str, &'static str); 4] {
    [
        ("User-Agent", UA),
        ("locale", "en"),
        ("Origin", "https://money.tmx.com"),
        ("Referer", "https://money.tmx.com/"),
    ]
}

/// `news._pace`: one call to a host every six-tenths of a second.
pub fn pace(host: &str) {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(HashMap::new()));
    let wait = {
        let mut m = last.lock().unwrap();
        let now = Instant::now();
        let w = m.get(host).map(|t| {
            let target = *t + Duration::from_millis(600);
            if target > now { target - now } else { Duration::ZERO }
        });
        m.insert(host.to_string(), now + w.unwrap_or(Duration::ZERO));
        w.unwrap_or(Duration::ZERO)
    };
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
}

/// `news.kind_of`: what an item is, told by where it came from. A wire carries
/// the company's own release; a publisher writes a story about it.
pub fn kind_of(source: &str) -> &'static str {
    let s = source.to_lowercase();
    if WIRE_MARKS.iter().any(|m| s.contains(m)) { "release" } else { "story" }
}

/// `news.clean_text`: the HTML entities resolved and the whitespace collapsed.
pub fn clean_text(t: &str) -> String {
    unescape(t).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `html.unescape`: CPython's own rule, since a headline carries whatever the
/// wire put in it.
///
/// A reference is `&` then a decimal or hexadecimal code point, or a name of
/// up to 32 characters, each with the semicolon optional. A name that is not
/// in the table is retried against its longest prefix that is, which is how
/// `&notit;` reads as `\u{ac}it;` -- the legacy names are recognised without
/// their semicolon.
fn unescape(t: &str) -> String {
    if !t.contains('&') {
        return t.to_string();
    }
    let mut out = String::with_capacity(t.len());
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            let ch = t[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        match charref(t, i + 1) {
            Some((text, next)) => {
                out.push_str(&text);
                i = next;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// One `&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\f <&#;]{1,32};?)`, replaced.
fn charref(t: &str, at: usize) -> Option<(String, usize)> {
    let b = t.as_bytes();
    if at < b.len() && b[at] == b'#' {
        let hex = at + 1 < b.len() && (b[at + 1] | 32) == b'x';
        let start = if hex { at + 2 } else { at + 1 };
        let mut end = start;
        while end < b.len() && (if hex { b[end].is_ascii_hexdigit() } else { b[end].is_ascii_digit() }) {
            end += 1;
        }
        if end == start {
            return None;
        }
        let num = u32::from_str_radix(&t[start..end], if hex { 16 } else { 10 }).unwrap_or(0x10_ffff + 1);
        let next = if end < b.len() && b[end] == b';' { end + 1 } else { end };
        return Some((codepoint(num), next));
    }
    // a name: up to 32 characters, none of them tab, newline, form feed,
    // space, `<`, `&`, `#` or `;`, and then an optional semicolon
    let mut end = at;
    while end < b.len() && end - at < 32 && !matches!(b[end], b'\t' | b'\n' | 0x0c | b' ' | b'<' | b'&' | b'#' | b';') {
        end += 1;
    }
    if end == at {
        return None;
    }
    let with_semi = end < b.len() && b[end] == b';';
    let stop = if with_semi { end + 1 } else { end };
    let name = &t[at..stop];
    if let Some(v) = lookup(name) {
        return Some((v.to_string(), stop));
    }
    // the longest prefix of the name that is in the table, the rest kept
    let mut cut = name.len();
    while cut > 1 {
        cut -= 1;
        if !name.is_char_boundary(cut) {
            continue;
        }
        if let Some(v) = lookup(&name[..cut]) {
            return Some((format!("{}{}", v, &name[cut..]), stop));
        }
    }
    None
}

fn lookup(name: &str) -> Option<&'static str> {
    crate::entities::HTML5
        .binary_search_by(|(k, _)| (*k).cmp(name))
        .ok()
        .map(|i| crate::entities::HTML5[i].1)
}

/// `html._invalid_charrefs` and `html._invalid_codepoints`: what CPython puts
/// in place of a code point that is not one.
fn codepoint(n: u32) -> String {
    const INVALID: [(u32, char); 27] = [
        (0x00, '\u{fffd}'), (0x0d, '\r'), (0x80, '\u{20ac}'), (0x81, '\u{81}'), (0x82, '\u{201a}'),
        (0x83, '\u{192}'), (0x84, '\u{201e}'), (0x85, '\u{2026}'), (0x86, '\u{2020}'), (0x87, '\u{2021}'),
        (0x88, '\u{2c6}'), (0x89, '\u{2030}'), (0x8a, '\u{160}'), (0x8b, '\u{2039}'), (0x8c, '\u{152}'),
        (0x8d, '\u{8d}'), (0x8e, '\u{17d}'), (0x8f, '\u{8f}'), (0x90, '\u{90}'), (0x91, '\u{2018}'),
        (0x92, '\u{2019}'), (0x93, '\u{201c}'), (0x94, '\u{201d}'), (0x95, '\u{2022}'), (0x96, '\u{2013}'),
        (0x97, '\u{2014}'), (0x98, '\u{2dc}'),
    ];
    const INVALID2: [(u32, char); 5] = [
        (0x99, '\u{2122}'), (0x9a, '\u{161}'), (0x9b, '\u{203a}'), (0x9c, '\u{153}'), (0x9d, '\u{9d}'),
    ];
    const INVALID3: [(u32, char); 2] = [(0x9e, '\u{17e}'), (0x9f, '\u{178}')];
    for (k, v) in INVALID.iter().chain(INVALID2.iter()).chain(INVALID3.iter()) {
        if *k == n {
            return v.to_string();
        }
    }
    if (0xd800..=0xdfff).contains(&n) || n > 0x10_ffff {
        return "\u{fffd}".to_string();
    }
    if is_invalid_codepoint(n) {
        return String::new();
    }
    char::from_u32(n).map(|c| c.to_string()).unwrap_or_default()
}

/// `html._invalid_codepoints`: the code points a document may not carry --
/// the C0 and C1 controls that are not whitespace, and the non-characters.
/// They are dropped rather than replaced.
fn is_invalid_codepoint(n: u32) -> bool {
    (0x1..=0x8).contains(&n)
        || n == 0xb
        || (0xe..=0x1f).contains(&n)
        || (0x7f..=0x9f).contains(&n)
        || (0xfdd0..=0xfdef).contains(&n)
        || matches!(n & 0xffff, 0xfffe | 0xffff)
}

/// An ISO instant in UTC, the way the app writes times.
fn utc_stamp(text: &str) -> String {
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
    let hh: i64 = match parts.first().and_then(|x| x.parse().ok()) { Some(v) => v, None => return String::new() };
    let mm: i64 = parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    let ss: i64 = parts.get(2).and_then(|x| x.split('.').next()?.parse().ok()).unwrap_or(0);
    let unix = bagholder_model::dates::to_days(y, m, dd) * 86400 + hh * 3600 + mm * 60 + ss - offset;
    stamp_of(unix)
}

fn stamp_of(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let rem = unix.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// `news.parse_tmx_news`.
pub fn parse_tmx_news(data: &Value, symbol: &str) -> Vec<Value> {
    let items = data
        .get("data")
        .and_then(|d| d.get("news"))
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    let mut rows = Vec::new();
    for it in items {
        let newsid = field_s(&it, "newsid");
        if !it.is_object() || newsid.is_empty() {
            continue;
        }
        let ts = utc_stamp(&field_s(&it, "datetime"));
        if ts.is_empty() {
            continue;
        }
        let source = clean_text(&field_s(&it, "source")).replace(" via QuoteMedia", "");
        rows.push(json!({
            "id": format!("tmx:{}", newsid),
            "headline": clean_text(&field_s(&it, "headline")),
            "source": source,
            "url": TMX_NEWS_URL.replacen("{}", symbol, 1).replacen("{}", &newsid, 1),
            "publishedAt": ts,
            "kind": kind_of(&source),
        }));
    }
    rows
}

/// `news.nasdaq_when`: Nasdaq gives a day and an age. The time is the age
/// taken off now, to the minute; an older item keeps the day alone at midnight
/// UTC.
pub fn nasdaq_when(row: &Value, now_unix: i64) -> String {
    let ago = field_s(row, "ago").to_lowercase();
    if let Some((n, unit)) = parse_ago(&ago) {
        let secs = match unit {
            "minute" => n * 60,
            "hour" => n * 3600,
            _ => n * 86400,
        };
        let t = now_unix - secs;
        let days = t.div_euclid(86400);
        let rem = t.rem_euclid(86400);
        let (y, m, d) = bagholder_model::dates::from_days(days);
        return format!("{:04}-{:02}-{:02}T{:02}:{:02}:00Z", y, m, d, rem / 3600, (rem % 3600) / 60);
    }
    match parse_created(&field_s(row, "created")) {
        Some((y, m, d)) => format!("{:04}-{:02}-{:02}T00:00:00Z", y, m, d),
        None => String::new(),
    }
}

/// `news._AGO`: `(\d+)\s+(minute|hour|day)s?\s+ago`, case-insensitive, leftmost
/// match, which is what Python's `search` finds.
fn parse_ago(text: &str) -> Option<(i64, &'static str)> {
    let b = text.as_bytes();
    for start in 0..b.len() {
        if !b[start].is_ascii_digit() {
            continue;
        }
        if start > 0 && b[start - 1].is_ascii_digit() {
            continue;
        }
        let mut i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let n: i64 = match text[start..i].parse() { Ok(v) => v, Err(_) => continue };
        let j = skip_space(b, i);
        if j == i {
            continue;
        }
        let rest = &text[j..];
        let unit = ["minute", "hour", "day"]
            .into_iter()
            .find(|u| rest.len() >= u.len() && rest[..u.len()].eq_ignore_ascii_case(u));
        let unit = match unit { Some(u) => u, None => continue };
        let mut k = j + unit.len();
        if k < b.len() && (b[k] == b's' || b[k] == b'S') {
            k += 1;
        }
        let m = skip_space(b, k);
        if m == k {
            continue;
        }
        if text[m..].len() >= 3 && text[m..m + 3].eq_ignore_ascii_case("ago") {
            return Some((n, unit));
        }
    }
    None
}

fn skip_space(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `%b %d, %Y`, which is what Nasdaq's `created` is.
fn parse_created(text: &str) -> Option<(i64, u32, u32)> {
    let t = text.trim();
    let (mon, rest) = t.split_once(' ')?;
    let m = MONTHS.iter().position(|x| *x == mon)? as u32 + 1;
    let (day, year) = rest.split_once(',')?;
    Some((year.trim().parse().ok()?, m, day.trim().parse().ok()?))
}

/// `news.parse_nasdaq_news`.
///
/// Nasdaq pads a symbol's feed with market-wide pieces; an item is kept only
/// when the symbol is among the ones Nasdaq itself lists for it. A feed asked
/// without a symbol keeps all of them. An item's kind is the feed's when it
/// has one, else what its publisher says; a release Nasdaq names no wire for
/// reads as Nasdaq's.
pub fn parse_nasdaq_news(data: &Value, now_unix: i64, symbol: &str, kind: Option<&str>) -> Vec<Value> {
    let want = symbol.trim().to_lowercase();
    let items = data
        .get("data")
        .and_then(|d| d.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut rows = Vec::new();
    for it in items {
        let id = field_s(&it, "id");
        let title = field_s(&it, "title");
        if !it.is_object() || id.is_empty() || title.is_empty() {
            continue;
        }
        if !want.is_empty() {
            let mut named: Vec<String> = it
                .get("related_symbols")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|x| bagholder_model::value::s(Some(x)).split('|').next().unwrap_or("").trim().to_lowercase())
                        .collect()
                })
                .unwrap_or_default();
            named.push(field_s(&it, "primarysymbol").trim().to_lowercase());
            if !named.contains(&want) {
                continue;
            }
        }
        let when = nasdaq_when(&it, now_unix);
        if when.is_empty() {
            continue;
        }
        let url = field_s(&it, "url");
        let publisher = clean_text(&field_s(&it, "publisher"));
        let source = if !publisher.is_empty() {
            publisher
        } else if kind == Some("release") {
            "Nasdaq".to_string()
        } else {
            String::new()
        };
        rows.push(json!({
            "id": format!("nasdaq:{}", id),
            "headline": clean_text(&title),
            "source": source.clone(),
            "url": if url.starts_with("http") { url } else { format!("https://www.nasdaq.com{}", url) },
            "publishedAt": when,
            "kind": kind.unwrap_or_else(|| kind_of(&source)),
        }));
    }
    rows
}

/// `news.source_for`: which wire answers for a listing.
pub fn source_for(symbol: &str, exchange: &str, currency: &str) -> String {
    if symbol == MARKET.0 && exchange.to_uppercase() == MARKET.1 {
        return "nasdaq".into();
    }
    match bagholder_model::venues::tmx_form(exchange, currency) {
        Some(":US") => "nasdaq".into(),
        None => String::new(),
        Some(_) => "tmx".into(),
    }
}

/// `news.fetch_symbol`: the latest items for one listing from its wire.
///
/// `None` means the wire failed and what is stored should stand; an empty list
/// means it answered with nothing.
pub fn fetch_symbol(
    conn: &rusqlite::Connection,
    symbol: &str,
    exchange: &str,
    currency: &str,
    today: &str,
    now_unix: i64,
) -> (String, Option<Vec<Value>>) {
    let mut src = source_for(symbol, exchange, currency);
    let sym = bagholder_model::venues::tmx_symbol(symbol);
    if sym.is_empty() {
        return (src, Some(vec![]));
    }
    if src.is_empty() {
        // A ticker asked for with no venue at all is an ambiguous name: TMX's
        // news answers on the bare ticker whatever venue it is asked under, so
        // `F` there is a Canadian company's halt notice and not Ford's
        // releases. Only Nasdaq is asked, whose items name the symbols they
        // belong to and are kept only when this one is among them.
        src = "nasdaq".into();
    }

    if symbol == MARKET.0 {
        pace("api.nasdaq.com");
        let url = NASDAQ_LATEST_URL.replace("{}", &PER_MARKET.to_string());
        let text = match get_text(&url, &nasdaq_headers()) { Ok(t) => t, Err(_) => return (src, None) };
        let data: Value = match serde_json::from_str(&text) { Ok(d) => d, Err(_) => return (src, None) };
        return (src, Some(parse_nasdaq_news(&data, now_unix, "", None)));
    }

    if src == "tmx" {
        // TMX names a listing by its venue and the news query answers nothing
        // under the wrong name, so it goes through the same lookup the quote
        // does and is remembered once.
        let code = match crate::quotes::tmx_quote_symbol(symbol, exchange, currency) {
            Some(c) => c,
            None => return (src, Some(vec![])),
        };
        let ask = |form: &str| -> Option<Value> {
            pace("app-money.tmx.com");
            let payload = json!({
                "operationName": "getNewsForSymbol",
                "variables": {"symbol": form, "page": 1, "limit": PER_SYMBOL, "locale": "en"},
                "query": TMX_NEWS_QUERY,
            });
            let data = post_json(crate::tmx::TMX_URL, &payload, &tmx_headers()).ok()?;
            let rows = parse_tmx_news(&data, form);
            if rows.is_empty() { None } else { Some(Value::Array(rows)) }
        };
        let got = crate::tmx::tmx_lookup(conn, &code, today, ask).0;
        return (src, Some(got.and_then(|v| v.as_array().cloned()).unwrap_or_default()));
    }

    pace("api.nasdaq.com");
    let url = NASDAQ_NEWS_URL.replacen("{}", &sym, 1).replacen("{}", &PER_SYMBOL.to_string(), 1);
    let text = match get_text(&url, &nasdaq_headers()) { Ok(t) => t, Err(_) => return (src, None) };
    let data: Value = match serde_json::from_str(&text) { Ok(d) => d, Err(_) => return (src, None) };
    let mut rows = parse_nasdaq_news(&data, now_unix, &sym, None);

    // the listing's own releases come on a feed of their own; each once,
    // beside the stories
    pace("api.nasdaq.com");
    let purl = NASDAQ_PRESS_URL.replacen("{}", &sym, 1).replacen("{}", &PER_SYMBOL.to_string(), 1);
    if let Ok(ptext) = get_text(&purl, &nasdaq_headers()) {
        if let Ok(pdata) = serde_json::from_str::<Value>(&ptext) {
            let seen: Vec<String> = rows.iter().map(|r| field_s(r, "id")).collect();
            for r in parse_nasdaq_news(&pdata, now_unix, &sym, Some("release")) {
                if !seen.contains(&field_s(&r, "id")) {
                    rows.push(r);
                }
            }
        }
    }
    (src, Some(rows))
}

/// `news.stale`: the listings whose news is older than the freshness window.
pub fn stale(
    conn: &rusqlite::Connection,
    listings: &[(String, String, String)],
    now_unix: f64,
    minutes: f64,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let fetched = bagholder_store::feeds::news_fetched_at(conn)?;
    let mut out = Vec::new();
    for (symbol, exchange, currency) in listings {
        let key = bagholder_store::feeds::news_key(symbol, exchange);
        let last = fetched.get(&key).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let fresh = match crate::quotes::instant_secs_public(&last) {
            Some(then) => (now_unix - then) <= minutes * 60.0,
            None => false,
        };
        if !fresh {
            out.push((symbol.clone(), exchange.clone(), currency.clone()));
        }
    }
    Ok(out)
}

/// `news.refresh`: read the wire for every stale listing; each answer replaces
/// that listing's rows. Returns how many answered. `on_new` is handed
/// everything the wire answered with and the ids the listing did not have
/// before; what is worth telling about is the notifier's to decide.
pub fn refresh(
    conn: &rusqlite::Connection,
    listings: &[(String, String, String)],
    today: &str,
    now_unix: f64,
    now_stamp: &str,
    mut on_new: Option<&mut dyn FnMut(&str, &str, &[Value], &[String])>,
) -> rusqlite::Result<usize> {
    let mut done = 0usize;
    for (symbol, exchange, currency) in stale(conn, listings, now_unix, FRESH_MINUTES)? {
        let (src, rows) = fetch_symbol(conn, &symbol, &exchange, &currency, today, now_unix as i64);
        let rows = match rows { Some(r) => r, None => continue };
        let before = match on_new { Some(_) => bagholder_store::feeds::news_ids(conn, &symbol, &exchange)?, None => vec![] };
        bagholder_store::feeds::replace_news(conn, &symbol, &exchange, &src, &rows, now_stamp)?;
        if let Some(f) = on_new.as_deref_mut() {
            let fresh: Vec<String> = rows
                .iter()
                .map(|r| field_s(r, "id"))
                .filter(|id| !before.contains(id))
                .collect();
            f(&symbol, &exchange, &rows, &fresh);
        }
        done += 1;
    }
    if done > 0 {
        bagholder_store::feeds::trim_news(conn, KEEP)?;
    }
    Ok(done)
}
