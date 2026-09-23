//! CSV import and folder watching.
//!
//! Three layouts are recognised:
//!
//! - canonical: a Wealthsimple activities export with transaction_date,
//!   activity_type, activity_sub_type, quantity, unit_price, net_cash_amount
//! - statement: a statement export with date, transaction, description, amount
//!   (fills are read out of the description text)
//! - legacy: Date / Action / Symbol / Quantity / Price / Amount
//!
//! Rows become local activities (never a Wealthsimple canonical id) and go
//! through `merge_local_rows`, which drops rows already stored. A watched
//! folder is scanned by the server itself: top-level .csv files, re-read only
//! when their size or modification time changes.

use indexmap::IndexMap;
use regex::Regex;
use rusqlite::{Connection, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

use bagholder_model::lenient;
use bagholder_model::textrules::{csv_rows, parse_float, parse_int, trim_space, splitlines, uuid4};

use crate::activities::ActivityRow;

pub const WATCH_META: &str = "watch_folder";
pub const WATCH_FILES_META: &str = "watch_files";
pub const WATCH_LAST_META: &str = "watch_last";

/// One row skipped by `parse_csv`, and why.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct Skipped {
    pub row: usize,
    pub message: String,
    pub raw: String,
}

/// The count of parsed rows by activity type (or, absent one, by category),
/// in the order each type was first seen.
pub type CountsByType = IndexMap<String, i64>;

/// What `parse_csv` makes of one file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct CsvReport {
    pub format: String,
    pub activities: Vec<ActivityRow>,
    pub skipped: Vec<Skipped>,
    pub footer_stripped: bool,
    pub counts_by_type: CountsByType,
    pub row_count: usize,
}

/// What `import_text` answers: `parse_csv`'s report, merged into the store.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub ok: bool,
    pub file: String,
    pub format: String,
    pub rows: usize,
    pub added: usize,
    pub duplicates: usize,
    pub skipped: Vec<Skipped>,
    pub skipped_count: usize,
    pub footer_stripped: bool,
    pub counts_by_type: CountsByType,
}

/// One row of a CSV file: its cells, keyed by normalized header, in header
/// order -- a later column of the same normalized name overwrites the
/// earlier one's value in place, as the old map-shaped row did.
#[derive(Debug, Clone, Default)]
pub struct CsvRow(Vec<(String, String)>);

impl CsvRow {
    fn from_cells(norms: &[String], cells: &[String]) -> CsvRow {
        let mut out: Vec<(String, String)> = Vec::new();
        for (j, n) in norms.iter().enumerate() {
            let v = cells.get(j).cloned().unwrap_or_default();
            match out.iter_mut().find(|(k, _)| k == n) {
                Some(entry) => entry.1 = v,
                None => out.push((n.clone(), v)),
            }
        }
        CsvRow(out)
    }

    /// The first non-empty cell among `keys`, trimmed.
    pub fn pick(&self, keys: &[&str]) -> String {
        for k in keys {
            if let Some((_, v)) = self.0.iter().find(|(kk, _)| kk == k) {
                let t = trim_space(v);
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
        String::new()
    }

    fn values(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(_, v)| v.as_str())
    }

    /// The row reproduced exactly as the old map-shaped row's `json_text` was
    /// -- what a skipped row's message quotes.
    pub fn raw(&self) -> String {
        let mut m = Map::new();
        for (k, v) in &self.0 {
            m.insert(k.clone(), json!(v));
        }
        crate::tables::json_text(&Value::Object(m))
    }
}

const MONTHS: [(&str, &str); 12] = [
    ("jan", "01"), ("feb", "02"), ("mar", "03"), ("apr", "04"), ("may", "05"), ("jun", "06"),
    ("jul", "07"), ("aug", "08"), ("sep", "09"), ("oct", "10"), ("nov", "11"), ("dec", "12"),
];

fn month(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    MONTHS.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).unwrap())
        }
    };
}

re!(re_header_space, r"[\s\-]+");
re!(re_money, r"[$£€,\s]|CAD|USD|cad|usd");
re!(re_iso, r"^(\d{4})-(\d{2})-(\d{2})(?:[T\s].*)?$");
re!(re_ymd, r"^(\d{4})[/.](\d{1,2})[/.](\d{1,2})(?:\s.*)?$");
re!(re_dmony, r"^(\d{1,2})[- ]([A-Za-z]{3})[- ](\d{4})$");
re!(re_mondy, r"^([A-Za-z]{3})[- ](\d{1,2}),?[- ](\d{4})$");
re!(re_dmy, r"^(\d{1,2})[/\-.](\d{1,2})[/\-.](\d{4})(?:\s.*)?$");
re!(re_serial, r"^\d{4,6}(\.\d+)?$");
re!(re_footer, r"(?i)^\s*as of\s+\d{4}-\d{2}-\d{2}");
re!(re_compact, r"[\s_\-]");
re!(re_dash, r"^([A-Za-z][A-Za-z0-9.\-]{0,20})\s+-\s+(.+)$");
re!(re_space, r"\s");
re!(re_spaces, r"\s+");
re!(re_ticker, r"^[A-Za-z][A-Za-z0-9.\-]{0,20}$");
re!(re_shares, r"(?i)(-?[\d,]+(?:\.\d+)?)\s+shares?\b");
re!(re_contracts, r"(?i)(-?[\d,]+(?:\.\d+)?)\s+contracts?\b");
re!(re_price, r"(?i)\bat\s+\$?([\d,]+(?:\.\d+)?)\s+per\s+share\b");
re!(re_executed, r"(?i)\(\s*executed at\s+(\d{4}-\d{2}-\d{2})\s*\)");
re!(re_roc, r"\breturn of capital\b|\broc\b");
re!(re_trade_word, r"\b(bought|sold|buy|sell)\b");
re!(re_fx_word, r"\bfx\b");
re!(re_conversion, r"conversion|convert");
re!(re_sell_word, r"\bsell|sold\b");
re!(re_dividend, r"dividend|distribution");
re!(re_transfer, r"transfer|trfout|trfin");
re!(re_fx, r"\bfx\b|conversion|convert");
re!(re_fee, r"\bfee\b|commission|fchrg");
re!(re_book_dated, r"(?i)([A-Z0-9]{8,}(?:CAD|USD))-\d{4}-\d{2}-\d{2}");
re!(re_book, r"(?i)([A-Z0-9]{8,}(?:CAD|USD))");
re!(re_key_ccy, r"(?i)\b(USD|CAD)\b");

/// A pattern match on a string already stripped of surrounding whitespace, so
/// no trailing newline reaches these patterns.
fn caps<'a>(r: &Regex, s: &'a str) -> Option<regex::Captures<'a>> {
    r.captures(s)
}

/// `normalize_header`.
pub fn normalize_header(h: &str) -> String {
    let s = h.replace('\u{feff}', "");
    let s = trim_space(&s).trim_matches(['"', '\'']);
    let s = trim_space(s).to_lowercase();
    re_header_space().replace_all(&s, "_").into_owned()
}

/// `parse_number`: currency marks and thousands separators dropped,
/// parentheses a negative, anything unreadable 0.
///
/// `nan` and `inf` are not numbers here. Read as numbers, the importer would
/// book a quantity or a price that is not a number -- a row that never matches
/// itself, so re-importing the file would add it again, and a NaN would empty
/// every total it reaches. So they are unreadable,
/// and read as 0 like any other unreadable cell.
pub fn parse_number(raw: &str) -> f64 {
    let s = trim_space(raw);
    if s.is_empty() || s == "-" || s == "—" || s.to_lowercase() == "n/a" {
        return 0.0;
    }
    let paren = s.starts_with('(') && s.ends_with(')');
    let s = s.replace(['(', ')'], "");
    let s = re_money().replace_all(&s, "").into_owned();
    if s.is_empty() {
        return 0.0;
    }
    match parse_float(&s) {
        Some(n) if n.is_finite() => if paren { -n.abs() } else { n },
        _ => 0.0,
    }
}

fn d2(text: &str) -> String {
    format!("{:02}", parse_int(text).unwrap_or(0))
}

/// `parse_date`: the dates the exports write, as YYYY-MM-DD, or "".
pub fn parse_date(raw: &str) -> String {
    let s = trim_space(raw);
    if s.is_empty() {
        return String::new();
    }
    if let Some(m) = caps(re_iso(), s) {
        return format!("{}-{}-{}", &m[1], &m[2], &m[3]);
    }
    if let Some(m) = caps(re_ymd(), s) {
        return format!("{}-{}-{}", &m[1], d2(&m[2]), d2(&m[3]));
    }
    if let Some(m) = caps(re_dmony(), s) {
        if let Some(mm) = month(&m[2]) {
            return format!("{}-{}-{}", &m[3], mm, d2(&m[1]));
        }
    }
    if let Some(m) = caps(re_mondy(), s) {
        if let Some(mm) = month(&m[1]) {
            return format!("{}-{}-{}", &m[3], mm, d2(&m[2]));
        }
    }
    if let Some(m) = caps(re_dmy(), s) {
        let a = parse_int(&m[1]).unwrap_or(0);
        let b = parse_int(&m[2]).unwrap_or(0);
        let y = &m[3];
        if a > 12 && b <= 12 {
            return format!("{}-{:02}-{:02}", y, b, a);
        }
        return format!("{}-{:02}-{:02}", y, a, b);
    }
    if re_serial().is_match(s) {
        if let Some(serial) = parse_float(s) {
            if 20000.0 < serial && serial < 80000.0 {
                // a spreadsheet's day count from 1899-12-30
                let days = serial.round_ties_even() as i64;
                let base = bagholder_model::dates::to_days(1899, 12, 30);
                let (y, mo, d) = bagholder_model::dates::from_days(base + days);
                return bagholder_model::dates::fmt(y, mo, d);
            }
        }
    }
    String::new()
}

/// `is_footer_line`.
pub fn is_footer_line(text: &str) -> bool {
    re_footer().is_match(text)
}

/// `detect_format`.
pub fn detect_format(headers: &[String]) -> &'static str {
    let norms: std::collections::HashSet<String> = headers.iter().map(|h| normalize_header(h)).collect();
    let has = |k: &str| norms.contains(k);
    let canon = ["transaction_date", "activity_type", "activity_sub_type", "net_cash_amount", "unit_price"];
    let legacy = ["date", "action", "symbol", "quantity", "price", "amount"];
    let canon_hits = canon.iter().filter(|h| has(h)).count();
    let legacy_hits = legacy.iter().filter(|h| has(h)).count();
    if canon_hits >= 3 {
        return "canonical";
    }
    if has("transaction_date") || has("activity_type") {
        return "canonical";
    }
    if ["date", "transaction", "description", "amount"].iter().all(|k| has(k)) {
        return "statement";
    }
    if legacy_hits >= 5 && (has("action") || has("date")) {
        return "legacy";
    }
    if has("action") && has("date") {
        return "legacy";
    }
    "unknown"
}

fn compact_lower(s: &str) -> String {
    re_compact().replace_all(&trim_space(s).to_lowercase(), "").into_owned()
}

/// `categorize`.
pub fn categorize(activity_type: &str, activity_sub_type: &str) -> &'static str {
    let t = compact_lower(activity_type);
    let s = compact_lower(activity_sub_type);
    let blob = format!("{} {}", t, s);
    let either = |k: &str| t.contains(k) || s.contains(k);
    if t == "fxexchange" || t == "fx" || s == "fxexchange" || blob.contains("fxexchange") {
        return "fx";
    }
    if ["expir", "exercise", "assign"].iter().any(|k| either(k)) {
        return "option_event";
    }
    if t == "trade" || s == "buy" || s == "sell" {
        return "trade";
    }
    for (k, cat) in [("dividend", "dividend"), ("deposit", "deposit"), ("withdraw", "withdrawal"), ("interest", "interest"), ("fee", "fee"), ("transfer", "transfer")] {
        if either(k) {
            return cat;
        }
    }
    "other"
}

/// `extract_instrument`: (symbol, name) from a statement
/// description.
pub fn extract_instrument(description: &str) -> (String, String) {
    let text = trim_space(description);
    if text.is_empty() {
        return (String::new(), String::new());
    }
    let colon = match text.find(':') {
        None => {
            return match caps(re_dash(), text) {
                Some(m) => (m[1].to_uppercase(), trim_space(&m[2]).to_string()),
                None => (String::new(), String::new()),
            };
        }
        Some(c) => c,
    };
    let left = trim_space(&text[..colon]);
    if let Some(m) = caps(re_dash(), left) {
        return (m[1].to_uppercase(), trim_space(&m[2]).to_string());
    }
    if re_space().is_match(left) {
        let sym = re_spaces().replace_all(left, " ").to_uppercase();
        return (sym.clone(), sym);
    }
    if re_ticker().is_match(left) {
        return (left.to_uppercase(), left.to_uppercase());
    }
    (String::new(), String::new())
}

pub struct Parsed {
    pub symbol: String,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub executed_at: String,
    pub fill_parsed: bool,
    pub contract_signed: f64,
    pub shares_signed: f64,
}

/// `parse_statement_description`.
pub fn parse_statement_description(description: &str) -> Parsed {
    let (symbol, name) = extract_instrument(description);
    let mut p = Parsed {
        symbol, name, quantity: 0.0, unit_price: 0.0, executed_at: String::new(),
        fill_parsed: false, contract_signed: 0.0, shares_signed: 0.0,
    };
    if let Some(m) = re_shares().captures(description) {
        p.shares_signed = parse_number(&m[1]);
        p.quantity = p.shares_signed.abs();
        p.fill_parsed = p.quantity > 0.0;
    } else if let Some(m) = re_contracts().captures(description) {
        p.contract_signed = parse_number(&m[1]);
        p.quantity = p.contract_signed.abs();
        p.fill_parsed = p.quantity > 0.0;
    }
    if let Some(m) = re_price().captures(description) {
        p.unit_price = parse_number(&m[1]);
    }
    if let Some(m) = re_executed().captures(description) {
        p.executed_at = parse_date(&m[1]);
    }
    p
}

impl Parsed {
    pub fn to_json(&self) -> Value {
        json!({
            "symbol": self.symbol, "name": self.name, "quantity": self.quantity, "unitPrice": self.unit_price,
            "executedAt": self.executed_at, "fillParsed": self.fill_parsed,
            "contractSigned": self.contract_signed, "sharesSigned": self.shares_signed,
        })
    }
}

/// `map_statement_type`: (activity type, sub-type, category) for a
/// statement's transaction code and description.
pub fn map_statement_type(code: &str, description: &str) -> (String, String, String) {
    let raw = trim_space(code).to_string();
    let c = re_compact().replace_all(&raw.to_uppercase(), "").into_owned();
    let blob = format!("{} {}", c, description).to_lowercase();
    let or = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { a.to_string() };
    let out = |a: &str, b: &str, cat: &str| (a.to_string(), b.to_string(), cat.to_string());
    if c.contains("EXPIR") || ["ASSIGN", "ASSIGNMENT", "EXERCISE"].contains(&c.as_str()) {
        return out(&raw, &raw.to_uppercase(), "option_event");
    }
    if c == "LOAN" || c == "RECALL" {
        return (or(&raw, &c), c.clone(), "other".into());
    }
    if ["STKDIS", "STKDIV", "SPIN", "SPINOFF"].contains(&c.as_str()) {
        return (or(&raw, "STKDIS"), "STKDIS".into(), "trade".into());
    }
    if c == "ROC" || c == "RETURNOFCAPITAL" {
        return (or(&raw, "ROC"), "ROC".into(), "other".into());
    }
    if c == "DIV" || c == "DIVIDEND" || c.contains("DIVIDEND") {
        return ("Dividend".into(), or(&raw, "DIV"), "dividend".into());
    }
    if c == "CONT" || c == "CONTRIBUTION" || c.contains("CONTRIB") {
        return ("Deposit".into(), or(&raw, "CONT"), "deposit".into());
    }
    if c == "WD" || c == "WITHDRAWAL" || c.contains("WITHDRAW") {
        return ("Withdrawal".into(), or(&raw, "WD"), "withdrawal".into());
    }
    if c == "INTCHARGED" || c == "INTPAID" || c == "INTEREST" || c.starts_with("INT") {
        return ("Interest".into(), or(&raw, "INTEREST"), "interest".into());
    }
    if c == "TRFOUT" || c == "TRFIN" || c == "TRANSFER" || c.starts_with("TRF") || c.contains("TRANSFER") {
        return ("Transfer".into(), or(&raw, "TRANSFER"), "transfer".into());
    }
    if c == "FXCONVERSION" || c == "FX" || c == "CONVERT" || c.contains("FX") || c.contains("CONVERT") {
        return ("FxExchange".into(), or(&raw, "FX"), "fx".into());
    }
    if c == "FEE" || c == "FCHRG" || c == "COMM" || c.contains("FEE") || c.contains("FCHRG") {
        return ("Fee".into(), or(&raw, "FEE"), "fee".into());
    }
    if c == "BUY" || c == "SELL" {
        return ("Trade".into(), c.clone(), "trade".into());
    }
    if c.contains("SELL") {
        return (or(&raw, "Trade"), "SELL".into(), "trade".into());
    }
    if c.contains("BUY") {
        return (or(&raw, "Trade"), "BUY".into(), "trade".into());
    }
    if re_roc().is_match(&blob) {
        return (or(&raw, "ROC"), "ROC".into(), "other".into());
    }
    if re_trade_word().is_match(&blob) && !re_fx_word().is_match(&blob) && !re_conversion().is_match(&blob) {
        let side = if re_sell_word().is_match(&blob) { "SELL" } else { "BUY" };
        return ("Trade".into(), side.into(), "trade".into());
    }
    if re_dividend().is_match(&blob) {
        return ("Dividend".into(), or(&raw, "DIV"), "dividend".into());
    }
    if blob.contains("interest") {
        return ("Interest".into(), or(&raw, "INTEREST"), "interest".into());
    }
    if re_transfer().is_match(&blob) {
        return ("Transfer".into(), or(&raw, "TRANSFER"), "transfer".into());
    }
    if re_fx().is_match(&blob) {
        return ("FxExchange".into(), or(&raw, "FX"), "fx".into());
    }
    if re_fee().is_match(&blob) {
        return ("Fee".into(), or(&raw, "FEE"), "fee".into());
    }
    if blob.contains("deposit") {
        return ("Deposit".into(), or(&raw, "DEPOSIT"), "deposit".into());
    }
    if blob.contains("withdraw") {
        return ("Withdrawal".into(), or(&raw, "WITHDRAWAL"), "withdrawal".into());
    }
    (or(&raw, "Unknown"), raw.to_uppercase(), categorize(&raw, description).into())
}

/// `book_id_from_file_name`.
pub fn book_id_from_file_name(name: &str) -> String {
    let n = name.rsplit('/').next().unwrap_or("");
    re_book_dated()
        .captures(n)
        .or_else(|| re_book().captures(n))
        .map(|m| m[1].to_uppercase())
        .unwrap_or_else(|| n.to_string())
}

fn or_default(v: String, d: &str) -> String {
    if v.is_empty() { d.to_string() } else { v }
}

/// `map_statement`: (activity, issue).
pub fn map_statement(row: &CsvRow, book_id: &str) -> (Option<ActivityRow>, Option<String>) {
    let settlement = parse_date(&row.pick(&["date", "settlement_date", "transaction_date"]));
    if settlement.is_empty() {
        return (None, None);
    }
    let code = row.pick(&["transaction", "activity_type", "type", "action"]);
    let description = row.pick(&["description", "memo", "details"]);
    let parsed = parse_statement_description(&description);
    let (mut activity_type, mut sub, mut category) = map_statement_type(&code, &description);
    let mut currency = or_default(row.pick(&["currency", "ccy"]), "CAD").to_uppercase();
    if let Some(m) = re_key_ccy().captures(&parsed.symbol) {
        if currency != "CAD" && currency != "USD" {
            currency = m[1].to_uppercase();
        }
    }
    let bal_raw = row.pick(&["balance"]);
    let balance = if bal_raw.is_empty() { None } else { Some(parse_number(&bal_raw)) };
    let net_cash = parse_number(&row.pick(&["amount", "net_cash_amount", "net_amount"]));
    let compact_code = re_compact().replace_all(&code.to_uppercase(), "").into_owned();
    let stk = ["STKDIS", "STKDIV", "SPIN", "SPINOFF"].contains(&compact_code.as_str());
    if stk {
        activity_type = "STKDIS".into();
        category = "trade".into();
        sub = if parsed.shares_signed < 0.0 { "SELL".into() } else { "BUY".into() };
    }
    if parsed.fill_parsed && category == "other" && compact_code != "LOAN" && compact_code != "RECALL" {
        sub = if net_cash < 0.0 { "BUY".into() } else { "SELL".into() };
        if activity_type.is_empty() || activity_type == "Unknown" {
            activity_type = "Trade".into();
        }
        category = "trade".into();
    }
    let mut quantity = parsed.quantity;
    let mut unit_price = parsed.unit_price;
    if stk {
        unit_price = 0.0;
    }
    if parsed.fill_parsed && unit_price == 0.0 && parsed.quantity > 0.0 && !stk {
        // the statement amount is full cash; options are premium x 100
        let denom = parsed.quantity * bagholder_model::symbols::option_multiplier(&parsed.symbol);
        unit_price = if denom > 0.0 { net_cash.abs() / denom } else { 0.0 };
    }
    if category == "option_event" && parsed.fill_parsed {
        if parsed.contract_signed < 0.0 {
            sub = "BUY".into();
        } else if parsed.contract_signed > 0.0 {
            sub = "SELL".into();
        }
    }
    if sub == "SELL" {
        quantity = -quantity.abs();
    } else if sub == "BUY" {
        quantity = quantity.abs();
    }
    let issue = if category == "trade" && (sub == "BUY" || sub == "SELL") && (!parsed.fill_parsed || parsed.quantity == 0.0) {
        Some(format!("Could not parse quantity/price from description for {}", sub))
    } else {
        None
    };
    let transaction_date = if parsed.executed_at.is_empty() { settlement.clone() } else { parsed.executed_at.clone() };
    (Some(ActivityRow {
        id: uuid4(),
        occurred_at: transaction_date.clone(),
        transaction_date: transaction_date.clone(),
        settlement_date: settlement,
        book_id: trim_space(book_id).to_string(),
        activity_type,
        activity_sub_type: sub,
        description,
        symbol: parsed.symbol,
        name: parsed.name,
        currency,
        quantity,
        unit_price,
        net_cash_amount: net_cash,
        category,
        balance,
        source: "statement".into(),
        ..Default::default()
    }), issue)
}

/// `map_canonical`.
pub fn map_canonical(row: &CsvRow) -> Option<ActivityRow> {
    let transaction_date = parse_date(&row.pick(&["transaction_date", "date", "trade_date", "activity_date"]));
    if transaction_date.is_empty() {
        return None;
    }
    let activity_type = row.pick(&["activity_type", "type"]);
    let sub = row.pick(&["activity_sub_type", "activity_subtype", "sub_type", "subtype"]);
    let settlement = or_default(parse_date(&row.pick(&["settlement_date", "settle_date"])), &transaction_date);
    Some(ActivityRow {
        id: uuid4(),
        occurred_at: transaction_date.clone(),
        transaction_date: transaction_date.clone(),
        settlement_date: settlement,
        account_id: row.pick(&["account_id", "account"]),
        account_type: row.pick(&["account_type"]),
        activity_type: or_default(activity_type.clone(), "Unknown"),
        activity_sub_type: sub.clone(),
        description: row.pick(&["description", "memo", "details"]),
        direction: row.pick(&["direction"]).to_uppercase(),
        symbol: row.pick(&["symbol", "ticker"]),
        name: row.pick(&["name", "security_name", "instrument"]),
        currency: or_default(row.pick(&["currency", "ccy"]), "CAD").to_uppercase(),
        quantity: parse_number(&row.pick(&["quantity", "qty"])),
        unit_price: parse_number(&row.pick(&["unit_price", "price", "fill_price"])),
        commission: parse_number(&row.pick(&["commission", "fee", "fees"])).abs(),
        net_cash_amount: parse_number(&row.pick(&["net_cash_amount", "amount", "net_amount", "net_cash"])),
        category: categorize(&activity_type, &sub).into(),
        source: "canonical".into(),
        ..Default::default()
    })
}

/// `map_legacy`.
pub fn map_legacy(row: &CsvRow) -> Option<ActivityRow> {
    let transaction_date = parse_date(&row.pick(&["date", "transaction_date"]));
    if transaction_date.is_empty() {
        return None;
    }
    let action = row.pick(&["action", "type", "activity"]).to_lowercase();
    let (activity_type, sub): (String, String) = if action == "buy" || action == "sell" {
        ("Trade".into(), action.to_uppercase())
    } else if action.contains("dividend") {
        ("Dividend".into(), "DIVIDEND".into())
    } else if action.contains("deposit") {
        ("Deposit".into(), "DEPOSIT".into())
    } else if action.contains("withdraw") {
        ("Withdrawal".into(), "WITHDRAWAL".into())
    } else if action.contains("interest") {
        ("Interest".into(), "INTEREST".into())
    } else if action.contains("fee") {
        ("Fee".into(), "FEE".into())
    } else if action.contains("fx") {
        ("FxExchange".into(), action.to_uppercase())
    } else if !action.is_empty() {
        let mut ch = action.chars();
        let first = ch.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
        (format!("{}{}", first, ch.as_str()), action.to_uppercase())
    } else {
        ("Other".into(), String::new())
    };
    let mut quantity = parse_number(&row.pick(&["quantity", "qty"]));
    if sub == "SELL" {
        quantity = -quantity.abs();
    } else if sub == "BUY" {
        quantity = quantity.abs();
    }
    Some(ActivityRow {
        id: uuid4(),
        occurred_at: transaction_date.clone(),
        transaction_date: transaction_date.clone(),
        settlement_date: transaction_date,
        account_id: or_default(row.pick(&["account_id", "account"]), "legacy"),
        account_type: row.pick(&["account_type"]),
        activity_type: activity_type.clone(),
        activity_sub_type: sub.clone(),
        description: row.pick(&["description", "memo"]),
        symbol: row.pick(&["symbol", "ticker"]),
        name: row.pick(&["name", "security_name"]),
        currency: or_default(row.pick(&["currency", "ccy"]), "CAD").to_uppercase(),
        quantity,
        unit_price: parse_number(&row.pick(&["price", "unit_price"])),
        commission: parse_number(&row.pick(&["commission", "fee", "fees"])).abs(),
        net_cash_amount: parse_number(&row.pick(&["amount", "net_cash_amount", "net_amount"])),
        category: categorize(&activity_type, &sub).into(),
        source: "legacy".into(),
        ..Default::default()
    })
}

fn skip(row: usize, message: &str, raw: String) -> Skipped {
    Skipped { row, message: message.to_string(), raw }
}

/// `parse_csv`: format, the parsed rows, what was skipped and why, whether a
/// footer line was stripped, and a count by activity type. An error is the
/// file the csv reader refuses.
pub fn parse_csv(text: &str, name: &str) -> std::result::Result<CsvReport, String> {
    let text = text.replace('\u{feff}', "");
    let mut footer = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in splitlines(&text) {
        if is_footer_line(line) {
            footer = true;
            continue;
        }
        lines.push(line);
    }
    let mut table = csv_rows(&lines.join("\n"))?;
    while table.last().map(|r| r.iter().all(|c| trim_space(c).is_empty())).unwrap_or(false) {
        table.pop();
    }
    if table.is_empty() {
        return Ok(CsvReport {
            format: "unknown".into(),
            skipped: vec![skip(1, "Empty file", String::new())],
            footer_stripped: footer,
            ..Default::default()
        });
    }
    let headers: Vec<String> = table[0].iter().map(|h| trim_space(&h.replace('\u{feff}', "")).to_string()).collect();
    let fmt = detect_format(&headers);
    let norms: Vec<String> = headers.iter().map(|h| normalize_header(h)).collect();
    let mut skipped: Vec<Skipped> = Vec::new();
    let mut activities: Vec<ActivityRow> = Vec::new();
    let mut counts: CountsByType = IndexMap::new();
    if fmt == "unknown" {
        skipped.push(skip(1, "Unrecognized CSV format. Expected a Wealthsimple activities export, a statement export with date/transaction/description/amount columns, or a Date/Action/Symbol file.", headers.join(",")));
        return Ok(CsvReport { format: fmt.to_string(), skipped, footer_stripped: footer, row_count: table.len() - 1, ..Default::default() });
    }
    let book = book_id_from_file_name(name);
    for (i, cells) in table.iter().enumerate().skip(1) {
        let i = i + 1;
        let row = CsvRow::from_cells(&norms, cells);
        if row.values().all(|v| trim_space(v).is_empty()) {
            continue;
        }
        if is_footer_line(&row.values().collect::<Vec<_>>().join(" ")) {
            footer = true;
            continue;
        }
        let raw = row.raw();
        let (activity, issue) = match fmt {
            "statement" => map_statement(&row, &book),
            "legacy" => (map_legacy(&row), None),
            _ => (map_canonical(&row), None),
        };
        if let Some(issue) = issue {
            skipped.push(skip(i, &issue, raw.clone()));
        }
        let activity = match activity {
            Some(a) => a,
            None => {
                skipped.push(skip(i, "Unparsed row (missing or invalid date)", raw));
                continue;
            }
        };
        let key = if activity.activity_type.is_empty() { activity.category.clone() } else { activity.activity_type.clone() };
        *counts.entry(key).or_insert(0) += 1;
        activities.push(activity);
    }
    Ok(CsvReport { format: fmt.to_string(), activities, skipped, footer_stripped: footer, counts_by_type: counts, row_count: table.len() - 1 })
}

/// `import_text`: parse one CSV and merge it into the store.
pub fn import_text(conn: &Connection, name: &str, text: &str) -> std::result::Result<ImportReport, String> {
    let report = parse_csv(text, name)?;
    let (added, duplicates) = if report.activities.is_empty() {
        (0, 0)
    } else {
        let merged = crate::merge::merge_local_rows(conn, &report.activities, &uuid4).map_err(|e| e.to_string())?;
        (merged.added, merged.duplicates)
    };
    Ok(ImportReport {
        ok: true,
        file: name.rsplit('/').next().unwrap_or("").to_string(),
        format: report.format,
        rows: report.row_count,
        added,
        duplicates,
        skipped_count: report.skipped.len(),
        skipped: report.skipped.into_iter().take(20).collect(),
        footer_stripped: report.footer_stripped,
        counts_by_type: report.counts_by_type,
    })
}

// --- folder watching (the server scans; no browser needed) ---------------------

/// `is_junk_name`.
pub fn is_junk_name(name: &str) -> bool {
    name.starts_with("._") || name.to_uppercase().contains("__MACOSX")
}

/// `is_csv_name`.
pub fn is_csv_name(name: &str) -> bool {
    name.to_lowercase().ends_with(".csv")
}

/// `os.path.expanduser`.
pub fn expanduser(p: &str) -> String {
    if p == "~" || p.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home.trim_end_matches('/'), &p[1..]);
        }
    }
    p.to_string()
}

/// One CSV file a watched folder holds, as `list_csv_files` reports it.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
pub struct CsvFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime: i64,
}

/// `list_csv_files`.
pub fn list_csv_files(folder: &str) -> Vec<CsvFile> {
    let mut names: Vec<String> = match std::fs::read_dir(folder) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => return vec![],
    };
    names.sort();
    let mut out = Vec::new();
    for n in names {
        let p = format!("{}/{}", folder.trim_end_matches('/'), n);
        let meta = match std::fs::metadata(&p) { Ok(m) => m, Err(_) => continue };
        if !meta.is_file() || !is_csv_name(&n) || is_junk_name(&n) {
            continue;
        }
        if meta.len() == 0 {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(CsvFile { path: p, name: n, size: meta.len(), mtime });
    }
    out
}

/// `watch_folder`.
pub fn watch_folder(conn: &Connection) -> Result<String> {
    crate::tables::get_meta(conn, WATCH_META, "")
}

/// What `set_watch_folder` (and, on success, the same-shaped part of
/// `scan_folder`) answers: a folder taken, or the reason it was refused.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchSet {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `set_watch_folder`.
pub fn set_watch_folder(conn: &Connection, path: &str) -> Result<WatchSet> {
    let p = expanduser(trim_space(path));
    if p.is_empty() {
        return Ok(WatchSet { ok: false, error: Some("Folder path required".into()), path: None });
    }
    if !std::path::Path::new(&p).is_dir() {
        return Ok(WatchSet { ok: false, error: Some(format!("Not a folder: {}", p)), path: None });
    }
    crate::tables::set_meta(conn, WATCH_META, &p)?;
    Ok(WatchSet { ok: true, error: None, path: Some(p) })
}

/// `clear_watch_folder`.
pub fn clear_watch_folder(conn: &Connection) -> Result<()> {
    crate::tables::set_meta(conn, WATCH_META, "")?;
    crate::tables::set_meta(conn, WATCH_FILES_META, "")?;
    crate::tables::set_meta(conn, WATCH_LAST_META, "")
}

/// One watched file's last scan, kept across scans in the `watch_files` meta
/// blob -- read leniently, since an older build's blob may carry looser types.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SeenFile {
    #[serde(deserialize_with = "lenient::number")]
    pub size: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub mtime: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub added: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub duplicates: f64,
    #[serde(deserialize_with = "lenient::text")]
    pub format: String,
    #[serde(deserialize_with = "lenient::text", rename = "scannedAt")]
    pub scanned_at: String,
}

fn seen_files(conn: &Connection) -> Result<IndexMap<String, SeenFile>> {
    let raw = crate::tables::get_meta(conn, WATCH_FILES_META, "")?;
    Ok(match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(m)) => m.into_iter().filter(|(_, v)| v.is_object()).filter_map(|(k, v)| SeenFile::deserialize(v).ok().map(|s| (k, s))).collect(),
        _ => IndexMap::new(),
    })
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = bagholder_model::dates::from_days(secs.div_euclid(86400));
    let r = secs.rem_euclid(86400);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, r / 3600, (r % 3600) / 60, r % 60)
}

/// The error text for a file that would not open: `[Errno N] message: 'path'`.
fn os_error(e: &std::io::Error, path: &str) -> String {
    let code = e.raw_os_error().unwrap_or(0);
    let msg = e.to_string();
    let words = msg.split(" (os error").next().unwrap_or(&msg);
    format!("[Errno {}] {}: '{}'", code, words, path)
}

/// One file `scan_folder` looked at: unchanged since the last scan, a read
/// that failed, or one it just imported.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(untagged)]
pub enum ScannedFile {
    Unchanged { file: String, unchanged: bool, added: f64, duplicates: f64, format: String },
    Failed { file: String, error: String },
    Scanned {
        file: String,
        unchanged: bool,
        added: usize,
        duplicates: usize,
        format: String,
        rows: usize,
        #[serde(rename = "skippedCount")]
        skipped_count: usize,
    },
}

/// What `scan_folder` answers: a refusal, or the folder scanned and what each
/// file in it did.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicates: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<ScannedFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
}

/// `scan_folder`: import every top-level CSV in the folder;
/// unchanged files are skipped unless forced.
pub fn scan_folder(conn: &Connection, folder: Option<&str>, force: bool) -> Result<ScanReport> {
    let given = match folder { Some(f) if !f.is_empty() => f.to_string(), _ => watch_folder(conn)? };
    let path = expanduser(trim_space(&given));
    if path.is_empty() {
        return Ok(ScanReport { ok: false, error: Some("No folder is being watched".into()), ..Default::default() });
    }
    if !std::path::Path::new(&path).is_dir() {
        return Ok(ScanReport { ok: false, error: Some(format!("Folder not found: {}", path)), path: Some(path), ..Default::default() });
    }
    let mut seen = seen_files(conn)?;
    let mut files: Vec<ScannedFile> = Vec::new();
    let (mut added, mut duplicates) = (0i64, 0i64);
    for f in list_csv_files(&path) {
        let prev = seen.get(&f.path).cloned();
        if !force && prev.as_ref().is_some_and(|p| p.size == f.size as f64 && p.mtime == f.mtime as f64) {
            let p = prev.unwrap();
            files.push(ScannedFile::Unchanged { file: f.name.clone(), unchanged: true, added: p.added, duplicates: p.duplicates, format: p.format.clone() });
            continue;
        }
        let text = match std::fs::read(&f.path) {
            // utf-8-sig with errors="replace"
            Ok(bytes) => {
                let t = String::from_utf8_lossy(&bytes).into_owned();
                t.strip_prefix('\u{feff}').map(|s| s.to_string()).unwrap_or(t)
            }
            Err(e) => {
                files.push(ScannedFile::Failed { file: f.name.clone(), error: os_error(&e, &f.path) });
                continue;
            }
        };
        let rep = match import_text(conn, &f.name, &text) {
            Ok(r) => r,
            Err(e) => return Err(rusqlite::Error::ToSqlConversionFailure(e.into())),
        };
        added += rep.added as i64;
        duplicates += rep.duplicates as i64;
        seen.insert(f.path.clone(), SeenFile {
            size: f.size as f64,
            mtime: f.mtime as f64,
            added: rep.added as f64,
            duplicates: rep.duplicates as f64,
            format: rep.format.clone(),
            scanned_at: stamp(),
        });
        files.push(ScannedFile::Scanned {
            file: f.name.clone(),
            unchanged: false,
            added: rep.added,
            duplicates: rep.duplicates,
            format: rep.format,
            rows: rep.rows,
            skipped_count: rep.skipped_count,
        });
    }
    let now = stamp();
    let kept: Map<String, Value> = seen
        .into_iter()
        .filter(|(k, _)| std::path::Path::new(k).exists())
        .map(|(k, v)| (k, serde_json::to_value(v).unwrap_or(Value::Null)))
        .collect();
    crate::tables::set_meta(conn, WATCH_FILES_META, &crate::tables::json_text(&Value::Object(kept)))?;
    crate::tables::set_meta(conn, WATCH_LAST_META, &now)?;
    Ok(ScanReport { ok: true, error: None, path: Some(path), added: Some(added), duplicates: Some(duplicates), files: Some(files), scanned_at: Some(now) })
}

/// One watched file, as `status` reports it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct StatusFile {
    pub file: String,
    pub added: f64,
    pub duplicates: f64,
    pub format: String,
    pub scanned_at: String,
}

/// `GET /api/watch`'s answer: the folder, whether one is set, when it was
/// last scanned, and each file it has seen.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub ok: bool,
    pub path: String,
    pub watching: bool,
    pub last_scan: String,
    pub files: Vec<StatusFile>,
}

/// `status`.
pub fn status(conn: &Connection) -> Result<WatchStatus> {
    let path = watch_folder(conn)?;
    let seen = seen_files(conn)?;
    let mut keys: Vec<&String> = seen.keys().collect();
    keys.sort();
    let files: Vec<StatusFile> = keys
        .into_iter()
        .map(|k| {
            let v = &seen[k];
            StatusFile {
                file: k.rsplit('/').next().unwrap_or("").to_string(),
                added: v.added,
                duplicates: v.duplicates,
                format: v.format.clone(),
                scanned_at: v.scanned_at.clone(),
            }
        })
        .collect();
    let watching = !path.is_empty();
    Ok(WatchStatus { ok: true, path, watching, last_scan: crate::tables::get_meta(conn, WATCH_LAST_META, "")?, files })
}
