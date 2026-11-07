//! The recorded replies (`tests/replies/<source>/`): real answers captured from
//! each source, and beside them copies edited by hand, named `wrong-shape-…` and
//! `wrong-meaning-…` for what is wrong, or `edited-…` for a real answer moved to
//! another moment (its session, its time). A reply captured with a status other than
//! 200 is named `…-status-<code>…`.

#![allow(dead_code)]

use std::path::PathBuf;

use bagholder_core::json::Value;
use bagholder_sources::{ask, reply};

pub fn dir(source: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies").join(source)
}

pub fn read(source: &str, name: &str) -> String {
    std::fs::read_to_string(dir(source).join(name)).unwrap_or_else(|e| panic!("{source}/{name}: {e}"))
}

pub fn json(source: &str, name: &str) -> Value {
    reply::parse(&read(source, name)).unwrap()
}

/// The real answers of a source whose name starts with `prefix`.
pub fn answers(source: &str, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir(source))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".json") && !n.starts_with("wrong-") && !n.contains("-status-"))
        .collect();
    out.sort();
    out
}

/// The shape committed in `shapes/<file>` is the recorded answers' shape: the
/// union of their paths, each marked with whether every answer that could carry
/// it does (`BAGHOLDER_BLESS=1` writes it).
pub fn shape_is_the_answers_union(file: &str, source: &str, prefix: &str, keyed: &[reply::Keyed]) {
    shape_is_the_union_of(file, &[(source, prefix)], keyed);
}

/// The same, over the recorded answers of several sources one reader reads with
/// one shape (the Bank's daily series and its noon archive).
pub fn shape_is_the_union_of(file: &str, sources: &[(&str, &str)], keyed: &[reply::Keyed]) {
    let mut shapes = Vec::new();
    for (source, prefix) in sources {
        let names = answers(source, prefix);
        assert!(!names.is_empty(), "no recorded answers for {source}/{prefix}");
        shapes.extend(names.iter().map(|n| reply::shape_keyed(&json(source, n), keyed)));
    }
    let union = reply::recorded(shapes.into_iter());
    let text = ask::shape_text(&union);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shapes").join(file);
    if std::env::var("BAGHOLDER_BLESS").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{} is not the union of the recorded answers", path.display());
}

/// The headers captured with a reply, from `<name>.headers` (curl's `-D`, the
/// last response's block), none where there is no such file.
pub fn headers(source: &str, name: &str) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(dir(source).join(format!("{name}.headers"))) else { return vec![] };
    let text = text.replace('\r', "");
    let last = text.trim_end().rsplit("\n\n").next().unwrap_or("").to_string();
    last.lines().filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).filter(|(k, _)| !k.starts_with("HTTP/")).collect()
}

/// Marks an answer's URL as a prefix (`Recorded::with_prefix`).
const PREFIX: &str = "prefix:";

/// A network that answers each URL it is given with a recorded reply, and
/// fails the test on any other request.
pub struct Recorded {
    /// The URL, what the request's body must contain (empty: anything), the
    /// status, the reply and its headers (from `<name>.headers` beside it, where
    /// the capture kept them).
    pub answers: Vec<(String, String, u16, Vec<u8>, Vec<(String, String)>)>,
    /// Yahoo's one-day chart of each symbol the spark is asked for, with its
    /// status (`with_spark`).
    pub spark: Vec<(String, u16, Value)>,
    /// TMX's answer to one listing's quote, for each form a quote request asks
    /// (`with_tmx_quote`); `*` for any form not named.
    pub tmx_quotes: Vec<(String, Value)>,
    pub asked: std::sync::Mutex<Vec<String>>,
    /// Each request's headers, in the order asked.
    pub headers: std::sync::Mutex<Vec<Vec<(String, String)>>>,
}

impl Recorded {
    pub fn new() -> Recorded {
        Recorded { answers: Vec::new(), spark: Vec::new(), tmx_quotes: Vec::new(), asked: std::sync::Mutex::new(Vec::new()), headers: std::sync::Mutex::new(Vec::new()) }
    }

    /// Answer `url` with the recorded reply `source/name`, captured with `status`.
    pub fn with(mut self, url: &str, status: u16, source: &str, name: &str) -> Recorded {
        self.answers.push((url.to_string(), String::new(), status, read(source, name).into_bytes(), headers(source, name)));
        self
    }

    /// Answer `url` with `body`, a recorded reply the test derived.
    pub fn with_text(mut self, url: &str, status: u16, body: String) -> Recorded {
        self.answers.push((url.to_string(), String::new(), status, body.into_bytes(), vec![]));
        self
    }

    /// Answer every request whose URL begins with `prefix`.
    pub fn with_prefix(mut self, prefix: &str, status: u16, source: &str, name: &str) -> Recorded {
        self.answers.push((format!("{PREFIX}{prefix}"), String::new(), status, read(source, name).into_bytes(), headers(source, name)));
        self
    }

    /// Answer the spark's result for `symbol` with Yahoo's recorded one-day chart
    /// of it (`source/name`), captured with `status`: the spark carries each
    /// symbol's one-day chart as its `response[0]` (`yahoo/spark-*.json`), and
    /// leaves out a symbol whose chart is a 404.
    pub fn with_spark(mut self, symbol: &str, status: u16, source: &str, name: &str) -> Recorded {
        self.spark.push((symbol.to_string(), status, json(source, name)));
        self
    }

    /// Answer `form` in a TMX quote request with the recorded reply to its own
    /// quote (`tmx/name`): the aliased request carries each listing's
    /// `getQuoteBySymbol` answer under its alias, an error naming that alias
    /// (`tmx/quotes-*.json`). `*` answers any form not named.
    pub fn with_tmx_quote(mut self, form: &str, name: &str) -> Recorded {
        self.tmx_quotes.push((form.to_string(), json("tmx", name)));
        self
    }

    /// The market's record knows no listing: TMX has no quote for it, Yahoo no
    /// chart. For a payer whose company's reader does not carry it, which is then
    /// asked of its market's record (`payers::run`).
    pub fn with_market_record_unknown(self) -> Recorded {
        self.with_tmx_quote("*", "quote-ZZZQX-unknown.json").with_prefix("https://query1.finance.yahoo.com/v8/finance/chart/", 404, "yahoo", "ZZZQX-status-404.json")
    }

    /// Answer TMX's quote and distributions for `symbol` with its recorded
    /// replies (`tmx/quote-<symbol>.json`, `tmx/dividends-<symbol>.json`): the
    /// market's record a company's is read beside.
    pub fn with_market_record(self, symbol: &str) -> Recorded {
        let url = "https://app-money.tmx.com/graphql";
        self.with_tmx_quote(symbol, &format!("quote-{symbol}.json")).with_body(url, &format!("getDividendsForSymbol\",\"variables\":{{\"symbol\":\"{symbol}\""), 200, "tmx", &format!("dividends-{symbol}.json"))
    }

    /// Answer a request to `url` whose body contains `needle`.
    pub fn with_body(mut self, url: &str, needle: &str, status: u16, source: &str, name: &str) -> Recorded {
        self.answers.push((url.to_string(), needle.to_string(), status, read(source, name).into_bytes(), headers(source, name)));
        self
    }
}

/// The recorded replies, shared between the network and the test that reads
/// what was asked.
pub struct Shared(pub std::sync::Arc<Recorded>);

impl bagholder_net::Transport for Shared {
    fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
        self.0.asked.lock().unwrap().push(ask.url.to_string());
        self.0.headers.lock().unwrap().push(ask.headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
        let sent = ask.body.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        if let Some(symbols) = ask.url.strip_prefix(SPARK).filter(|_| !self.0.spark.is_empty()) {
            return Ok((200, ask.url.to_string(), vec![], spark_of(&self.0.spark, symbols).canonical().into_bytes()));
        }
        if ask.url == TMX && sent.contains("\"operationName\":\"getQuotes\"") && !self.0.tmx_quotes.is_empty() {
            return Ok((200, ask.url.to_string(), vec![], tmx_quotes_of(&self.0.tmx_quotes, &sent).canonical().into_bytes()));
        }
        let hit = |u: &String| match u.strip_prefix(PREFIX) {
            Some(prefix) => ask.url.starts_with(prefix),
            None => u == ask.url,
        };
        match self.0.answers.iter().find(|(u, needle, _, _, _)| hit(u) && sent.contains(needle.as_str())) {
            Some((_, _, status, body, headers)) => Ok((*status, ask.url.to_string(), headers.clone(), body.clone())),
            None => panic!("a request no recorded reply answers: {}", ask.url),
        }
    }
}

const SPARK: &str = "https://query1.finance.yahoo.com/v7/finance/spark?symbols=";
const TMX: &str = "https://app-money.tmx.com/graphql";

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    match v {
        Value::Object(m) => m.get(k).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}

fn decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// The spark's reply to the symbols asked (`a,b,…&range=…`): each one's recorded
/// chart as its result, a 404's left out. A symbol no chart is recorded for fails
/// the test.
fn spark_of(charts: &[(String, u16, Value)], query: &str) -> Value {
    let named = query.split('&').next().unwrap();
    let mut results = Vec::new();
    for symbol in named.split(',').map(decoded) {
        let Some((_, status, chart)) = charts.iter().find(|(s, _, _)| *s == symbol) else { panic!("a spark symbol no recorded chart answers: {symbol}") };
        if *status != 200 {
            continue;
        }
        let Value::Array(r) = field(field(chart, "chart"), "result") else { panic!("{symbol}'s chart holds no result") };
        results.push(object(vec![("symbol", Value::String(symbol.clone())), ("response", Value::Array(vec![r[0].clone()]))]));
    }
    object(vec![("spark", object(vec![("result", Value::Array(results)), ("error", Value::Null)]))])
}

/// The forms a TMX quote request asks, each with its alias, in order.
pub fn tmx_asked(body: &str) -> Vec<(String, String)> {
    let query = match field(&reply::parse(body).unwrap(), "query") {
        Value::String(q) => q.clone(),
        _ => panic!("a quote request with no query"),
    };
    let mut out = Vec::new();
    for part in query.split(": getQuoteBySymbol(symbol: \"").skip(1).zip(query.split(": getQuoteBySymbol(symbol: \"")) {
        let (after, before) = part;
        let alias = before.rsplit(|c: char| c == ' ' || c == '{').next().unwrap().to_string();
        out.push((alias, after.split('"').next().unwrap().to_string()));
    }
    out
}

/// TMX's reply to a quote request: each form's recorded answer under its alias,
/// its errors naming that alias. A form no answer is recorded for fails the test.
fn tmx_quotes_of(answers: &[(String, Value)], body: &str) -> Value {
    let mut data = Vec::new();
    let mut errors = Vec::new();
    for (alias, form) in tmx_asked(body) {
        let Some((_, v)) = answers.iter().find(|(f, _)| *f == form).or_else(|| answers.iter().find(|(f, _)| f == "*")) else { panic!("a TMX quote no recorded reply answers: {form}") };
        data.push((alias.clone(), field(field(v, "data"), "getQuoteBySymbol").clone()));
        if let Value::Array(es) = field(v, "errors") {
            for e in es {
                let Value::Object(mut m) = e.clone() else { continue };
                m.insert("path".into(), Value::Array(vec![Value::String(alias.clone())]));
                errors.push(Value::Object(m));
            }
        }
    }
    let data = Value::Object(data.into_iter().collect());
    if errors.is_empty() {
        object(vec![("data", data)])
    } else {
        object(vec![("errors", Value::Array(errors)), ("data", data)])
    }
}

/// The reply TMX gives a quote request for `form` alone, from the recorded reply
/// to its own quote (`tmx/name`), as `with_tmx_quote` answers it.
pub fn tmx_quote_alone(form: &str, name: &str) -> Value {
    let body = format!("{{\"operationName\":\"getQuotes\",\"variables\":{{}},\"query\":\"query getQuotes {{ q0: getQuoteBySymbol(symbol: \\\"{form}\\\", locale: \\\"en\\\") {{ symbol }} }}\"}}");
    tmx_quotes_of(&[(form.to_string(), json("tmx", name))], &body)
}

/// A network answered by `recorded`, on a clock set to `at`.
pub fn net(recorded: &std::sync::Arc<Recorded>, at: &str) -> bagholder_net::Net {
    let clock = std::sync::Arc::new(bagholder_net::ManualClock::at(at.parse().unwrap()));
    // one run of the app across every instant a test asks at
    bagholder_net::Net::answered_by(clock, std::sync::Arc::new(bagholder_net::Limiter::new()), Box::new(Shared(recorded.clone()))).started_at(bagholder_core::jiff::Timestamp::MIN)
}

/// Put an instrument straight into the book's file, as a record would have: the
/// facts a reader stores are for instruments the book holds.
pub fn instrument_in_book(book: &std::path::Path, id: bagholder_core::InstrumentId, kind: &str, currency: &str) {
    let conn = rusqlite::Connection::open(book).unwrap();
    conn.execute("INSERT INTO instruments(id, kind, currency, created_at) VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z')", rusqlite::params![id.to_string(), kind, currency]).unwrap();
}

/// Harvest's High Income Shares press page as it stood before the months
/// `later` were announced: the recorded page, its links to those months'
/// releases (`…-october-2026-distributions/`) taken out.
pub fn harvest_press_before(later: &[&str]) -> String {
    let page = read("harvest", "press-income-shares.html");
    let mut out = String::with_capacity(page.len());
    let mut rest = page.as_str();
    while let Some(at) = rest.find("<a ") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find("</a>").map_or(rest.len(), |e| at + e + "</a>".len());
        let link = &rest[at..end];
        if !later.iter().any(|m| link.contains(&format!("-{m}-distributions"))) {
            out.push_str(link);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}
