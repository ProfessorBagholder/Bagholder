//! A file the person imports (`docs/plans/stage-3c-switch.md` §6): a source with
//! no execution and no ids of its own. Each row is a record, kept as the file
//! wrote it, keyed by the account it goes to and its content, so a row that two
//! overlapping exports both carry is one record, and two identical rows in one
//! file are two (their occurrence tells them apart).
//!
//! Three layouts are read, each by the headers it must have: Wealthsimple's
//! activity export (`transaction_date`, `activity_type`, `activity_sub_type`,
//! `net_cash_amount`), its statement export (`date`, `transaction`,
//! `description`, `amount`), and a plain one (`date`, `action`, `symbol`,
//! `quantity`, `price`, `amount`). Everything is read strictly: a day is written
//! `YYYY-MM-DD`, a number as digits with an optional sign, currency mark,
//! thousands commas and parentheses for a negative; anything else is not read,
//! and the row is kept with its problem, counted in no figure.
//!
//! Reading a row (`state`) is shared by the import, which names the account and
//! the instrument the book knows it by (`Payload`), and the mapping, which reads
//! the kept cells again.

use std::collections::BTreeMap;

use bagholder_book::import::mapping::{rule, Cash, Qty};
use bagholder_book::mapping::{Draft, MapContext, Mapped, Mapping};
use bagholder_book::person::Named;
use bagholder_core::account::AccountRef;
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::record::Problem;
use bagholder_core::transaction::Kind;
use bagholder_core::{Broker, Currency, Dec, Leg, Money, SourceName};
use serde::{Deserialize, Serialize};

pub const SOURCE: &str = "csv";

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

/// Which export a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    Activities,
    Statement,
    Simple,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Activities, Layout::Statement, Layout::Simple];

    /// The headers a file of this layout must have.
    pub fn required(self) -> &'static [&'static str] {
        match self {
            Layout::Activities => &["transaction_date", "activity_type", "activity_sub_type", "net_cash_amount"],
            Layout::Statement => &["date", "transaction", "description", "amount"],
            Layout::Simple => &["date", "action", "symbol", "quantity", "price", "amount"],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Layout::Activities => "activities",
            Layout::Statement => "statement",
            Layout::Simple => "simple",
        }
    }
}

/// A file read: its layout and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRead {
    pub layout: Layout,
    pub rows: Vec<RowRead>,
    /// Lines that are the export's notes (`As of 2024-01-31`), not rows.
    pub notes: Vec<usize>,
}

/// One row: its line in the file, its cells by header, and how many rows
/// before it in the file had the same cells.
#[derive(Clone, Debug, PartialEq)]
pub struct RowRead {
    pub line: usize,
    pub cells: BTreeMap<String, String>,
    pub occurrence: u32,
}

/// The key a cell past the header's last column is kept under: no header
/// normalizes to it, and a row carrying one does not read.
fn extra(column: usize) -> String {
    format!("#{column}")
}

/// A header as the layouts name it: lower case, spaces and hyphens as `_`.
pub fn header(h: &str) -> String {
    let h = h.trim_start_matches('\u{feff}').trim().to_lowercase();
    let mut out = String::new();
    let mut gap = false;
    for c in h.chars() {
        if c.is_whitespace() || c == '-' {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        }
    }
    out
}

/// The file's lines as CSV records (RFC 4180), each with the line it starts on.
/// A quote left open, or one inside a cell that is not quoted, is a file that
/// does not read.
fn records(text: &str) -> Result<Vec<(usize, Vec<String>)>, String> {
    let mut out = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let (mut line, mut start) = (1usize, 1usize);
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut was_quoted = false;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    cell.push(c);
                }
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() && !was_quoted => {
                quoted = true;
                was_quoted = true;
            }
            '"' => return Err(format!("line {line} has a quote inside a cell that is not quoted")),
            ',' => {
                row.push(std::mem::take(&mut cell));
                was_quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                was_quoted = false;
                out.push((start, std::mem::take(&mut row)));
                line += 1;
                start = line;
            }
            _ if was_quoted => return Err(format!("line {line} has text after a quoted cell's closing quote")),
            _ => cell.push(c),
        }
    }
    if quoted {
        return Err(format!("the quote opened on line {start} is never closed"));
    }
    if !cell.is_empty() || !row.is_empty() || was_quoted {
        row.push(cell);
        out.push((start, row));
    }
    Ok(out)
}

/// A line that is an export's note rather than a row: `As of YYYY-MM-DD…` in its
/// first cell and nothing in the others.
fn is_note(cells: &[String]) -> bool {
    let first = cells.first().map(|c| c.trim()).unwrap_or("");
    let rest_empty = cells.iter().skip(1).all(|c| c.trim().is_empty());
    let Some(after) = first.strip_prefix("As of ") else { return false };
    rest_empty && after.get(..10).is_some_and(|d| d.parse::<jiff::civil::Date>().is_ok())
}

/// Read a file: its layout from its header row, then every row. A file whose
/// header is no layout's, or that does not read as CSV, is refused with why.
pub fn read_file(text: &str) -> Result<FileRead, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = records(text)?.into_iter().filter(|(_, cells)| cells.iter().any(|c| !c.trim().is_empty()));
    let Some((_, head)) = lines.next() else { return Err("the file is empty".into()) };
    let headers: Vec<String> = head.iter().map(|h| header(h)).collect();
    for (i, h) in headers.iter().enumerate() {
        if headers[..i].contains(h) {
            return Err(format!("the header {h:?} is there twice"));
        }
    }
    let fits: Vec<Layout> = Layout::ALL.into_iter().filter(|l| l.required().iter().all(|r| headers.iter().any(|h| h == r))).collect();
    let layout = match fits.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "its headers ({}) are none of the layouts read: {}",
                headers.join(", "),
                Layout::ALL.iter().map(|l| format!("{} ({})", l.as_str(), l.required().join(", "))).collect::<Vec<_>>().join("; ")
            ))
        }
        more => return Err(format!("its headers fit more than one layout: {}", more.iter().map(|l| l.as_str()).collect::<Vec<_>>().join(", "))),
    };
    let mut rows: Vec<RowRead> = Vec::new();
    let mut notes = Vec::new();
    for (line, cells) in lines {
        if is_note(&cells) {
            notes.push(line);
            continue;
        }
        let mut by: BTreeMap<String, String> = BTreeMap::new();
        for (i, v) in cells.into_iter().enumerate() {
            let key = headers.get(i).cloned().unwrap_or_else(|| extra(i + 1));
            if !(key.starts_with('#') && v.trim().is_empty()) {
                by.insert(key, v);
            }
        }
        let occurrence = rows.iter().filter(|r| r.cells == by).count() as u32;
        rows.push(RowRead { line, cells: by, occurrence });
    }
    Ok(FileRead { layout, rows, notes })
}

/// What a row states, read strictly.
#[derive(Clone, Debug, PartialEq)]
pub struct Stated {
    pub day: jiff::civil::Date,
    pub settle: Option<jiff::civil::Date>,
    pub kind: Kind,
    /// The instrument, by the symbol the row gives and what kind it is.
    pub instrument: Option<(String, InstrumentKind)>,
    /// Signed: into the account is positive.
    pub quantity: Option<Dec>,
    pub price: Option<Dec>,
    /// Signed: into the account is positive.
    pub cash: Option<Dec>,
    pub fee: Option<Dec>,
    pub currency: Option<Currency>,
    /// The account the row names, as the file writes it.
    pub account: Option<String>,
    /// For a distribution, the units the row states it was paid on.
    pub paid_on: Option<Dec>,
    /// What the row states that the kind does not place (a row read, with its problems).
    pub problems: Vec<Problem>,
}

fn unreadable(why: String) -> Problem {
    Problem::new("unreadable", why)
}

fn cell<'a>(cells: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    cells.get(name).map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// A day, written `YYYY-MM-DD`.
pub fn day(what: &str, v: &str) -> Result<jiff::civil::Date, Problem> {
    let ok = v.len() == 10 && v.as_bytes()[4] == b'-' && v.as_bytes()[7] == b'-';
    let parsed = if ok { v.parse::<jiff::civil::Date>().ok() } else { None };
    parsed.ok_or_else(|| unreadable(format!("the {what} {v:?} is not a day written YYYY-MM-DD")))
}

/// A number as an export writes one: an optional sign, an optional `$`, digits
/// with commas between thousands, an optional fraction; or the same in
/// parentheses, a negative.
pub fn number(what: &str, v: &str) -> Result<Dec, Problem> {
    let bad = || unreadable(format!("the {what} {v:?} is not a number"));
    let (neg_paren, inner) = match v.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => (false, v),
    };
    let (neg_sign, inner) = match inner.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, inner.strip_prefix('+').unwrap_or(inner)),
    };
    if neg_paren && neg_sign {
        return Err(bad());
    }
    let inner = inner.strip_prefix('$').unwrap_or(inner);
    let (whole, frac) = match inner.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (inner, None),
    };
    let groups: Vec<&str> = whole.split(',').collect();
    let grouped = groups.len() == 1 || (!groups[0].is_empty() && groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !grouped || !groups.iter().all(|g| digits(g)) || frac.is_some_and(|f| !digits(f)) {
        return Err(bad());
    }
    let text = format!("{}{}{}", if neg_paren || neg_sign { "-" } else { "" }, groups.concat(), frac.map(|f| format!(".{f}")).unwrap_or_default());
    Dec::parse(&text).map_err(|_| bad())
}

fn opt_number(cells: &BTreeMap<String, String>, name: &str, what: &str) -> Result<Option<Dec>, Problem> {
    cell(cells, name).map(|v| number(what, v)).transpose()
}

fn currency(cells: &BTreeMap<String, String>) -> Result<Option<Currency>, Problem> {
    cell(cells, "currency").map(|v| Currency::parse(&v.to_uppercase()).map_err(|_| unreadable(format!("the currency {v:?} is not a currency")))).transpose()
}

/// Read a row of `layout`: what it states, or why it does not read.
pub fn state(layout: Layout, cells: &BTreeMap<String, String>) -> Result<Stated, Problem> {
    if let Some(k) = cells.keys().find(|k| k.starts_with('#')) {
        return Err(unreadable(format!("the row has a cell in column {} past the header's last", &k[1..])));
    }
    match layout {
        Layout::Activities => activities(cells),
        Layout::Statement => statement(cells),
        Layout::Simple => simple(cells),
    }
}

/// Wealthsimple's activity export: its activity type and sub-type read by the
/// same table as Wealthsimple's rows everywhere (`import::mapping::rule`).
fn activities(cells: &BTreeMap<String, String>) -> Result<Stated, Problem> {
    let day_text = cell(cells, "transaction_date").ok_or_else(|| unreadable("the row has no transaction_date".into()))?;
    let date = day("transaction_date", day_text)?;
    let settle = cell(cells, "settlement_date").map(|v| day("settlement_date", v)).transpose()?;
    let ty = cell(cells, "activity_type").unwrap_or("");
    let sub = cell(cells, "activity_sub_type").unwrap_or("");
    let cash = opt_number(cells, "net_cash_amount", "net_cash_amount")?;
    let quantity = opt_number(cells, "quantity", "quantity")?;
    let price = opt_number(cells, "unit_price", "unit_price")?;
    let commission = opt_number(cells, "commission", "commission")?;
    let direction = cell(cells, "direction").unwrap_or("").to_uppercase();
    let mut out = Stated {
        day: date,
        settle,
        kind: Kind::Unclassified,
        instrument: None,
        quantity: None,
        price: None,
        cash: None,
        fee: commission.filter(|c| !c.is_zero()).map(|c| c.abs()),
        currency: currency(cells)?,
        account: cell(cells, "account_id").map(str::to_string),
        paid_on: None,
        problems: vec![],
    };
    let Some(r) = rule(ty, sub, cash, &direction) else {
        out.problems.push(Problem::new("unclassified", format!("a row of activity type {ty:?} and sub-type {sub:?}, which is not placed")));
        return Ok(out);
    };
    out.kind = r.kind;
    out.quantity = match (r.quantity, quantity) {
        (Qty::None, _) | (_, None) => None,
        (Qty::In, Some(q)) => Some(q.abs()),
        (Qty::Out, Some(q)) => Some(q.abs().neg()),
        (Qty::AsSigned, Some(q)) => Some(q),
    };
    out.cash = match (r.cash, cash) {
        (Cash::None, _) | (_, None) => None,
        (Cash::AsSigned, Some(c)) => Some(c),
        (Cash::Paid, Some(c)) => Some(c.abs().neg()),
        (Cash::Received, Some(c)) => Some(c.abs()),
    };
    if let Some(kind) = r.instrument {
        match cell(cells, "symbol") {
            Some(s) => out.instrument = Some((s.to_string(), kind)),
            None => out.problems.push(Problem::new("instrument-not-named", format!("a {} that names no symbol", r.kind))),
        }
    }
    if out.instrument.is_some() && !matches!(r.quantity, Qty::None) && out.quantity.is_none_or(|q| q.is_zero()) {
        out.problems.push(Problem::new("quantity-not-stated", format!("a {} with no quantity", r.kind)));
        out.quantity = None;
    }
    out.price = price.filter(|_| out.quantity.is_some());
    if r.kind == Kind::Dividend {
        out.paid_on = quantity.map(|q| q.abs()).filter(|q| q.is_positive());
    }
    Ok(out)
}

/// `SYM - Name: …` or `SYM: …`, the way a statement's description names what the
/// row is about.
fn described_symbol(description: &str) -> Option<String> {
    let (left, _) = description.split_once(':')?;
    let sym = left.split_once(" - ").map(|(s, _)| s).unwrap_or(left).trim();
    let ok = !sym.is_empty() && sym.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '.' || c == '-');
    ok.then(|| sym.to_string())
}

/// The units a statement's description says moved: `Bought 10.0000 shares` or
/// `Sold 2 shares`.
fn described_units(description: &str) -> Option<(bool, Dec)> {
    let words: Vec<&str> = description.split_whitespace().collect();
    words.windows(3).find_map(|w| {
        let bought = match w[0] {
            "Bought" => true,
            "Sold" => false,
            _ => return None,
        };
        if !matches!(w[2].trim_end_matches([',', '.']), "share" | "shares") {
            return None;
        }
        number("quantity", w[1]).ok().filter(|q| q.is_positive()).map(|q| (bought, q))
    })
}

/// `(executed at YYYY-MM-DD)`: the day a fill was made, where the statement
/// files it under its settlement.
fn executed_at(description: &str) -> Result<Option<jiff::civil::Date>, Problem> {
    let Some((_, after)) = description.split_once("(executed at ") else { return Ok(None) };
    let v = after.split(')').next().unwrap_or("");
    day("executed-at day", v.trim()).map(Some)
}

/// Wealthsimple's statement export: a transaction code, and a description that
/// names what a fill moved.
fn statement(cells: &BTreeMap<String, String>) -> Result<Stated, Problem> {
    let date = day("date", cell(cells, "date").ok_or_else(|| unreadable("the row has no date".into()))?)?;
    let code = cell(cells, "transaction").unwrap_or("").to_uppercase();
    let description = cell(cells, "description").unwrap_or("");
    let amount = number("amount", cell(cells, "amount").ok_or_else(|| unreadable("the row has no amount".into()))?)?;
    let mut out = Stated {
        day: date,
        settle: None,
        kind: Kind::Unclassified,
        instrument: None,
        quantity: None,
        price: None,
        cash: Some(amount),
        fee: None,
        currency: currency(cells)?,
        account: None,
        paid_on: None,
        problems: vec![],
    };
    let kind = match code.as_str() {
        "BUY" => Kind::Buy,
        "SELL" => Kind::Sell,
        "DIV" => Kind::Dividend,
        "INT" => Kind::Interest,
        "FEE" => Kind::Fee,
        "CONT" => Kind::Deposit,
        "WD" => Kind::Withdrawal,
        "TRFIN" => Kind::TransferIn,
        "TRFOUT" => Kind::TransferOut,
        _ => {
            out.problems.push(Problem::new("unclassified", format!("a row of transaction code {code:?}, which is not placed")));
            return Ok(out);
        }
    };
    out.kind = kind;
    if matches!(kind, Kind::Buy | Kind::Sell | Kind::Dividend) {
        match described_symbol(description) {
            Some(s) => out.instrument = Some((s, InstrumentKind::Security)),
            None => out.problems.push(Problem::new("instrument-not-named", format!("a {kind} whose description names no symbol: {description:?}"))),
        }
    }
    if matches!(kind, Kind::Buy | Kind::Sell) {
        if let Some(d) = executed_at(description)? {
            out.settle = Some(out.day);
            out.day = d;
        }
        match described_units(description) {
            Some((bought, q)) if bought == (kind == Kind::Buy) => out.quantity = Some(if bought { q } else { q.neg() }),
            Some(_) => out.problems.push(Problem::new("sign-against-kind", format!("a {kind} whose description says the other way: {description:?}"))),
            None => out.problems.push(Problem::new("quantity-not-stated", format!("a {kind} whose description states no units: {description:?}"))),
        }
    }
    Ok(out)
}

/// A plain file: an action, a symbol, units, a price and an amount a row.
fn simple(cells: &BTreeMap<String, String>) -> Result<Stated, Problem> {
    let date = day("date", cell(cells, "date").ok_or_else(|| unreadable("the row has no date".into()))?)?;
    let action = cell(cells, "action").unwrap_or("").to_lowercase();
    let quantity = opt_number(cells, "quantity", "quantity")?;
    let price = opt_number(cells, "price", "price")?;
    let amount = opt_number(cells, "amount", "amount")?;
    let fee = match (cell(cells, "fee"), cell(cells, "commission")) {
        (Some(_), Some(_)) => return Err(unreadable("the row states both a fee and a commission".into())),
        (Some(v), None) => Some(number("fee", v)?),
        (None, Some(v)) => Some(number("commission", v)?),
        (None, None) => None,
    };
    let mut out = Stated {
        day: date,
        settle: None,
        kind: Kind::Unclassified,
        instrument: None,
        quantity: None,
        price: None,
        cash: amount,
        fee: fee.filter(|f| !f.is_zero()).map(|f| f.abs()),
        currency: currency(cells)?,
        account: cell(cells, "account").map(str::to_string),
        paid_on: None,
        problems: vec![],
    };
    let kind = match action.as_str() {
        "buy" => Kind::Buy,
        "sell" => Kind::Sell,
        "dividend" => Kind::Dividend,
        "interest" => Kind::Interest,
        "fee" => Kind::Fee,
        "deposit" => Kind::Deposit,
        "withdrawal" => Kind::Withdrawal,
        _ => {
            out.problems.push(Problem::new("unclassified", format!("a row of action {action:?}, which is not placed")));
            return Ok(out);
        }
    };
    out.kind = kind;
    let symbol = cell(cells, "symbol");
    if matches!(kind, Kind::Buy | Kind::Sell | Kind::Dividend) {
        match symbol {
            Some(s) => out.instrument = Some((s.to_string(), InstrumentKind::Security)),
            None => out.problems.push(Problem::new("instrument-not-named", format!("a {kind} that names no symbol"))),
        }
    }
    if matches!(kind, Kind::Buy | Kind::Sell) {
        match quantity.filter(|q| !q.is_zero()) {
            Some(q) => out.quantity = Some(if kind == Kind::Buy { q.abs() } else { q.abs().neg() }),
            None => out.problems.push(Problem::new("quantity-not-stated", format!("a {kind} with no quantity"))),
        }
        out.price = price.filter(|_| out.quantity.is_some());
        // a trade's amount is what it paid or received, whichever way the file signs it
        out.cash = amount.map(|a| if kind == Kind::Buy { a.abs().neg() } else { a.abs() });
    }
    if kind == Kind::Dividend {
        out.paid_on = quantity.map(|q| q.abs()).filter(|q| q.is_positive());
    }
    Ok(out)
}

/// A row as the book keeps it: the cells as the file wrote them, the account it
/// goes to and the instrument the book knows it by, both named when it was
/// imported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub layout: Layout,
    pub cells: BTreeMap<String, String>,
    pub occurrence: u32,
    /// The broker and the broker's id of the account.
    pub account: (String, String),
    /// Why the row is not placed in that account (it names another the book does not hold).
    pub unplaced: Option<String>,
    pub instrument: Option<Named>,
}

impl Payload {
    /// The record's key: the account, the layout, the cells and the occurrence,
    /// never the file's name, so the same row in another export is the same record.
    pub fn key(&self) -> String {
        serde_json::to_string(&(&self.account, self.layout, &self.cells, self.occurrence)).expect("strings serialize")
    }
}

/// The file's rows as transactions, version 1.
pub struct CsvMapping;

fn leg() -> Leg {
    Leg::named("row")
}

impl Mapping for CsvMapping {
    fn source(&self) -> SourceName {
        source()
    }

    fn version(&self) -> u32 {
        1
    }

    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let p: Payload = match serde_json::from_str(payload) {
            Ok(p) => p,
            Err(e) => return Mapped::unreadable(format!("a file's row that does not read: {e}")),
        };
        let account = match Broker::parse(&p.account.0) {
            Ok(b) => AccountRef::new(b, p.account.1.clone()),
            Err(e) => return Mapped::unreadable(format!("a file's row in an account that does not read: {e}")),
        };
        let s = match state(p.layout, &p.cells) {
            Ok(s) => s,
            Err(problem) => return Mapped { problems: vec![problem], ..Mapped::default() },
        };
        let unplaced = |day, problems: Vec<Problem>| Mapped {
            legs: vec![Draft {
                leg: leg(),
                account: account.clone(),
                occurred_at: None,
                trade_date: day,
                settle_date: None,
                kind: Kind::Unclassified,
                effect: None,
                instrument: None,
                quantity: None,
                price: None,
                cash: None,
                fee: None,
                fx_rate: None,
                paid_on: None,
            }],
            problems,
            adjustments: vec![],
        };
        let mut problems = s.problems.clone();
        if let Some(why) = &p.unplaced {
            problems.push(Problem::new("account-unknown", why.clone()));
            return unplaced(s.day, problems);
        }
        if s.kind == Kind::Unclassified {
            return unplaced(s.day, problems);
        }
        // the file's currency, or the instrument's the book named where the file states none
        let currency = s.currency.or_else(|| p.instrument.as_ref().and_then(|n| Currency::parse(&n.currency).ok()));
        let Some(currency) = currency else {
            problems.push(Problem::new("currency-unstated", format!("a {} whose currency the file does not state", s.kind)));
            return unplaced(s.day, problems);
        };
        let instrument = match (&s.instrument, &p.instrument) {
            (None, _) => None,
            (Some(_), Some(n)) => match n.draft(s.day) {
                Ok(d) => Some(d),
                Err(why) => return Mapped::unreadable(format!("a file's row whose instrument does not read: {why}")),
            },
            (Some((symbol, _)), None) => {
                problems.push(Problem::new("instrument-not-named", format!("the book named no instrument for {symbol}")));
                return unplaced(s.day, problems);
            }
        };
        let money = |d: Dec| Money::new(d, currency);
        let quantity = if instrument.is_some() { s.quantity } else { None };
        Mapped {
            legs: vec![Draft {
                leg: leg(),
                account,
                occurred_at: None,
                trade_date: s.day,
                settle_date: s.settle,
                kind: s.kind,
                effect: None,
                instrument,
                quantity,
                price: s.price.filter(|_| quantity.is_some()).map(money),
                cash: s.cash.map(money),
                fee: s.fee.map(money),
                fx_rate: None,
                paid_on: s.paid_on,
            }],
            problems,
            adjustments: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_file_is_read_as_the_layout_its_headers_are() {
        let f = read_file("\u{feff}Transaction Date,Activity-Type,activity_sub_type,Net Cash Amount,symbol\n2024-01-15,Trade,BUY,-255.00,AAA\n").unwrap();
        assert_eq!(f.layout, Layout::Activities);
        assert_eq!(f.rows[0].cells["transaction_date"], "2024-01-15");
        assert_eq!(read_file("date,transaction,description,amount,balance\n").unwrap().layout, Layout::Statement);
        assert_eq!(read_file("Date,Action,Symbol,Quantity,Price,Amount\n").unwrap().layout, Layout::Simple);
        for bad in ["foo,bar\n1,2\n", "", "\n\n", "date,date,transaction,description,amount\n"] {
            assert!(read_file(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn csv_is_read_strictly() {
        let f = read_file("date,transaction,description,amount\r\n2024-04-01,BUY,\"Buy, with a comma\nand a line\",\"($1,234.56)\"\r\n2024-04-02,FEE,\"a \"\"quoted\"\" word\",-5\r\n").unwrap();
        assert_eq!(f.rows.len(), 2);
        assert_eq!(f.rows[0].cells["description"], "Buy, with a comma\nand a line");
        assert_eq!((f.rows[0].line, f.rows[1].line), (2, 4));
        assert_eq!(f.rows[1].cells["description"], "a \"quoted\" word");
        for bad in ["date,transaction,description,amount\n2024-04-01,BUY,\"open,1\n", "date,transaction,description,amount\n2024-04-01,BUY,a \"b\",1\n", "date,transaction,description,amount\n2024-04-01,BUY,\"a\"b,1\n"] {
            assert!(read_file(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn identical_rows_in_a_file_are_told_apart_by_occurrence_and_notes_are_not_rows() {
        let f = read_file("Date,Action,Symbol,Quantity,Price,Amount\n2024-03-01,Buy,ZZZ,5,20,-100\nAs of 2024-03-31 balances,,,,,\n\n2024-03-01,Buy,ZZZ,5,20,-100\n").unwrap();
        assert_eq!(f.rows.iter().map(|r| r.occurrence).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(f.notes, vec![3]);
    }

    #[test]
    fn a_number_is_read_only_as_an_export_writes_one() {
        for (text, want) in [("1,234.56", "1234.56"), ("($1,234.56)", "-1234.56"), ("-$5.00", "-5.00"), ("$0.50", "0.50"), ("+3", "3"), ("12", "12"), ("1234567", "1234567")] {
            assert_eq!(number("n", text).unwrap(), Dec::parse(want).unwrap(), "{text}");
        }
        for bad in ["", "1,23", "12,345,67", ",123", "1.2.3", "abc", "NaN", "inf", "1e5", "(-5)", "5 CAD", "$", "-", "1.", ".5", "1 234"] {
            assert!(number("n", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_day_is_read_only_as_yyyy_mm_dd() {
        assert_eq!(day("d", "2024-05-01").unwrap(), jiff::civil::date(2024, 5, 1));
        // each of these could be read more than one way, or says a time whose day depends on a zone
        for bad in ["06/07/2024", "2024/05/02", "04-May-2024", "45410", "2024-02-30", "2024-05-01T10:00:00Z", "2024-5-1"] {
            assert!(day("d", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_row_past_the_header_does_not_read() {
        let f = read_file("Date,Action,Symbol,Quantity,Price,Amount\n2024-03-01,Buy,ZZZ,5,20,-100,surprise\n2024-03-02,Buy,ZZZ,5,20,-100,\n").unwrap();
        assert!(state(Layout::Simple, &f.rows[0].cells).is_err());
        assert!(state(Layout::Simple, &f.rows[1].cells).is_ok());
    }

    #[test]
    fn each_layout_states_its_rows() {
        let s = state(Layout::Activities, &cells(&[("transaction_date", "2024-01-15"), ("activity_type", "Trade"), ("activity_sub_type", "SELL"), ("symbol", "AAA"), ("quantity", "10"), ("unit_price", "25.50"), ("net_cash_amount", "255.00"), ("currency", "cad"), ("commission", "-1")])).unwrap();
        assert_eq!((s.kind, s.quantity, s.cash, s.fee, s.currency), (Kind::Sell, Some(Dec::parse("-10").unwrap()), Some(Dec::parse("255").unwrap()), Some(Dec::parse("1").unwrap()), Some(Currency::parse("CAD").unwrap())));
        assert_eq!(s.instrument, Some(("AAA".into(), InstrumentKind::Security)));

        let s = state(Layout::Statement, &cells(&[("date", "2024-02-03"), ("transaction", "BUY"), ("description", "AAPL - Apple Inc.: Bought 10.0000 shares (executed at 2024-02-01)"), ("amount", "($1,500.00)")])).unwrap();
        assert_eq!((s.kind, s.day, s.settle, s.quantity, s.cash), (Kind::Buy, jiff::civil::date(2024, 2, 1), Some(jiff::civil::date(2024, 2, 3)), Some(Dec::parse("10").unwrap()), Some(Dec::parse("-1500").unwrap())));
        assert_eq!(s.instrument, Some(("AAPL".into(), InstrumentKind::Security)));
        assert!(s.problems.is_empty());

        let s = state(Layout::Simple, &cells(&[("date", "2024-03-02"), ("action", "Sell"), ("symbol", "ZZZ"), ("quantity", "2"), ("price", "22"), ("amount", "44"), ("currency", "USD")])).unwrap();
        assert_eq!((s.kind, s.quantity, s.price, s.cash), (Kind::Sell, Some(Dec::parse("-2").unwrap()), Some(Dec::parse("22").unwrap()), Some(Dec::parse("44").unwrap())));
    }

    #[test]
    fn a_row_no_layout_places_is_kept_with_what_it_says() {
        for (layout, c) in [
            (Layout::Activities, cells(&[("transaction_date", "2024-01-15"), ("activity_type", "Mystery"), ("activity_sub_type", ""), ("net_cash_amount", "1")])),
            (Layout::Statement, cells(&[("date", "2024-01-15"), ("transaction", "LOAN"), ("description", "x"), ("amount", "0")])),
            (Layout::Simple, cells(&[("date", "2024-01-15"), ("action", "Gift"), ("symbol", ""), ("quantity", ""), ("price", ""), ("amount", "1")])),
        ] {
            let s = state(layout, &c).unwrap();
            assert_eq!(s.kind, Kind::Unclassified);
            assert_eq!(s.problems[0].code, "unclassified");
        }
        // a statement fill whose description states no units, or states the other way
        let s = state(Layout::Statement, &cells(&[("date", "2024-02-03"), ("transaction", "SELL"), ("description", "AAPL - Apple Inc.: Bought 1 share"), ("amount", "5")])).unwrap();
        assert_eq!(s.problems[0].code, "sign-against-kind");
        let s = state(Layout::Statement, &cells(&[("date", "2024-02-03"), ("transaction", "BUY"), ("description", "AAPL: something"), ("amount", "5")])).unwrap();
        assert_eq!(s.problems[0].code, "quantity-not-stated");
    }

    fn payload(layout: Layout, c: BTreeMap<String, String>, instrument: Option<Named>) -> String {
        serde_json::to_string(&Payload { layout, cells: c, occurrence: 0, account: ("manual".into(), "manual".into()), unplaced: None, instrument }).unwrap()
    }

    fn ctx() -> (bagholder_book::zones::Zones, bagholder_core::RecordId) {
        (bagholder_book::zones::Zones::new(), bagholder_core::RecordId::parse("01900000-0000-7000-8000-000000000001").unwrap())
    }

    #[test]
    fn a_row_maps_to_its_transaction_in_the_file_s_currency() {
        let (zones, record) = ctx();
        let c = MapContext { connection: None, record, zones: &zones };
        let named = Named { refs: vec![("isin".into(), "US0000000001".into())], kind: "security".into(), currency: "USD".into(), symbol: "ZZZ".into(), contract: None };
        let m = CsvMapping.map(&c, &payload(Layout::Simple, cells(&[("date", "2024-03-01"), ("action", "buy"), ("symbol", "ZZZ"), ("quantity", "5"), ("price", "20"), ("amount", "100"), ("currency", "USD")]), Some(named)));
        assert!(m.problems.is_empty(), "{:?}", m.problems);
        let d = &m.legs[0];
        assert_eq!((d.kind, d.quantity, d.cash.map(|c| c.amount), d.price.map(|p| p.amount)), (Kind::Buy, Some(Dec::parse("5").unwrap()), Some(Dec::parse("-100").unwrap()), Some(Dec::parse("20").unwrap())));
        assert_eq!(d.account, AccountRef::new(Broker::named("manual"), "manual"));
        // a deposit states no currency: nothing is assumed
        let m = CsvMapping.map(&c, &payload(Layout::Simple, cells(&[("date", "2024-03-01"), ("action", "deposit"), ("symbol", ""), ("quantity", ""), ("price", ""), ("amount", "100")]), None));
        assert_eq!((m.legs[0].kind, m.problems[0].code.as_str()), (Kind::Unclassified, "currency-unstated"));
        // a row that does not read gives no transaction and says why
        let m = CsvMapping.map(&c, &payload(Layout::Simple, cells(&[("date", "01/03/2024"), ("action", "buy")]), None));
        assert!(m.legs.is_empty());
        assert_eq!(m.problems[0].code, "unreadable");
    }

    #[test]
    fn a_row_naming_an_account_the_book_does_not_hold_counts_in_no_figure() {
        let (zones, record) = ctx();
        let c = MapContext { connection: None, record, zones: &zones };
        let p = Payload { layout: Layout::Simple, cells: cells(&[("date", "2024-03-01"), ("action", "deposit"), ("symbol", ""), ("quantity", ""), ("price", ""), ("amount", "1"), ("currency", "CAD")]), occurrence: 0, account: ("manual".into(), "manual".into()), unplaced: Some("the row names account X".into()), instrument: None };
        let m = CsvMapping.map(&c, &serde_json::to_string(&p).unwrap());
        assert_eq!((m.legs[0].kind, m.legs[0].cash, m.problems[0].code.as_str()), (Kind::Unclassified, None, "account-unknown"));
    }

    #[test]
    fn the_key_is_the_row_not_the_file() {
        let a = Payload { layout: Layout::Simple, cells: cells(&[("date", "2024-03-01")]), occurrence: 0, account: ("manual".into(), "manual".into()), unplaced: None, instrument: None };
        let mut b = a.clone();
        assert_eq!(a.key(), b.key());
        b.occurrence = 1;
        assert_ne!(a.key(), b.key());
        let mut c = a.clone();
        c.account.1 = "other".into();
        assert_ne!(a.key(), c.key());
    }
}
