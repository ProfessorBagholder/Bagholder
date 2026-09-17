//! SEDAR+ document filings, fetched over HTTP one issuer at a time.
//!
//! SEDAR+ is the Canadian securities filing system. It publishes no API and
//! sits behind a bot manager that turns away ordinary HTTP clients at the TLS
//! handshake, so this speaks to it through the browser helper: one paced
//! session resolves an issuer to its nine-digit profile number, lists that
//! profile's filings, and downloads a filing as its PDF. Everything is on
//! demand and paced two seconds apart; nothing sweeps or monitors.
//!
//! The site is a server-rendered form application: a page posts its whole
//! form back to viewInstance/update.html with a callback node, name and the
//! view key, and answers with HTML fragments.

use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::browser::Session;
use crate::disclosures::{self as d, Fetched, SourceError};
use crate::news::unescape;
use bagholder_model::textrules::{parse_int, trim_space};

pub const SOURCE: &str = "SEDAR+";
pub const BASE: &str = "https://www.sedarplus.ca";
/// Between actions, so a lookup is a person's pace, not a sweep.
pub const PACE: Duration = Duration::from_secs(2);
pub const TIMEOUT: Duration = Duration::from_secs(90);
pub const DOC_TIMEOUT: Duration = Duration::from_secs(180);
/// Rows asked for in one document search.
pub const SEARCH_LIMIT: usize = 100;
/// How long an issuer's walked document page is reused.
pub const SCOPE_TTL: Duration = Duration::from_secs(120);

pub const CA_EXCHANGES: [&str; 11] = ["TSX", "TSXV", "TSX-V", "CSE", "CNSX", "NEO", "NEO EXCHANGE", "CBOE CANADA", "AQL", "TSX VENTURE", "CANADIAN SECURITIES EXCHANGE"];

fn unavailable(m: impl Into<String>) -> SourceError {
    SourceError::Unavailable(m.into())
}

/// `sedar.available`: whether the helper that clears the gate is installed.
pub fn available() -> bool {
    Session::available()
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).unwrap())
        }
    };
}

// --- character offsets, as Python slices a str ---------------------------------

/// The byte offset `n` characters before `at`, or the start.
fn back_chars(s: &str, at: usize, n: usize) -> usize {
    s[..at].char_indices().rev().nth(n.saturating_sub(1)).map(|(i, _)| i).unwrap_or(0).min(at)
}

/// The byte offset `n` characters after `at`, or the end.
fn fwd_chars(s: &str, at: usize, n: usize) -> usize {
    s[at..].char_indices().nth(n).map(|(i, _)| at + i).unwrap_or(s.len())
}

// --- the session ---------------------------------------------------------------

struct State {
    session: Option<Session>,
    last: Option<Instant>,
    scope: ScopeCache,
}

/// The issuer document pages walked recently, by profile number.
#[derive(Default)]
pub struct ScopeCache(HashMap<String, (Instant, String)>);

impl ScopeCache {
    /// A page still inside its window.
    pub fn fresh(&self, profile_no: &str) -> Option<String> {
        self.0.get(profile_no).filter(|(expiry, _)| *expiry > Instant::now()).map(|(_, h)| h.clone())
    }

    /// Keep a walked page; a failed walk is never kept.
    pub fn keep(&mut self, profile_no: &str, html: &Option<String>) {
        if let Some(h) = html {
            self.0.insert(profile_no.to_string(), (Instant::now() + SCOPE_TTL, h.clone()));
        }
    }

    pub fn remove(&mut self, profile_no: &str) {
        self.0.remove(profile_no);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// The cached page, else the walk's, kept when it succeeded.
    pub fn get_or_walk(&mut self, profile_no: &str, walk: impl FnOnce() -> Option<String>) -> Option<String> {
        if let Some(h) = self.fresh(profile_no) {
            return Some(h);
        }
        let html = walk();
        self.keep(profile_no, &html);
        html
    }
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { session: None, last: None, scope: ScopeCache::default() }))
}

fn pace(st: &mut State) {
    if let Some(t) = st.last {
        let next = t + PACE;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
    }
    st.last = Some(Instant::now());
}

fn session(st: &mut State) -> Fetched<&mut Session> {
    if st.session.is_none() {
        st.session = Some(Session::new().ok_or_else(|| unavailable("the browser helper is not installed"))?);
    }
    Ok(st.session.as_mut().unwrap())
}

/// `sedar.reset`: drop the session so the next call opens a fresh one.
pub fn reset() {
    state().lock().unwrap().session = None;
}

// --- the form protocol ---------------------------------------------------------

re!(re_field, r"(?i)<(input|select|textarea)\b([^>]*)>");
re!(re_name, r#"name="([^"]*)""#);
re!(re_type, r#"type="([^"]*)""#);
re!(re_value, r#"value="([^"]*)""#);
re!(re_selected, r#"<option[^>]*selected[^>]*value="([^"]*)"|value="([^"]*)"[^>]*selected"#);
re!(re_vi_param, r#"(?i)<input\b([^>]*class="[^"]*viewInstanceFormParameter[^"]*"[^>]*)>"#);
re!(re_search_action, r#"(?s)(?:appSearchButton|-searchButton)[^>]*?onclick="[^"]*?cat\w*Callback\('(W\d+)','(\w+)'[^"]*?containerNodeId:'(W\d+)'"#);
re!(re_menu_anchor, r"(?s)<a[^>]*?catCallback\('(W\d+)','invokeMenuCb'[^>]*>(.*?)</a>");
re!(re_callback_node, r"catCallback\('(W\d+)'");

/// `sedar._form_fields`: the form as the browser serializes it before a
/// callback -- every named input, each select's chosen option, checked boxes
/// only, the callback fields left for the caller.
pub fn form_fields(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in re_field().captures_iter(html) {
        let tag = m[1].to_lowercase();
        let attrs = &m[2];
        let name = match re_name().captures(attrs) { Some(c) => c[1].to_string(), None => continue };
        if name.starts_with("_CB") {
            continue;
        }
        if tag == "input" {
            let typ = re_type().captures(attrs).map(|c| c[1].to_string()).unwrap_or_else(|| "text".into()).to_lowercase();
            if typ == "submit" || typ == "button" || typ == "file" {
                continue;
            }
            if (typ == "checkbox" || typ == "radio") && !attrs.contains("checked") {
                continue;
            }
            let val = re_value().captures(attrs).map(|c| unescape(&c[1])).unwrap_or_default();
            out.push((name, val));
        } else if tag == "select" {
            let end = m.get(0).unwrap().end();
            // Python's html.find("</select>", end) of -1 slices to one short of
            // the end
            let stop = match html[end..].find("</select>") {
                Some(i) => end + i,
                None => html.char_indices().next_back().map(|(i, _)| i).unwrap_or(0).max(end),
            };
            let body = &html[end..stop];
            let val = re_selected()
                .captures(body)
                .map(|c| unescape(c.get(1).or_else(|| c.get(2)).map(|g| g.as_str()).unwrap_or("")))
                .unwrap_or_default();
            out.push((name, val));
        }
    }
    out
}

/// `sedar._vi_params`: the hidden viewInstanceFormParameter inputs every
/// callback carries.
pub fn vi_params(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in re_vi_param().captures_iter(html) {
        if let Some(name) = re_name().captures(&m[1]) {
            let val = re_value().captures(&m[1]).map(|c| unescape(&c[1])).unwrap_or_default();
            out.push((name[1].to_string(), val));
        }
    }
    out
}

/// `sedar._search_action`: (node, name, container) for a page's Search control.
pub fn search_action(page: &str) -> Option<(String, String, String)> {
    re_search_action().captures(page).map(|m| (m[1].to_string(), m[2].to_string(), m[3].to_string()))
}

/// `sedar._issuer_menu_node`: on a reporting-issuer result, the menu node that
/// opens the issuer itself.
pub fn issuer_menu_node(html: &str, name: Option<&str>) -> Option<String> {
    let mut fallback: Option<String> = None;
    let want: String = text(name.unwrap_or("")).chars().take(20).collect::<String>().to_lowercase();
    for m in re_menu_anchor().captures_iter(html) {
        let t = text(&m[2]);
        if t.is_empty() || t.to_lowercase().contains("search for profiles") {
            continue;
        }
        if !want.is_empty() && t.to_lowercase().contains(&want) {
            return Some(m[1].to_string());
        }
        if fallback.is_none() {
            fallback = Some(m[1].to_string());
        }
    }
    fallback
}

/// Python's `str.lower()` index of `needle` in `hay`, as a character index.
fn lower_find_chars(hay: &str, needle: &str) -> Option<usize> {
    let lowered: String = hay.to_lowercase();
    let byte = lowered.find(needle)?;
    Some(lowered[..byte].chars().count())
}

fn byte_of_char(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// `sedar._docs_menu_node`: on an issuer profile, the "Search and download
/// documents for this profile" menu node.
pub fn docs_menu_node(html: &str) -> Option<String> {
    let idx_chars = lower_find_chars(html, "search and download documents for this profile")?;
    let idx = byte_of_char(html, idx_chars);
    let start = html[..idx].rfind("<a ")?;
    re_callback_node().captures(&html[start..idx]).map(|c| c[1].to_string())
}

/// `urllib.parse.urlencode` over pairs: `quote_plus` on each side.
pub fn urlencode(pairs: &[(String, String)]) -> String {
    fn quote_plus(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(b as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
    pairs.iter().map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v))).collect::<Vec<_>>().join("&")
}

/// One opened service instance: its page, ids and session headers.
pub struct View {
    pub app: String,
    pub inst: String,
    pub key: String,
    pub sid: String,
    pub page: String,
    pub reference: String,
}

re!(re_inst_url, r"viewInstance/view\.html\?id=([0-9a-f]+)");
re!(re_inst_page, r"update\.html\?id=([0-9a-f]+)");
re!(re_key, r"viewInstanceKey:'([^']+)'");
re!(re_sid, r"sessionId:'([^']+)'");
re!(re_app, r"/(csa-\w+)/viewInstance");

impl View {
    fn open(st: &mut State, service: &str) -> Fetched<View> {
        pace(st);
        let url = format!("{}/csa-party/service/create.html?targetAppCode=csa-party&service={}", BASE, service);
        let r = session(st)?
            .request("GET", &url, &[], None, TIMEOUT, false)
            .map_err(|e| unavailable(format!("could not open {}: {}", service, e)))?;
        let page = r.text();
        let head: String = page.chars().take(2000).collect();
        if r.url.contains("validate.perfdrive.com") || head.contains("validate.perfdrive.com") {
            return Err(unavailable("the SEDAR+ bot gate turned the request away"));
        }
        let inst = re_inst_url().captures(&r.url).or_else(|| re_inst_page().captures(&page)).map(|c| c[1].to_string());
        let key = re_key().captures(&page).map(|c| c[1].to_string());
        let sid = re_sid().captures(&page).map(|c| c[1].to_string());
        let (inst, key, sid) = match (inst, key, sid) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => return Err(unavailable(format!("SEDAR+ did not return the {} form", service))),
        };
        let app = re_app().captures(&r.url).map(|c| c[1].to_string()).unwrap_or_else(|| "csa-party".into());
        let reference = format!("{}/{}/viewInstance/view.html?id={}", BASE, app, inst);
        Ok(View { app, inst, key, sid, page, reference })
    }

    fn headers(&self, asynchronous: bool) -> Vec<(&str, String)> {
        let mut h = vec![
            ("x-catalyst-session-global", self.sid.clone()),
            ("x-security-token", "null".to_string()),
            ("Referer", self.reference.clone()),
            ("Origin", BASE.to_string()),
            ("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8".to_string()),
        ];
        if asynchronous {
            h.push(("x-catalyst-async", "true".into()));
            h.push(("x-catalyst-secured", "true".into()));
            h.push(("X-Requested-With", "XMLHttpRequest".into()));
        }
        h
    }

    /// `_View.callback`: post the form back with one callback.
    #[allow(clippy::too_many_arguments)]
    fn callback(
        &self,
        st: &mut State,
        node: &str,
        name: &str,
        value: Option<&str>,
        extra: &[(&str, &str)],
        container: Option<&str>,
        json_frag: bool,
        html: Option<&str>,
    ) -> Fetched<String> {
        let source = html.unwrap_or(&self.page);
        let mut data: Vec<(String, String)> = if json_frag {
            vi_params(source)
        } else {
            form_fields(source).into_iter().filter(|(k, _)| !extra.iter().any(|(e, _)| e == k)).collect()
        };
        data.push(("_CBNODE_".into(), node.into()));
        data.push(("_CBNAME_".into(), name.into()));
        data.push(("_VIKEY_".into(), self.key.clone()));
        if let Some(v) = value {
            data.push(("_CBVALUE_".into(), v.into()));
        }
        if let Some(c) = container {
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            data.push(("_CBHTMLFRAG_".into(), "true".into()));
            data.push(("_CBHTMLFRAGID_".into(), ms.to_string()));
            data.push(("_CBHTMLFRAGNODEID_".into(), c.into()));
            data.push(("_CBASYNCUPDATE_".into(), "true".into()));
        }
        if json_frag {
            data.push(("_CBJSONFRAG_".into(), "true".into()));
        }
        for (k, v) in extra {
            data.push((k.to_string(), v.to_string()));
        }
        pace(st);
        let body = urlencode(&data);
        let headers = self.headers(container.is_some() || json_frag);
        let hdrs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let url = format!("{}/{}/viewInstance/update.html?id={}", BASE, self.app, self.inst);
        session(st)?
            .request("POST", &url, &hdrs, Some(&body), TIMEOUT, false)
            .map(|r| r.text())
            .map_err(|e| unavailable(format!("callback {}/{} failed: {}", node, name, e)))
    }

    /// `_View.refresh_identity`: adopt the new view instance after a
    /// navigation that pushed one.
    pub fn refresh_identity(&mut self, html: &str) -> bool {
        let inst = re_inst_url().captures(html).or_else(|| re_inst_page().captures(html)).map(|c| c[1].to_string());
        let key = re_key().captures(html).map(|c| c[1].to_string());
        if let (Some(inst), Some(key)) = (inst, key) {
            self.inst = inst;
            self.key = key;
            if let Some(sid) = re_sid().captures(html) {
                self.sid = sid[1].to_string();
            }
            self.reference = format!("{}/{}/viewInstance/view.html?id={}", BASE, self.app, self.inst);
            self.page = html.to_string();
            return true;
        }
        false
    }
}

// --- parsers (pure; the contract the tests pin down) -----------------------------

re!(re_tags, r"<[^>]+>");
re!(re_ws, r"\s+");

/// `sedar._text`.
pub fn text(s: &str) -> String {
    let t = re_tags().replace_all(s, " ");
    let t = unescape(&t);
    trim_space(&re_ws().replace_all(&t, " ")).to_string()
}

/// `sedar.filing_id`: the document's drmKey when the row carries one,
/// otherwise a short digest of the profile, file name and submitted time.
pub fn filing_id(url: &str, profile_no: &str, file: &str, submitted: &str) -> String {
    static DRM: OnceLock<Regex> = OnceLock::new();
    if let Some(m) = DRM.get_or_init(|| Regex::new(r"drmKey=([0-9a-f]+)").unwrap()).captures(url) {
        return format!("drm:{}", &m[1]);
    }
    let seed = [profile_no, file, submitted, url].join("|");
    let digest = openssl::sha::sha1(seed.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    format!("h:{}", &hex[..16])
}

re!(re_issuer, r#"appReceiveFocus">\s*([^<]*?\((\d{9})\))\s*</span>"#);
re!(re_doc_link, r#"(?s)<a class="appDocumentView appResourceLink appDocumentLink" href="([^"]+)"[^>]*>\s*<span>(.*?)</span>"#);
re!(re_submitted, r#"<span aria-hidden="true">\s*(\d{1,2} \w{3} \d{4}[^<]*?)\s*</span>"#);
re!(re_size, r"(?i)(\d[\d.,]* ?(?:KB|MB|bytes))");

/// `sedar.parse_filings`: document search rows into filings, as the page gives
/// them.
pub fn parse_filings(html: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for m in re_doc_link().captures_iter(html) {
        let whole = m.get(0).unwrap();
        let url = unescape(&m[1]);
        let before = &html[back_chars(html, whole.start(), 2600)..whole.start()];
        let after = &html[whole.end()..fwd_chars(html, whole.end(), 1400)];
        // the last issuer before this link is this row's
        let issuer = re_issuer().captures_iter(before).last();
        let sub = re_submitted().captures(after);
        let size = re_size().captures(after);
        let profile_no = issuer.as_ref().map(|c| c[2].to_string()).unwrap_or_default();
        let file = text(&m[2]);
        let submitted = sub.map(|c| trim_space(&c[1]).to_string()).unwrap_or_default();
        out.push(json!({
            "id": filing_id(&url, &profile_no, &file, &submitted),
            "issuer": issuer.as_ref().map(|c| text(&c[1])).unwrap_or_default(),
            "profileNo": profile_no,
            "file": file,
            "submitted": submitted,
            "submittedAt": iso(&submitted),
            "size": size.map(|c| c[1].to_string()).unwrap_or_default(),
            "url": url,
        }));
    }
    out
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `sedar._iso`: "13 Sep 2026 20:42 EDT" as "2026-09-13T20:42", the zone
/// dropped (it is only ever Eastern).
pub fn iso(submitted: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| Regex::new(r"^(\d{1,2}) (\w{3}) (\d{4})(?:\s+(\d{1,2}):(\d{2}))?").unwrap());
    let m = match r.captures(submitted) { Some(m) => m, None => return String::new() };
    let mon = match MONTHS.iter().position(|x| *x == &m[2]) { Some(i) => i + 1, None => return String::new() };
    let n = |i: usize| m.get(i).and_then(|g| parse_int(g.as_str())).unwrap_or(0);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}", n(3), mon, n(1), n(4), n(5))
}

re!(re_ri_row, r"(?s)<tr[^>]*appTblRow[^>]*>(.*?)</tr>");
re!(re_td, r"(?s)<td[^>]*>(.*?)</td>");
re!(re_nine, r"^\d{9}$");

/// `sedar.parse_reporting_issuers`: reporting-issuer rows, every field indexed
/// off the profile-number cell.
pub fn parse_reporting_issuers(html: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for row in re_ri_row().captures_iter(html) {
        let cells: Vec<String> = re_td().captures_iter(&row[1]).map(|c| text(&c[1])).collect();
        let idx = match cells.iter().position(|c| re_nine().is_match(c)) { Some(i) => i as i64, None => continue };
        let cell = |i: i64| if i >= 0 && (i as usize) < cells.len() { cells[i as usize].clone() } else { String::new() };
        out.push(json!({
            "name": cell(idx - 1),
            "profileNo": cells[idx as usize],
            "provinces": cell(idx + 3),
            "jurisdiction": cell(idx + 4),
            "type": cell(idx + 5),
        }));
    }
    out
}

// --- high-level operations -------------------------------------------------------

/// A lookup's outcome; `NotFound` is Python's `ProfileNotFound`.
pub enum Lookup {
    Found(Vec<Value>),
    NotFound,
}

/// `sedar.resolve_profile`: every reporting-issuer profile matching a name or
/// number, best first.
pub fn resolve_profile(query: &str) -> Fetched<Lookup> {
    let q = trim_space(query).to_string();
    if q.is_empty() {
        return Ok(Lookup::NotFound);
    }
    let html = {
        let mut st = state().lock().unwrap();
        let view = View::open(&mut st, "searchReportingIssuers")?;
        let (node, name, container) = search_action(&view.page)
            .ok_or_else(|| unavailable("could not find the reporting-issuer search control on the page"))?;
        view.callback(&mut st, &node, &name, None, &[("QueryString", &q)], Some(&container), false, None)?
    };
    let mut rows = parse_reporting_issuers(&html);
    if rows.is_empty() {
        return Ok(Lookup::NotFound);
    }
    Ok(Lookup::Found(rank(&mut rows, &q)))
}

/// The best-first order `resolve_profile` sorts matches into.
pub fn rank(rows: &mut [Value], q: &str) -> Vec<Value> {
    let ql = q.to_lowercase();
    let key = |r: &Value| {
        let name = r["name"].as_str().unwrap_or("").to_lowercase();
        (r["profileNo"].as_str().unwrap_or("") != q, !name.contains(&ql), !name.starts_with(&ql))
    };
    rows.sort_by_key(key);
    rows.to_vec()
}

/// `sedar.list_filings`: filings for one issuer, resolving the issuer from the
/// query when no profile number is given.
pub fn list_filings(query: Option<&str>, profile_no: Option<&str>, limit: usize) -> Fetched<Option<Value>> {
    let mut profile: Option<Value> = None;
    let mut profile_no = profile_no.filter(|p| !p.is_empty()).map(|p| p.to_string());
    if profile_no.is_none() {
        if let Some(q) = query.filter(|q| !q.is_empty()) {
            match resolve_profile(q)? {
                Lookup::NotFound => return Ok(None),
                Lookup::Found(matches) => {
                    profile_no = Some(matches[0]["profileNo"].as_str().unwrap_or("").to_string());
                    profile = Some(matches[0].clone());
                }
            }
        }
    }
    let mut scoped = Value::Null;
    let html = {
        let mut st = state().lock().unwrap();
        match &profile_no {
            Some(p) => {
                let name = profile.as_ref().and_then(|x| x["name"].as_str()).map(|s| s.to_string()).or_else(|| query.map(|s| s.to_string()));
                match scoped_documents(&mut st, p, name.as_deref()) {
                    Some(h) => {
                        scoped = json!(true);
                        h
                    }
                    None => {
                        scoped = json!(false);
                        View::open(&mut st, "searchDocuments")?.page
                    }
                }
            }
            None => View::open(&mut st, "searchDocuments")?.page,
        }
    };
    let mut filings = parse_filings(&html);
    if let Some(p) = &profile_no {
        filings.retain(|f| {
            let no = f["profileNo"].as_str().unwrap_or("");
            no.is_empty() || no == p
        });
    }
    filings.truncate(limit.max(1));
    let profile_out = match (profile, &profile_no) {
        (Some(pr), _) => pr,
        (None, Some(p)) => json!({"profileNo": p}),
        (None, None) => Value::Null,
    };
    Ok(Some(json!({"profile": profile_out, "scoped": scoped, "filings": filings})))
}

/// `sedar._scoped_documents`: the issuer's document page, reused for a short
/// window so a run of its documents walks the chain once.
fn scoped_documents(st: &mut State, profile_no: &str, name: Option<&str>) -> Option<String> {
    if let Some(h) = st.scope.fresh(profile_no) {
        return Some(h);
    }
    let html = scoped_documents_uncached(st, profile_no, name).ok().flatten();
    st.scope.keep(profile_no, &html);
    html
}

/// `sedar._scoped_documents_uncached`: search the reporting-issuer list for
/// the profile, open the issuer, and follow its documents link; each step
/// pushes a new view instance.
fn scoped_documents_uncached(st: &mut State, profile_no: &str, name: Option<&str>) -> Fetched<Option<String>> {
    let mut view = View::open(st, "searchReportingIssuers")?;
    let (node, cbname, container) = match search_action(&view.page) { Some(a) => a, None => return Ok(None) };
    let ri = view.callback(st, &node, &cbname, None, &[("QueryString", profile_no)], Some(&container), false, None)?;
    let issuer_node = match issuer_menu_node(&ri, name) { Some(n) => n, None => return Ok(None) };
    let profile_page = view.callback(st, &issuer_node, "invokeMenuCb", None, &[], None, false, Some(&ri))?;
    view.refresh_identity(&profile_page);
    let docs_node = match docs_menu_node(&view.page) { Some(n) => n, None => return Ok(None) };
    let page = view.page.clone();
    let docs = view.callback(st, &docs_node, "invokeMenuCb", None, &[], None, false, Some(&page))?;
    view.refresh_identity(&docs);
    if view.page.contains("appDocumentLink") {
        return Ok(Some(view.page.clone()));
    }
    if let Some((a, b, c)) = search_action(&view.page) {
        let page = view.page.clone();
        return Ok(Some(view.callback(st, &a, &b, None, &[], Some(&c), false, Some(&page))?));
    }
    Ok(Some(view.page.clone()))
}

/// `sedar.newest`: the newest filings across SEDAR+.
pub fn newest(limit: usize) -> Fetched<Vec<Value>> {
    let html = {
        let mut st = state().lock().unwrap();
        View::open(&mut st, "searchDocuments")?.page
    };
    let mut f = parse_filings(&html);
    f.truncate(limit.max(1));
    Ok(f)
}

/// `sedar._is_document`: a real document, not the site's HTML error page.
fn is_document(a: &crate::browser::Answer) -> bool {
    let ct = a.header("content-type").unwrap_or("").to_lowercase();
    if a.status != 200 || a.body.is_empty() {
        return false;
    }
    if ct.contains("text/html") {
        return false;
    }
    let trimmed = a.body.iter().position(|b| !b" \t\n\r\x0b\x0c".contains(b)).map(|i| &a.body[i..]).unwrap_or(&[]);
    trimmed.first() != Some(&b'<')
}

/// `sedar._download_bytes`: re-scope to the profile, match the document by its
/// drmKey, fetch it in that live session. A document URL is bound to the
/// session that minted it, so it is re-minted here rather than reused.
pub fn download_bytes(profile_no: &str, doc_id: &str, name: Option<&str>) -> Fetched<Option<(Vec<u8>, String)>> {
    let key = doc_id.rsplit(':').next().unwrap_or("").to_string();
    let answer = {
        let mut st = state().lock().unwrap();
        let html = scoped_documents(&mut st, profile_no, name)
            .ok_or_else(|| unavailable("could not open the profile's documents to download from"))?;
        let row = parse_filings(&html).into_iter().find(|f| !key.is_empty() && f["url"].as_str().unwrap_or("").contains(&key));
        let row = match row { Some(r) => r, None => return Ok(None) };
        let url = unescape(row["url"].as_str().unwrap_or(""));
        let referer = format!("{}/csa-party/viewInstance/view.html", BASE);
        session(&mut st)?
            .request("GET", &url, &[("Referer", &referer)], None, DOC_TIMEOUT, true)
            .map_err(|e| unavailable(format!("document fetch failed: {}", e)))?
    };
    if !is_document(&answer) {
        // the scoped URLs went stale; re-walk next time
        state().lock().unwrap().scope.remove(profile_no);
        return Err(unavailable(format!("document did not download (status {})", answer.status)));
    }
    let ct = answer.header("content-type").unwrap_or("application/pdf").to_string();
    Ok(Some((answer.body, ct)))
}

// --- the provider interface ------------------------------------------------------

/// `sedar.covers`: Canadian listings; a US listing is EDGAR's.
pub fn covers(_symbol: &str, exchange: &str, currency: &str) -> bool {
    let ex = exchange.to_uppercase();
    let cur = currency.to_uppercase();
    if cur == "USD" || ["NASDAQ", "NYSE", "AMEX", "ARCA", "US"].contains(&ex.as_str()) {
        return false;
    }
    cur == "CAD" || CA_EXCHANGES.contains(&ex.as_str()) || (ex.is_empty() && cur.is_empty())
}

/// `sedar._sedar_category`: a document name in the shared vocabulary.
pub fn category(file: &str) -> &'static str {
    let f = file.to_lowercase();
    let any = |ks: &[&str]| ks.iter().any(|k| f.contains(k));
    if f.contains("news release") || f.contains("press release") {
        return d::NEWS;
    }
    if any(&["md&a", "financial statement", "annual report", "interim", "certification", "52-109", "financial report"]) {
        return d::FINANCIALS;
    }
    if f.contains("material change") {
        return d::EVENTS;
    }
    if any(&["circular", "proxy", "voting results", "meeting", "information circular"]) {
        return d::GOVERNANCE;
    }
    if any(&["prospectus", "offering", "45-106", "exempt distribution", "45-102", "rights offering", "45-108"]) {
        return d::OFFERINGS;
    }
    if any(&["insider", "early warning", "45-101", "issuer bid"]) {
        return d::INSIDER;
    }
    d::OTHER
}

/// `sedar.categorize`.
pub fn categorize(row: &Value) -> String {
    category(row.get("type").and_then(|v| v.as_str()).unwrap_or("")).to_string()
}

/// `sedar._split_type_title`: a document file name into a type and the
/// language qualifier beside it.
pub fn split_type_title(file: &str) -> (String, String) {
    static PDF: OnceLock<Regex> = OnceLock::new();
    static TAIL: OnceLock<Regex> = OnceLock::new();
    static PAREN: OnceLock<Regex> = OnceLock::new();
    let no_pdf = PDF.get_or_init(|| Regex::new(r"(?i)\.pdf$").unwrap()).replace_all(file, "");
    let name = trim_space(&no_pdf).to_string();
    for rx in [
        TAIL.get_or_init(|| Regex::new(r"(?i)[-–]\s*(English|French)\s*$").unwrap()),
        PAREN.get_or_init(|| Regex::new(r"(?i)\((English|French)\)\s*$").unwrap()),
    ] {
        if let Some(m) = rx.captures(&name) {
            let start = m.get(0).unwrap().start();
            let typ = name[..start].trim_matches([' ', '-', '–']).to_string();
            let lang = &m[1];
            // str.title(): the first letter up, the rest down
            let mut ch = lang.chars();
            let titled = format!("{}{}", ch.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default(), ch.as_str().to_lowercase());
            return (typ, format!("({})", titled));
        }
    }
    (name, String::new())
}

/// `sedar._to_item`.
pub fn to_item(raw: &Value, profile_no: &str) -> Value {
    let g = |k: &str| raw.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (typ, title) = split_type_title(&g("file"));
    let pno = { let p = g("profileNo"); if p.is_empty() { profile_no.to_string() } else { p } };
    json!({
        "id": format!("sedar:{}", g("id")),
        "source": SOURCE,
        "category": category(&g("file")),
        "date": g("submittedAt"),
        "dateText": g("submitted"),
        "type": typ,
        "title": title,
        "size": g("size"),
        "url": g("url"),
        "issuer": g("issuer"),
        "profileNo": pno,
    })
}

/// `sedar.fetch`: one Canadian issuer's filings as disclosure items.
pub fn fetch(symbol: &str, name: &str, _exchange: &str, _currency: &str, limit: usize, profile_no: &str) -> Fetched<Vec<Value>> {
    let query = if name.is_empty() { symbol } else { name };
    let result = match list_filings(Some(query), Some(profile_no), limit)? {
        Some(r) => r,
        None => return Ok(vec![]),
    };
    let pno = result["profile"].get("profileNo").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Ok(result["filings"].as_array().cloned().unwrap_or_default().iter().map(|r| to_item(r, &pno)).collect())
}

/// `sedar.document`: one stored row's document. (bytes, content type).
pub fn document(row: &Value) -> Fetched<(Vec<u8>, String)> {
    let g = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let issuer = g("issuer");
    match download_bytes(&g("profileNo"), &g("id"), if issuer.is_empty() { None } else { Some(&issuer) })? {
        Some(x) => Ok(x),
        None => Err(SourceError::Other(format!("ProfileNotFound: no document {} in profile {}", d::repr_quoted(&g("id")), g("profileNo")))),
    }
}
