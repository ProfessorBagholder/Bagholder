//! News for the symbols the book holds and watches, from every public
//! per-symbol source that proved to carry it, merged: TMX Money for Canadian
//! listings (its press releases and its "In The Media" stories) and Nasdaq for
//! US ones, and beside them Yahoo Finance's news gateway, Seeking Alpha's
//! per-symbol feed and Google News. No one source carries everything -- TMX
//! has no stories at all for some of the funds a book holds, where Google does
//! -- so a listing's news is the union of what each of them has for it, one row
//! per story however many carry it.
//!
//! An item belongs to a listing only when its source says so: TMX's press
//! releases are the ones it files under the listing, and its In The Media
//! stories, Nasdaq, Yahoo and Seeking Alpha tag every item with tickers, an
//! item kept only where those tags name the listing. Google News tags nothing,
//! so an item from it is kept only when its own headline names the listing --
//! the listing's name, or its ticker in an exchange's own form -- never because
//! a search returned it.

use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use bagholder_model::unichars::is_space;
pub use bagholder_sources::html::unescape;
use bagholder_model::value::{field_s, s as py_s};
use bagholder_model::venues::{tmx_form, tmx_symbol};
use bagholder_store::feeds::{self as sf, Feed, NewsItem, NewsKind};

/// TMX's quote page has two news tabs on one query: "Press Releases", the
/// companies' own wire items, and "In The Media", publishers' stories about
/// the company (companyInNews). Both are read.
const TMX_NEWS_QUERY: &str = "query getNewsForSymbol($symbol: String!, $page: Int!, $limit: Int!, $locale: String!, $companyInNews: Boolean) { news: getNewsForSymbol(symbol: $symbol, page: $page, limit: $limit, locale: $locale, companyInNews: $companyInNews) { headline datetime source newsid summary topic } }";

pub const TMX_NEWS_URL: &str = "https://money.tmx.com/en/quote/{}/news/{}";
pub const NASDAQ_NEWS_URL: &str = "https://api.nasdaq.com/api/news/topic/articlebysymbol?q={}|STOCKS&offset=0&limit={}";
pub const NASDAQ_LATEST_URL: &str = "https://api.nasdaq.com/api/news/topic/latestnews?offset=0&limit={}";
/// A US listing's own releases, beside its news.
pub const NASDAQ_PRESS_URL: &str = "https://api.nasdaq.com/api/news/topic/press_release?q=symbol:{}|assetclass:stocks&offset=0&limit={}";

pub const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";

pub const PER_SYMBOL: i64 = 12;
/// The newest items a listing keeps once every source is merged.
pub const PER_LISTING: usize = 60;
/// The market-wide feed is a listing of its own: Nasdaq's latest news,
/// whatever it names.
pub const MARKET: (&str, &str, &str) = ("*", "MARKET", "");
pub const PER_MARKET: i64 = 50;
pub const FRESH_MINUTES: i64 = 15;
/// Items kept in the database, newest first: every listing's merged sources.
pub const KEEP: i64 = 4000;
/// Listings read side by side in a pass; each host stays paced across all of them.
pub const LISTINGS_AT_ONCE: usize = 4;

pub const YAHOO_GATEWAY: &str = "https://nexus-gateway-prod.media.yahoo.com/";
/// The query Yahoo's own quote page sends for a ticker's News tab, unchanged.
pub const YAHOO_NEWS_QUERY: &str = "query FinancePolarisTickerNews($listInput:LightyearListInput!,$clientContext:ClientContext!,$mlRecsInput:MLRecsInput!,$gqlContext:[GqlContext]=[],$imageResize:[ImageResizeInput!]!=[],$first:Int,$mlRecsFirst:Int,$after:String,$offset:Int){lightyearList(list_input:$listInput,cc:$clientContext,first:$first){...HydratedLightyearListStoryVideoStreamPolarisWithPagination}}\nfragment ResizedResolutions on ImageResized{url height width transformLabel}\nfragment Image on Image{type:imgType originalUrl:url originalHeight:height originalWidth:width resolutions:resized(resizeInput:$imageResize){...ResizedResolutions}}\nfragment ContentAttributes on ContentAttributes{description summary pubDate:publishTime displayTime isHosted canonicalUrl clickthroughUrl(cc:$clientContext) provider{displayName url providerContentUrl providerId} thumbnail{...Image} mabMeta{mabLogString}}\nfragment FinanceStockTickers on Finance{stockTickers{symbol}}\nfragment StoryData on Story{id:uuid __typename title previewUrl(cc:$clientContext) isPremiumNews isLiveBlog embeddedLiveBlog{status} contentAttributes{...ContentAttributes} finance{...FinanceStockTickers}}\nfragment VideoData on Video{id:uuid __typename title duration previewUrl(cc:$clientContext) liveEventInfo{scheduledStartTime scheduledStopTime status} contentAttributes{...ContentAttributes} finance{...FinanceStockTickers}}\nfragment OutlinkData on Outlink{__typename uuid description displayTime headline url provider{displayName url providerContentUrl providerId} contentAttributes{thumbnail{...Image}}}\nfragment HydratedAssetRefStoryOrVideo on AssetRef{__typename asset(gqlContext:$gqlContext){__typename ... on Story{...StoryData} ... on Video{...VideoData} ... on Outlink{...OutlinkData}}}\nfragment HydratedLightyearListStoryVideoStreamPolarisWithPagination on LightyearList{main:mlRecsStream(mlRecsInput:$mlRecsInput,first:$mlRecsFirst,after:$after,offset:$offset){edges{node{...HydratedAssetRefStoryOrVideo}} pagination:pageInfo{nextPage:hasNextPage endCursor} totalCount}}";
pub const YAHOO_HEADERS: [(&str, &str); 3] = [
    ("x-yahoo-cg-client-name", "finance"),
    ("Origin", "https://finance.yahoo.com"),
    ("Referer", "https://finance.yahoo.com/"),
];
pub const SA_NEWS_URL: &str = "https://seekingalpha.com/api/sa/combined/{}.xml";
pub const GNEWS_URL: &str = "https://news.google.com/rss/search?q={}&hl=en-CA&gl=CA&ceid=CA:en";
pub const FEED_HEADERS: [(&str, &str); 2] = [
    ("User-Agent", UA),
    ("Accept", "application/rss+xml, application/xml, text/xml, */*"),
];

pub fn nasdaq_headers() -> [(&'static str, &'static str); 4] {
    [
        ("User-Agent", UA),
        ("Accept", "application/json, text/plain, */*"),
        ("Origin", "https://www.nasdaq.com"),
        ("Referer", "https://www.nasdaq.com/"),
    ]
}

/// TMX Money's own page, not the quote client's.
pub fn tmx_headers() -> [(&'static str, &'static str); 4] {
    [
        ("User-Agent", UA),
        ("locale", "en"),
        ("Origin", "https://money.tmx.com"),
        ("Referer", "https://money.tmx.com/"),
    ]
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

// ---------------------------------------------------------------------------
// the network, and the clock
// ---------------------------------------------------------------------------

/// A request that did not answer: the status when there was one.
#[derive(Debug, Clone)]
pub struct NetError {
    pub code: Option<u16>,
    pub text: String,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

pub type GetFn<'a> = dyn Fn(&str, &[(&str, &str)]) -> Result<String, NetError> + Sync + 'a;
pub type PostFn<'a> = dyn Fn(&str, &Value, &[(&str, &str)]) -> Result<Value, NetError> + Sync + 'a;

/// The requests a read makes, so a test can answer them.
pub struct Net<'a> {
    pub get: &'a GetFn<'a>,
    pub post: &'a PostFn<'a>,
    /// Whether hosts are paced; a test's answers are not.
    pub pace: bool,
}

fn net_error(e: crate::http::FetchError) -> NetError {
    NetError { code: e.code(), text: crate::http::describe_failure(&e) }
}

fn live_get(url: &str, headers: &[(&str, &str)]) -> Result<String, NetError> {
    crate::http::get_text(url, headers).map_err(net_error)
}

fn live_post(url: &str, payload: &Value, headers: &[(&str, &str)]) -> Result<Value, NetError> {
    crate::http::post_json(url, payload, headers).map_err(net_error)
}

/// The internet.
pub const LIVE: Net<'static> = Net { get: &live_get, post: &live_post, pace: true };

/// Now, and the day it is.
#[derive(Debug, Clone)]
pub struct Clock {
    pub today: String,
    pub now: i64,
}

impl Clock {
    pub fn at(now: i64) -> Clock {
        let (y, m, d) = bagholder_model::dates::from_days(now.div_euclid(86400));
        Clock { today: bagholder_model::dates::fmt(y, m, d), now }
    }
    pub fn stamp(&self) -> String {
        stamp_of(self.now)
    }
}

fn log(line: &str) {
    eprintln!("{}", line);
}

/// Wait for this host's next turn. Turns are handed out under a lock, so
/// listings read side by side still ask each host one at a time, `seconds`
/// apart.
pub fn pace(host: &str, seconds: f64) {
    bagholder_net::machine::turn(host, std::time::Duration::from_secs_f64(seconds.max(0.0)));
}

fn paced(net: &Net, host: &str, seconds: f64) {
    if net.pace {
        pace(host, seconds);
    }
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// What an item is, told by where it came from: a wire carries the company's
/// own release, a publisher writes a story about it.
///
/// A release wire, by its name: GlobeNewswire, Business Wire, PR Newswire,
/// ACCESS Newswire (Accesswire), TheNewsWire, Canada Newswire (CNW), TMX
/// Newsfile, Marketwired, NewMediaWire, Cision, PRWeb. A newsroom whose name
/// only contains the letters is a publisher: WIRED, and the plural news
/// services, MT Newswires and Dow Jones Newswires, write stories.
pub fn kind_of(source: &str) -> NewsKind {
    static WIRES: OnceLock<Regex> = OnceLock::new();
    let wires = re(&WIRES, r"(?i)business ?wire|accesswire|newmediawire|marketwired|newsfile|cision|\bcnw\b|prweb");
    if wires.is_match(source) {
        return NewsKind::Release;
    }
    // `newswire(?!s)`
    let low: Vec<char> = source.chars().flat_map(|c| c.to_lowercase()).collect();
    let word: Vec<char> = "newswire".chars().collect();
    for i in 0..low.len() {
        if low[i..].starts_with(&word) && low.get(i + word.len()) != Some(&'s') {
            return NewsKind::Release;
        }
    }
    NewsKind::Story
}

/// The HTML entities resolved and the whitespace collapsed.
pub fn clean_text(t: &str) -> String {
    unescape(t).split(is_space).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `SUMMARY_CHARS`: a sentence or two of what a source said beneath its
/// headline.
pub const SUMMARY_CHARS: usize = 400;

// What a wire puts before its first sentence: it names the wire and the day, which the row already
// carries, so it is taken off rather than shown as the summary's opening.
const WIRES: &str = r"globe ?newswire|business ?wire|cnw(?: group)?|newsfile(?: corp)?|accesswire|the ?news ?wire|pr ?newswire|newmediawire|marketwired|cision";

/// A wire's dateline off the front of its own summary, however many times it
/// prints one, and the bare date some leave behind (`le 8 septembre 2026 – `).
pub fn strip_dateline(text: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    static D: OnceLock<Regex> = OnceLock::new();
    let a = re(&A, &format!(r"(?i)^.{{0,80}}?(?:\((?:{w})[^)]*\)|(?:{w}))[^.]{{0,60}}?[-–—]{{1,2}}\s+", w = WIRES));
    let b = re(&B, &format!(r"(?i)^[^./]{{0,60}}/\s*(?:{w})\s*/[^./]{{0,40}}/\s*", w = WIRES));
    let d = re(&D, r"(?i)^(?:le\s+)?\d{1,2}(?:er)?\s+[a-zéû]{3,10}\.?\s+\d{4}\s*[-–—,]?\s+|^[A-Za-zéû]{3,10}\.?\s+\d{1,2},?\s+\d{4}\s*[-–—,]?\s+");
    let trim = |x: &str| x.trim_start_matches([' ', '-', '–', '—', ',', '/']).to_string();
    let mut text = text.to_string();
    for _ in 0..3 {
        let mut cut = a.replacen(&text, 1, "").to_string();
        cut = b.replacen(&cut, 1, "").to_string();
        cut = trim(&d.replacen(&trim(&cut), 1, ""));
        if cut == text {
            return text;
        }
        text = cut;
    }
    text
}

/// What a source said under the headline, as plain text: tags out, one space
/// between words, cut at a sentence end rather than mid-word.
///
/// A summary that only repeats the headline is not one, and neither is a
/// feed's markup (Google's `description` is an anchor and a publisher).
pub fn summary_text(raw: &str, headline: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    let text = strip_dateline(&clean_text(&re(&TAGS, r"<[^>]+>").replace_all(raw, " ")));
    // a source that carries a placeholder instead of a summary ("...", "-", "N/A") has none
    if text.chars().filter(|c| c.is_alphanumeric()).count() < 12 {
        return String::new();
    }
    let (tk, hk) = (news_text(&text), news_text(headline));
    if text.is_empty() || tk == hk || (tk.starts_with(&hk) && text.chars().count() < headline.chars().count() + 12) {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= SUMMARY_CHARS {
        return text;
    }
    let cut: String = chars[..SUMMARY_CHARS].iter().collect();
    let stop = [". ", "? ", "! "].iter().filter_map(|p| cut.rfind(p)).max();
    match stop {
        // rfind is a byte offset; the sentence end has to sit past the halfway mark of the cut
        Some(i) if cut[..i].chars().count() > SUMMARY_CHARS / 2 => cut[..i + 1].trim().to_string(),
        _ => format!("{}…", cut.trim_end()),
    }
}

// ---------------------------------------------------------------------------
// times
// ---------------------------------------------------------------------------

fn digits(t: &str) -> Option<i64> {
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

/// An ISO date or instant as seconds since the epoch and the year it names,
/// read the way `datetime.fromisoformat` reads one: a date alone is midnight,
/// a time with no offset is UTC.
fn iso_unix(text: &str) -> Option<(i64, i64)> {
    let t = text.trim();
    if !t.is_ascii() {
        return None;
    }
    let (y, m, d, rest) = if t.len() >= 10 && t.as_bytes()[4] == b'-' {
        (digits(&t[0..4])?, digits(&t[5..7])?, digits(&t[8..10])?, &t[10..])
    } else if t.len() >= 8 {
        (digits(&t[0..4])?, digits(&t[4..6])?, digits(&t[6..8])?, &t[8..])
    } else {
        return None;
    };
    if t.len() >= 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] != b'-' {
        return None;
    }
    if !(1..=12).contains(&m) || d < 1 || d > bagholder_model::dates::days_in_month(y, m as u32) as i64 {
        return None;
    }
    let mut secs = 0i64;
    let mut offset = 0i64;
    if !rest.is_empty() {
        let clock = &rest[1..];
        let (clock, zone) = match clock.find(['Z', '+', '-']) {
            Some(p) => (&clock[..p], &clock[p..]),
            None => (clock, ""),
        };
        let clock = clock.split('.').next().unwrap_or("");
        let parts: Vec<&str> = if clock.contains(':') {
            clock.split(':').collect()
        } else {
            clock.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap_or("")).collect()
        };
        if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.len() != 2) {
            return None;
        }
        let hh = digits(parts[0])?;
        let mm = if parts.len() > 1 { digits(parts[1])? } else { 0 };
        let ss = if parts.len() > 2 { digits(parts[2])? } else { 0 };
        if hh > 23 || mm > 59 || ss > 59 {
            return None;
        }
        secs = hh * 3600 + mm * 60 + ss;
        if zone == "Z" {
            offset = 0;
        } else if !zone.is_empty() {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let z = zone[1..].replace(':', "");
            let z = z.split('.').next().unwrap_or("");
            let oh = digits(z.get(0..2)?)?;
            let om = if z.len() >= 4 { digits(&z[2..4])? } else if z.len() == 2 { 0 } else { return None };
            offset = sign * (oh * 3600 + om * 60);
        }
    }
    Some((bagholder_model::dates::to_days(y, m as u32, d as u32) * 86400 + secs - offset, y))
}

/// An ISO instant in UTC, the way the app writes times.
fn utc_stamp(text: &str) -> String {
    let s = text.trim();
    let norm = if s.ends_with('Z') { format!("{}+00:00", &s[..s.len() - 1]) } else { s.to_string() };
    iso_unix(&norm).map(|(u, _)| stamp_of(u)).unwrap_or_default()
}

fn stamp_of(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let rem = unix.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// An RFC 822 date, as a feed writes one: `Fri, 05 Sep 2026 10:00:00 -0400`.
fn rfc822_unix(text: &str) -> Option<(i64, i64)> {
    let mut t = text.trim();
    if let Some(p) = t.find(',') {
        t = &t[p + 1..];
    }
    let parts: Vec<&str> = t.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    let (mut dd, mut mon) = (parts[0], parts[1]);
    if MONTHS.iter().all(|x| !x.eq_ignore_ascii_case(&mon[..mon.len().min(3)])) {
        std::mem::swap(&mut dd, &mut mon);
    }
    let m = MONTHS.iter().position(|x| mon.len() >= 3 && x.eq_ignore_ascii_case(&mon[..3]))? as u32 + 1;
    let d = digits(dd)?;
    let mut y = digits(parts[2])?;
    if parts[2].len() <= 2 {
        y += if y > 68 { 1900 } else { 2000 };
    }
    let hms: Vec<&str> = parts[3].split(':').collect();
    let hh = digits(hms.first()?)?;
    let mm = hms.get(1).map(|x| digits(x)).unwrap_or(Some(0))?;
    let ss = hms.get(2).map(|x| digits(x)).unwrap_or(Some(0))?;
    if d < 1 || d > bagholder_model::dates::days_in_month(y, m) as i64 || hh > 23 || mm > 59 || ss > 61 {
        return None;
    }
    let zone = parts.get(4).copied().unwrap_or("").to_uppercase();
    let offset = match zone.as_str() {
        "EST" => -5 * 3600, "EDT" => -4 * 3600, "CST" => -6 * 3600, "CDT" => -5 * 3600,
        "MST" => -7 * 3600, "MDT" => -6 * 3600, "PST" => -8 * 3600, "PDT" => -7 * 3600,
        z if (z.starts_with('+') || z.starts_with('-')) && z.len() == 5 && digits(&z[1..]).is_some() => {
            let n = digits(&z[1..]).unwrap();
            (if z.starts_with('-') { -1 } else { 1 }) * ((n / 100) * 3600 + (n % 100) * 60)
        }
        _ => 0,
    };
    Some((bagholder_model::dates::to_days(y, m, d as u32) * 86400 + hh * 3600 + mm * 60 + ss - offset, y))
}

/// A feed's date -- RFC 822 or ISO -- in the app's own form; "" when it is not
/// a real date (a quote page Google dates 1970).
pub fn iso(value: &str) -> String {
    let text = value.trim();
    if text.is_empty() {
        return String::new();
    }
    let got = if text.len() >= 4 && text.as_bytes()[..4].iter().all(|b| b.is_ascii_digit()) {
        iso_unix(&text.replace('Z', "+00:00"))
    } else {
        rfc822_unix(text)
    };
    match got {
        Some((u, y)) if y >= 2000 => stamp_of(u),
        _ => String::new(),
    }
}

fn instant(text: &str) -> Option<i64> {
    iso_unix(&text.replace('Z', "+00:00")).map(|(u, _)| u)
}

/// One element's text, its CDATA unwrapped.
fn tag(xml: &str, name: &str) -> String {
    let pat = format!(r"(?s)<{}[^>]*>(.*?)</{}>", regex::escape(name), regex::escape(name));
    let m = match Regex::new(&pat).ok().and_then(|r| r.captures(xml).map(|c| c.get(1).map(|g| g.as_str().to_string()).unwrap_or_default())) {
        Some(m) => m,
        None => return String::new(),
    };
    static CDATA: OnceLock<Regex> = OnceLock::new();
    let cdata = re(&CDATA, r"(?s)^\s*<!\[CDATA\[(.*?)\]\]>\s*$");
    match cdata.captures(&m) {
        Some(c) => clean_text(c.get(1).map(|g| g.as_str()).unwrap_or("")),
        None => clean_text(&m),
    }
}

fn all_between(text: &str, open: &str, close: &str) -> Vec<String> {
    let pat = format!(r"(?s){}(.*?){}", regex::escape(open), regex::escape(close));
    Regex::new(&pat).unwrap().captures_iter(text).map(|c| c[1].to_string()).collect()
}

fn sha16(text: &str) -> String {
    let d = openssl::sha::sha1(text.as_bytes());
    d.iter().map(|b| format!("{:02x}", b)).collect::<String>()[..16].to_string()
}

/// `urllib.parse.quote`: everything but letters, digits, `_.-~` and `safe`
/// percent-encoded.
fn quote(text: &str, safe: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// TMX
// ---------------------------------------------------------------------------

/// Whether TMX's own topic codes on an item name the listing, asked in the
/// venue's form: a Canadian listing's code carries its market (`PNG:CA`,
/// `HG:CNX`, `HBIX:AQL`) and a US listing's is the bare ticker (`ASTS`), so
/// Telus (`T`) is never named by AT&T's `T`, nor HydroGraph (`HG`) by the
/// NYSE's. The codes are TMX's tag, not a reading of the headline.
pub fn tmx_names(topic: &str, symbol: &str) -> bool {
    let up = symbol.trim().to_uppercase();
    let (bare, suffix) = match up.split_once(':') { Some((b, s)) => (b.to_string(), s.to_string()), None => (up.clone(), String::new()) };
    if bare.is_empty() {
        return false;
    }
    let us = suffix == "US";
    for code in topic.trim().trim_matches(|c| c == '[' || c == ']').split(',') {
        let code = code.trim().to_uppercase();
        let (head, market) = code.split_once(':').unwrap_or((code.as_str(), ""));
        if head == bare && ((us && (market.is_empty() || market == "US")) || (!us && ["CA", "CNX", "AQL"].contains(&market))) {
            return true;
        }
    }
    false
}

/// TMX's items for a symbol (in the venue's form, as `tmx_quote_symbol`
/// gives it) into news rows: the headline, its exact time, the wire it came
/// on, and TMX's page for it. `media` is the In The Media tab: publishers'
/// stories, each kept only where TMX's own topic codes name the listing,
/// since a story is about a company only as TMX tags it.
pub fn parse_tmx_news(data: &Value, symbol: &str, media: bool) -> Vec<NewsItem> {
    let items = data.get("data").and_then(|d| d.get("news")).and_then(|n| n.as_array()).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    for it in items {
        let newsid = field_s(&it, "newsid");
        if !it.is_object() || newsid.is_empty() {
            continue;
        }
        if media && !tmx_names(&field_s(&it, "topic"), symbol) {
            continue;
        }
        let ts = utc_stamp(&field_s(&it, "datetime"));
        if ts.is_empty() {
            continue;
        }
        let source = clean_text(&field_s(&it, "source")).replace(" via QuoteMedia", "");
        rows.push(NewsItem {
            id: format!("tmx:{}", newsid),
            headline: clean_text(&field_s(&it, "headline")),
            source: source.clone(),
            url: TMX_NEWS_URL.replacen("{}", symbol, 1).replacen("{}", &newsid, 1),
            published_at: ts,
            summary: summary_text(&field_s(&it, "summary"), &field_s(&it, "headline")),
            kind: if media { NewsKind::Story } else { kind_of(&source) },
            via: if media { Feed::TmxMedia } else { Feed::Tmx },
        });
    }
    rows
}

// ---------------------------------------------------------------------------
// Nasdaq
// ---------------------------------------------------------------------------

/// Nasdaq gives a day and an age. The time is the age
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

/// `(\d+)\s+(minute|hour|day)s?\s+ago`, case-insensitive, leftmost
/// match.
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


/// `%b %d, %Y`, which is what Nasdaq's `created` is.
fn parse_created(text: &str) -> Option<(i64, u32, u32)> {
    let t = text.trim();
    let (mon, rest) = t.split_once(' ')?;
    let m = MONTHS.iter().position(|x| *x == mon)? as u32 + 1;
    let (day, year) = rest.split_once(',')?;
    Some((year.trim().parse().ok()?, m, day.trim().parse().ok()?))
}

///
/// Nasdaq pads a symbol's feed with market-wide pieces; an item is kept only
/// when the symbol is among the ones Nasdaq itself lists for it. A feed asked
/// without a symbol keeps all of them. An item's kind is the feed's when it
/// has one, else what its publisher says; a release Nasdaq names no wire for
/// reads as Nasdaq's.
pub fn parse_nasdaq_news(data: &Value, now_unix: i64, symbol: &str, kind: Option<NewsKind>) -> Vec<NewsItem> {
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
        } else if kind == Some(NewsKind::Release) {
            "Nasdaq".to_string()
        } else {
            String::new()
        };
        rows.push(NewsItem {
            id: format!("nasdaq:{}", id),
            headline: clean_text(&title),
            source: source.clone(),
            url: if url.starts_with("http") { url } else { format!("https://www.nasdaq.com{}", url) },
            published_at: when,
            summary: String::new(),
            kind: kind.unwrap_or_else(|| kind_of(&source)),
            via: Feed::Nasdaq,
        });
    }
    rows
}

// ---------------------------------------------------------------------------
// Yahoo Finance: the news gateway behind a quote page's News tab
// ---------------------------------------------------------------------------

/// The ticker the gateway tags items with: `PNG.V`, `SXHI.TO`, `HG.CN`,
/// `HBIX.NE`, or `ASTS`, as every other Yahoo read names the listing; a
/// Canadian listing with no venue takes the one TMX answered to.
pub fn yahoo_form(conn: &Connection, symbol: &str, exchange: &str, currency: &str) -> String {
    if tmx_form(exchange, currency).is_none() {
        return String::new();
    }
    let forms = crate::quotes::yahoo_forms(&bagholder_model::input::Listing::new(symbol, exchange, currency, ""));
    if let Some(first) = forms.first() {
        if exchange.trim().is_empty() && first.ends_with(".TO") {
            let remembered = crate::tmx::tmx_remembered(conn, &tmx_symbol(symbol));
            let bare = crate::tmx::tmx_bare(&remembered);
            let suffix = match &remembered[bare.len()..] { ":CNX" => Some(".CN"), ":AQL" => Some(".NE"), _ => None };
            if let Some(sfx) = suffix {
                return format!("{}{}", crate::quotes::yahoo_root(symbol), sfx);
            }
        }
    }
    forms.first().map(|f| f.to_uppercase()).unwrap_or_default()
}

fn otc_twin(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 5 && b[..4].iter().all(|c| c.is_ascii_uppercase()) && (b[4] == b'F' || b[4] == b'Y')
}

/// The gateway's items for a ticker, kept where Yahoo's own ticker tags name
/// it. A Canadian company's items are often tagged only with its US
/// over-the-counter twin (`CHHYF` for `CH.V`): a twin Yahoo tags alone beside
/// the listing's own ticker on an item names it too, never a partner's symbol
/// on an item that names several. An item Yahoo tags with nothing is kept when
/// its headline names the listing.
pub fn parse_yahoo_news(data: &Value, form: &str, symbol: &str, name: &str) -> Vec<NewsItem> {
    let edges = data
        .pointer("/data/lightyearList/main/edges")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let mut assets: Vec<(Value, HashSet<String>)> = Vec::new();
    for edge in edges {
        let asset = edge.pointer("/node/asset").cloned().unwrap_or(Value::Null);
        if truthy(asset.get("id")) && truthy(asset.get("title")) {
            let mut tags: HashSet<String> = asset
                .pointer("/finance/stockTickers")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().filter(|t| t.is_object()).map(|t| field_s(t, "symbol").trim().to_uppercase()).collect())
                .unwrap_or_default();
            tags.remove("");
            assets.push((asset, tags));
        }
    }
    let mut twins: HashSet<String> = HashSet::new();
    if form.contains('.') {
        for (_, tags) in &assets {
            let other: Vec<&String> = tags.iter().filter(|t| t.as_str() != form).collect();
            if tags.contains(form) && other.len() == 1 && otc_twin(other[0]) {
                twins.insert(other[0].clone());
            }
        }
    }
    let us = !form.contains('.');
    let mut rows = Vec::new();
    for (asset, tags) in &assets {
        let title = clean_text(&field_s(asset, "title"));
        let sym = if symbol.is_empty() { form.split('.').next().unwrap_or("") } else { symbol };
        if !(tags.contains(form) || tags.iter().any(|t| twins.contains(t)) || (tags.is_empty() && names_listing(&title, sym, name, us))) {
            continue;
        }
        let attrs = asset.get("contentAttributes").cloned().unwrap_or(Value::Null);
        let when = iso(&py_s(attrs.get("pubDate")));
        if when.is_empty() {
            continue;
        }
        let provider = clean_text(&py_s(attrs.pointer("/provider/displayName")));
        let source = if provider.is_empty() { "Yahoo Finance".to_string() } else { provider };
        let url = if truthy(attrs.get("canonicalUrl")) { py_s(attrs.get("canonicalUrl")) } else { py_s(attrs.get("clickthroughUrl")) };
        rows.push(NewsItem {
            id: format!("yahoo:{}", field_s(asset, "id")),
            headline: title.clone(),
            source: source.clone(),
            url,
            published_at: when,
            summary: summary_text(&{ let x = py_s(attrs.get("summary")); if x.is_empty() { py_s(attrs.get("description")) } else { x } }, &title),
            kind: kind_of(&source),
            via: Feed::Yahoo,
        });
    }
    rows
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `None` when there is nothing to ask: no answer, so it never counts as the
/// listing having been read.
pub fn fetch_yahoo(net: &Net, ask: &Ask) -> Result<Option<Vec<NewsItem>>, NetError> {
    let form = &ask.yahoo;
    if form.is_empty() {
        return Ok(None);
    }
    let alias = "finance-US-en-US-ticker-all";
    let variables = json!({
        "clientContext": {"device": "DESKTOP", "region": "US", "site": "finance", "lang": "en-US"},
        "gqlContext": [{"listAlias": format!("list={}", alias)}], "imageResize": [],
        "listInput": {"disableDedupe": false, "enableBlockedContent": false, "filterClientContext": false, "getFullList": true,
                      "enableQueryTimeLicenseCheck": true, "queryVariables": {"tickerSymbol": [form]}, "slug": format!("list={}", alias)},
        "mlRecsInput": {"count": 200, "instance": "FINANCE"}, "first": 100, "mlRecsFirst": 50,
    });
    paced(net, "nexus-gateway-prod.media.yahoo.com", 1.0);
    let data = (net.post)(YAHOO_GATEWAY, &json!({"query": YAHOO_NEWS_QUERY, "operationName": "FinancePolarisTickerNews", "variables": variables}), &YAHOO_HEADERS)?;
    Ok(Some(parse_yahoo_news(&data, form, &ask.symbol, &ask.name)))
}

// ---------------------------------------------------------------------------
// Seeking Alpha: a ticker's combined feed
// ---------------------------------------------------------------------------

/// Seeking Alpha's name for a listing: `PNG:CA` for the TSX and TSX-V, the
/// bare ticker for a US one. It has no form for the CSE or Cboe Canada, which
/// it reaches only through a US OTC symbol the app does not keep, so those are
/// left to the other sources.
pub fn sa_form(symbol: &str, exchange: &str, currency: &str) -> String {
    let sym = tmx_symbol(symbol).to_uppercase();
    match tmx_form(exchange, currency) {
        Some(":US") => sym,
        Some("") => format!("{}:CA", sym),
        _ => String::new(),
    }
}

/// The feed's items, kept only where their own `sa:symbol` tags name the
/// listing.
pub fn parse_sa_news(xml: &str, form: &str) -> Vec<NewsItem> {
    let mut rows = Vec::new();
    for item in all_between(xml, "<item>", "</item>") {
        let symbols: HashSet<String> = all_between(&item, "<sa:symbol>", "</sa:symbol>").iter().map(|x| clean_text(x).to_uppercase()).collect();
        if !symbols.contains(&form.to_uppercase()) {
            continue;
        }
        let (guid, title, when) = (tag(&item, "guid"), tag(&item, "title"), iso(&tag(&item, "pubDate")));
        if guid.is_empty() || title.is_empty() || when.is_empty() {
            continue;
        }
        let link = tag(&item, "link");
        rows.push(NewsItem {
            id: format!("sa:{}", sha16(&guid)),
            headline: title.clone(),
            source: "Seeking Alpha".to_string(),
            url: if link.is_empty() { guid.clone() } else { link },
            published_at: when,
            summary: summary_text(&tag(&item, "description"), &title),
            kind: NewsKind::Story,
            via: Feed::Sa,
        });
    }
    rows
}

/// `None` when Seeking Alpha has no feed for the listing: nothing was read,
/// and nothing failed.
pub fn fetch_sa(net: &Net, symbol: &str, exchange: &str, currency: &str) -> Result<Option<Vec<NewsItem>>, NetError> {
    let form = sa_form(symbol, exchange, currency);
    if form.is_empty() {
        return Ok(None);
    }
    paced(net, "seekingalpha.com", 1.0);
    match (net.get)(&SA_NEWS_URL.replace("{}", &quote(&form, ":")), &FEED_HEADERS) {
        Ok(xml) => Ok(Some(parse_sa_news(&xml, &form))),
        Err(e) if e.code == Some(404) || e.text.contains("404") => Ok(None),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Google News: every publisher, no tags
// ---------------------------------------------------------------------------

const CORPORATE: [&str; 27] = ["inc", "incorporated", "corp", "corporation", "ltd", "limited", "plc", "co", "company", "holdings", "holding", "group",
    "nv", "sa", "ag", "se", "lp", "llc", "the", "class", "units", "unit", "shares", "common", "ordinary", "adr", "trust"];
fn corporate(w: &str) -> bool {
    CORPORATE.contains(&w)
}
const CA_VENUES: [&str; 9] = ["TSX", "TSXV", "TSX-V", "CVE", "CSE", "CNSX", "CN", "NEO", "CBOE CANADA"];
const US_VENUES: [&str; 9] = ["NASDAQ", "NYSE", "NYSEARCA", "NYSE ARCA", "NYSEAMERICAN", "NYSE AMERICAN", "AMEX", "BATS", "CBOE"];
const CA_SUFFIXES: [&str; 8] = [".TO", ".V", ".CN", ".C", ".NE", ":CA", ":CNX", ":AQL"];
/// Words that join a name's words and say nothing themselves.
const JOINERS: [&str; 12] = ["of", "and", "de", "du", "des", "la", "le", "et", "for", "on", "at", "y"];
/// Words many companies' and funds' names start with, which name none of them
/// alone: places, trades, kinds of company.
const GENERIC: &[&str] = &["canadian", "canada", "american", "america", "national", "international", "global", "general", "united", "universal",
    "northern", "southern", "eastern", "western", "northwest", "pacific", "atlantic", "arctic", "central", "british",
    "european", "chinese", "mexican", "brazil", "quebec", "ontario", "alberta", "manitoba", "california", "nevada", "arizona",
    "alaska", "texas", "frontier", "pioneer", "liberty", "patriot", "heritage", "capital", "energy", "energies", "silver",
    "golden", "digital", "quantum", "advanced", "applied", "intuitive", "precision", "premium", "select", "strategic",
    "strategy", "summit", "bright", "lithium", "uranium", "copper", "nickel", "cobalt", "graphite", "metals", "mining",
    "resources", "minerals", "petroleum", "natural", "health", "healthcare", "medical", "pharma", "therapeutics",
    "sciences", "science", "technology", "technologies", "software", "systems", "network", "networks", "solutions",
    "services", "industries", "industrial", "financial", "finance", "investment", "investments", "partners", "income",
    "dividend", "growth", "equity", "innovation", "innovative", "materials", "hydrogen", "battery", "electric", "motors",
    "aerospace", "defence", "defense", "security", "securities", "standard", "interactive", "entertainment", "communications",
    "telecom", "wireless", "insurance", "realty", "properties", "estate", "infrastructure", "renewable", "renewables",
    "environmental", "agricultural", "foods", "brands", "consumer", "retail", "bancorp", "banking", "credit", "mortgage",
    "royalty", "royalties", "exploration", "minerals", "robotics", "biotech", "semiconductor", "semiconductors", "solar"];

fn label_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"(?i)^(?:(class|series)\s+\w+(\s+(units?|shares?))?$|^(units?|shares?|etf|fund|common( shares)?)$)")
}

fn fundish_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"(?i)\b(etf|fund|portfolio|trust)\b")
}

/// The pages a quote site keeps per ticker, which Google lists beside the
/// news: a price, a chart, statements.
fn quote_page_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, concat!(r"(?i)price and chart|\b(stock|share) price\s*[,&]|share price - |holdings list|^technical analysis of|etf profile:|stock forecast and price target|",
                   r"^etfs investing in|forecast\s*[–—-]\s*price target|price prediction|tokenomics|price today: live|\brstock\b|",
                   r"\s[–—]\s(?:TSX|TSXV|CSE|NEO|NASDAQ|NYSE|AMEX|OTC)\s?:\s?[A-Z0-9.]+\s*$|^\$[^$]+\$$"))
}

/// Accents off, so `Québec` and `Quebec` are one word.
fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfkd().filter(|c| unicode_normalization::char::canonical_combining_class(*c) == 0).collect()
}

fn words(text: &str) -> Vec<String> {
    let low = fold(text).to_lowercase();
    ascii_runs(&low, |c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn ascii_runs(text: &str, keep: impl Fn(char) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if keep(c) {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn py_strip<'a>(text: &'a str, chars: &str) -> &'a str {
    text.trim_matches(|c| chars.contains(c))
}

fn py_split(text: &str) -> Vec<&str> {
    text.split(is_space).filter(|w| !w.is_empty()).collect()
}

/// A listing's name as the press writes it, from the book's record of it:
/// without what the record appends (`(the "ETF")`, `- Class A`, `- ETF`), and a
/// manager's name put in front of its fund's (`Ninepoint Partners LP - Cameco
/// Highshares ETF` is `Ninepoint Cameco Highshares ETF`).
pub fn search_name(name: &str) -> String {
    static PARENS: OnceLock<Regex> = OnceLock::new();
    static DASH: OnceLock<Regex> = OnceLock::new();
    static CLASS: OnceLock<Regex> = OnceLock::new();
    let text = re(&PARENS, r"\([^)]*\)").replace_all(name, " ").to_string();
    let parts: Vec<String> = re(&DASH, r"\s+[-–—]\s+").split(&text).map(|p| py_strip(p, " .,-").to_string()).collect();
    let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty() && !label_re().is_match(p)).collect();
    if parts.is_empty() {
        return String::new();
    }
    let fundish = fundish_re();
    let text = if parts.len() > 1 && !fundish.is_match(&parts[0]) && parts[1..].iter().any(|p| fundish.is_match(p)) {
        let fund = parts[1..].iter().find(|p| fundish.is_match(p)).unwrap().clone();
        let brand: Vec<&str> = py_split(&parts[0])
            .into_iter()
            .filter(|w| { let l = py_strip(&w.to_lowercase(), ".,").to_string(); !corporate(&l) && l != "partners" })
            .collect();
        let head = brand.first().copied().unwrap_or("");
        if head.is_empty() || fund.to_lowercase().starts_with(&head.to_lowercase()) { fund } else { format!("{} {}", head, fund) }
    } else {
        parts[0].clone()
    };
    let text = re(&CLASS, r"(?i)\s+(class|series)\s+[a-z]\b.*$").replace_all(&text, "").to_string();
    let mut ws = py_split(&text);
    while let Some(last) = ws.last() {
        let letters: String = last.to_lowercase().chars().filter(|c| c.is_ascii_lowercase()).collect();
        if corporate(&letters) {
            ws.pop();
        } else {
            break;
        }
    }
    clean_text(py_strip(&ws.join(" "), " ,"))
}

/// The words of a name that name the company: its search name less the
/// corporate ones.
fn brand(name: &str) -> Vec<String> {
    words(&search_name(name)).into_iter().filter(|w| !corporate(w)).collect()
}

enum Piece {
    Lit(String),
    Space,
}

/// Every place a sequence of literal text and optional spaces can end when it
/// starts at `at`, compared without case.
fn ends(text: &[char], at: usize, pieces: &[Piece], out: &mut Vec<usize>) {
    match pieces.first() {
        None => out.push(at),
        Some(Piece::Space) => {
            if at < text.len() && is_space(text[at]) {
                ends(text, at + 1, &pieces[1..], out);
            }
            ends(text, at, &pieces[1..], out);
        }
        Some(Piece::Lit(lit)) => {
            let mut i = at;
            for p in lit.chars() {
                if i >= text.len() || !same_letter(text[i], p) {
                    return;
                }
                i += 1;
            }
            ends(text, i, &pieces[1..], out);
        }
    }
}

fn same_letter(c: char, p: char) -> bool {
    c == p || c.eq_ignore_ascii_case(&p) || (c == 'ı' && p.eq_ignore_ascii_case(&'i'))
}

/// `[A-Za-z0-9]`, read without case as `re` reads it.
fn alnum_nocase(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == 'ı'
}

/// Whether a headline names the listing, which is the only way a Google item
/// is kept, since a search returns whatever mentions a name anywhere on a page:
/// - its ticker in a venue's own form: `TSXV:QNC`, `CNSX:HG`, `PLTE:CA`,
///   `CCHI.TO`, `(PLTE)`, `$QNC` (a Canadian venue's for a Canadian listing, a
///   US one's for a US listing);
/// - its ticker as a word in capitals, three letters or more, in a headline
///   that is not all capitals;
/// - its name as far as its second word that means something, written as a
///   name (`Quantum eMotion`, `Bank of Montreal`, the last word possibly
///   shortened: `CHAR Tech`; never `National Bank of Greece` for National Bank
///   of Canada, nor `Canadian natural gas`);
/// - its name's first word alone, six letters or more and not one many names
///   start with, capitalised in a sentence-case headline (`Why Charbone shares
///   jumped`) or, in a title-case one, opening it or styled as the company
///   styles it (`Harvest ETFs Announces`, `MDI Joins HydroGraph`).
pub fn names_listing(headline: &str, symbol: &str, name: &str, us: bool) -> bool {
    let head = fold(headline);
    let letters: Vec<char> = head.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let mostly_caps = !letters.is_empty() && (letters.iter().filter(|c| c.is_ascii_uppercase()).count() as f64) > 0.7 * letters.len() as f64;
    let sym = tmx_symbol(symbol).to_uppercase();
    let chars: Vec<char> = head.chars().collect();
    if !sym.is_empty() {
        let mut forms: Vec<Vec<Piece>> = Vec::new();
        for v in if us { &US_VENUES[..] } else { &CA_VENUES[..] } {
            let mut pieces = Vec::new();
            for (i, w) in v.split(' ').enumerate() {
                if i > 0 {
                    pieces.push(Piece::Space);
                }
                pieces.push(Piece::Lit(w.to_string()));
            }
            pieces.extend([Piece::Space, Piece::Lit(":".into()), Piece::Space, Piece::Lit(sym.clone())]);
            forms.push(pieces);
        }
        forms.push(vec![Piece::Lit(format!("({})", sym))]);
        forms.push(vec![Piece::Lit(format!("${}", sym))]);
        if !us {
            for x in CA_SUFFIXES {
                forms.push(vec![Piece::Lit(format!("{}{}", sym, x))]);
            }
        }
        for start in 0..chars.len() {
            if start > 0 && alnum_nocase(chars[start - 1]) {
                continue;
            }
            for f in &forms {
                let mut got = Vec::new();
                ends(&chars, start, f, &mut got);
                if got.iter().any(|&e| e >= chars.len() || !alnum_nocase(chars[e])) {
                    return true;
                }
            }
        }
        let symc: Vec<char> = sym.chars().collect();
        if symc.len() >= 3 && !mostly_caps {
            for start in 0..chars.len() {
                if chars[start..].starts_with(&symc)
                    && !(start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == '.' || chars[start - 1] == '$'))
                    && !(start + symc.len() < chars.len() && chars[start + symc.len()].is_ascii_alphanumeric())
                {
                    return true;
                }
            }
        }
    }
    let brand = brand(name);
    let tokens = ascii_runs(&head, |c| c.is_ascii_alphanumeric());
    let words: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
    let named = |t: &str| mostly_caps || t.chars().any(|c| c.is_ascii_uppercase());
    let joiner = |w: &str| JOINERS.contains(&w);
    let meaning: Vec<usize> = brand.iter().enumerate().filter(|(_, w)| !joiner(w)).map(|(i, _)| i).collect();
    if meaning.len() >= 2 {
        let prefix = &brand[..meaning[1] + 1];
        let n = prefix.len();
        if words.len() >= n {
            for i in 0..=(words.len() - n) {
                let chunk = &words[i..i + n];
                let last = &chunk[n - 1];
                if chunk[..n - 1] != prefix[..n - 1] || !(*last == prefix[n - 1] || (last.len() >= 4 && prefix[n - 1].starts_with(last.as_str()))) {
                    continue;
                }
                if !tokens[i..i + n].iter().filter(|t| !joiner(&t.to_lowercase())).all(|t| named(t)) {
                    continue;
                }
                let (mut j, mut k) = (i + n, n);
                while k < brand.len() && joiner(&brand[k]) && j < words.len() && words[j] == brand[k] {
                    j += 1;
                    k += 1;
                }
                if k > n && (k >= brand.len() || j >= words.len() || words[j] != brand[k]) {
                    continue;
                }
                return true;
            }
        }
    }
    if !meaning.is_empty() && meaning[0] == 0 {
        let first = &brand[0];
        if first.len() >= 6 && !first.chars().all(|c| c.is_ascii_digit()) && !GENERIC.contains(&first.as_str()) {
            let long_words: Vec<&String> = tokens.iter().filter(|t| t.len() > 3 && t.chars().all(|c| c.is_ascii_alphabetic())).collect();
            let title_case = long_words.len() >= 3
                && (long_words.iter().filter(|t| t.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)).count() as f64) > 0.6 * long_words.len() as f64;
            for (i, t) in tokens.iter().enumerate() {
                if t.to_lowercase() != *first || !t.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                    continue;
                }
                if i == 0 || (t.chars().skip(1).any(|c| c.is_ascii_uppercase()) && !mostly_caps) || !(title_case || mostly_caps) {
                    return true;
                }
            }
        }
    }
    false
}

/// Google's items for a search, each kept only where its headline names the
/// listing and is not a quote site's page for it. Google's titles end in ` -
/// Publisher`, which is taken off so the same story from another source is one
/// row; a page Google dates before 2000 is not news.
pub fn parse_google_news(xml: &str, symbol: &str, name: &str, us: bool) -> Vec<NewsItem> {
    let mut rows = Vec::new();
    for item in all_between(xml, "<item>", "</item>") {
        let (mut title, source, link) = (tag(&item, "title"), tag(&item, "source"), tag(&item, "link"));
        let when = iso(&tag(&item, "pubDate"));
        if title.is_empty() || link.is_empty() || when.is_empty() {
            continue;
        }
        let tail = format!(" - {}", source);
        if !source.is_empty() && title.ends_with(&tail) {
            title = title[..title.len() - tail.len()].trim_end_matches(is_space).to_string();
        }
        if quote_page_re().is_match(&title) || !names_listing(&title, symbol, name, us) {
            continue;
        }
        rows.push(NewsItem {
            id: format!("gnews:{}", sha16(&link)),
            headline: title,
            source: if source.is_empty() { "Google News".to_string() } else { source.clone() },
            url: link,
            published_at: when,
            summary: String::new(),
            kind: kind_of(&source),
            via: Feed::Gnews,
        });
    }
    rows
}

/// What Google is asked for a listing: its name in quotes when the book has
/// one, and its ticker in its venue's form (`"TSXV:CH"`, `"CSE:HG"`,
/// `"NEO:HBIX"`).
pub fn google_queries(symbol: &str, exchange: &str, currency: &str, name: &str) -> Vec<String> {
    let sym = tmx_symbol(symbol).to_uppercase();
    let mut out = Vec::new();
    let clean = search_name(name);
    if !clean.is_empty() && clean.to_uppercase() != sym {
        out.push(format!("\"{}\"", clean));
    }
    let form = tmx_form(exchange, currency);
    let ex = exchange.trim().to_uppercase();
    let venue = match form {
        Some(":CNX") => Some("CSE"),
        Some(":AQL") => Some("NEO"),
        Some("") => Some(if ex == "TSX-V" || ex == "TSXV" { "TSXV" } else { "TSX" }),
        Some(":US") => if ex.starts_with("NYSE") { Some("NYSE") } else if ex == "NASDAQ" || ex.is_empty() { Some("NASDAQ") } else { None },
        _ => None,
    };
    if let (false, Some(v)) = (sym.is_empty(), venue) {
        out.push(format!("\"{}:{}\"", v, sym));
    }
    out
}

/// `None` when there is nothing to search for.
pub fn fetch_google(net: &Net, symbol: &str, exchange: &str, currency: &str, name: &str) -> Result<Option<Vec<NewsItem>>, NetError> {
    let us = tmx_form(exchange, currency) == Some(":US");
    let queries = google_queries(symbol, exchange, currency, name);
    if queries.is_empty() {
        return Ok(None);
    }
    let (mut rows, mut seen) = (Vec::new(), HashSet::new());
    for q in queries {
        paced(net, "news.google.com", 1.5);
        let xml = (net.get)(&GNEWS_URL.replace("{}", &quote(&q, "/")), &FEED_HEADERS)?;
        for r in parse_google_news(&xml, symbol, name, us) {
            if seen.insert(r.id.clone()) {
                rows.push(r);
            }
        }
    }
    Ok(Some(rows))
}

// ---------------------------------------------------------------------------
// the merge
// ---------------------------------------------------------------------------

/// Beside the listing's wire (TMX's or Nasdaq's), read in this order; a later
/// source's copy of a story an earlier one carries is the same row. Each is
/// read at most this often per listing, Google and Seeking Alpha less often
/// than the wire so neither is asked more than it tolerates.
pub const EXTRA_SOURCES: [Feed; 3] = [Feed::Yahoo, Feed::Sa, Feed::Gnews];

pub fn source_minutes(source: Feed) -> i64 {
    match source {
        Feed::Sa | Feed::Gnews => 30,
        _ => FRESH_MINUTES,
    }
}

/// One story's copies from several sources; a wire's day-only time can sit a
/// day off another's.
pub const SAME_STORY_HOURS: i64 = 26;

/// What a source beside the wire is asked with for one listing.
#[derive(Debug, Clone, Default)]
pub struct Ask {
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub name: String,
    /// The Yahoo ticker, "" when Yahoo has none for the listing.
    pub yahoo: String,
}

/// A wire's rows, and the feeds of it that failed or answered nothing this
/// time (`tmx`, the press releases; `tmx-media`, In The Media; `nasdaq`;
/// `nasdaq-press`), whose stored items stand in for them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WireAnswer {
    pub rows: Vec<NewsItem>,
    pub missing: HashSet<Feed>,
}

impl WireAnswer {
    pub fn of(rows: Vec<NewsItem>) -> WireAnswer {
        WireAnswer { rows, missing: HashSet::new() }
    }
}

pub type WireFn<'a> = dyn Fn(&Connection, &str, &str, &str, &Clock) -> (Feed, Option<WireAnswer>) + Sync + 'a;
pub type ExtraFn<'a> = dyn Fn(Feed, &Ask) -> Result<Option<Vec<NewsItem>>, NetError> + Sync + 'a;

/// How a listing is read: its wire, and the sources beside it.
pub struct Readers<'a> {
    pub wire: &'a WireFn<'a>,
    pub extra: &'a ExtraFn<'a>,
}

fn live_wire(conn: &Connection, symbol: &str, exchange: &str, currency: &str, clock: &Clock) -> (Feed, Option<WireAnswer>) {
    fetch_symbol(conn, &LIVE, symbol, exchange, currency, clock)
}

fn live_extra(key: Feed, ask: &Ask) -> Result<Option<Vec<NewsItem>>, NetError> {
    read_extra(&LIVE, key, ask)
}

/// Every source read from the internet.
pub const LIVE_READERS: Readers<'static> = Readers { wire: &live_wire, extra: &live_extra };

pub fn read_extra(net: &Net, key: Feed, ask: &Ask) -> Result<Option<Vec<NewsItem>>, NetError> {
    match key {
        Feed::Yahoo => fetch_yahoo(net, ask),
        Feed::Sa => fetch_sa(net, &ask.symbol, &ask.exchange, &ask.currency),
        _ => fetch_google(net, &ask.symbol, &ask.exchange, &ask.currency, &ask.name),
    }
}

/// The sources beside the wire that have something to ask for a listing:
/// Yahoo a ticker form, Seeking Alpha a feed, Google a search. A listing with
/// no venue and no currency is left to the wire.
pub fn sources_for(conn: &Connection, symbol: &str, exchange: &str, currency: &str, name: &str) -> Vec<Feed> {
    if symbol == MARKET.0 || tmx_form(exchange, currency).is_none() {
        return vec![];
    }
    let have = [
        !yahoo_form(conn, symbol, exchange, currency).is_empty(),
        !sa_form(symbol, exchange, currency).is_empty(),
        !google_queries(symbol, exchange, currency, name).is_empty(),
    ];
    EXTRA_SOURCES.iter().zip(have).filter(|(_, h)| *h).map(|(k, _)| *k).collect()
}

/// Two copies of one headline are one story when they were published within a
/// day of each other; a trading halt, a resumption or a distribution notice
/// repeats its title word for word months later.
pub fn same_story(a: &str, b: &str) -> bool {
    match (instant(a), instant(b)) {
        (Some(x), Some(y)) => (x - y).abs() <= SAME_STORY_HOURS * 3600,
        _ => false,
    }
}

/// A headline as the same story reads under any source: lower case,
/// punctuation and spacing gone.
pub fn news_text(headline: &str) -> String {
    words(headline).join(" ")
}

fn stamp_key(source: Feed, symbol: &str, exchange: &str) -> String {
    format!("news_source_fetched:{}:{}", source.as_str(), sf::news_key(symbol, exchange))
}

fn meta(conn: &Connection, key: &str) -> String {
    bagholder_store::tables::get_meta(conn, key, "").unwrap_or_default()
}

fn due(conn: &Connection, source: Feed, symbol: &str, exchange: &str, now: i64) -> bool {
    let last = meta(conn, &stamp_key(source, symbol, exchange));
    match if last.is_empty() { None } else { instant(&last) } {
        None => true,
        Some(then) => now - then > source_minutes(source) * 60,
    }
}

/// Every source's items for one listing, merged newest first: (wire, rows,
/// sources asked).
///
/// The wire and every source that is due are read at once. A source that
/// fails, answers with nothing, or is not due this pass keeps the items it had
/// stored, so its stories stay on the list until it answers again. Rows is
/// `None` only when nothing answered, and the listing's stored news then
/// stands as it was. One story carried by several sources is one row: the same
/// headline keeps the copy of the first source in the order the wire, Yahoo,
/// Seeking Alpha, Google, where the copies were published within a day of each
/// other.
pub fn fetch_listing(
    conn: &Connection,
    readers: &Readers,
    symbol: &str,
    exchange: &str,
    currency: &str,
    name: &str,
    force: bool,
    clock: &Clock,
) -> (Feed, Option<Vec<NewsItem>>, HashSet<Feed>) {
    if symbol == MARKET.0 {
        let (src, rows) = (readers.wire)(conn, symbol, exchange, currency, clock);
        let answered: HashSet<Feed> = if rows.is_some() { [src].into() } else { HashSet::new() };
        return (src, rows.map(|w| w.rows), answered);
    }
    let extras: Vec<Feed> = sources_for(conn, symbol, exchange, currency, name)
        .into_iter()
        .filter(|k| force || due(conn, *k, symbol, exchange, clock.now))
        .collect();
    let ask = Ask {
        symbol: symbol.to_string(),
        exchange: exchange.to_string(),
        currency: currency.to_string(),
        name: name.to_string(),
        yahoo: if extras.contains(&Feed::Yahoo) { yahoo_form(conn, symbol, exchange, currency) } else { String::new() },
    };
    let mut results: HashMap<Feed, Vec<NewsItem>> = HashMap::new();
    let (src, primary) = std::thread::scope(|scope| {
        let jobs: Vec<(Feed, std::thread::ScopedJoinHandle<Result<Option<Vec<NewsItem>>, NetError>>)> = extras
            .iter()
            .map(|k| {
                let (k2, ask) = (*k, &ask);
                (*k, scope.spawn(move || (readers.extra)(k2, ask)))
            })
            .collect();
        let wire = (readers.wire)(conn, symbol, exchange, currency, clock);
        for (k, job) in jobs {
            match job.join() {
                Ok(Ok(Some(got))) => {
                    results.insert(k, got);
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => log(&format!("bagholder news: {} from {} failed: {}", symbol, k.as_str(), e)),
                Err(_) => log(&format!("bagholder news: {} from {} failed", symbol, k.as_str())),
            }
        }
        wire
    });
    let mut answered: HashSet<Feed> = results.keys().cloned().collect();
    if primary.is_some() {
        answered.insert(src);
    }
    if answered.is_empty() {
        return (src, None, answered);
    }
    // every source asked this pass waits its turn again, a failing one too
    answered.extend(extras.iter().cloned());
    let mut stored: HashMap<Feed, Vec<NewsItem>> = HashMap::new();
    let mut stored_feed: HashMap<Feed, Vec<NewsItem>> = HashMap::new();
    for r in sf::news_for(conn, symbol, exchange).unwrap_or_default() {
        let origin = sf::Feed::of_id(&r.id);
        let item = match r.item() { Some(i) => i, None => continue };
        if let Some(o) = origin {
            stored.entry(o).or_default().push(item.clone());
        }
        stored_feed.entry(item.via).or_default().push(item);
    }
    let mut merged: Vec<NewsItem> = Vec::new();
    let mut ids: HashSet<String> = HashSet::new();
    let mut texts: HashMap<String, Vec<String>> = HashMap::new();
    let mut add = |items: &[NewsItem]| {
        for r in items {
            let text = news_text(&r.headline);
            if ids.contains(&r.id) || (!text.is_empty() && texts.get(&text).map(|ws| ws.iter().any(|w| same_story(&r.published_at, w))).unwrap_or(false)) {
                continue;
            }
            ids.insert(r.id.clone());
            if !text.is_empty() {
                texts.entry(text).or_default().push(r.published_at.clone());
            }
            merged.push(r.clone());
        }
    };
    // a source that answers with nothing for a listing it had items for has
    // not lost its history: a throttled or degraded answer reads that way, and
    // its stored items stay until it answers again
    let empty = Vec::new();
    match &primary {
        Some(w) if !w.rows.is_empty() => add(&w.rows),
        _ => {
            let mut both = stored.get(&Feed::Tmx).cloned().unwrap_or_default();
            both.extend(stored.get(&Feed::Nasdaq).cloned().unwrap_or_default());
            add(&both);
        }
    }
    if let Some(w) = &primary {
        let mut feeds: Vec<&Feed> = w.missing.iter().collect();
        feeds.sort();
        for feed in feeds {
            add(stored_feed.get(feed).unwrap_or(&empty));
        }
    }
    for k in EXTRA_SOURCES {
        match results.get(&k) {
            Some(got) if !got.is_empty() => add(got),
            _ => add(stored.get(&k).unwrap_or(&empty)),
        }
    }
    let mut merged = merged;
    merged.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    merged.truncate(PER_LISTING);
    (src, Some(merged), answered)
}

pub type OnNew<'a> = dyn Fn(&Connection, &str, &str, &[NewsItem], &[String]) + Sync + 'a;

/// One listing's news read from every source and stored in place of what it
/// had: (wire, rows), rows `None` when nothing answered. `on_new` is handed the
/// rows and the ids the listing lacked.
pub fn read_listing(
    conn: &Connection,
    readers: &Readers,
    symbol: &str,
    exchange: &str,
    currency: &str,
    name: &str,
    force: bool,
    clock: &Clock,
    on_new: Option<&OnNew>,
) -> rusqlite::Result<(Feed, Option<Vec<NewsItem>>)> {
    let (src, rows, answered) = fetch_listing(conn, readers, symbol, exchange, currency, name, force, clock);
    let rows = match rows { Some(r) => r, None => return Ok((src, None)) };
    let extra_answered: Vec<Feed> = answered.iter().filter(|k| EXTRA_SOURCES.contains(k)).cloned().collect();
    let (mut before, mut before_text, mut first_read) = (HashSet::new(), HashMap::<String, Vec<String>>::new(), HashSet::new());
    if on_new.is_some() {
        // new is what the listing did not hold under any source: not an id it
        // had, not a story it had under another source's id, and nothing from
        // a source read for the listing the first time, whose back catalogue
        // is history, as every stream's is when it is first met
        for r in sf::news_for(conn, symbol, exchange)? {
            before.insert(r.id.clone());
            let t = news_text(&r.headline);
            if !t.is_empty() {
                before_text.entry(t).or_default().push(r.published_at.clone());
            }
        }
        for k in &extra_answered {
            if meta(conn, &stamp_key(*k, symbol, exchange)).is_empty() {
                first_read.insert(*k);
            }
        }
    }
    let stamp = clock.stamp();
    sf::replace_news(conn, symbol, exchange, &rows, &stamp)?;
    for k in &extra_answered {
        bagholder_store::tables::set_meta(conn, &stamp_key(*k, symbol, exchange), &stamp)?;
    }
    if let Some(f) = on_new {
        let mut new_ids: Vec<String> = rows
            .iter()
            .filter(|r| {
                !before.contains(&r.id)
                    && !Feed::of_id(&r.id).map(|f| first_read.contains(&f)).unwrap_or(false)
                    && !before_text.get(&news_text(&r.headline)).map(|ws| ws.iter().any(|w| same_story(&r.published_at, w))).unwrap_or(false)
            })
            .map(|r| r.id.clone())
            .collect();
        new_ids.sort();
        new_ids.dedup();
        f(conn, symbol, exchange, &rows, &new_ids);
    }
    Ok((src, Some(rows)))
}

/// Which wire answers for a listing: TMX for the Canadian venues it carries,
/// Nasdaq for US ones and for the market feed.
pub fn source_for(symbol: &str, exchange: &str, currency: &str) -> Option<Feed> {
    if symbol == MARKET.0 && exchange.to_uppercase() == MARKET.1 {
        return Some(Feed::Nasdaq);
    }
    match tmx_form(exchange, currency) {
        Some(":US") => Some(Feed::Nasdaq),
        None => None,
        Some(_) => Some(Feed::Tmx),
    }
}

/// The latest items for one listing from its wire.
///
/// `None` means the wire failed and what is stored should stand; an empty
/// answer means it answered with nothing.
pub fn fetch_symbol(conn: &Connection, net: &Net, symbol: &str, exchange: &str, currency: &str, clock: &Clock) -> (Feed, Option<WireAnswer>) {
    // A ticker asked for with no venue at all is an ambiguous name: TMX's
    // news answers on the bare ticker whatever venue it is asked under, so
    // `F` there is a Canadian company's halt notice and not Ford's
    // releases. Only Nasdaq is asked, whose items name the symbols they
    // belong to and are kept only when this one is among them, so nothing
    // comes back rather than another company's news.
    let src = source_for(symbol, exchange, currency).unwrap_or(Feed::Nasdaq);
    let sym = tmx_symbol(symbol);
    if sym.is_empty() {
        return (src, Some(WireAnswer::default()));
    }
    let parse = |text: &str| serde_json::from_str::<Value>(text).map_err(|e| NetError { code: None, text: e.to_string() });

    if symbol == MARKET.0 {
        paced(net, "api.nasdaq.com", 0.6);
        let got = (net.get)(&NASDAQ_LATEST_URL.replace("{}", &PER_MARKET.to_string()), &nasdaq_headers()).and_then(|t| parse(&t));
        return match got {
            Ok(data) => (src, Some(WireAnswer::of(parse_nasdaq_news(&data, clock.now, "", None)))),
            Err(e) => {
                log(&format!("bagholder news: {} from {} failed: {}", sym, src.as_str(), e));
                (src, None)
            }
        };
    }

    if src == Feed::Tmx {
        // TMX names a listing by its venue, and the news query answers nothing
        // under the wrong name: the same code the quote asks under, through
        // the same lookup, so a record with a wrong or missing venue resolves
        // here as it does everywhere else and is remembered once
        let code = match crate::quotes::tmx_quote_symbol(symbol, exchange, currency) {
            Some(c) => c,
            None => return (src, Some(WireAnswer::default())),
        };
        let ask = |form: &str| -> Result<Option<WireAnswer>, NetError> {
            // both tabs: the press releases, then the stories publishers wrote
            // about the company. A tab that fails is left out rather than
            // failing the other one, and named, so the stories it had stored
            // stand in for it.
            let (mut rows, mut missing) = (Vec::new(), HashSet::new());
            for media in [false, true] {
                paced(net, "app-money.tmx.com", 0.6);
                let payload = json!({
                    "operationName": "getNewsForSymbol",
                    "variables": {"symbol": form, "page": 1, "limit": PER_SYMBOL, "locale": "en", "companyInNews": media},
                    "query": TMX_NEWS_QUERY,
                });
                let data = match (net.post)(crate::tmx::TMX_URL, &payload, &tmx_headers()) {
                    Ok(d) => d,
                    Err(e) => {
                        if !media {
                            return Err(e);
                        }
                        log(&format!("bagholder news: {} stories from tmx failed: {}", form, e));
                        missing.insert(Feed::TmxMedia);
                        continue;
                    }
                };
                let got = parse_tmx_news(&data, form, media);
                if got.is_empty() {
                    missing.insert(if media { Feed::TmxMedia } else { Feed::Tmx });
                }
                rows.extend(got);
            }
            // an answer with no rows is no answer, so the lookup tries the
            // form TMX resolves instead
            Ok(if rows.is_empty() { None } else { Some(WireAnswer { rows, missing }) })
        };
        return match crate::tmx::tmx_lookup_try(conn, &code, &clock.today, ask) {
            Ok((Some(got), _)) => (src, Some(got)),
            Ok((None, _)) => (src, Some(WireAnswer { rows: vec![], missing: [Feed::Tmx, Feed::TmxMedia].into() })),
            Err(e) => {
                log(&format!("bagholder news: {} from {} failed: {}", sym, src.as_str(), e));
                (src, None)
            }
        };
    }

    paced(net, "api.nasdaq.com", 0.6);
    let url = NASDAQ_NEWS_URL.replacen("{}", &sym, 1).replacen("{}", &PER_SYMBOL.to_string(), 1);
    let data = match (net.get)(&url, &nasdaq_headers()).and_then(|t| parse(&t)) {
        Ok(d) => d,
        Err(e) => {
            log(&format!("bagholder news: {} from {} failed: {}", sym, src.as_str(), e));
            return (src, None);
        }
    };
    let mut answer = WireAnswer::of(parse_nasdaq_news(&data, clock.now, &sym, None));
    if answer.rows.is_empty() {
        answer.missing.insert(Feed::Nasdaq);
    }
    // the listing's own releases come on a feed of their own; each once,
    // beside the stories
    paced(net, "api.nasdaq.com", 0.6);
    let purl = NASDAQ_PRESS_URL.replacen("{}", &sym, 1).replacen("{}", &PER_SYMBOL.to_string(), 1);
    match (net.get)(&purl, &nasdaq_headers()).and_then(|t| parse(&t)) {
        Ok(pdata) => {
            let seen: HashSet<String> = answer.rows.iter().map(|r| r.id.clone()).collect();
            let press: Vec<NewsItem> = parse_nasdaq_news(&pdata, clock.now, &sym, Some(NewsKind::Release))
                .into_iter()
                .filter(|r| !seen.contains(&r.id))
                .map(|mut r| { r.via = Feed::NasdaqPress; r })
                .collect();
            if press.is_empty() {
                answer.missing.insert(Feed::NasdaqPress);
            }
            answer.rows.extend(press);
        }
        Err(e) => {
            answer.missing.insert(Feed::NasdaqPress);
            log(&format!("bagholder news: {} releases from nasdaq failed: {}", sym, e));
        }
    }
    (src, Some(answer))
}

/// A listing whose news is wanted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Listing {
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    pub name: String,
}

/// The listings with a source to read: the wire older than `minutes`, or any
/// source beside it that has something to ask and is due. Freshness is each
/// source's own: a listing whose wire was just read by a copy of the app that
/// did not ask the other sources, or by a pass where one of them was not yet
/// due, still has them to read.
pub fn stale(conn: &Connection, listings: &[Listing], now: i64, minutes: i64) -> rusqlite::Result<Vec<Listing>> {
    let fetched = sf::news_fetched_at(conn)?;
    let mut out = Vec::new();
    for l in listings {
        let (symbol, exchange, currency, name) = (&l.symbol, &l.exchange, &l.currency, &l.name);
        let last = fetched.get(&sf::news_key(symbol, exchange)).cloned().unwrap_or_default();
        let old = match if last.is_empty() { None } else { instant(&last) } {
            None => true,
            Some(then) => now - then > minutes * 60,
        };
        if old || sources_for(conn, symbol, exchange, currency, name).iter().any(|k| due(conn, *k, symbol, exchange, now)) {
            out.push(l.clone());
        }
    }
    Ok(out)
}

/// Read every source for every stale listing, a few listings side by side;
/// each answer replaces that listing's rows. Returns how many answered.
/// `on_new(conn, symbol, exchange, rows, new_ids)` is handed everything the
/// listing's sources answered with and the ids it did not have before; what is
/// worth telling about is the notifier's to decide. `on_start(listings)` is
/// told what the pass will read and `on_done(listing, answered)` each listing
/// as it lands, so a page can say a read is under way and show each listing's
/// items as they arrive rather than at the end of the pass.
pub fn refresh(
    open: &(dyn Fn() -> Option<Connection> + Sync),
    readers: &Readers,
    listings: &[Listing],
    clock: &Clock,
    on_new: Option<&OnNew>,
    on_start: Option<&dyn Fn(&[Listing])>,
    on_done: Option<&(dyn Fn(&Listing, bool) + Sync)>,
    at_once: usize,
) -> rusqlite::Result<usize> {
    let conn = match open() { Some(c) => c, None => return Ok(0) };
    let due = stale(&conn, listings, clock.now, FRESH_MINUTES)?;
    if let Some(f) = on_start {
        f(&due);
    }
    if due.is_empty() {
        return Ok(0);
    }
    let queue = Mutex::new(due.iter().collect::<std::collections::VecDeque<_>>());
    let done = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..at_once.min(due.len()).max(1) {
            scope.spawn(|| {
                let c = match open() { Some(c) => c, None => return };
                loop {
                    let listing = match queue.lock().unwrap().pop_front() { Some(l) => l, None => return };
                    let (symbol, exchange, currency, name) = (&listing.symbol, &listing.exchange, &listing.currency, &listing.name);
                    let rows = match read_listing(&c, readers, symbol, exchange, currency, name, false, clock, on_new) {
                        Ok((_, rows)) => rows,
                        Err(e) => {
                            log(&format!("bagholder news: {} read failed: {}", symbol, e));
                            None
                        }
                    };
                    if let Some(f) = on_done {
                        f(listing, rows.is_some());
                    }
                    if rows.is_some() {
                        done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
        }
    });
    let n = done.into_inner();
    if n > 0 {
        sf::trim_news(&conn, KEEP)?;
    }
    Ok(n)
}
