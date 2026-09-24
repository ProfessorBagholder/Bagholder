//! Evolve (research 2, 2026-09-24): the fund's page
//! (`evolveetfs.com/product/<ticker>/`) states the schedule in its
//! `DISTRIBUTION FREQUENCY` box (`Twice per month`) and lists the distributions
//! in one table per year: Ex-Dividend Date:, Record Date:, Payment Date:, Payment
//! Amount:, Distribution Frequency:, dates MM/DD/YY (padding uneven, `04/8/26`),
//! amounts `$0.31000`, oldest first, announced ones included. The page states
//! no currency per row; the fund pays in the currency its units trade in.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::SourceName;
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::{html_tables, unescape};
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{money_text, Distribution, Payer, Record};
use crate::reply::Mismatch;
use crate::venue;

pub const SOURCE: &str = "evolve";
pub const HOST: &str = "evolveetfs.com";
const HEADER: [&str; 5] = ["Ex-Dividend Date:", "Record Date:", "Payment Date:", "Payment Amount:", "Distribution Frequency:"];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word.trim() {
        "Twice per month" => 24,
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" | "Semi-annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

/// `04/8/26` as a day.
fn mmddyy(s: &str) -> Option<Date> {
    let mut it = s.trim().split('/');
    let (m, d, y): (i8, i8, i16) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() || !(0..100).contains(&y) {
        return None;
    }
    Date::new(2000 + y, m, d).ok()
}

fn mismatch(path: &str, why: String) -> Mismatch {
    Mismatch { path: path.into(), why }
}

/// The word in the page's schedule box: the heading that follows its title.
fn schedule_word(html: &str) -> Option<String> {
    let i = html.find("DISTRIBUTION FREQUENCY")?;
    let rest = &html[i..];
    let h2 = rest.find("<h2")?;
    let rest = &rest[h2..];
    let start = rest.find('>')? + 1;
    let end = rest[start..].find("</h2>")? + start;
    Some(unescape(rest[start..end].trim()))
}

pub fn parse_page(html: &str, currency: bagholder_core::Currency) -> Outcome<Record> {
    let tables: Vec<Vec<Vec<String>>> = html_tables(html).into_iter().filter(|t| t.first().is_some_and(|h| h.iter().map(String::as_str).eq(HEADER.iter().copied()))).collect();
    if tables.is_empty() {
        return Outcome::Mismatch(mismatch("table.tablepress", "the page carries no table of distributions".into()));
    }
    let per_year = match schedule_word(html) {
        Some(w) => match per_year(&w) {
            Some(n) => Some(n),
            None => return Outcome::Mismatch(mismatch("DISTRIBUTION FREQUENCY", format!("{w:?} is not a schedule this reader knows"))),
        },
        None => return Outcome::Mismatch(mismatch("DISTRIBUTION FREQUENCY", "the page states no schedule".into())),
    };
    let mut rows = Vec::new();
    for (t, table) in tables.iter().enumerate() {
        for (n, r) in table.iter().enumerate().skip(1) {
            let path = format!("table {t} row {n}");
            let [ex, record, pay, amount, _frequency] = r.as_slice() else {
                return Outcome::Mismatch(mismatch(&path, format!("{} cells, not 5", r.len())));
            };
            let (Some(ex), Some(record), Some(pay)) = (mmddyy(ex), mmddyy(record), mmddyy(pay)) else {
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

pub struct Evolve;

impl Payer for Evolve {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["evolve"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let ticker = venue::root(&need.listing.symbol).to_ascii_lowercase();
        let url = format!("https://{HOST}/product/{ticker}/");
        let page = match ask::send(net, &Ask::get(&url, &[]), &[404]) {
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
