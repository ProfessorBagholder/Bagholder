//! What a fund company's distribution release announces for one of its funds,
//! read from the release itself: the release is out the moment the company
//! announces, where its fund pages and the exchange's record list the
//! distribution days later (Harvest, 2026-10-23: the release at 7:30, every
//! fund page and TMX's record still at September's that evening).
//!
//! Releases read 2026-10-23 (Harvest, Hamilton, BMO, Ninepoint, Canoe, as TMX
//! keeps them) share one shape: a table with a row per fund, its ticker in a
//! cell of its own and its amount in a later one (`$0.5000 per unit`,
//! `0.120`); the days either in that row, under a header naming each
//! (`Ex-Date and Record Date`, `Pay Date`), or in one sentence for every fund
//! ("paid on or about November 6, 2026 to securityholders of record on
//! October 30, 2026, with an ex-dividend date of October 30, 2026"). A release
//! stating no ex-date goes ex on its record date: Canadian and US trades have
//! settled the next business day since May 2024, which puts the two on one day
//! (CSA Staff Notice 24-319, Canada from 2024-05-27; SEC Release 34-96930, the
//! US from 2024-05-28).

use bagholder_core::jiff::civil::Date;
use bagholder_core::Dec;

use crate::html::{html_tables, text};
use crate::payers::money_text;

/// A distribution a release announces for one fund.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announced {
    pub cash: Dec,
    pub ex_date: Date,
    pub record_date: Option<Date>,
    pub pay_date: Option<Date>,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// `November 6, 2026`, `Nov. 6, 2026`, `Sept 29, 2026` as a day.
pub fn long_day(s: &str) -> Option<Date> {
    let mut it = s.split_whitespace();
    let word = it.next()?.trim_end_matches('.').to_ascii_lowercase();
    let m = MONTHS.iter().position(|n| word.starts_with(n) && (word.len() == 3 || word == "sept" || full(n, &word)))? as i8 + 1;
    let d: i8 = it.next()?.trim_end_matches(',').parse().ok()?;
    let y: i16 = it.next()?.trim_end_matches(['.', ',', ')', ';']).parse().ok()?;
    Date::new(y, m, d).ok()
}

fn full(short: &str, word: &str) -> bool {
    const NAMES: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    NAMES.iter().any(|n| n.starts_with(short) && *n == word)
}

/// The day stated after the first of the phrases `after` (in their order) that
/// `t` follows with one.
fn day_after(t: &str, after: &[&str]) -> Option<Date> {
    let low = t.to_ascii_lowercase();
    after.iter().find_map(|a| low.find(a).and_then(|i| long_day(&t[i + a.len()..])))
}

/// An amount as a release writes it: `$0.5000 per unit`, `US$0.12`, `0.120`.
fn amount(cell: &str) -> Option<Dec> {
    let c = cell.trim().trim_start_matches("US").trim_start_matches("CA");
    let c = c.split_whitespace().next()?;
    if !c.trim_start_matches('$').starts_with(|ch: char| ch.is_ascii_digit()) || !c.contains('.') {
        return None;
    }
    money_text(c)
}

/// What the release `html` announces for the fund `ticker` (`PLTE`, `HYLD.U`):
/// None where no table row names it. A row naming it without an amount, or a
/// release whose days cannot be found, is an error saying which.
pub fn announced(html: &str, ticker: &str) -> Result<Option<Announced>, String> {
    let mut found = None;
    'tables: for table in html_tables(html) {
        let header: Vec<String> = table.first().map(|h| h.iter().map(|c| c.to_ascii_lowercase()).collect()).unwrap_or_default();
        for row in &table {
            let Some(at) = row.iter().position(|c| c.trim() == ticker) else { continue };
            let Some(cash) = row[at + 1..].iter().find_map(|c| amount(c)) else {
                return Err(format!("the release names {ticker} without an amount"));
            };
            // the row's own days, by the header over each, where the header has a column per cell
            let (mut ex, mut record, mut pay) = (None, None, None);
            if header.len() == row.len() {
                for (h, c) in header.iter().zip(row) {
                    let Some(d) = long_day(c) else { continue };
                    if h.contains("ex") {
                        ex = ex.or(Some(d));
                    }
                    if h.contains("record") {
                        record = record.or(Some(d));
                    }
                    if h.contains("pay") {
                        pay = pay.or(Some(d));
                    }
                }
            }
            found = Some((cash, ex, record, pay));
            break 'tables;
        }
    }
    let t = text(&without(&without(html, "script"), "style"));
    // a release for one fund alone states its amount in a sentence: the fund
    // named by its ticker, then "a monthly distribution of $0.10 per unit"
    let found = found.or_else(|| {
        let named = t.split(|c: char| c.is_whitespace() || c == '(' || c == ')').any(|w| w == ticker);
        let low = t.to_ascii_lowercase();
        let at = low.find("distribution of ").map(|i| i + "distribution of ".len())?;
        named.then(|| amount(&t[at..])).flatten().map(|cash| (cash, None, None, None))
    });
    let Some((cash, ex, record, pay)) = found else { return Ok(None) };
    // else the one sentence the release states its days in
    let record = record.or_else(|| day_after(&t, &["of record on ", "of record at the close of business on ", "at the close of business on ", "of record as of "]));
    let ex = ex
        .or_else(|| day_after(&t, &["ex-dividend date of ", "ex-dividend date for these distributions is anticipated to be ", "ex-dividend date is ", "ex-distribution date of ", "ex-date of "]))
        .or(record);
    let pay = pay.or_else(|| day_after(&t, &["paid on or about ", "payable on or about ", "payable on ", "will be paid on ", "paid on ", "payment date of "]));
    let Some(ex_date) = ex else { return Err(format!("the release states no ex-date or record date for {ticker}")) };
    Ok(Some(Announced { cash, ex_date, record_date: record, pay_date: pay }))
}

/// `html` without its `<tag …>…</tag>` elements.
pub fn without(html: &str, tag: &str) -> String {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(&open) {
        out.push_str(&rest[..at]);
        rest = match rest[at..].find(&close) {
            Some(end) => &rest[at + end + close.len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}
