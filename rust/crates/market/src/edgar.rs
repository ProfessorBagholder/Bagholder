//! SEC EDGAR: US regulatory filings, one filer at a time.
//!
//! EDGAR publishes a documented JSON interface with no bot gate and no key; it
//! asks only for a descriptive User-Agent. A US listing is resolved to its
//! filer's CIK through the SEC's own list of tickers with their exchanges, the
//! filer's recent filings are read from the submissions API, and each is
//! normalized into the shared disclosure shape. A listing on a US exchange or the
//! US over-the-counter market is the SEC's row for its ticker, whatever exchange
//! the row names (the list holds each ticker once, and names some listings' no
//! exchange, or a newer one's). A listing on another market
//! reaches the SEC only where its exchange pairs it with a US listing (the TSX's
//! interlisted list), never by its ticker. Documents are static URLs, so
//! downloading needs no session.

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use crate::disclosures::{self as d, Enrichment, Fetched, SourceError};
use crate::stored::feeds::{FiledDocument, Regulator};

pub const SOURCE: &str = "SEC";
pub const TICKERS_URL: &str = "https://www.sec.gov/files/company_tickers_exchange.json";
pub const SUBMISSIONS_URL: &str = "https://data.sec.gov/submissions/CIK{}.json";
pub const TIMEOUT: u64 = 30;
/// SEC allows up to ten requests a second; this stays well under.
pub const PACE: Duration = Duration::from_millis(300);

/// SEC's fair-access policy asks for a User-Agent that names the caller with a
/// contact address; `BAGHOLDER_SEC_UA` sets your own.
pub fn ua() -> String {
    std::env::var("BAGHOLDER_SEC_UA").unwrap_or_else(|_| "Bagholder/1.0 (filings admin@bagholder.app)".into())
}

/// EDGAR needs nothing more.
pub fn available() -> bool {
    true
}

fn pace() -> Fetched<()> {
    bagholder_net::machine::turn("sec.gov", PACE)
        .map_err(|r| SourceError::Unavailable(format!("sec.gov refused a request and is resting until {}", r.until)))
}

/// The standard reason phrase for an HTTP status, for failure messages.
fn reason(code: u16) -> &'static str {
    match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// A failed request in the transport's own words, or its status with the reason.
fn describe(e: &bagholder_net::client::Error) -> String {
    match e {
        bagholder_net::client::Error::Status(c) => format!("HTTP Error {}: {}", c, reason(*c)),
        e => e.to_string(),
    }
}

fn get(url: &str, what: &str) -> Fetched<bagholder_net::client::Response> {
    pace()?;
    let ua = ua();
    let headers = [("User-Agent", ua.as_str()), ("Accept-Encoding", "gzip, deflate"), ("Accept", "application/json")];
    bagholder_net::client::request("GET", url, &headers, None, Duration::from_secs(TIMEOUT)).map_err(failed(what))
}

/// A request to the SEC that did not answer, as `what` failing in the transport's
/// words; one refused offline asks no one, so it is no failure of the SEC's.
fn failed(what: &str) -> impl Fn(bagholder_net::client::Error) -> SourceError + '_ {
    move |e| match e {
        bagholder_net::client::Error::Offline => SourceError::Offline,
        e => SourceError::Unavailable(format!("{}: {}", what, describe(&e))),
    }
}

fn get_json(url: &str) -> Fetched<Value> {
    let resp = get(url, "EDGAR request failed")?;
    serde_json::from_str(&resp.text()).map_err(|e| SourceError::Unavailable(format!("EDGAR returned unreadable data: {}", e)))
}

/// {TICKER: [(cik, title)]} of every row of the SEC's list of tickers with their
/// exchanges, the ticker as the SEC writes it: each filer the list names for it
/// (one, where the list holds the ticker once).
pub type Tickers = HashMap<String, Vec<(i64, String)>>;

/// The list as last read, and when, on the wall clock (which goes on while the
/// machine sleeps, so a day is a day); and the read that failed since, if one did:
/// when, how many times in a row, and why.
#[derive(Default)]
struct ListState {
    read: Option<(SystemTime, std::sync::Arc<Tickers>)>,
    failed: Option<(SystemTime, u32, SourceError)>,
}

fn tickers() -> &'static Mutex<ListState> {
    static T: OnceLock<Mutex<ListState>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(ListState::default()))
}

/// The SEC's `{"fields": [...], "data": [[...], ...]}` list read: every row, by its
/// ticker, whatever exchange it names. A list whose fields are not the ones read
/// is a reply that does not read, never an empty list.
pub fn parse_tickers(data: &Value) -> Fetched<Tickers> {
    let shape = |why: &str| SourceError::Other(format!("the SEC's ticker list does not read: {why}"));
    let fields: Vec<&str> = data.get("fields").and_then(|f| f.as_array()).ok_or_else(|| shape("no fields"))?.iter().filter_map(|f| f.as_str()).collect();
    let at = |name: &str| fields.iter().position(|f| *f == name).ok_or_else(|| shape(&format!("no {name} field")));
    let (cik, name, ticker) = (at("cik")?, at("name")?, at("ticker")?);
    let rows = data.get("data").and_then(|d| d.as_array()).ok_or_else(|| shape("no data"))?;
    let mut out = Tickers::new();
    for row in rows {
        let cell = |i: usize| row.get(i).cloned().unwrap_or(Value::Null);
        let t = cell(ticker).as_str().unwrap_or("").trim().to_uppercase();
        let c = match cell(cik) {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        };
        let (Some(c), false) = (c, t.is_empty()) else { continue };
        let filers = out.entry(t).or_default();
        if !filers.iter().any(|(k, _)| *k == c) {
            filers.push((c, cell(name).as_str().unwrap_or("").to_string()));
        }
    }
    Ok(out)
}

/// The SEC's ticker list, read again once the copy kept is as old as a listing's
/// filers stand (`disclosures::FILERS_RECHECK_HOURS`) on the wall clock: the daily
/// re-check reads today's list, never the one read at start. A read that fails is
/// the failure, kept for every listing that asks for the list: it is read again only
/// once its rest is over (`disclosures::FAILED_READ_REST`, growing with each failure
/// in a row, `market::grown_rest`), never by each listing on its own schedule.
pub fn ticker_map() -> Fetched<std::sync::Arc<Tickers>> {
    ticker_map_at(crate::wall_now(), || get_json(TICKERS_URL))
}

/// `ticker_map` at `now`, the list read by `read` where it is due.
pub fn ticker_map_at(now: SystemTime, read: impl FnOnce() -> Fetched<Value>) -> Fetched<std::sync::Arc<Tickers>> {
    let mut state = tickers().lock().unwrap_or_else(|e| e.into_inner());
    let since = |at: SystemTime| now.duration_since(at).unwrap_or(Duration::ZERO);
    let fresh = Duration::from_secs_f64(d::FILERS_RECHECK_HOURS * 3600.0);
    if let Some((_, t)) = state.read.as_ref().filter(|(at, _)| since(*at) < fresh) {
        return Ok(t.clone());
    }
    if let Some((_, _, why)) = state.failed.as_ref().filter(|(at, n, _)| since(*at) < bagholder_sources::market::grown_rest(d::FAILED_READ_REST, *n)) {
        return Err(why.clone());
    }
    match read().and_then(|v| parse_tickers(&v)) {
        Ok(t) => {
            let out = std::sync::Arc::new(t);
            state.read = Some((now, out.clone()));
            state.failed = None;
            Ok(out)
        }
        // not asked (offline): no failure of the SEC's, and nothing rests
        Err(SourceError::Offline) => Err(SourceError::Offline),
        Err(e) => {
            let n = state.failed.as_ref().map_or(0, |f| f.1) + 1;
            state.failed = Some((now, n, e.clone()));
            Err(e)
        }
    }
}

/// Prime the ticker map, as the tests stand in for SEC's published list.
pub fn set_ticker_map(t: Tickers) {
    let mut state = tickers().lock().unwrap_or_else(|e| e.into_inner());
    *state = ListState { read: Some((SystemTime::now(), std::sync::Arc::new(t))), failed: None };
}

/// A listing's symbol as the SEC writes it: a class with a dash (`BRK-B`).
pub fn sec_ticker(listing: &bagholder_core::ListingKey) -> String {
    listing.symbol().replace('.', "-")
}

/// A US listing's filer, on an exchange or over the counter: its CIK and the SEC's
/// name for it, from the SEC's list; why none, for a ticker the list names no
/// filer for or more than one, and for a listing on another market.
pub fn filer_of(listing: &bagholder_core::ListingKey) -> Fetched<Result<(i64, String), String>> {
    if !(listing.us() || listing.otc()) {
        return Ok(Err(format!("{} reaches the SEC only through the TSX's interlisted list.", listing.named())));
    }
    let t = sec_ticker(listing);
    Ok(match ticker_map()?.get(&t).map(Vec::as_slice) {
        Some([one]) => Ok(one.clone()),
        Some([_, _, ..]) => Err(format!("The SEC's list names more than one filer for {t}.")),
        _ => Err(format!("The SEC's list names no filer for {t}.")),
    })
}

const TITLES: [(&str, &str); 20] = [
    ("10-K", "Annual report"), ("10-Q", "Quarterly report"), ("8-K", "Current report"),
    ("20-F", "Annual report (foreign issuer)"), ("40-F", "Annual report (Canadian issuer)"),
    ("6-K", "Report of foreign private issuer"), ("DEF 14A", "Proxy statement"), ("DEFA14A", "Proxy soliciting material"),
    ("S-1", "Registration statement"), ("F-1", "Registration statement"), ("424B4", "Prospectus"),
    ("3", "Initial insider ownership"), ("4", "Insider transaction"), ("5", "Annual insider statement"),
    ("144", "Notice of proposed sale"), ("SC 13D", "Beneficial ownership (activist)"),
    ("SC 13G", "Beneficial ownership (passive)"), ("13F-HR", "Institutional holdings"),
    ("25", "Delisting notice"), ("425", "Business combination"),
];

pub fn category(form: &str) -> &'static str {
    static FDIGIT: OnceLock<Regex> = OnceLock::new();
    let f = form.to_uppercase();
    let starts = |ps: &[&str]| ps.iter().any(|p| f.starts_with(p));
    if starts(&["10-K", "10-Q", "20-F", "40-F", "6-K", "ARS", "N-CSR"]) {
        return d::FINANCIALS;
    }
    if f.starts_with("8-K") || f == "25" || f.starts_with("25-") {
        return d::EVENTS;
    }
    if f.contains("14A") || f.contains("14C") || f.starts_with("DEF") || f.starts_with("PRE") {
        return d::GOVERNANCE;
    }
    if starts(&["S-", "424", "POS", "DRS", "EFFECT", "425"]) || FDIGIT.get_or_init(|| Regex::new(r"^F-\d").unwrap()).is_match(&f) {
        return d::OFFERINGS;
    }
    if ["3", "4", "5", "3/A", "4/A", "5/A", "144"].contains(&f.as_str())
        || f.contains("13D")
        || f.contains("13G")
        || starts(&["SC 13", "SCHEDULE 13", "13F"])
    {
        return d::INSIDER;
    }
    d::OTHER
}

pub fn categorize(row: &FiledDocument) -> String {
    category(&row.form).to_string()
}

/// The plain-English title beside the form code; EDGAR often
/// repeats the form as the description, and then our own label reads better.
pub fn title(form: &str, description: &str) -> String {
    let dd = d::clean(description);
    let f = form.to_uppercase();
    if !dd.is_empty() {
        let up = dd.to_uppercase();
        if up != f && up != format!("FORM {}", f) {
            return dd;
        }
    }
    TITLES.iter().find(|(k, _)| *k == f).map(|(_, v)| v.to_string()).unwrap_or_default()
}

fn s(v: Option<&Value>) -> String {
    bagholder_model::value::s(v.filter(|x| !x.is_null()))
}

/// One field of `filings.recent`: a list read leniently -- a missing key
/// reads as empty, and an entry that is not a string reads as the text it
/// had.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct RecentList(#[serde(default)] pub Vec<Value>);

impl RecentList {
    fn get(&self, i: usize) -> String {
        s(self.0.get(i))
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// `filings.recent`, the parallel arrays the submissions answer carries one
/// filing's fields in.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct Recent {
    #[serde(default)]
    pub form: RecentList,
    #[serde(default, rename = "filingDate")]
    pub filing_date: RecentList,
    #[serde(default, rename = "primaryDocument")]
    pub primary_document: RecentList,
    #[serde(default, rename = "accessionNumber")]
    pub accession_number: RecentList,
    #[serde(default, rename = "primaryDocDescription")]
    pub primary_doc_description: RecentList,
}

/// `filings` on the submissions answer.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct SubmissionFilings {
    #[serde(default)]
    pub recent: Recent,
}

/// SEC's submissions answer for one issuer, the parts this app reads.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct Submissions {
    #[serde(default)]
    pub filings: SubmissionFilings,
}

/// The submissions answer into items. Split from
/// the request so the two implementations can be compared on one answer.
pub fn parse_submissions(sub: &Submissions, cik: i64, limit: usize) -> Fetched<Vec<FiledDocument>> {
    let recent = &sub.filings.recent;
    let forms = &recent.form;
    let dates = &recent.filing_date;
    let docs = &recent.primary_document;
    let accns = &recent.accession_number;
    let descs = &recent.primary_doc_description;
    let mut items = Vec::new();
    for i in 0..forms.len().min(dates.len()).min(accns.len()) {
        let acc = accns.get(i);
        let doc = docs.get(i);
        let url = if !doc.is_empty() {
            format!("https://www.sec.gov/Archives/edgar/data/{}/{}/{}", cik, acc.replace('-', ""), doc)
        } else {
            format!("https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK={}", cik)
        };
        let form = forms.get(i);
        let date = dates.get(i);
        items.push(FiledDocument {
            id: format!("sec:{}", acc),
            source: Regulator::Sec,
            category: category(&form).to_string(),
            date: date.clone(),
            date_text: date,
            title: title(&form, &descs.get(i)),
            form,
            size: String::new(),
            url,
            issuer: String::new(),
            profile_no: String::new(),
        });
    }
    items.truncate(limit.max(1));
    Ok(items)
}

/// A filer's recent filings, newest first.
pub fn fetch(cik: i64, limit: usize) -> Fetched<Vec<FiledDocument>> {
    fetch_with(cik, limit, &get_json)
}

/// `fetch` with the submissions request given.
pub fn fetch_with(cik: i64, limit: usize, get_json: &dyn Fn(&str) -> Fetched<Value>) -> Fetched<Vec<FiledDocument>> {
    let raw = get_json(&SUBMISSIONS_URL.replace("{}", &format!("{:010}", cik)))?;
    let sub: Submissions = match &raw {
        Value::Object(_) => serde_json::from_value(raw).map_err(|e| SourceError::Other(format!("EDGAR's submissions answer does not read: {e}")))?,
        other => {
            return Err(SourceError::Other(format!(
                "AttributeError: '{}' object has no attribute 'get'",
                match other { Value::Array(_) => "list", Value::String(_) => "str", Value::Null => "NoneType", Value::Bool(_) => "bool", _ => "float" }
            )))
        }
    };
    parse_submissions(&sub, cik, limit)
}

/// A Schedule 13G/13D's title and summary read
/// from its XML fields. Split from the download so it can be compared on one
/// document.
pub fn enrichment_from_xml(typ: &str, xml: &str) -> Option<Enrichment> {
    if !xml.contains("reportingPersonName") {
        return None;
    }
    let vals = |tag: &str| -> Vec<String> {
        let r = Regex::new(&format!("<{}>([^<]+)</{}>", regex::escape(tag), regex::escape(tag))).unwrap();
        r.captures_iter(xml).map(|c| bagholder_model::textrules::trim_space(&c[1]).to_string()).collect()
    };
    let mut owners: Vec<String> = Vec::new();
    for n in vals("reportingPersonName") {
        if !n.is_empty() && !owners.contains(&n) {
            owners.push(n);
        }
    }
    if owners.is_empty() {
        return None;
    }
    let issuer = vals("issuerName").into_iter().next().unwrap_or_default();
    let pct = vals("classPercent").into_iter().next().unwrap_or_default();
    let submission = vals("submissionType").into_iter().next().unwrap_or_else(|| typ.to_string());
    let amended = submission.contains("/A");
    let single = owners.len() == 1;
    let who = format!("{}{}", owners[0], if owners.len() > 1 { " and affiliates" } else { "" });
    let subject = if !pct.is_empty() {
        format!("{}% stake \u{2014} {}", pct, owners[0])
    } else {
        format!("Beneficial ownership \u{2014} {}", owners[0])
    };
    let verb = if amended { if single { "amends its" } else { "amend their" } } else if single { "reports" } else { "report" };
    let tail = if amended { " Schedule 13G report of beneficial ownership" } else { " beneficial ownership" };
    let stake = if pct.is_empty() { String::new() } else { format!(" of {}%", pct) };
    let of_issuer = if issuer.is_empty() { String::new() } else { format!(" of {}", issuer) };
    let summary = format!("{} {}{}{}{}'s common shares.", who, verb, tail, stake, of_issuer);
    Some(Enrichment { subject: subject.chars().take(90).collect(), summary: summary.chars().take(240).collect(), final_: false })
}

/// A deterministic title and summary for a Schedule 13G or
/// 13D, read from its raw XML rather than the rendered page.
pub fn enrichment(row: &FiledDocument) -> Option<Enrichment> {
    enrichment_with(row, &document)
}

/// `enrichment` with the download given.
pub fn enrichment_with(row: &FiledDocument, document: &dyn Fn(&FiledDocument) -> Fetched<(Vec<u8>, String)>) -> Option<Enrichment> {
    let typ = row.form.to_uppercase();
    if !typ.starts_with("SCHEDULE 13") {
        // every other form carries its own name, which needs no download
        return crate::formnames::title_of(&typ).map(|t| Enrichment { subject: t, summary: String::new(), final_: false });
    }
    static XSL: OnceLock<Regex> = OnceLock::new();
    let raw_url = XSL.get_or_init(|| Regex::new(r"/xsl[^/]*/").unwrap()).replace_all(&row.url, "/").into_owned();
    let mut probe = row.clone();
    probe.url = raw_url;
    let (data, _) = document(&probe).ok()?;
    enrichment_from_xml(&typ, &String::from_utf8_lossy(&data))
}

/// Of an accession's listing, the file that is the
/// filing's substance -- the largest real document, the primary as the
/// tiebreak -- or None for the primary itself.
pub fn pick_content(items: &[Value], primary: &str) -> Option<String> {
    static SKIP: OnceLock<Regex> = OnceLock::new();
    let skip = SKIP.get_or_init(|| Regex::new(r"(?i)(?:-index|-index-headers)\.(?:htm|html)$|^\d{10}-\d\d-\d{6}\.txt$|R\d+\.htm$").unwrap());
    let mut cands: Vec<(String, i64)> = Vec::new();
    for it in items {
        let n = s(it.get("name"));
        let low = n.to_lowercase();
        if ![".htm", ".html", ".txt", ".xml"].iter().any(|e| low.ends_with(e)) {
            continue;
        }
        if low.contains("index") || skip.is_match(&low) {
            continue;
        }
        let size = match it.get("size") {
            Some(Value::Number(x)) => x.as_i64().unwrap_or(0),
            Some(Value::String(t)) if !t.is_empty() => bagholder_model::textrules::parse_int(t)?,
            _ => 0,
        };
        cands.push((n, size));
    }
    if cands.is_empty() {
        return None;
    }
    cands.sort_by(|a, b| (-a.1, a.0 != primary).cmp(&(-b.1, b.0 != primary)));
    let best = cands[0].0.clone();
    if best == primary { None } else { Some(best) }
}

/// The filing's substance rather than its cover form.
pub fn content(row: &FiledDocument) -> Fetched<(Vec<u8>, String)> {
    content_with(row, &get_json, &document)
}

/// `content` with the index request and the download given.
pub fn content_with(
    row: &FiledDocument,
    get_json: &dyn Fn(&str) -> Fetched<Value>,
    document: &dyn Fn(&FiledDocument) -> Fetched<(Vec<u8>, String)>,
) -> Fetched<(Vec<u8>, String)> {
    let url = &row.url;
    if !url.starts_with("https://www.sec.gov/") {
        return document(row);
    }
    let (base, primary) = match url.rsplit_once('/') { Some(p) => p, None => return document(row) };
    let listing = match get_json(&format!("{}/index.json", base)) { Ok(v) => v, Err(_) => return document(row) };
    let items = listing.get("directory").and_then(|d| d.get("item")).and_then(|i| i.as_array()).cloned().unwrap_or_default();
    match pick_content(&items, primary) {
        None => document(row),
        Some(best) => {
            let mut probe = row.clone();
            probe.url = format!("{}/{}", base, best);
            document(&probe).or_else(|_| document(row))
        }
    }
}

/// A document fetched directly by its address, for a call that needs only
/// the bytes at a URL and not a stored row's other fields.
fn fetch_url(url: &str) -> Fetched<(Vec<u8>, String)> {
    if !url.starts_with("https://www.sec.gov/") {
        return Err(SourceError::Unavailable("not an SEC document url".into()));
    }
    pace()?;
    let ua = ua();
    let headers = [("User-Agent", ua.as_str()), ("Accept-Encoding", "gzip, deflate")];
    let resp = bagholder_net::client::request("GET", url, &headers, None, Duration::from_secs(TIMEOUT)).map_err(failed("EDGAR document fetch failed"))?;
    // email.message's get_content_type: the type alone, lower-cased, text/plain
    // where there is none or it is malformed
    let ct = resp
        .headers
        .iter()
        .find(|(k, _)| k == "content-type")
        .map(|(_, v)| v.split(';').next().unwrap_or("").trim().to_lowercase())
        .filter(|v| v.matches('/').count() == 1)
        .unwrap_or_else(|| "text/plain".into());
    Ok((resp.body, ct))
}

/// One EDGAR document, fetched directly. (bytes, content
/// type as `get_content_type()` gives it).
pub fn document(row: &FiledDocument) -> Fetched<(Vec<u8>, String)> {
    fetch_url(&row.url)
}
