//! SEC EDGAR: US regulatory filings, one issuer at a time.
//!
//! EDGAR publishes a documented JSON interface with no bot gate and no key; it
//! asks only for a descriptive User-Agent. A ticker is resolved to its SEC
//! CIK, the issuer's recent filings read from the submissions API, and each
//! normalized into the shared disclosure shape. Documents are static URLs, so
//! downloading needs no session.

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::disclosures::{self as d, Enrichment, Fetched, SourceError};
use bagholder_store::feeds::{FiledDocument, Regulator};

pub const SOURCE: &str = "SEC";
pub const TICKERS_URL: &str = "https://www.sec.gov/files/company_tickers.json";
pub const SUBMISSIONS_URL: &str = "https://data.sec.gov/submissions/CIK{}.json";
pub const TIMEOUT: u64 = 30;
/// SEC allows up to ten requests a second; this stays well under.
pub const PACE: Duration = Duration::from_millis(300);
pub const US_EXCHANGES: [&str; 11] = ["NASDAQ", "NYSE", "NYSEARCA", "NYSEAMERICAN", "AMEX", "ARCA", "BATS", "US", "OTC", "OTCMKTS", "CBOE"];

/// SEC's fair-access policy asks for a User-Agent that names the caller with a
/// contact address; `BAGHOLDER_SEC_UA` sets your own.
pub fn ua() -> String {
    std::env::var("BAGHOLDER_SEC_UA").unwrap_or_else(|_| "Bagholder/1.0 (filings admin@bagholder.app)".into())
}

fn us_exchange(ex: &str) -> bool {
    US_EXCHANGES.contains(&ex.to_uppercase().as_str())
}

/// EDGAR needs nothing more.
pub fn available() -> bool {
    true
}

fn pace() {
    bagholder_net::machine::turn("sec.gov", PACE);
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

fn describe(e: &bagholder_net::client::Error) -> String {
    match e {
        bagholder_net::client::Error::Status(c) => format!("HTTP Error {}: {}", c, reason(*c)),
        bagholder_net::client::Error::Transport(m) => format!("<urlopen error {}>", m),
    }
}

fn get(url: &str, what: &str) -> Fetched<bagholder_net::client::Response> {
    pace();
    let ua = ua();
    let headers = [("User-Agent", ua.as_str()), ("Accept-Encoding", "gzip, deflate"), ("Accept", "application/json")];
    bagholder_net::client::request("GET", url, &headers, None, Duration::from_secs(TIMEOUT))
        .map_err(|e| SourceError::Unavailable(format!("{}: {}", what, describe(&e))))
}

fn get_json(url: &str) -> Fetched<Value> {
    let resp = get(url, "EDGAR request failed")?;
    serde_json::from_str(&resp.text()).map_err(|e| SourceError::Unavailable(format!("EDGAR returned unreadable data: {}", e)))
}

pub type Tickers = HashMap<String, (i64, String)>;

fn tickers() -> &'static Mutex<Option<Tickers>> {
    static T: OnceLock<Mutex<Option<Tickers>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(None))
}

/// {TICKER: (cik, title)} from SEC's published list,
/// loaded once.
pub fn ticker_map() -> Fetched<Tickers> {
    let mut slot = tickers().lock().unwrap();
    if let Some(t) = slot.as_ref() {
        return Ok(t.clone());
    }
    let data = get_json(TICKERS_URL)?;
    let rows: Vec<Value> = match &data {
        Value::Object(m) => m.values().cloned().collect(),
        Value::Array(a) => a.clone(),
        _ => vec![],
    };
    let mut out = Tickers::new();
    for row in rows {
        let t = bagholder_model::value::s(row.get("ticker").filter(|v| !v.is_null())).to_uppercase();
        if t.is_empty() {
            continue;
        }
        let cik = match row.get("cik_str") {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            Some(Value::String(s)) => bagholder_model::textrules::parse_int(s).unwrap_or(0),
            _ => 0,
        };
        let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        out.insert(t, (cik, title));
    }
    *slot = Some(out.clone());
    Ok(out)
}

/// Prime the ticker map, as the tests stand in for SEC's published list.
pub fn set_ticker_map(t: Tickers) {
    *tickers().lock().unwrap() = Some(t);
}

/// A ticker as SEC writes it -- no venue suffix, dots to dashes.
pub fn bare(symbol: &str) -> String {
    let mut s = bagholder_model::textrules::trim_space(symbol).to_uppercase();
    for suf in [".TO", ".V", ".CN", ".NE", ".U"] {
        if s.ends_with(suf) {
            s.truncate(s.len() - suf.len());
        }
    }
    s.replace('.', "-")
}

/// A US listing, or any ticker SEC knows.
pub fn covers(symbol: &str, exchange: &str, currency: &str) -> bool {
    if us_exchange(exchange) || currency.to_uppercase() == "USD" {
        return true;
    }
    match ticker_map() {
        Ok(m) => m.contains_key(&bare(symbol)),
        Err(_) => false,
    }
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

/// The issuer's recent filings, newest first, or nothing when
/// SEC does not know the ticker or the name guard rejects a collision.
pub fn fetch(symbol: &str, name: &str, exchange: &str, currency: &str, limit: usize) -> Fetched<Vec<FiledDocument>> {
    fetch_with(symbol, name, exchange, currency, limit, &get_json)
}

/// `fetch` with the submissions request given.
pub fn fetch_with(symbol: &str, name: &str, exchange: &str, currency: &str, limit: usize, get_json: &dyn Fn(&str) -> Fetched<Value>) -> Fetched<Vec<FiledDocument>> {
    let map = ticker_map()?;
    let (cik, sec_title) = match map.get(&bare(symbol)) { Some(t) => t.clone(), None => return Ok(vec![]) };
    let us_listed = us_exchange(exchange) || currency.to_uppercase() == "USD";
    if !us_listed && !name.is_empty() && !d::names_match(name, &sec_title) {
        // a Canadian ticker colliding with a US filer
        return Ok(vec![]);
    }
    let raw = get_json(&SUBMISSIONS_URL.replace("{}", &format!("{:010}", cik)))?;
    let sub: Submissions = match &raw {
        Value::Object(_) => serde_json::from_value(raw).unwrap_or_default(),
        other => {
            return Err(SourceError::Other(format!(
                "AttributeError: '{}' object has no attribute 'get'",
                match other { Value::Array(_) => "list", Value::String(_) => "str", Value::Null => "NoneType", Value::Bool(_) => "bool", _ => "float" }
            )))
        }
    };
    parse_submissions(&sub, cik, limit)
}

/// Whether SEC knows a filer for this instrument, from the
/// ticker map alone.
pub fn has_filer(symbol: &str, name: &str, exchange: &str, currency: &str) -> bool {
    let map = match ticker_map() { Ok(m) => m, Err(_) => return false };
    let (_, title) = match map.get(&bare(symbol)) { Some(t) => t, None => return false };
    let us_listed = us_exchange(exchange) || currency.to_uppercase() == "USD";
    !(!us_listed && !name.is_empty() && !d::names_match(name, title))
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
    pace();
    let ua = ua();
    let headers = [("User-Agent", ua.as_str()), ("Accept-Encoding", "gzip, deflate")];
    let resp = bagholder_net::client::request("GET", url, &headers, None, Duration::from_secs(TIMEOUT))
        .map_err(|e| SourceError::Unavailable(format!("EDGAR document fetch failed: {}", describe(&e))))?;
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
