//! Wealthsimple's tax slips, read from the text of their PDFs as
//! `pdftotext -layout` lays it out (brief 82, PR 1): the T5008, one row a sale;
//! the consolidated T3, its boxes and, from 2024, each fund's own distributions;
//! and the T5 statement, one row an income type.
//!
//! Every slip carries its own arithmetic, and a slip is read only where it holds:
//! a T5008's rows add up to its totals line, a T3's per-fund rows to each fund's
//! totals and those to the consolidated slip's boxes, a T5's rows to its totals
//! row. A slip that does not add up is refused with what did not, never read in
//! part. The layouts are the ones Wealthsimple's slips took for 2023 to 2025, read
//! from the person's own slips on 2026-11-12 with every value masked.

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Currency, Dec, Money};

/// One sale on a T5008.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sale {
    pub day: Date,
    /// The slip's type code of the securities (`SHS`).
    pub type_code: String,
    pub quantity: Dec,
    /// The identification of the securities, as the slip writes it (its name).
    pub security: String,
    /// Box 20, cost or book value; none where the slip leaves it blank.
    pub cost: Option<Money>,
    /// Box 21, proceeds of disposition.
    pub proceeds: Money,
}

/// A T5008: one account's sales of a year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T5008 {
    pub year: i16,
    pub sales: Vec<Sale>,
}

/// An amount as a slip writes it: `$1,234.56`, `-$1,234.56`, `$-1,234.56` or
/// `($1,234.56)`.
fn amount(text: &str) -> Option<Dec> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => (false, t),
    };
    let (negative, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (negative, t),
    };
    let t = t.strip_prefix('$')?;
    let (negative, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (negative, t),
    };
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
        return None;
    }
    let d = Dec::parse(&t.replace(',', "")).ok()?;
    Some(if negative { d.neg() } else { d })
}

/// A line's words, each with the column it starts at, in characters.
fn words(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<(usize, usize)> = None;
    for (col, (i, c)) in line.char_indices().enumerate() {
        if c.is_whitespace() {
            if let Some((s, scol)) = start.take() {
                out.push((scol, &line[s..i]));
            }
        } else if start.is_none() {
            start = Some((i, col));
        }
    }
    if let Some((s, scol)) = start {
        out.push((scol, &line[s..]));
    }
    out
}

/// The column a text starts at on a line, in characters.
fn column_of(line: &str, text: &str) -> Option<usize> {
    line.find(text).map(|i| line[..i].chars().count())
}

/// The amounts on a line, each with its column and the currency written after it.
fn amounts(line: &str) -> Vec<(usize, Dec, Option<Currency>)> {
    let ws = words(line);
    let mut out = Vec::new();
    for (k, (col, w)) in ws.iter().enumerate() {
        if let Some(d) = amount(w) {
            let currency = ws.get(k + 1).and_then(|(_, c)| if c.len() == 3 && c.chars().all(|x| x.is_ascii_uppercase()) { Currency::parse(c).ok() } else { None });
            out.push((*col, d, currency));
        }
    }
    out
}

fn sum(mut values: impl Iterator<Item = Dec>) -> Result<Dec, String> {
    values.try_fold(Dec::ZERO, |a, b| a.checked_add(b).map_err(|e| e.to_string()))
}

/// Within a cent.
fn agrees(a: Dec, b: Dec) -> bool {
    a.checked_sub(b).map(|d| d.abs() < Dec::new(1, 2).expect("a cent")).unwrap_or(false)
}

/// A T5008 as Wealthsimple lays it out: its year, from the line its French labels
/// follow, and each sale's row, a name that wraps continuing on the lines under it.
pub fn t5008(text: &str) -> Result<T5008, String> {
    let lines: Vec<&str> = text.lines().collect();
    let year = lines
        .windows(2)
        .find(|w| w[1].trim_start().starts_with("Année"))
        .and_then(|w| w[0].split_whitespace().next())
        .filter(|t| t.len() == 4)
        .and_then(|t| t.parse::<i16>().ok())
        .ok_or("the slip states no year where its layout puts one")?;
    // the column the proceeds stand in, from the table's own header
    let proceeds_col = lines.iter().find_map(|l| column_of(l, "Proceeds of")).ok_or("the slip has no Proceeds of Disposition column")?;
    let mut sales: Vec<Sale> = Vec::new();
    let mut ident_col: Option<usize> = None;
    let mut totals: Option<Vec<(usize, Dec, Option<Currency>)>> = None;
    let mut want_totals = false;
    for l in &lines {
        if want_totals {
            let a = amounts(l);
            if !a.is_empty() {
                totals = Some(a);
                want_totals = false;
            }
            continue;
        }
        if l.contains("Total des produits de disposition") {
            want_totals = true;
            ident_col = None;
            continue;
        }
        let t = l.trim_start();
        let lead = l.chars().take_while(|c| c.is_whitespace()).count();
        let ws = words(l);
        let first = ws.first().map(|w| w.1).unwrap_or("");
        let is_day = first.len() == 5 && first.as_bytes()[2] == b'/' && first.chars().filter(|c| c.is_ascii_digit()).count() == 4;
        if is_day {
            let row = sale(l, year, proceeds_col).map_err(|e| format!("a sale on {first}: {e}"))?;
            ident_col = Some(row.1);
            sales.push(row.0);
            continue;
        }
        // a wrapped name: text alone, where the name column starts
        match (ident_col, sales.last_mut()) {
            (Some(col), Some(s)) if !t.is_empty() && lead.abs_diff(col) <= 2 && amounts(l).is_empty() => {
                s.security = format!("{} {}", s.security, t.trim_end());
            }
            _ => ident_col = None,
        }
    }
    let totals = totals.ok_or("the slip has no totals line")?;
    if totals.len() != 3 {
        return Err(format!("the totals line has {} amounts, not proceeds, cost and gain", totals.len()));
    }
    if sales.is_empty() {
        return Err("the slip lists no sale".into());
    }
    if let Some(s) = sales.iter().find(|s| s.proceeds.currency != Currency::CAD || s.cost.as_ref().is_some_and(|c| c.currency != Currency::CAD)) {
        return Err(format!("a sale on {} is in {}, which the slips read so far never were: not read rather than guessed", s.day, s.proceeds.currency));
    }
    let proceeds = sum(sales.iter().map(|s| s.proceeds.amount))?;
    let cost = sum(sales.iter().filter_map(|s| s.cost.as_ref().map(|c| c.amount)))?;
    let gain = proceeds.checked_sub(cost).map_err(|e| e.to_string())?;
    for (what, ours, theirs) in [("proceeds", proceeds, totals[0].1), ("cost", cost, totals[1].1), ("gain", gain, totals[2].1)] {
        if !agrees(ours, theirs) {
            return Err(format!("the sales' {what} come to {} but the slip's total is {}", ours.to_text(), theirs.to_text()));
        }
    }
    Ok(T5008 { year, sales })
}

/// One sale's row: its day, type code, quantity, name, and its amounts, the
/// one at or right of the proceeds column being the proceeds. Also the column
/// its name starts at, where a wrapped name continues.
fn sale(line: &str, year: i16, proceeds_col: usize) -> Result<(Sale, usize), String> {
    let ws = words(line);
    let [(_, day_text), (_, type_code), (_, qty_text), ..] = ws.as_slice() else { return Err("fewer than a day, a type code and a quantity".into()) };
    let num = |r: std::ops::Range<usize>| day_text.get(r).and_then(|x| x.parse::<i8>().ok()).ok_or_else(|| format!("{day_text:?} is not a month and day"));
    let day = Date::new(year, num(0..2)?, num(3..5)?).map_err(|e| e.to_string())?;
    let quantity = Dec::parse(&qty_text.replace(',', "")).map_err(|e| format!("quantity {qty_text:?}: {e}"))?;
    let found = amounts(line);
    let first_amount = found.first().map(|a| a.0).ok_or("no amount")?;
    let ident: Vec<&(usize, &str)> = ws[3..].iter().filter(|(c, _)| *c < first_amount).collect();
    let ident_col = ident.first().map(|w| w.0).ok_or("no identification of the securities")?;
    let security = ident.iter().map(|w| w.1).collect::<Vec<_>>().join(" ");
    let money = |a: &(usize, Dec, Option<Currency>)| -> Result<Money, String> { Ok(Money::new(a.1, a.2.ok_or_else(|| format!("{} states no currency", a.1.to_text()))?)) };
    let (cost, proceeds) = match found.as_slice() {
        [c, p] => (Some(money(c)?), money(p)?),
        [p] if p.0 + 4 >= proceeds_col => (None, money(p)?),
        [c] => return Err(format!("only a cost, {}, and no proceeds", c.1.to_text())),
        more => return Err(format!("{} amounts where a sale has a cost and proceeds", more.len())),
    };
    Ok((Sale { day, type_code: type_code.to_string(), quantity, security, cost, proceeds }, ident_col))
}

/// One distribution of a fund, as the T3's breakdown states it: the amounts it
/// allocates by box, in dollars to four places, and its cash and non-cash parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub record: Date,
    pub paid: Date,
    pub boxes: std::collections::BTreeMap<u16, Dec>,
    pub cash: Dec,
    pub non_cash: Dec,
}

/// One fund on a T3's breakdown, by the name the slip gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustFund {
    pub name: String,
    pub distributions: Vec<Distribution>,
}

/// A consolidated T3: one account's trust income of a year, its boxes as the slip
/// states them, and, where the slip carries its breakdown (from 2024), each fund's
/// distributions, which add up to those boxes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T3 {
    pub year: i16,
    pub boxes: std::collections::BTreeMap<u16, Dec>,
    pub funds: Vec<TrustFund>,
}

/// The boxes a breakdown's columns are, which the slip's own boxes must equal the
/// funds' totals of.
const BREAKDOWN_BOXES: [u16; 7] = [49, 23, 21, 26, 25, 34, 42];

/// A day written `2025-09-16`.
fn iso_day(t: &str) -> Option<Date> {
    t.parse::<Date>().ok().filter(|_| t.len() == 10)
}

/// A consolidated T3 as Wealthsimple lays it out: the slip's boxes, each box
/// number followed in reading order by its amount, from the slip's title to its
/// recipient's name (the trust's year end, after `Année`, is no box); and, on its
/// breakdown page, each fund: its name, its columns by their `(Box NN)`, `Cash`
/// and `Non-Cash` headings, its rows and its totals row.
pub fn t3(text: &str) -> Result<T3, String> {
    let lines: Vec<&str> = text.lines().collect();
    let title = lines.iter().position(|l| l.contains("Statement of Trust Income Allocations and Designations")).ok_or("no T3 slip in the text")?;
    let end = lines[title..].iter().position(|l| l.contains("Recipient")).map(|i| title + i).ok_or("the slip has no recipient's box where its layout puts one")?;
    let mut year = None;
    let mut boxes = std::collections::BTreeMap::new();
    let mut pending: Option<u16> = None;
    for l in &lines[title..end] {
        let (head, tail) = match l.find("Année") {
            Some(i) => (&l[..i], &l[i..]),
            None => (*l, ""),
        };
        if year.is_none() {
            year = tail.split_whitespace().nth(1).filter(|t| t.len() == 4).and_then(|t| t.parse::<i16>().ok());
        }
        for (_, w) in words(head) {
            if w.len() == 2 && w.chars().all(|c| c.is_ascii_digit()) {
                pending = w.parse().ok();
            } else if let Some(a) = amount(w) {
                if let Some(b) = pending.take() {
                    if boxes.insert(b, a).is_some() {
                        return Err(format!("box {b} is stated twice"));
                    }
                }
            }
        }
    }
    let year = year.ok_or("the slip states no year")?;
    let funds = breakdown(&lines)?;
    if !funds.is_empty() {
        for b in BREAKDOWN_BOXES {
            let total = sum(funds.iter().flat_map(|f| f.distributions.iter()).map(|d| d.boxes.get(&b).copied().unwrap_or(Dec::ZERO)))?;
            let slip = boxes.get(&b).copied().unwrap_or(Dec::ZERO);
            if !agrees(total.round(2, bagholder_core::Rounding::HalfUp), slip) {
                return Err(format!("the funds' box {b} comes to {} but the slip's box {b} is {}", total.to_text(), slip.to_text()));
            }
        }
    }
    Ok(T3 { year, boxes, funds })
}

/// What a breakdown's column is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Box(u16),
    Cash,
    NonCash,
}

/// The funds of a T3's breakdown, none where the slip has none.
fn breakdown(lines: &[&str]) -> Result<Vec<TrustFund>, String> {
    let Some(start) = lines.iter().position(|l| l.contains("ÉTAT DES REVENUS DE FIDUCIE")) else { return Ok(vec![]) };
    let mut funds: Vec<TrustFund> = Vec::new();
    let mut name: Option<String> = None;
    let mut labels: Vec<(usize, Column)> = Vec::new();
    let mut current: Option<TrustFund> = None;
    for l in &lines[start + 1..] {
        let ws = words(l);
        let Some(first) = ws.first() else { continue };
        let day = iso_day(first.1);
        if let (Some(record), Some(paid)) = (day, ws.get(1).and_then(|w| iso_day(w.1))) {
            let fund = current.get_or_insert_with(|| TrustFund { name: name.clone().unwrap_or_default(), distributions: vec![] });
            if fund.name.is_empty() {
                return Err(format!("a distribution of {record} under no fund's name"));
            }
            let row = columns(l, &labels).map_err(|e| format!("{}'s distribution of {record}: {e}", fund.name))?;
            let get = |c: Column| row.iter().find(|(k, _)| *k == c).map(|(_, v)| *v).unwrap_or(Dec::ZERO);
            fund.distributions.push(Distribution {
                record,
                paid,
                boxes: row.iter().filter_map(|(k, v)| if let Column::Box(b) = k { Some((*b, *v)) } else { None }).collect(),
                cash: get(Column::Cash),
                non_cash: get(Column::NonCash),
            });
            continue;
        }
        if l.contains("Totals") {
            let Some(fund) = current.take() else { continue };
            let totals = columns(l, &labels).map_err(|e| format!("{}'s totals: {e}", fund.name))?;
            for (c, total) in &totals {
                let ours = sum(fund.distributions.iter().map(|d| match c {
                    Column::Box(b) => d.boxes.get(b).copied().unwrap_or(Dec::ZERO),
                    Column::Cash => d.cash,
                    Column::NonCash => d.non_cash,
                }))?;
                if ours != *total {
                    return Err(format!("{}'s distributions come to {} in its {c:?} column but its totals row says {}", fund.name, ours.to_text(), total.to_text()));
                }
            }
            funds.push(fund);
            name = None;
            labels.clear();
            continue;
        }
        // the column headings: (Box NN), Cash and Non-Cash, by where each stands
        for (col, w) in &ws {
            if *w == "(Box" {
                continue;
            }
            if let Some(b) = w.strip_suffix(')').and_then(|x| x.parse::<u16>().ok()).filter(|_| l[..l.char_indices().nth(*col).map(|x| x.0).unwrap_or(0)].trim_end().ends_with("(Box")) {
                labels.push((col.saturating_sub(5), Column::Box(b)));
            } else if *w == "Non-Cash" {
                labels.push((*col, Column::NonCash));
            } else if *w == "Cash" {
                labels.push((*col, Column::Cash));
            }
        }
        // a fund's name: a line of its own at the left margin, before its columns
        if first.0 == 0 && ws.len() >= 2 && labels.is_empty() && !l.contains('$') {
            name = Some(ws.iter().map(|w| w.1).collect::<Vec<_>>().join(" "));
        }
    }
    if let Some(f) = current {
        return Err(format!("{}'s distributions have no totals row", f.name));
    }
    Ok(funds)
}

/// A breakdown row's amounts, each to the column heading it stands under: one
/// amount to each heading, in order left to right.
fn columns(line: &str, labels: &[(usize, Column)]) -> Result<Vec<(Column, Dec)>, String> {
    let found = amounts(line);
    let mut labels = labels.to_vec();
    labels.sort_by_key(|l| l.0);
    if found.len() != labels.len() {
        return Err(format!("{} amounts under {} headed columns", found.len(), labels.len()));
    }
    let mut out = Vec::new();
    for ((col, value, _), (at, c)) in found.iter().zip(&labels) {
        if col.abs_diff(*at) > 12 {
            return Err(format!("an amount at column {col} under no heading (the nearest, {c:?}, stands at {at})"));
        }
        out.push((*c, *value));
    }
    Ok(out)
}

/// A column of a T5 statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum T5Column {
    ReturnOfCapital,
    Box(u16),
}

/// One row of a T5 statement: a security, by the name the statement gives it
/// (`------` for interest on cash), and its amounts by column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomeRow {
    pub security: String,
    pub amounts: std::collections::BTreeMap<T5Column, Dec>,
}

/// A T5 statement: one account's investment income of a year, a row a security.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T5 {
    pub year: i16,
    pub rows: Vec<IncomeRow>,
}

/// A T5 statement as Wealthsimple lays its first page out: its year, from `FOR
/// THE YEAR …`; its columns, return of capital and each `(Box NN)`; each row's
/// amounts in order, its name from the lines at the margin nearest it; and its
/// totals row, which states only the columns that come to more than nothing,
/// each by where it stands.
pub fn t5(text: &str) -> Result<T5, String> {
    let page = text.split('\u{c}').next().unwrap_or("");
    let lines: Vec<&str> = page.lines().collect();
    let year = lines
        .iter()
        .find(|l| l.contains("FOR THE YEAR"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|t| t.trim_matches(|c: char| !c.is_ascii_digit()).parse::<i16>().ok())
        .ok_or("the statement states no year")?;
    let hdr = lines.iter().position(|l| l.contains("Return of Capital")).ok_or("the statement has no Return of Capital column")?;
    let tot = lines[hdr..].iter().position(|l| l.trim_start().starts_with("Totals")).map(|i| hdr + i).ok_or("the statement has no totals row")?;
    let roc_col = column_of(lines[hdr], "Return of Capital").expect("found above");
    let hws = words(lines[hdr]);
    let type_col = hws.iter().position(|w| w.1 == "Type").and_then(|i| i.checked_sub(1)).map(|i| hws[i].0).ok_or("the statement has no type column")?;
    let mut labels: Vec<(usize, T5Column)> = vec![(roc_col, T5Column::ReturnOfCapital)];
    for l in &lines[hdr.saturating_sub(3)..tot] {
        let ws = words(l);
        for (k, (col, w)) in ws.iter().enumerate() {
            if *w == "(Box" {
                let b = ws.get(k + 1).and_then(|x| x.1.strip_suffix(')')).and_then(|x| x.parse::<u16>().ok()).ok_or_else(|| format!("a (Box heading at column {col} names no box"))?;
                labels.push((*col, T5Column::Box(b)));
            }
        }
    }
    labels.sort_by_key(|l| l.0);
    let near = |col: usize| labels.iter().min_by_key(|l| l.0.abs_diff(col)).filter(|l| l.0.abs_diff(col) <= 14).map(|l| l.1);
    let mut rows: Vec<(usize, IncomeRow)> = Vec::new();
    let mut margin: Vec<(usize, String)> = Vec::new();
    for (i, l) in lines.iter().enumerate().take(tot).skip(hdr + 1) {
        let found = amounts(l);
        let name: Vec<&str> = words(l).into_iter().filter(|w| w.0 < type_col).map(|w| w.1).collect();
        let at_margin = words(l).first().is_some_and(|w| w.0 == 0);
        if found.is_empty() {
            if at_margin && !name.is_empty() {
                margin.push((i, name.join(" ")));
            }
            continue;
        }
        if found.len() != labels.len() {
            return Err(format!("a row with {} amounts under {} columns", found.len(), labels.len()));
        }
        let mut amounts_by = std::collections::BTreeMap::new();
        for ((col, v, _), (at, c)) in found.iter().zip(&labels) {
            if col.abs_diff(*at) > 14 {
                return Err(format!("an amount at column {col} under no heading"));
            }
            amounts_by.insert(*c, *v);
        }
        if at_margin && !name.is_empty() {
            margin.push((i, name.join(" ")));
        }
        rows.push((i, IncomeRow { security: String::new(), amounts: amounts_by }));
    }
    if rows.is_empty() {
        return Err("the statement lists no row".into());
    }
    // each margin line to the row nearest it, the row above on a tie
    for (i, part) in margin {
        let k = (0..rows.len()).min_by_key(|&k| (rows[k].0.abs_diff(i), rows[k].0 > i)).expect("a row");
        let s = &mut rows[k].1.security;
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&part);
    }
    if let Some((_, r)) = rows.iter().find(|r| r.1.security.is_empty()) {
        return Err(format!("a row with no security's name: {:?}", r.amounts));
    }
    let mut totals = std::collections::BTreeMap::new();
    for (col, v, _) in amounts(lines[tot]) {
        let c = near(col).ok_or_else(|| format!("a total at column {col} under no heading"))?;
        totals.insert(c, v);
    }
    for (_, c) in &labels {
        let ours = sum(rows.iter().map(|r| r.1.amounts.get(c).copied().unwrap_or(Dec::ZERO)))?;
        let theirs = totals.get(c).copied().unwrap_or(Dec::ZERO);
        if ours != theirs {
            return Err(format!("the rows come to {} in the {c:?} column but the totals row says {}", ours.to_text(), theirs.to_text()));
        }
    }
    Ok(T5 { year, rows: rows.into_iter().map(|r| r.1).collect() })
}

/// A slip read, of whichever kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slip {
    T3(T3),
    T5(T5),
    T5008(T5008),
}

impl Slip {
    /// The tax year the slip states.
    pub fn year(&self) -> i16 {
        match self {
            Slip::T3(s) => s.year,
            Slip::T5(s) => s.year,
            Slip::T5008(s) => s.year,
        }
    }
}

/// A slip of `kind` read from its text, or why it is refused.
pub fn read(kind: crate::documents::SlipKind, text: &str) -> Result<Slip, String> {
    use crate::documents::SlipKind;
    match kind {
        SlipKind::T3 => t3(text).map(Slip::T3),
        SlipKind::T5 => t5(text).map(Slip::T5),
        SlipKind::T5008 => t5008(text).map(Slip::T5008),
    }
}

// -- what is never kept of a slip ----------------------------------------------

/// Where the payer's side of a slip begins: in every layout read (the person's
/// slips, 2023 to 2025), the recipient's name and address stand left of this
/// column and Wealthsimple's own name, address and account line right of it.
const PAYER_SIDE: usize = 110;

/// The runs of text on a line, as character columns `[start, end)`: words apart by
/// fewer than three spaces are one run (a name, an address line). A page's form
/// feed is no text and ends a run, wherever it falls on the line.
fn runs(line: &[char]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut page_ended = false;
    for (i, &c) in line.iter().enumerate() {
        match c {
            '\u{c}' => page_ended = true,
            ' ' => {}
            _ => {
                match out.last_mut() {
                    Some(r) if !page_ended && i - r.1 < 3 => r.1 = i + 1,
                    _ => out.push((i, i + 1)),
                }
                page_ended = false;
            }
        }
    }
    out
}

/// A slip's text with what identifies the person taken out, column for column, so
/// every reader reads it exactly as before: each number written as a social
/// insurance number (`999 999 999`, `999-999-999`, `999999999`) has its digits
/// written `#`, and the recipient's name and address, wherever they are printed,
/// are written `#` throughout. The name and address are found under each label
/// that heads them (`Recipient's name and address` on a T3's or a T5's slip, `Name
/// and Address of` beside it on a T5008): the lines below it whose text starts
/// at one column left of the payer's side, up to the first that starts at
/// another. Every run of text elsewhere that is one of those lines whole (a
/// statement's own address block) is taken out with them. A slip with no such
/// label, or a label with no name and address under it, is refused: it is never
/// kept with them in it.
pub fn redact(text: &str) -> Result<String, String> {
    let mut lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
    for line in &mut lines {
        mask_sins(line);
    }
    let mut theirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut labels = 0;
    for i in 0..lines.len() {
        let label: String = lines[i].iter().collect();
        if !(label.contains("Recipient's name") || label.contains("Name and Address of")) {
            continue;
        }
        labels += 1;
        let mut column = None;
        let mut block = Vec::new();
        // a name and its address lines, never more than a few lines below the label
        for (j, line) in lines.iter().enumerate().skip(i + 1).take(10) {
            let Some(&(a, b)) = runs(line).iter().find(|r| r.0 < PAYER_SIDE) else { continue };
            match column {
                None => column = Some(a),
                Some(c) if a.abs_diff(c) <= 1 => {}
                Some(_) => break,
            }
            block.push((j, a, b));
        }
        if block.len() < 2 {
            return Err(format!("no name and address under the label on line {}", i + 1));
        }
        for (j, a, b) in block {
            theirs.insert(lines[j][a..b].iter().collect());
        }
    }
    if labels == 0 {
        return Err("no recipient's name and address found, so it cannot be taken out".into());
    }
    for line in &mut lines {
        for (a, b) in runs(line) {
            if theirs.contains(&line[a..b].iter().collect::<String>()) {
                line[a..b].fill('#');
            }
        }
    }
    Ok(lines.iter().map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\n"))
}

/// Each number on a line written as a social insurance number, its digits `#`:
/// nine digits in threes, apart by one space, one hyphen or nothing, the same
/// between each, with no digit, point or comma on either side and no dollar sign
/// before (an amount, a quantity or a date is never one).
fn mask_sins(line: &mut [char]) {
    let before = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '$'));
    let after = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ','));
    let three = |line: &[char], at: usize| line.get(at..at + 3).is_some_and(|s| s.iter().all(char::is_ascii_digit));
    let mut i = 0;
    while i < line.len() {
        let starts = three(line, i) && !(i > 0 && before(line.get(i - 1)));
        let end = if starts {
            [None, Some(' '), Some('-')].into_iter().find_map(|sep| {
                let w = 3 + usize::from(sep.is_some());
                let end = i + 2 * w + 3;
                let apart = sep.is_none_or(|s| line.get(i + 3) == Some(&s) && line.get(i + w + 3) == Some(&s));
                (apart && three(line, i + w) && three(line, i + 2 * w) && !after(line.get(end))).then_some(end)
            })
        } else {
            None
        };
        match end {
            Some(end) => {
                for c in &mut line[i..end] {
                    if c.is_ascii_digit() {
                        *c = '#';
                    }
                }
                i = end;
            }
            None => i += 1,
        }
    }
}
