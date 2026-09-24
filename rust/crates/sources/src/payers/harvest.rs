//! Harvest (research 2, 2026-09-24): the fund's page
//! (`harvestportfolios.com/etf/<ticker>/`, which redirects the High Income Shares
//! funds to their own path) lists every distribution in tables of one of two
//! layouts, dates YYYY/MM/DD, amounts `$0.2700`, newest first:
//!
//! - High Income Shares: Record Date, Ex-dividend Date, Pay Date, Amount, Total
//!   Amount S.I., Type; the schedule is its `Distribution` row (`Monthly
//!   Variable`);
//! - the others: Ex-dividend Date, Record Date, Payment Date, Class A,
//!   Distribution Frequency, one table per year; the schedule is its `Cash
//!   Distribution Frequency` (`Monthly`; `Semi-Monthly` and `Twice Monthly` both
//!   name twice a month).
//!
//! The page states its currency in the fund's facts (`Currency CAD`); where it
//! does not, the fund pays in the currency its units trade in.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::html_tables;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{money_text, Distribution, Payer, Record};
use crate::reply::Mismatch;
use crate::venue;

pub const SOURCE: &str = "harvest";
pub const HOST: &str = "harvestportfolios.com";
/// Without a User-Agent every page answers 403; with one, `/etf/<ticker>/`
/// redirects to the fund's own page (observed 2026-09-24).
const HEADERS: [(&str, &str); 1] = [("User-Agent", ask::USER_AGENT)];
const INCOME_SHARES: [&str; 6] = ["Record Date", "Ex-dividend Date", "Pay Date", "Amount", "Total Amount S.I.", "Type"];
const CORE: [&str; 5] = ["Ex-dividend Date", "Record Date", "Payment Date", "Class A", "Distribution Frequency"];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word.trim() {
        "Monthly" | "Monthly Variable" => 12,
        "Semi-Monthly" | "Twice Monthly" => 24,
        "Quarterly" => 4,
        "Annually" | "Annual" => 1,
        _ => return None,
    })
}

/// `2026/08/31` as a day.
fn ymd(s: &str) -> Option<Date> {
    let mut it = s.trim().split('/');
    let (y, m, d) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() {
        return None;
    }
    Date::new(y, m, d).ok()
}

fn mismatch(path: &str, why: String) -> Mismatch {
    Mismatch { path: path.into(), why }
}

/// The schedule the page states: a `Distribution` or `Cash Distribution
/// Frequency` row, or a cell reading `Cash Distribution Frequency: Monthly`.
fn schedule_word(tables: &[Vec<Vec<String>>]) -> Option<String> {
    for row in tables.iter().flatten() {
        match row.as_slice() {
            [label, word] if matches!(label.trim_end_matches(':'), "Distribution" | "Cash Distribution Frequency") => return Some(word.clone()),
            _ => {}
        }
        for cell in row {
            if let Some(word) = cell.strip_prefix("Cash Distribution Frequency:") {
                return Some(word.trim().to_string());
            }
        }
    }
    None
}

pub fn parse_page(html: &str, trades_in: Currency) -> Outcome<Record> {
    let tables = html_tables(html);
    let is = |t: &Vec<Vec<String>>, header: &[&str]| t.first().is_some_and(|h| h.iter().map(|c| c.trim()).eq(header.iter().copied()));
    let history: Vec<&Vec<Vec<String>>> = tables.iter().filter(|t| is(t, &INCOME_SHARES) || is(t, &CORE)).collect();
    if history.is_empty() {
        return Outcome::Mismatch(mismatch("table.tablepress", "the page carries no table of distributions".into()));
    }
    let per_year = match schedule_word(&tables) {
        Some(w) => match per_year(&w) {
            Some(n) => Some(n),
            None => return Outcome::Mismatch(mismatch("Distribution", format!("{w:?} is not a schedule this reader knows"))),
        },
        None => return Outcome::Mismatch(mismatch("Distribution", "the page states no schedule".into())),
    };
    let currency = match tables.iter().flatten().find(|r| r.len() == 2 && r[0] == "Currency").map(|r| r[1].clone()) {
        // `CAD`, or `CAD-Unhedged`: the currency, then how the fund hedges
        Some(c) => match Currency::parse(c.trim().split('-').next().unwrap_or("")) {
            Ok(c) => c,
            Err(_) => return Outcome::Mismatch(mismatch("Currency", format!("{c:?} is not a currency"))),
        },
        None => trades_in,
    };
    let mut rows = Vec::new();
    for (t, table) in history.iter().enumerate() {
        let income_shares = is(table, &INCOME_SHARES);
        for (n, r) in table.iter().enumerate().skip(1) {
            let path = format!("table {t} row {n}");
            let (ex, record, pay, amount) = match (income_shares, r.as_slice()) {
                (true, [record, ex, pay, amount, _total, _type]) => (ex, record, pay, amount),
                (false, [ex, record, pay, amount, _frequency]) => (ex, record, pay, amount),
                _ => return Outcome::Mismatch(mismatch(&path, format!("{} cells", r.len()))),
            };
            let (Some(ex), Some(record), Some(pay)) = (ymd(ex), ymd(record), ymd(pay)) else {
                return Outcome::Mismatch(mismatch(&path, format!("the dates {ex:?}, {record:?}, {pay:?} are not days")));
            };
            let Some(cash) = money_text(amount) else {
                return Outcome::Mismatch(mismatch(&path, format!("{amount:?} is not an amount")));
            };
            rows.push(Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash, reinvested: None, currency });
        }
    }
    Outcome::Answered(Record { rows, per_year })
}

pub struct Harvest;

impl Payer for Harvest {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["harvest"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let ticker = venue::root(&need.listing.symbol).to_ascii_lowercase();
        let url = format!("https://{HOST}/etf/{ticker}/");
        let page = match ask::send(net, &Ask::get(&url, &HEADERS), &[404]) {
            Outcome::Answered(r) => r,
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
        };
        let outcome = match ask::text(&page.body) {
            Ok(h) => parse_page(h, need.listing.currency),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
