//! CDS's tax breakdown forms (brief 82, PR 2): what each trust states per record
//! date for the year's distributions, read from the form its issuer filed with CDS,
//! for the person's own copy alone (the owner's personal-use ruling, 2026-11-09).
//!
//! Two places hold them. Tax years from 2025 are in CDS's Canadian Tax Breakdown
//! Reporting Service: one page lists every filing (received, CUSIP, revised, name,
//! type, year, form id) and each form is a PDF, its figures drawn as form objects
//! that only `pdftotext` renders (`pdftext::layout_text`). Earlier years are listed
//! one page a year on www.cds.ca, each form an Excel 97 workbook (`xls::table`).
//! Both are CRA's T3/RL-16 template: a column per distribution, rows for its record
//! and payment dates, its total, cash, non-cash and income per unit, then one row
//! per T3 box, as a percent of the income per unit or as dollars per unit.
//!
//! Nothing here reads the network: the readers below take the pages and forms as
//! they were answered. Nothing a form states is ever written to the repository.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::Dec;
use serde_json::Value;

/// The boxes whose amounts make up a distribution: what the form's own check
/// figure adds to 100%. Box 30 is part of box 21.
const ALLOCATED: [&str; 8] = ["21", "49", "23", "24", "25", "26", "42", "X"];
/// The foreign tax the fund paid: boxes 24 and 25 state the foreign income before
/// it, the cash is after it.
const FOREIGN_TAX: [&str; 2] = ["33", "34"];

/// One filing in a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filing {
    /// When CDS received it, as the list writes it.
    pub received: String,
    pub cusip: String,
    pub revised: bool,
    pub name: String,
    /// `T3R16`, `T3`, `T5`, …
    pub kind: String,
    pub year: i16,
    pub form: FormRef,
}

/// Where a filing's form is fetched from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormRef {
    /// The breakdown service's form id, posted to fetch its PDF.
    Ctbs(String),
    /// A workbook's address under www.cds.ca/services/taxforms/.
    Xls(String),
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&nbsp;", " ").replace("&#39;", "'").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Each `<tr>…</tr>`'s cells, as raw html.
fn rows_of(html: &str) -> Vec<Vec<String>> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(s) = lower[at..].find("<tr") {
        let start = at + s;
        let end = lower[start..].find("</tr>").map_or(lower.len(), |e| start + e);
        let row = &html[start..end];
        let row_lower = &lower[start..end];
        let mut cells = Vec::new();
        let mut c = 0;
        while let Some(o) = row_lower[c..].find("<td") {
            let open = c + o;
            let Some(gt) = row_lower[open..].find('>') else { break };
            let body = open + gt + 1;
            let close = row_lower[body..].find("</td>").map_or(row.len(), |e| body + e);
            cells.push(row[body..close].to_string());
            c = close;
        }
        if !cells.is_empty() {
            out.push(cells);
        }
        at = end.max(start + 3);
    }
    out
}

/// The breakdown service's list: date, CUSIP, revised, name, type, year, and the
/// form's id in its download form.
pub fn ctbs_list(html: &str) -> Vec<Filing> {
    rows_of(html)
        .into_iter()
        .filter_map(|cells| {
            if cells.len() < 7 {
                return None;
            }
            let id_at = cells[6].find("name=\"taxFormId\"")?;
            let rest = &cells[6][id_at..];
            let v = rest.find("value=\"")? + 7;
            let id = rest[v..].split('"').next()?.to_string();
            Some(Filing {
                received: strip_tags(&cells[0]),
                cusip: strip_tags(&cells[1]),
                revised: strip_tags(&cells[2]) == "Y",
                name: strip_tags(&cells[3]),
                kind: strip_tags(&cells[4]).replace(' ', ""),
                year: strip_tags(&cells[5]).parse().ok()?,
                form: FormRef::Ctbs(id),
            })
        })
        .collect()
}

/// A year's list on www.cds.ca: each row's spans by class (`Date`, `Cusip`,
/// `ReplacementInd`, `Company`, `Type`) and its workbook's link.
pub fn year_list(html: &str, year: i16) -> Vec<Filing> {
    let span = |cell: &str, class: &str| -> Option<String> {
        let at = cell.find(&format!("CLASS=\"{class}\"")).or_else(|| cell.find(&format!("class=\"{class}\"")))?;
        let body = &cell[at..];
        let open = body.find('>')? + 1;
        let close = body[open..].find('<')? + open;
        Some(strip_tags(&body[open..close]))
    };
    rows_of(html)
        .into_iter()
        .filter_map(|cells| {
            let joined = cells.join(" ");
            let href_at = joined.find("HREF=\"").or_else(|| joined.find("href=\""))? + 6;
            let href = joined[href_at..].split('"').next()?.to_string();
            if !href.to_ascii_lowercase().ends_with(".xls") {
                return None;
            }
            Some(Filing {
                received: span(&joined, "Date").unwrap_or_default(),
                cusip: span(&joined, "Cusip")?,
                revised: span(&joined, "ReplacementInd").is_some_and(|r| r == "Y"),
                name: span(&joined, "Company")?,
                kind: span(&joined, "Type")?.replace([' ', ','], ""),
                year,
                form: FormRef::Xls(href),
            })
        })
        .collect()
}

/// The words of a name that tell funds apart, lowercased: `&` read as `and`, and
/// the words every filing carries left out.
fn significant(name: &str) -> Vec<String> {
    const COMMON: [&str; 12] = ["the", "and", "of", "etf", "fund", "units", "unit", "class", "series", "trust", "inc", "corp"];
    name.to_lowercase()
        .replace('&', " and ")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !COMMON.contains(w))
        .map(str::to_string)
        .collect()
}

/// A fund's candidate filings for a year: the list's T3 rows that share a word
/// that tells funds apart with its name, the most shared first, three at most (the
/// form's own SYMBOL then decides). A revised filing is taken before the one it
/// replaces, and a CUSIP already known is the only candidate.
pub fn candidates<'a>(rows: &'a [Filing], fund_name: &str, year: i16, cusip: Option<&str>) -> Vec<&'a Filing> {
    let t3 = |f: &&Filing| f.year == year && f.kind.starts_with("T3");
    if let Some(c) = cusip {
        let mut known: Vec<&Filing> = rows.iter().filter(t3).filter(|f| f.cusip == c).collect();
        known.sort_by(|a, b| b.revised.cmp(&a.revised).then_with(|| b.received.cmp(&a.received)));
        return known.into_iter().take(1).collect();
    }
    let want = significant(fund_name);
    let mut scored: Vec<(usize, &Filing)> = rows
        .iter()
        .filter(t3)
        .map(|f| {
            let theirs = significant(&f.name);
            (want.iter().filter(|w| theirs.contains(w)).count(), f)
        })
        .filter(|(n, _)| *n > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.revised.cmp(&a.1.revised)).then_with(|| b.1.received.cmp(&a.1.received)));
    let mut seen: Vec<&str> = Vec::new();
    scored
        .into_iter()
        .filter(|(_, f)| {
            // one form a CUSIP: its latest filing
            if seen.contains(&f.cusip.as_str()) {
                return false;
            }
            seen.push(&f.cusip);
            true
        })
        .take(3)
        .map(|(_, f)| f)
        .collect()
}

/// One distribution as the form states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub record: Date,
    pub paid: Option<Date>,
    pub total: Dec,
    pub cash: Dec,
    pub non_cash: Dec,
    pub income: Dec,
    /// Each box's figure: a percent of the income per unit, or dollars per unit.
    pub boxes: BTreeMap<String, Dec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub symbol: String,
    pub cusip: String,
    /// The boxes are percents of the income per unit, not dollars per unit.
    pub percent: bool,
    pub distributions: Vec<Distribution>,
}

/// What a form gives the ledger for one record date (`acb::PerUnit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerUnit {
    pub record: Date,
    pub return_of_capital: Dec,
    pub non_cash: Dec,
}

fn number(s: &str) -> Option<Dec> {
    let t = s.trim().replace([',', '$', '%'], "");
    if t.is_empty() {
        return None;
    }
    Dec::parse(&t).ok()
}

fn date(s: &str) -> Option<Date> {
    s.trim().replace('/', "-").parse().ok()
}

fn method(s: &str) -> Option<bool> {
    let t = s.trim().to_uppercase();
    if t == "1" || t.contains("PER CENT") || t.contains("PERCENT") {
        Some(true)
    } else if t == "2" || t.contains("RATE") {
        Some(false)
    } else {
        None
    }
}

/// A workbook's cells as text: a number as its digits, a blank as "".
pub fn grid_of(table: &[Vec<Value>]) -> Vec<Vec<String>> {
    table
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => {
                        let f = n.as_f64().unwrap_or(0.0);
                        // a whole number as written, a fraction to its shortest digits
                        if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { format!("{f}") }
                    }
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect()
}

/// A workbook form, found by its labels wherever the template puts them.
pub fn form_from_grid(grid: &[Vec<String>]) -> Result<Form, String> {
    let starts = |cell: &str, label: &str| cell.trim().to_lowercase().starts_with(&label.to_lowercase());
    // the first value right of a label in its row
    let beside = |label: &str| -> Option<String> {
        grid.iter().find_map(|r| {
            let at = r.iter().position(|c| starts(c, label))?;
            r[at + 1..].iter().find(|c| !c.trim().is_empty()).map(|c| c.trim().to_string())
        })
    };
    let symbol = beside("SYMBOL:").ok_or("no SYMBOL")?;
    let cusip = beside("CUSIP#:").ok_or("no CUSIP")?;
    let percent = beside("CALCULATION METHOD:").as_deref().and_then(method).ok_or("no calculation method")?;
    let columns: Vec<usize> = grid
        .iter()
        .find_map(|r| {
            let cols: Vec<usize> = r.iter().enumerate().filter(|(_, c)| starts(c, "Distribution ")).map(|(i, _)| i).collect();
            (!cols.is_empty()).then_some(cols)
        })
        .ok_or("no distribution columns")?;
    let row = |label: &str| grid.iter().find(|r| r.iter().any(|c| starts(c, label)));
    let need = |label: &str| row(label).ok_or_else(|| format!("no row {label:?}"));
    let total = need("Total Distribution ($) Per Unit")?;
    let record = need("Record Date")?;
    let paid = need("Payment Date")?;
    let cash = need("Total Cash Distribution ($) Per Unit")?;
    let non_cash = need("Total Non Cash Distribution ($) Per Unit")?;
    let income = need("Total Income ($) per unit")?;
    let boxes: Vec<(String, &Vec<String>)> = grid
        .iter()
        .filter_map(|r| {
            let first = r.first()?.split_whitespace().next()?.to_string();
            ["21", "49", "23", "24", "25", "26", "42", "X", "30", "33", "34"].contains(&first.as_str()).then_some((first, r))
        })
        .collect();
    let at = |r: &Vec<String>, c: usize| r.get(c).cloned().unwrap_or_default();
    let mut distributions = Vec::new();
    for c in columns {
        let Some(rd) = date(&at(record, c)) else { continue };
        let zero = |s: String| number(&s).unwrap_or(Dec::ZERO);
        let mut figures = BTreeMap::new();
        for (b, r) in &boxes {
            if let Some(v) = number(&at(r, c)) {
                figures.insert(b.clone(), v);
            }
        }
        distributions.push(Distribution { record: rd, paid: date(&at(paid, c)), total: zero(at(total, c)), cash: zero(at(cash, c)), non_cash: zero(at(non_cash, c)), income: zero(at(income, c)), boxes: figures });
    }
    if distributions.is_empty() {
        return Err("no distribution with a record date".into());
    }
    Ok(Form { symbol, cusip, percent, distributions })
}

/// A form as `pdftotext -layout` lays it out: each label's figures on the line
/// after it, under the `Distribution N` headings.
pub fn form_from_layout(text: &str) -> Result<Form, String> {
    let lines: Vec<&str> = text.lines().collect();
    let after = |key: &str| -> Option<String> {
        lines.iter().find_map(|l| {
            let at = l.find(key)?;
            let rest = l[at + key.len()..].trim_start().trim_start_matches(':').trim_start();
            rest.split_whitespace().next().map(str::to_string)
        })
    };
    let symbol = after("SYMBOL/SYMBOLE").ok_or("no SYMBOL")?;
    let cusip = after("CUSIP/CUSIP#").ok_or("no CUSIP")?;
    let percent = after("CALCULATION METHOD/MÉTHODE DE CALCUL").as_deref().and_then(method).ok_or("no calculation method")?;
    // each heading's first character, in the line's characters
    let head = lines.iter().position(|l| l.contains("Distribution 1")).ok_or("no distribution columns")?;
    let chars = |l: &str| l.chars().collect::<Vec<char>>();
    let starts: Vec<usize> = {
        let h = chars(lines[head]);
        let word: Vec<char> = "Distribution".chars().collect();
        (0..h.len()).filter(|&i| h[i..].starts_with(&word)).collect()
    };
    let first = *starts.first().ok_or("no distribution columns")?;
    // a line's tokens right of the labels, each under its nearest heading
    let figures = |l: &str| -> BTreeMap<usize, String> {
        let cs = chars(l);
        let mut out = BTreeMap::new();
        let mut i = 0;
        while i < cs.len() {
            if cs[i].is_whitespace() {
                i += 1;
                continue;
            }
            let s = i;
            while i < cs.len() && !cs[i].is_whitespace() {
                i += 1;
            }
            if s + 3 < first {
                continue;
            }
            let col = starts.iter().enumerate().min_by_key(|(_, &h)| h.abs_diff(s)).map(|(n, _)| n).unwrap_or(0);
            out.insert(col, cs[s..i].iter().collect());
        }
        out
    };
    // the figures on the next line after a label that is not blank: an empty row's
    // next line is its French label, with none, never the row below's
    let row = |pred: &dyn Fn(&str) -> bool| -> Option<BTreeMap<usize, String>> {
        let at = lines.iter().skip(head + 1).position(|l| pred(l))? + head + 1;
        let next = lines[at + 1..].iter().find(|l| !l.trim().is_empty())?;
        Some(figures(next)).filter(|f| !f.is_empty())
    };
    let label = |text: &'static str| move |l: &str| l.trim_start().starts_with(text);
    let need = |name: &str, r: Option<BTreeMap<usize, String>>| r.ok_or_else(|| format!("no row {name:?}"));
    let total = need("total", row(&label("Total Distribution ($) Per Unit")))?;
    let record = need("record", row(&label("Record Date")))?;
    let paid = row(&label("Payment Date")).unwrap_or_default();
    let cash = need("cash", row(&label("Total Cash Distribution ($) Per Unit")))?;
    let non_cash = row(&label("Total Non Cash Distribution ($) Per Unit")).unwrap_or_default();
    let income = need("income", row(&label("Total Income ($) per unit")))?;
    let mut boxes: Vec<(String, BTreeMap<usize, String>)> = Vec::new();
    for b in ["21", "49", "23", "24", "25", "26", "42", "X", "30", "33", "34"] {
        let is_box = move |l: &str| {
            let mut w = l.split_whitespace();
            w.next() == Some(b) && l.trim_start().len() > b.len() && !l.contains("Distribution")
        };
        if let Some(f) = row(&is_box) {
            boxes.push((b.to_string(), f));
        }
    }
    let mut distributions = Vec::new();
    for (n, rd) in &record {
        let Some(rd) = date(rd) else { continue };
        let get = |m: &BTreeMap<usize, String>| m.get(n).and_then(|s| number(s)).unwrap_or(Dec::ZERO);
        let mut figures = BTreeMap::new();
        for (b, m) in &boxes {
            if let Some(v) = m.get(n).and_then(|s| number(s)) {
                figures.insert(b.clone(), v);
            }
        }
        distributions.push(Distribution { record: rd, paid: paid.get(n).and_then(|s| date(s)), total: get(&total), cash: get(&cash), non_cash: get(&non_cash), income: get(&income), boxes: figures });
    }
    if distributions.is_empty() {
        return Err("no distribution with a record date".into());
    }
    Ok(Form { symbol, cusip, percent, distributions })
}

/// Each record date's return of capital and non-cash distribution per unit. On
/// every record date the boxes' amounts per unit, less the foreign tax paid, must
/// add to what was paid, cash plus non-cash, within a cent (the ledger's own
/// identity: what is paid is the income less foreign tax, plus return of capital):
/// a form whose percentages are of something else is set aside whole, and the
/// ledger places the slip's box 42 instead.
pub fn per_unit(form: &Form) -> Result<Vec<PerUnit>, String> {
    let cent = Dec::new(1, 2).map_err(|e| e.to_string())?;
    let hundred = Dec::from_int(100);
    let mut out = Vec::new();
    for d in &form.distributions {
        let amount = |v: Dec| -> Result<Dec, String> {
            if form.percent { d.income.mul_div_rounded(v, hundred, 8, bagholder_core::Rounding::HalfEven).map_err(|e| e.to_string()) } else { Ok(v) }
        };
        let mut sum = Dec::ZERO;
        for b in ALLOCATED {
            if let Some(v) = d.boxes.get(b) {
                sum = sum.checked_add(amount(*v)?).map_err(|e| e.to_string())?;
            }
        }
        for b in FOREIGN_TAX {
            if let Some(v) = d.boxes.get(b) {
                sum = sum.checked_sub(amount(*v)?).map_err(|e| e.to_string())?;
            }
        }
        let whole = d.cash.checked_add(d.non_cash).map_err(|e| e.to_string())?;
        let off = sum.checked_sub(whole).map_err(|e| e.to_string())?;
        if off.abs() > cent {
            return Err(format!("on {} the boxes come to {} a unit against {} paid", d.record, sum.to_text(), whole.to_text()));
        }
        let roc = d.boxes.get("42").map_or(Ok(Dec::ZERO), |v| amount(*v))?;
        out.push(PerUnit { record: d.record, return_of_capital: roc, non_cash: d.non_cash });
    }
    Ok(out)
}

/// The breakdown service, the 2025-on forms.
pub const CTBS: &str = "https://ctbsext.posttrade.cds.ca/ctbsExt";
/// The 2019-2024 lists and workbooks.
pub const FORMS: &str = "https://www.cds.ca/services/taxforms";
/// The first tax year the breakdown service holds; earlier ones are on www.cds.ca.
pub const CTBS_FROM: i16 = 2025;
/// Every request to either host this far apart at least: a list page is about 2 MB.
const GAP: std::time::Duration = std::time::Duration::from_secs(3);

fn ask(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<bagholder_net::Reply, String> {
    let host = bagholder_net::host_of(url);
    crate::http::pace_host(&host, GAP);
    let ask = bagholder_net::Ask { method, body, timeout: std::time::Duration::from_secs(60), ..bagholder_net::Ask::get(url, headers) };
    let reply = bagholder_net::machine::net().send(&ask).map_err(|e| format!("{host}: {e:?}"))?;
    if reply.status >= 400 {
        return Err(format!("{host} answered {}", reply.status));
    }
    Ok(reply)
}

/// The breakdown service with its terms accepted for this read: the session cookie
/// its one landing page sets, carried to the forms read after it, never kept.
pub struct Ctbs {
    cookie: String,
}

/// The breakdown service's list, its terms accepted (the owner's personal-use
/// ruling, 2026-11-09): this copy's own session, never shared.
pub fn ctbs_open() -> Result<(Ctbs, Vec<Filing>), String> {
    let headers = [("User-Agent", crate::http::UA), ("Accept", "text/html")];
    let reply = ask("GET", &format!("{CTBS}/external-landing"), &headers, None)?;
    let cookie = reply
        .headers
        .iter()
        .filter(|(k, _)| k == "set-cookie")
        .filter_map(|(_, v)| v.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    let list = ctbs_list(&String::from_utf8_lossy(&reply.body));
    Ok((Ctbs { cookie }, list))
}

impl Ctbs {
    /// A filing's PDF.
    pub fn form(&self, id: &str) -> Result<Vec<u8>, String> {
        let body = format!("taxFormId={id}");
        let mut headers = vec![("User-Agent", crate::http::UA), ("Content-Type", "application/x-www-form-urlencoded")];
        if !self.cookie.is_empty() {
            headers.push(("Cookie", &self.cookie));
        }
        ask("POST", &format!("{CTBS}/startDownloadFormExternal"), &headers, Some(body.as_bytes())).map(|r| r.body)
    }
}

/// A year's list on www.cds.ca (2019 to 2024).
pub fn year_listing(year: i16) -> Result<Vec<Filing>, String> {
    let headers = [("User-Agent", crate::http::UA), ("Accept", "text/html")];
    let reply = ask("GET", &format!("{FORMS}/{year}.html"), &headers, None)?;
    Ok(year_list(&String::from_utf8_lossy(&reply.body), year))
}

/// A workbook by its link in a year's list.
pub fn workbook(href: &str) -> Result<Vec<u8>, String> {
    let headers = [("User-Agent", crate::http::UA)];
    ask("GET", &format!("{FORMS}/{}", href.trim_start_matches("./")), &headers, None).map(|r| r.body)
}

/// What a fund's form for a year gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The form whose SYMBOL is the listing's: its CUSIP, its reference, and each
    /// record date's figures.
    Read { cusip: String, form: String, per_unit: Vec<PerUnit> },
    /// That form, set aside: its boxes do not add to what was paid (`per_unit`).
    SetAside { cusip: String, form: String, why: String },
    /// No candidate's SYMBOL was the listing's, or the list has none.
    None,
}

/// A fund's form for a year, from a list already read: the candidates (`candidates`)
/// read in turn, at most three, the first whose SYMBOL is the listing's taken.
/// `fetch` reads one filing's form as bytes (a PDF or a workbook).
pub fn find(rows: &[Filing], fund_name: &str, symbol: &str, year: i16, cusip: Option<&str>, fetch: &dyn Fn(&Filing) -> Result<Vec<u8>, String>) -> Result<Found, String> {
    for f in candidates(rows, fund_name, year, cusip) {
        let bytes = fetch(f)?;
        let form = match &f.form {
            FormRef::Ctbs(_) => match crate::pdftext::layout_text(&bytes) {
                Some(t) => form_from_layout(&t),
                // no pdftotext: nothing read, and the slips place the year
                None => return Ok(Found::None),
            },
            FormRef::Xls(_) => crate::xls::table(&bytes).and_then(|t| form_from_grid(&grid_of(&t))),
        };
        let Ok(form) = form else { continue };
        if !form.symbol.eq_ignore_ascii_case(symbol) {
            continue;
        }
        let reference = match &f.form {
            FormRef::Ctbs(id) => id.clone(),
            FormRef::Xls(href) => href.clone(),
        };
        return Ok(match per_unit(&form) {
            Ok(per_unit) => Found::Read { cusip: form.cusip, form: reference, per_unit },
            Err(why) => Found::SetAside { cusip: form.cusip, form: reference, why },
        });
    }
    Ok(Found::None)
}

/// Whether a listing's form for `year` is read now, as the slips are (brief 82): a
/// past year once; the year just ended once a day from February 1 until a read on
/// or after April 1, when every issuer's form is in (60 to 67 days after the year
/// ends); the year still running, never.
pub fn due(year: i16, last_read: Option<Date>, today: Date) -> bool {
    let this = today.year();
    if year >= this {
        return false;
    }
    if year < this - 1 {
        return last_read.is_none();
    }
    let (feb, apr) = (Date::new(this, 2, 1), Date::new(this, 4, 1));
    let (Ok(feb), Ok(apr)) = (feb, apr) else { return false };
    if today < feb {
        return false;
    }
    match last_read {
        None => true,
        Some(last) => last < apr && last < today,
    }
}
