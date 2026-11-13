//! The person's tax slips read from Wealthsimple's Documents (brief 82, PR 1;
//! `SPEC.md` tax-slips-1 to tax-slips-4): the list asked as the Documents page asks
//! it, each slip's file opened by its link, its text laid out by the app's own
//! `pdftotext -layout` page by page, and kept once. Every slip and every reply here
//! is made up. Each slip's file is a PDF made from one of the made-up slips
//! (`tests/replies/slips`), so its text comes back as `pdftotext` lays a file out,
//! never as the text it was made from.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use bagholder_book::Book;
use bagholder_broker::{Answer, Failure};
use bagholder_core::json::{self, Value};
use bagholder_core::{Broker, ConnectionId};
use bagholder_net::{Ask, Limiter, ManualClock, Net, NetError, Transport};
use bagholder_wealthsimple::documents::{self, Documents, SlipKind};
use bagholder_wealthsimple::slips;

fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

fn replies() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/slips")
}

fn slip_text(name: &str) -> String {
    std::fs::read_to_string(replies().join(format!("{name}.txt"))).unwrap()
}

fn kind_of(name: &str) -> SlipKind {
    SlipKind::of(name.split('-').next().unwrap()).unwrap()
}

/// A character as Windows-1252 writes it (the PDF's `WinAnsiEncoding`), `?` where
/// it has none.
fn cp1252(c: char) -> u8 {
    match c {
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => c as u32 as u8,
        '€' => 0x80,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        _ => b'?',
    }
}

/// A PDF of `text`: each page a page (a form feed ends one), each line at its own
/// row in Courier, which `pdftotext -layout` lays out as columns again.
fn pdf_of(text: &str) -> Vec<u8> {
    // ten points a character's height, six its width (Courier's is 0.6 em), twelve a row
    const SIZE: usize = 10;
    const ADVANCE: usize = 6;
    const LEAD: usize = 12;
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec()];
    let mut pages = Vec::new();
    for page in text.split('\u{c}') {
        let lines: Vec<&str> = page.split('\n').collect();
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) * ADVANCE + 40;
        let height = lines.len() * LEAD + 40;
        let mut ops = format!("BT /F1 {SIZE} Tf\n").into_bytes();
        for (i, line) in lines.iter().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            ops.extend(format!("1 0 0 1 20 {} Tm (", height - 20 - (i + 1) * LEAD).into_bytes());
            for c in line.chars() {
                if matches!(c, '(' | ')' | '\\') {
                    ops.push(b'\\');
                }
                ops.push(cp1252(c));
            }
            ops.extend(b") Tj\n");
        }
        ops.extend(b"ET");
        let mut stream = format!("<< /Length {} >>\nstream\n", ops.len()).into_bytes();
        stream.extend(ops);
        stream.extend(b"\nendstream");
        objects.push(stream);
        pages.push((objects.len(), width, height));
    }
    let pages_at = objects.len() + pages.len() + 1;
    let mut kids = Vec::new();
    for (content, w, h) in &pages {
        objects.push(format!("<< /Type /Page /Parent {pages_at} 0 R /MediaBox [0 0 {w} {h}] /Resources << /Font << /F1 1 0 R >> >> /Contents {content} 0 R >>").into_bytes());
        kids.push(format!("{} 0 R", objects.len()));
    }
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), kids.len()).into_bytes());
    assert_eq!(objects.len(), pages_at);
    objects.push(format!("<< /Type /Catalog /Pages {pages_at} 0 R >>").into_bytes());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).into_bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").into_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root {} 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1, objects.len()).into_bytes());
    out
}

/// The app's own step from a file to its text.
fn pages(bytes: &[u8]) -> Option<String> {
    bagholder_market::pdftext::layout_pages(bytes)
}

/// The slips are read with `pdftotext`, as the app's image carries it: a machine
/// without it reads none, and these tests say so rather than pass.
fn need_pdftotext() {
    assert!(pages(&pdf_of("a page")).is_some(), "pdftotext (poppler-utils) is needed to read a tax slip's file; install it to run these tests");
}

/// An entry of the documents list, as the Documents page's reply states one.
fn entry(id: &str, kind: &str, account: &str, year: i16, created: &str, key: &str) -> Value {
    json::parse(&format!(
        r#"{{"id":"{id}","createdAt":"{created}","availableAt":"{day}","displayAt":null,"filename":null,"period":"{year}-01-01","frequency":"year","type":"{kind}","downloadUrl":null,"uploaderName":null,"s3BucketName":"made-up-bucket","s3Key":"{key}","category":"tax","account":{{"id":"{account}","type":"non_registered"}},"documents":null}}"#,
        day = &created[..10]
    ))
    .unwrap()
}

fn link(key: &str) -> String {
    format!("https://files.example.com/{key}?signature=made-up")
}

/// Wealthsimple's Documents, made up: its list, and each file by the link it signs.
#[derive(Default)]
struct Stand {
    list: Vec<Value>,
    files: BTreeMap<String, Vec<u8>>,
    downloaded: Vec<String>,
    lapsed: bool,
}

impl Documents for Stand {
    fn tax_documents(&mut self) -> Answer<Vec<Value>> {
        Ok(self.list.clone())
    }
    fn signed_link(&mut self, bucket: Option<&str>, key: &str) -> Answer<String> {
        assert_eq!(bucket, Some("made-up-bucket"));
        Ok(link(key))
    }
    fn download(&mut self, url: &str) -> Answer<Vec<u8>> {
        if self.lapsed {
            return Err(Failure::Lapsed("made up".into()));
        }
        self.downloaded.push(url.to_string());
        self.files.get(url).cloned().ok_or_else(|| Failure::Refused(format!("no file at {url}")))
    }
}

impl Stand {
    fn slip(&mut self, id: &str, name: &str, text: &str, account: &str, created: &str) {
        let kind = kind_of(name);
        let year = slips::read(kind, &slip_text(name)).unwrap().year();
        let key = format!("{id}-key");
        self.list.push(entry(id, kind.name(), account, year, created, &key));
        self.files.insert(link(&key), pdf_of(text));
    }
}

fn book() -> (tempfile::TempDir, Book, ConnectionId) {
    let home = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", at("2026-03-01T12:00:00Z")).unwrap();
    let conn = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", at("2026-03-01T12:00:00Z")).unwrap();
    (home, book, conn)
}

const NAMES: [&str; 10] = ["t3-made-up-a", "t3-made-up-b", "t3-made-up-c", "t5-made-up-a", "t5-made-up-b", "t5-made-up-c", "t5008-made-up-a", "t5008-made-up-b", "t5008-made-up-c", "t5008-made-up-d"];

/// Every slip the list states is read from its file once, kept with its pages
/// apart, and reads as the slip it was made from; a document of a kind no reader
/// holds is passed over, its file never asked for; the next read asks for no file
/// it holds.
#[test]
fn each_slip_the_list_states_is_read_from_its_file_once_and_kept() {
    need_pdftotext();
    let mut stand = Stand::default();
    for (i, n) in NAMES.iter().enumerate() {
        stand.slip(&format!("made-up-document-{i}"), n, &slip_text(n), &format!("made-up-account-{i}"), "2026-03-01T10:00:00.000Z");
    }
    stand.list.push(entry("made-up-fees", "crm2_fee", "made-up-account-0", 2025, "2026-02-20T10:00:00.000Z", "made-up-fees-key"));
    let (_home, book, conn) = book();
    let kept = documents::keep_new(&book, conn, &mut stand, &pages, at("2026-03-02T22:00:00Z")).unwrap();
    assert_eq!(kept.unread, Vec::<(String, String)>::new());
    assert_eq!(kept.new.iter().map(|(s, r)| (s.id.clone(), r.clone())).collect::<Vec<_>>(), (0..NAMES.len()).map(|i| (format!("made-up-document-{i}"), Ok(()))).collect::<Vec<_>>());
    assert_eq!(stand.downloaded.len(), NAMES.len());
    let stored = book.tax_slips(conn).unwrap();
    assert_eq!(stored.len(), NAMES.len());
    for (i, n) in NAMES.iter().enumerate() {
        let s = stored.iter().find(|s| s.document == format!("made-up-document-{i}")).unwrap();
        let kind = kind_of(n);
        assert_eq!((s.kind.as_str(), s.account_key.clone()), (kind.name(), format!("made-up-account-{i}")));
        assert!(s.text.contains('\u{c}'), "{n}: kept with its pages apart");
        assert!(!s.text.contains("Samplesworth") && !s.text.contains("Sampletown"), "{n}: its recipient's name and address not kept");
        assert!(!s.text.split_whitespace().any(|w| w.len() == 9 && w.chars().all(|c| c.is_ascii_digit())), "{n}: no social insurance number kept");
        assert_eq!(slips::read(kind, &s.text).unwrap(), slips::read(kind, &slip_text(n)).unwrap(), "{n}");
    }
    let again = documents::keep_new(&book, conn, &mut stand, &pages, at("2026-03-03T22:00:00Z")).unwrap();
    assert!(again.new.is_empty() && again.unread.is_empty());
    assert_eq!(stand.downloaded.len(), NAMES.len(), "no file held is asked for again");
    assert_eq!(book.last_read(conn, documents::READ_PART).unwrap(), Some(at("2026-03-03T22:00:00Z")));
}

/// A slip Wealthsimple amends is a document of its own: for its account, kind
/// and year the one made last is the slip, whichever was kept first, and the one
/// before stays kept.
#[test]
fn a_slip_amended_is_the_slip_and_the_one_before_stays_kept() {
    need_pdftotext();
    for amended_first in [false, true] {
        let mut stand = Stand::default();
        let (original, amended) = (("made-up-original", "2026-02-26T09:00:00.000Z"), ("made-up-amended", "2026-04-08T15:30:00.000Z"));
        let order = if amended_first { [amended, original] } else { [original, amended] };
        for (id, created) in order {
            let name = if id == amended.0 { "t5008-made-up-d" } else { "t5008-made-up-c" };
            stand.slip(id, name, &slip_text(name), "made-up-account-1", created);
        }
        let (_home, book, conn) = book();
        documents::keep_new(&book, conn, &mut stand, &pages, at("2026-04-09T22:00:00Z")).unwrap();
        assert_eq!(book.tax_slips(conn).unwrap().iter().map(|s| s.document.as_str()).collect::<Vec<_>>(), vec!["made-up-amended"]);
        assert_eq!(book.tax_slip_documents(conn).unwrap(), BTreeSet::from(["made-up-amended".to_string(), "made-up-original".to_string()]));
    }
}

/// A file with no text to read is not kept, and is read again at the next read;
/// a slip its reader refuses, or one that states another year than its document,
/// is kept as it came, its refusal said.
#[test]
fn a_file_with_no_text_is_read_again_and_a_slip_refused_is_kept_as_it_came() {
    need_pdftotext();
    let mut stand = Stand::default();
    stand.slip("made-up-document-1", "t5-made-up-a", &slip_text("t5-made-up-a"), "made-up-account-1", "2026-02-27T10:00:00.000Z");
    let url = link("made-up-document-1-key");
    let file = stand.files.insert(url.clone(), b"<html>an error page</html>".to_vec()).unwrap();
    let (_home, book, conn) = book();
    let kept = documents::keep_new(&book, conn, &mut stand, &pages, at("2026-02-28T22:00:00Z")).unwrap();
    assert!(kept.new.is_empty());
    assert_eq!(kept.unread.len(), 1);
    assert!(kept.unread[0].1.contains("has no text"), "{:?}", kept.unread);
    assert!(book.tax_slips(conn).unwrap().is_empty());
    stand.files.insert(url, file);
    let kept = documents::keep_new(&book, conn, &mut stand, &pages, at("2026-03-01T22:00:00Z")).unwrap();
    assert_eq!(kept.new.len(), 1);

    // one sale's proceeds raised by a thousand dollars: the slip's totals no longer hold
    let t = slip_text("t5008-made-up-c");
    let i = t.find(" CAD").and_then(|i| t[i + 4..].find('$').map(|j| i + 4 + j)).unwrap();
    stand.slip("made-up-document-2", "t5008-made-up-c", &format!("{}$1{}", &t[..i], &t[i + 1..]), "made-up-account-2", "2026-02-27T10:00:00.000Z");
    // a slip under a document of the year after its own
    let mut other = entry("made-up-document-3", "t3", "made-up-account-3", 2025, "2026-03-13T10:00:00.000Z", "made-up-document-3-key");
    if let Value::Object(m) = &mut other {
        m.insert("period".into(), Value::String("2026-01-01".into()));
    }
    stand.list.push(other);
    stand.files.insert(link("made-up-document-3-key"), pdf_of(&slip_text("t3-made-up-a")));
    let kept = documents::keep_new(&book, conn, &mut stand, &pages, at("2026-03-14T22:00:00Z")).unwrap();
    let said: BTreeMap<String, String> = kept.new.iter().map(|(s, r)| (s.id.clone(), r.clone().unwrap_err())).collect();
    assert!(said["made-up-document-2"].contains("proceeds come to"), "{said:?}");
    assert_eq!(said["made-up-document-3"], "the slip states the year 2025, its document 2026");
    let stored = book.tax_slips(conn).unwrap();
    assert_eq!(stored.len(), 3, "a refused slip is kept as it came");
    assert!(stored.iter().all(|s| !s.text.contains("Samplesworth")), "a refused slip is kept without its recipient's name and address");
}

/// A sign-in that lapses keeps nothing and notes no read: the next read asks again.
#[test]
fn a_lapsed_sign_in_keeps_nothing_and_notes_no_read() {
    let mut stand = Stand { lapsed: true, ..Stand::default() };
    stand.list.push(entry("made-up-document-1", "t3", "made-up-account-1", 2025, "2026-03-13T10:00:00.000Z", "made-up-key-1"));
    let (_home, book, conn) = book();
    let e = documents::keep_new(&book, conn, &mut stand, &pages, at("2026-03-14T22:00:00Z")).unwrap_err();
    assert!(e.contains("lapsed"), "{e}");
    assert!(book.tax_slip_documents(conn).unwrap().is_empty());
    assert_eq!(book.last_read(conn, documents::READ_PART).unwrap(), None);
}

// -- the client, over a made-up Wealthsimple ------------------------------------

/// One request as it was sent.
#[derive(Clone, Debug)]
struct Sent {
    method: String,
    url: String,
    authorized: bool,
    body: Option<Value>,
}

/// Wealthsimple made up: the documents list a page at a time, a signed link for
/// any stored file, and each file at its link.
struct Wire {
    sent: Arc<Mutex<Vec<Sent>>>,
    entries: Vec<Value>,
    files: BTreeMap<String, Vec<u8>>,
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    match v {
        Value::Object(m) => m.get(key).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}

fn number(v: &Value) -> usize {
    match v {
        Value::Number(n) => n.parse().unwrap(),
        other => panic!("not a number: {other:?}"),
    }
}

impl Transport for Wire {
    fn answer(&self, ask: &Ask) -> Result<bagholder_net::Answer, NetError> {
        let body = ask.body.map(|b| json::parse(std::str::from_utf8(b).unwrap()).unwrap());
        let authorized = ask.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
        self.sent.lock().unwrap().push(Sent { method: ask.method.to_string(), url: ask.url.to_string(), authorized, body: body.clone() });
        let Some(body) = body else {
            return Ok(match self.files.get(ask.url) {
                Some(f) => (200, ask.url.to_string(), vec![], f.clone()),
                None => (403, ask.url.to_string(), vec![], b"made up: no such file".to_vec()),
            });
        };
        let reply = match field(&body, "operationName") {
            Value::String(op) if op == "FetchIdentityPaginatedDocuments" => {
                let vars = field(&body, "variables");
                let (offset, limit) = (number(field(vars, "offset")), number(field(vars, "limit")));
                let page: Vec<Value> = self.entries.iter().skip(offset).take(limit).cloned().collect();
                format!(r#"{{"data":{{"identity":{{"id":"identity-1","documents":{{"totalCount":{},"offset":{offset},"results":{}}}}}}}}}"#, self.entries.len(), Value::Array(page).canonical())
            }
            Value::String(op) if op == "DocumentSignedUrlCreate" => {
                let key = match field(field(&body, "variables"), "key") {
                    Value::String(k) => k.clone(),
                    other => panic!("a key: {other:?}"),
                };
                std::fs::read_to_string(replies().join("made-up-signed-link.json")).unwrap().replace("made-up-key-1", &key)
            }
            other => panic!("not an operation this test answers: {other:?}"),
        };
        Ok((200, ask.url.to_string(), vec![], reply.into_bytes()))
    }
}

fn client_over(wire: Wire) -> (tempfile::TempDir, Net) {
    let net = Net::answered_by(Arc::new(ManualClock::at(at("2026-03-14T12:00:00Z"))), Arc::new(Limiter::new()), Box::new(wire));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.json"), r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2026-03-14T13:00:00Z","wssdi":"device-1"}"#).unwrap();
    (dir, net)
}

fn made_up_list() -> Vec<Value> {
    let page = json::parse(&std::fs::read_to_string(replies().join("made-up-documents.json")).unwrap()).unwrap();
    match field(field(field(field(&page, "data"), "identity"), "documents"), "results") {
        Value::Array(a) => a.clone(),
        other => panic!("{other:?}"),
    }
}

/// The list is asked a page of fifty at a time, its tax category alone, until
/// every entry it counts has come; a stored file by the link Wealthsimple signs
/// for it, its bucket where it states one; each file at its link with the
/// sign-in never sent, and a link that is not https never asked.
#[test]
fn the_list_and_each_file_are_asked_as_the_documents_page_asks_them() {
    let mut entries = made_up_list();
    // fifty more entries of a kind no reader holds: a second page
    for i in 0..50 {
        entries.push(entry(&format!("made-up-fees-{i}"), "crm2_fee", "made-up-account-1", 2025, "2026-02-20T10:00:00.000Z", &format!("made-up-fees-key-{i}")));
    }
    let files = BTreeMap::from([
        (link("made-up-key-1"), b"%PDF-made-up-t3".to_vec()),
        ("https://files.example.com/made-up-document-3?signature=made-up".to_string(), b"%PDF-made-up-t5008".to_vec()),
        (link("made-up-key-4"), b"%PDF-made-up-t5".to_vec()),
    ]);
    let sent = Arc::new(Mutex::new(Vec::new()));
    let (dir, net) = client_over(Wire { sent: sent.clone(), entries: entries.clone(), files: files.clone() });
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    let text = |b: &[u8]| Some(String::from_utf8_lossy(b).into_owned());
    let found = documents::read_new(&mut client, &BTreeSet::new(), &text).unwrap();
    assert_eq!(found.unread, Vec::<(String, String)>::new());
    assert_eq!(
        found.fetched.iter().map(|f| (f.slip.id.as_str(), f.slip.kind, f.slip.year, f.slip.account.as_str(), f.text.as_str())).collect::<Vec<_>>(),
        vec![
            ("made-up-document-1", SlipKind::T3, 2025, "made-up-account-1", "%PDF-made-up-t3"),
            ("made-up-document-3", SlipKind::T5008, 2025, "made-up-account-1", "%PDF-made-up-t5008"),
            ("made-up-document-4", SlipKind::T5, 2025, "made-up-account-2", "%PDF-made-up-t5"),
        ]
    );
    let sent = sent.lock().unwrap().clone();
    let lists: Vec<&Sent> = sent.iter().filter(|s| s.body.as_ref().is_some_and(|b| field(b, "operationName") == &Value::String("FetchIdentityPaginatedDocuments".into()))).collect();
    assert_eq!(lists.len(), 2, "two pages");
    for (s, offset) in lists.iter().zip([0, 50]) {
        let vars = field(s.body.as_ref().unwrap(), "variables");
        assert!(s.authorized);
        assert_eq!(field(vars, "id"), &Value::String("identity-1".into()));
        assert_eq!((number(field(vars, "offset")), number(field(vars, "limit"))), (offset, 50));
        assert_eq!(field(vars, "categories"), &Value::Array(vec![Value::String("tax".into())]));
        assert_eq!(field(vars, "locale"), &Value::String("en-CA".into()));
    }
    let signed: Vec<Value> = sent.iter().filter_map(|s| s.body.as_ref()).filter(|b| field(b, "operationName") == &Value::String("DocumentSignedUrlCreate".into())).map(|b| field(b, "variables").clone()).collect();
    assert_eq!(signed, vec![json::parse(r#"{"bucket":"made-up-bucket","key":"made-up-key-1"}"#).unwrap(), json::parse(r#"{"key":"made-up-key-4"}"#).unwrap()]);
    let gets: Vec<(&str, bool)> = sent.iter().filter(|s| s.method == "GET").map(|s| (s.url.as_str(), s.authorized)).collect();
    assert_eq!(gets.len(), 3);
    assert_eq!(gets.iter().map(|(u, a)| (u.to_string(), *a)).collect::<BTreeSet<_>>(), files.keys().map(|k| (k.clone(), false)).collect::<BTreeSet<_>>(), "each file at its link, the sign-in never sent with it");

    // a stated link that is not https is never asked
    let mut plain = made_up_list();
    if let Value::Object(m) = &mut plain[2] {
        m.insert("downloadUrl".into(), Value::String("http://files.example.com/made-up-document-3".into()));
    }
    let sent = Arc::new(Mutex::new(Vec::new()));
    let (dir, net) = client_over(Wire { sent: sent.clone(), entries: plain, files });
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    let found = documents::read_new(&mut client, &BTreeSet::new(), &text).unwrap();
    assert_eq!(found.unread.iter().map(|(d, _)| d.as_str()).collect::<Vec<_>>(), vec!["made-up-document-3"]);
    assert!(found.unread[0].1.contains("not a link its files are read from"), "{:?}", found.unread);
    assert!(!sent.lock().unwrap().iter().any(|s| s.url.starts_with("http://")));
}

// -- when the list is read ------------------------------------------------------

/// Brief 82's load, over every day of five years and runs of days the app is
/// off: the first read takes every past year; then, each year, every day the app
/// runs from 1 February reads once, until the first read on or after 1 April,
/// when the year just ended's slips are all in; no other day reads.
#[test]
fn the_list_is_read_once_then_daily_from_february_until_the_year_s_slips_are_all_in() {
    // the app off on a varying run of days: a made-up pattern, the same each run
    let off = |n: i64| (n * 7919 % 31) < 9 || (40..75).contains(&(n % 365));
    for start in ["2024-01-01", "2024-02-15", "2024-04-01", "2024-07-09", "2024-12-31"] {
        let first: jiff::civil::Date = start.parse().unwrap();
        let mut last: Option<jiff::civil::Date> = None;
        let mut reads = Vec::new();
        let mut day = first;
        let mut n = 0;
        while day.year() < 2029 {
            if (day == first || !off(n)) && documents::list_due(last, day) {
                reads.push(day);
                last = Some(day);
            }
            day = day.tomorrow().unwrap();
            n += 1;
        }
        assert_eq!(reads.first(), Some(&first), "{start}: the first day the app runs reads every past year");
        for year in first.year()..2029 {
            let feb = jiff::civil::date(year, 2, 1);
            let apr = jiff::civil::date(year, 4, 1);
            let in_year: Vec<_> = reads.iter().copied().filter(|d| d.year() == year && *d != first).collect();
            // every day the app runs from February to March reads, once (after the first read)
            let ran: Vec<_> = (0..).map(|i| feb.checked_add(jiff::Span::new().days(i)).unwrap()).take_while(|d| *d < apr).filter(|d| *d > first && !off(d.since(first).unwrap().get_days() as i64)).collect();
            assert_eq!(in_year.iter().copied().filter(|d| *d < apr).collect::<Vec<_>>(), ran, "{start}, {year}: daily from February");
            // one read on or after 1 April, the first day the app runs, unless the
            // first read already fell on or after it that year
            let after: Vec<_> = in_year.iter().copied().filter(|d| *d >= apr).collect();
            let first_after = (0..).map(|i| apr.checked_add(jiff::Span::new().days(i)).unwrap()).take_while(|d| d.year() == year).find(|d| *d > first && !off(d.since(first).unwrap().get_days() as i64));
            let expect: Vec<_> = if first.year() == year && first >= apr { vec![] } else { first_after.into_iter().collect() };
            assert_eq!(after, expect, "{start}, {year}: once its slips are all in, then none");
        }
    }
}
