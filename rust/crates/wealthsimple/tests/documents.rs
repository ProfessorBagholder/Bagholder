//! Every field a recorded or edited answer carries is one its document asks for.
//!
//! The readers are tested on these answers; a field written into one by hand that
//! the document never selects passes every reader test and never arrives from
//! Wealthsimple (2.0.0 read each account's worth from `financials`, which the
//! accounts document did not ask for, and showed none). So each answer here is
//! held to the selection of the document the client sends for it, fragments and
//! aliases resolved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bagholder_core::json::{self, Value};

#[derive(Debug)]
enum Sel {
    Field { key: String, children: Vec<Sel> },
    Spread(String),
    Inline(Vec<Sel>),
}

struct Doc {
    operation: Vec<Sel>,
    fragments: BTreeMap<String, Vec<Sel>>,
}

fn tokens(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let c: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '#' {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch.is_whitespace() || ch == ',' {
            i += 1;
        } else if ch == '"' {
            let s = i;
            i += 1;
            while i < c.len() && c[i] != '"' {
                i += if c[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push(c[s..i.min(c.len())].iter().collect());
        } else if ch == '.' && c.get(i + 1) == Some(&'.') && c.get(i + 2) == Some(&'.') {
            out.push("...".into());
            i += 3;
        } else if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            let s = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.' || c[i] == '-') {
                i += 1;
            }
            out.push(c[s..i].iter().collect());
        } else {
            out.push(ch.to_string());
            i += 1;
        }
    }
    out
}

struct Parser {
    t: Vec<String>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> &str {
        self.t.get(self.i).map(String::as_str).unwrap_or("")
    }
    fn next(&mut self) -> String {
        self.i += 1;
        self.t[self.i - 1].clone()
    }
    fn skip_balanced(&mut self, open: &str, close: &str) {
        if self.peek() != open {
            return;
        }
        let mut depth = 0;
        loop {
            let t = self.next();
            if t == open {
                depth += 1;
            } else if t == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }
    fn directives(&mut self) {
        while self.peek() == "@" {
            self.next();
            self.next();
            self.skip_balanced("(", ")");
        }
    }
    fn set(&mut self) -> Vec<Sel> {
        assert_eq!(self.next(), "{");
        let mut out = Vec::new();
        while self.peek() != "}" {
            if self.peek() == "..." {
                self.next();
                if self.peek() == "on" {
                    self.next();
                    self.next();
                    self.directives();
                    out.push(Sel::Inline(self.set()));
                } else if self.peek() == "{" || self.peek() == "@" {
                    self.directives();
                    out.push(Sel::Inline(self.set()));
                } else {
                    out.push(Sel::Spread(self.next()));
                    self.directives();
                }
                continue;
            }
            let mut key = self.next();
            if self.peek() == ":" {
                self.next();
                self.next(); // the field's name: the answer carries it under the alias
            }
            self.skip_balanced("(", ")");
            self.directives();
            let children = if self.peek() == "{" { self.set() } else { Vec::new() };
            out.push(Sel::Field { key: std::mem::take(&mut key), children });
        }
        self.next();
        out
    }
}

fn parse(src: &str) -> Doc {
    let mut p = Parser { t: tokens(src), i: 0 };
    let mut operation = None;
    let mut fragments = BTreeMap::new();
    while p.i < p.t.len() {
        match p.next().as_str() {
            "fragment" => {
                let name = p.next();
                assert_eq!(p.next(), "on");
                p.next();
                p.directives();
                fragments.insert(name, p.set());
            }
            "query" | "mutation" => {
                if p.peek() != "(" && p.peek() != "{" {
                    p.next();
                }
                p.skip_balanced("(", ")");
                p.directives();
                assert!(operation.is_none(), "one operation per document");
                operation = Some(p.set());
            }
            other => panic!("unexpected {other:?} at the top of a document"),
        }
    }
    Doc { operation: operation.expect("an operation"), fragments }
}

/// The fields `sels` select, fragments and inline fragments resolved, each with the
/// selections under it (a field selected twice has both sets).
fn flatten<'a>(doc: &'a Doc, sels: &'a [Sel], out: &mut BTreeMap<&'a str, Vec<&'a Sel>>) {
    for s in sels {
        match s {
            Sel::Field { key, .. } => out.entry(key.as_str()).or_default().push(s),
            Sel::Spread(name) => flatten(doc, doc.fragments.get(name).unwrap_or_else(|| panic!("no fragment {name}")), out),
            Sel::Inline(children) => flatten(doc, children, out),
        }
    }
}

/// The paths in `value` that `sels` do not select.
fn unasked(doc: &Doc, sels: &[&Sel], value: &Value, path: &str, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for v in items {
                unasked(doc, sels, v, &format!("{path}[]"), out);
            }
        }
        Value::Object(map) => {
            let mut fields = BTreeMap::new();
            for s in sels {
                if let Sel::Field { children, .. } = s {
                    flatten(doc, children, &mut fields);
                }
            }
            for (k, v) in map {
                match fields.get(k.as_str()) {
                    Some(under) => unasked(doc, under, v, &format!("{path}.{k}"), out),
                    None => out.push(format!("{path}.{k}")),
                }
            }
        }
        _ => {}
    }
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The fields of `answer` (a whole reply, `{"data": …}`) the document `name` does not ask for.
fn not_asked(name: &str, answer: &Value) -> Vec<String> {
    let src = std::fs::read_to_string(dir().join("graphql").join(format!("{name}.graphql"))).unwrap();
    let doc = parse(&src);
    let root = Sel::Field { key: "data".into(), children: doc.operation.iter().map(|s| match s {
        Sel::Field { key, children } => Sel::Field { key: key.clone(), children: children.iter().map(clone).collect() },
        other => clone(other),
    }).collect() };
    let data = match answer {
        Value::Object(m) => m.get("data").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let mut out = Vec::new();
    unasked(&doc, &[&root], &data, "data", &mut out);
    out
}

fn clone(s: &Sel) -> Sel {
    match s {
        Sel::Field { key, children } => Sel::Field { key: key.clone(), children: children.iter().map(clone).collect() },
        Sel::Spread(n) => Sel::Spread(n.clone()),
        Sel::Inline(c) => Sel::Inline(c.iter().map(clone).collect()),
    }
}

fn reply(path: &str) -> Value {
    json::parse(&std::fs::read_to_string(dir().join("tests/replies").join(path)).unwrap()).unwrap()
}

#[test]
fn every_answer_the_pull_is_tested_on_is_what_its_document_asks_for() {
    for (document, answer) in [
        ("FetchAllAccounts", "edited-accounts-one.json"),
        ("FetchAllAccounts", "../wealthsimple-ops/edited-accounts-margin-boost.json"),
        ("FetchAllAccounts", "../wealthsimple-ops/edited-accounts-margin-boost-off.json"),
        ("FetchActivityFeedItems", "edited-activity-november.json"),
        ("FetchActivityFeedItems", "edited-new-trade.json.later"),
        ("FetchAccountsWithBalance", "edited-balances.json"),
        ("Securities", "edited-securities.json"),
        ("FetchAccountHistoricalFinancials", "history-1.json"),
        ("FetchAccountHistoricalFinancials", "history-2.json"),
        ("FetchAccountHistoricalFinancials", "history-3.json"),
        ("FetchHoldingsExportPositionsAsOfDate", "positions@anon-tfsa-1@2025-11-18.json"),
    ] {
        assert_eq!(not_asked(document, &reply(&format!("wealthsimple-pull/{answer}"))), Vec::<String>::new(), "{answer}, asked by {document}");
    }
}

#[test]
fn a_field_the_document_does_not_ask_for_is_named() {
    let src = std::fs::read_to_string(dir().join("tests/replies/wealthsimple-pull/edited-accounts-one.json")).unwrap();
    let answer = json::parse(&src.replacen("\"nickname\"", "\"notAsked\": 1, \"nickname\"", 1)).unwrap();
    let missing = not_asked("FetchAllAccounts", &answer);
    assert_eq!(missing, vec!["data.identity.accounts.edges[].node.notAsked".to_string()]);
}
