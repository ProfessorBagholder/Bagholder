//! The forms of statement row Wealthsimple's monthly statements write, recorded
//! with made-up values (`tests/replies/wealthsimple-statements`, its README):
//! each recorded row is read as it states; the lists per currency are read and
//! never the top-level list; and the made-up broker (`src/generated.rs`) states
//! no row of a form not recorded.

use std::collections::BTreeSet;

use bagholder_broker::codes;
use bagholder_core::json::{self, Value};
use bagholder_core::{Currency, Dec};
use bagholder_wealthsimple::adapter::Source;
use bagholder_wealthsimple::generated::{Generated, Shape, Size, ASSIGNED, SHAPES, STATED_OTHERWISE};
use bagholder_wealthsimple::statement::{self, BROKERAGE};

const RECORDED: [&str; 4] = ["forms-brokerage.json", "forms-coins.json", "forms-cash.json", "lira-2025-06.json"];

/// A recorded reply's `data.monthlyStatement`.
fn recorded(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-statements").join(name);
    let v = json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    get(&get(&v, "data"), "monthlyStatement")
}

fn get(v: &Value, key: &str) -> Value {
    match v {
        Value::Object(m) => m.get(key).cloned().unwrap_or_else(|| panic!("no {key}")),
        other => panic!("{key} of {other:?}"),
    }
}

fn opt(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Object(m) => m.get(key).cloned(),
        other => panic!("{key} of {other:?}"),
    }
}

fn text(v: &Value, key: &str) -> String {
    match get(v, key) {
        Value::String(s) => s,
        other => panic!("{key} is {other:?}"),
    }
}

fn list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        other => panic!("not a list: {other:?}"),
    }
}

/// A statement's rows as it lists them, each with the currency of its list: a
/// statement with lists per currency by them, one with none (a coin account's,
/// a cash statement) in the account's own, CAD here.
fn listed(statement: &Value) -> Vec<(Currency, Value)> {
    let data = get(statement, "data");
    match opt(&data, "activitiesPerCurrency") {
        Some(Value::Array(per)) => per.iter().flat_map(|p| list(&get(p, "currentTransactions")).into_iter().map(|r| (Currency::parse(&text(p, "currency")).unwrap(), r))).collect(),
        Some(Value::Null) | None => list(&get(&data, "currentTransactions")).into_iter().map(|r| (Currency::CAD, r)).collect(),
        Some(other) => panic!("activitiesPerCurrency is {other:?}"),
    }
}

/// Every row a statement lists, the top-level list's too.
fn every_row(statement: &Value) -> Vec<Value> {
    let data = get(statement, "data");
    let mut out = list(&get(&data, "currentTransactions"));
    if let Some(Value::Array(per)) = opt(&data, "activitiesPerCurrency") {
        for p in per {
            out.extend(list(&get(&p, "currentTransactions")));
        }
    }
    out
}

/// The day `(executed at D)` states, read here on its own.
fn executed(description: &str) -> Option<jiff::civil::Date> {
    let (_, after) = description.split_once("(executed at ")?;
    Some(after.get(..10).unwrap().parse().unwrap())
}

/// The decimals a number is written with.
fn places(n: &str) -> usize {
    n.split_once('.').map(|(_, f)| f.len()).unwrap_or(0)
}

/// A contract's row: `SYM K USD CALL|PUT D: …`.
fn contract_terms(description: &str) -> Option<(String, String)> {
    let w: Vec<&str> = description.split(' ').collect();
    (w.len() > 4 && w[2] == "USD" && (w[3] == "CALL" || w[3] == "PUT")).then(|| (w[0].to_string(), w[1].to_string()))
}

#[test]
fn every_recorded_row_is_read_as_it_states_and_every_fill_states_its_symbol_and_units_exactly() {
    let (mut fills, mut contracts, mut rates) = (0, 0, 0);
    for name in RECORDED {
        let s = recorded(name);
        let want = listed(&s);
        let rows = statement::rows(&s, Currency::CAD).unwrap();
        assert_eq!(rows.len(), want.len(), "{name}");
        for (r, (currency, w)) in rows.iter().zip(&want) {
            let description = text(w, "description");
            assert_eq!(r.code, text(w, "transactionType"), "{name}: {description}");
            assert_eq!(r.description, description, "{name}");
            assert_eq!(r.cash, Dec::parse(&text(w, "cashMovement")).unwrap(), "{name}: {description}");
            assert_eq!(r.balance, Dec::parse(&text(w, "balance")).unwrap(), "{name}: {description}");
            assert_eq!(r.day, text(w, "transactionDate").parse::<jiff::civil::Date>().unwrap(), "{name}: {description}");
            assert_eq!(r.currency, *currency, "{name}: {description}");
            assert_eq!(r.executed, executed(&description), "{name}: {description}");
        }
        for w in every_row(&s) {
            let d = text(&w, "description");
            let code = text(&w, "transactionType");
            if code == "BUY" || code == "SELL" {
                let (symbol, units) = if let Some(rest) = d.strip_prefix("Purchase of ").or_else(|| d.strip_prefix("Sale of ")) {
                    let words: Vec<&str> = rest.split(' ').collect();
                    assert!(places(words[0]) == 10 || words[0].contains('E'), "a coin's units to 10 places or with an exponent: {d}");
                    (words[1].to_string(), words[0].to_string())
                } else {
                    let (symbol, rest) = d.split_once(" - ").unwrap_or_else(|| panic!("a fill of no known form: {d}"));
                    let (_, what) = rest.split_once(": ").unwrap();
                    let units = what.split(' ').nth(1).unwrap();
                    assert_eq!(places(units), 4, "a share's units to 4 places: {d}");
                    (symbol.to_string(), units.to_string())
                };
                assert_eq!(codes::fill_of(&d), Some((symbol, Dec::parse_scientific(&units).unwrap())), "{name}: {d}");
                fills += 1;
            }
            if let Some((_, strike)) = contract_terms(&d) {
                assert_eq!(codes::fill_of(&d), None, "a contract's row moves no shares or coins: {d}");
                assert_eq!(places(&strike), 2, "a strike to 2 places: {d}");
                contracts += 1;
            }
            for (_, after) in d.match_indices("FX Rate: ").map(|(i, m)| d.split_at(i + m.len())) {
                let rate: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                assert_eq!(places(&rate), 4, "a rate to 4 places: {d}");
                rates += 1;
            }
        }
    }
    assert!(fills > 20 && contracts > 20 && rates > 20, "{fills} fills, {contracts} contracts, {rates} rates");
}

#[test]
fn the_lists_per_currency_are_read_and_never_the_top_level_list() {
    let s = recorded("forms-brokerage.json");
    let data = get(&s, "data");
    let per = list(&get(&data, "activitiesPerCurrency"));
    let of = |c: &str| list(&get(per.iter().find(|p| text(p, "currency") == c).unwrap(), "currentTransactions"));
    let (cad, usd) = (of("CAD"), of("USD"));
    let top = list(&get(&data, "currentTransactions"));
    assert_eq!(top, cad[..cad.len() - 1], "the top-level list is the CAD list without its last row");
    let rows = statement::rows(&s, Currency::CAD).unwrap();
    assert_eq!(rows.len(), cad.len() + usd.len());
    let said = |c: Currency| rows.iter().filter(|r| r.currency == c).map(|r| r.description.clone()).collect::<Vec<_>>();
    assert_eq!(said(Currency::CAD), cad.iter().map(|r| text(r, "description")).collect::<Vec<_>>());
    assert_eq!(said(Currency::USD), usd.iter().map(|r| text(r, "description")).collect::<Vec<_>>());
}

/// A description's form: dates as `D`, numbers as `N` (a leading `-` or `$`
/// kept), the leading ticker as `SYM` wherever it recurs, the security's name
/// between ` - ` and `:` as `NAME`, a contract's `CALL` or `PUT` as `RIGHT`, a
/// coin's symbol after `Purchase of N ` or `Sale of N ` as `SYM`, and the free
/// text after `Margin Interest Charges for ` as `NAME`.
fn form(description: &str) -> String {
    let d = dates(description);
    if d.strip_prefix("Margin Interest Charges for ").is_some_and(|rest| !rest.is_empty()) {
        return "Margin Interest Charges for NAME".into();
    }
    let ticker = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '.') && t.chars().any(|c| c.is_ascii_uppercase());
    let mut out = d.clone();
    let mut symbol = None;
    for (i, _) in d.match_indices(" - ") {
        let token = d[..i].rsplit(' ').next().unwrap_or("");
        let Some(colon) = d[i + 3..].find(':') else { continue };
        if ticker(token) {
            out = format!("{}SYM - NAME{}", &d[..i - token.len()], &d[i + 3 + colon..]);
            symbol = Some(token.to_string());
            break;
        }
    }
    if symbol.is_none() {
        let first = d.split(' ').next().unwrap_or("");
        if ticker(first) {
            symbol = Some(first.to_string());
        } else if let Some(rest) = d.strip_prefix("Purchase of ").or_else(|| d.strip_prefix("Sale of ")) {
            symbol = rest.split(' ').nth(1).map(str::to_string);
        }
    }
    if let Some(s) = symbol {
        out = words_replaced(&out, &s, "SYM");
    }
    out = words_replaced(&words_replaced(&out, "CALL", "RIGHT"), "PUT", "RIGHT");
    numbers(&out)
}

/// `yyyy-mm-dd` as `D`.
fn dates(s: &str) -> String {
    let b = s.as_bytes();
    let is_date = |i: usize| i + 10 <= b.len() && (0..10).all(|k| if k == 4 || k == 7 { b[i + k] == b'-' } else { b[i + k].is_ascii_digit() });
    let (mut out, mut i) = (String::new(), 0);
    while i < s.len() {
        if is_date(i) {
            out.push('D');
            i += 10;
        } else {
            let c = s[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Each whole word `from` as `to`.
fn words_replaced(s: &str, from: &str, to: &str) -> String {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let (mut out, mut last) = (String::new(), 0);
    for (i, _) in s.match_indices(from) {
        if i < last || word(s[..i].chars().next_back()) || word(s[i + from.len()..].chars().next()) {
            continue;
        }
        out.push_str(&s[last..i]);
        out.push_str(to);
        last = i + from.len();
    }
    out.push_str(&s[last..]);
    out
}

/// Every number (`3`, `4.8200`, `3.0E-8`) as `N`.
fn numbers(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (String::new(), 0);
    let digits = |mut i: usize| {
        while i < c.len() && c[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    while i < c.len() {
        if !c[i].is_ascii_digit() {
            out.push(c[i]);
            i += 1;
            continue;
        }
        i = digits(i);
        if i + 1 < c.len() && c[i] == '.' && c[i + 1].is_ascii_digit() {
            i = digits(i + 1);
        }
        if i + 1 < c.len() && c[i] == 'E' {
            let at = if c[i + 1] == '-' { i + 2 } else { i + 1 };
            if at < c.len() && c[at].is_ascii_digit() {
                i = digits(at);
            }
        }
        out.push('N');
    }
    out
}

#[test]
fn a_form_names_what_varies_and_keeps_the_words() {
    for (d, want) in [
        ("XQJ - Examplia Wobbleworks Plc.: Bought 4.8200 shares at $3.00 per share (executed at 2034-03-06)", "SYM - NAME: Bought N shares at $N per share (executed at D)"),
        ("XQN 72.00 USD CALL 2034-06-20: Bought 1 contract (executed at 2034-03-06), Fee: $3.7300", "SYM N USD RIGHT D: Bought N contract (executed at D), Fee: $N"),
        ("XQB 72.00 USD PUT 2034-06-20: Assigned -1 contract (executed at 2034-03-03)", "SYM N USD RIGHT D: Assigned -N contract (executed at D)"),
        ("Sale of 3.0E-8 XQX (executed at 2034-04-27), FX Rate: 1.8536, Fee charged $0.01", "Sale of N SYM (executed at D), FX Rate: N, Fee charged $N"),
        ("Convert USD (executed at 2034-03-17) - $19USD = $35.22CAD", "Convert USD (executed at D) - $NUSD = $NCAD"),
        ("Margin Interest Charges for Example Period", "Margin Interest Charges for NAME"),
        ("XQS - Examplia Wobbleworks Plc.: Transfer of 4.8200 XQS into the account (executed at 2034-05-14)", "SYM - NAME: Transfer of N SYM into the account (executed at D)"),
        ("Referral bonus (2034-04-18)", "Referral bonus (D)"),
        ("ACCOUNTING_REIMBURSEMENT (executed at 2034-04-02)", "ACCOUNTING_REIMBURSEMENT (executed at D)"),
    ] {
        assert_eq!(form(d), want, "{d}");
    }
}

/// What a row's `unit` names: a currency (`$CAD`), a symbol, nothing (null),
/// or no `unit` at all (a cash statement's rows).
fn unit_style(row: &Value) -> &'static str {
    match opt(row, "unit") {
        None => "absent",
        Some(Value::Null) => "null",
        Some(Value::String(s)) if s.starts_with('$') => "currency",
        Some(Value::String(_)) => "symbol",
        Some(other) => panic!("a unit of {other:?}"),
    }
}

/// Each row's (statement typename, code, form, unit style).
fn forms(statement: &Value) -> Vec<(String, String, String, &'static str)> {
    let typename = text(&get(statement, "data"), "__typename");
    every_row(statement).iter().map(|r| (typename.clone(), text(r, "transactionType"), form(&text(r, "description")), unit_style(r))).collect()
}

#[test]
fn every_row_the_made_up_broker_states_in_a_statement_has_a_recorded_form() {
    let known: BTreeSet<_> = RECORDED.iter().flat_map(|n| forms(&recorded(n))).collect();
    let shapes: Vec<Shape> = SHAPES.iter().chain(STATED_OTHERWISE.iter()).chain(ASSIGNED.iter()).copied().collect();
    let today: jiff::civil::Date = "2026-10-25".parse().unwrap();
    let mut seen: BTreeSet<(String, String, String, &'static str)> = BTreeSet::new();
    for seed in [13, 1, 5, 9] {
        let mut g = Generated::shaped(Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed, deep: 0 }, today, &shapes);
        for account in g.account_ids() {
            for month in g.statement_months(&account) {
                if let Some(s) = g.statement(&account, month, BROKERAGE).unwrap() {
                    seen.extend(forms(&s));
                }
            }
        }
    }
    let unrecorded: Vec<String> = seen.iter().filter(|f| !known.contains(*f)).map(|(t, c, f, u)| format!("{t} | {c} | {f} | {u}")).collect();
    assert!(unrecorded.is_empty(), "{} forms the made-up broker states that no recorded row has:\n{}", unrecorded.len(), unrecorded.join("\n"));
    // the coin account's statement, the fills, the conversion and the assignment are among them
    for code in ["BUY", "SELL", "SELLTOOPEN", "ASSIGN", "FXCONVERSION", "CONT", "DIV", "TRFIN", "TRFOUT"] {
        assert!(seen.iter().any(|(_, c, _, _)| c == code), "no {code} row stated");
    }
    assert!(seen.iter().any(|(_, c, f, u)| c == "BUY" && f.contains("Fee charged") && *u == "null"), "a coin account's statement");
}

/// The codes the made-up broker's statements state in place of the ones they
/// stated before are placed as those were.
#[test]
fn each_code_the_made_up_broker_states_now_is_placed_as_the_one_it_replaced() {
    for (before, now) in [("TRFOUTTF", "TRFOUT"), ("TRFINTF", "TRFIN"), ("EFT", "CONT"), ("EFTOUT", "WD"), ("INT", "FPLINT")] {
        assert!(codes::kind(now).is_some(), "{now} is placed");
        assert_eq!(codes::kind(before), codes::kind(now), "{before} and {now}");
    }
}

/// The README in the statements' folder lists every recorded form, and only them.
#[test]
fn the_readme_lists_every_recorded_form() {
    let readme = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-statements/README.md")).unwrap();
    let (_, listed) = readme.split_once("typename | code | form | unit style\n").expect("the README's list");
    let listed: BTreeSet<String> = listed.lines().take_while(|l| !l.starts_with("```")).map(str::to_string).collect();
    let known: BTreeSet<String> = RECORDED.iter().flat_map(|n| forms(&recorded(n))).map(|(t, c, f, u)| format!("{t} | {c} | {f} | {u}")).collect();
    assert_eq!(listed, known);
}
