//! Hamilton ETFs (research 2, 2026-09-24): the fund's page
//! (`hamiltonetfs.com/etf/<ticker>/`, a class's `.` written `-`) states the
//! schedule in its details (`Distributions`: `Monthly`, `Semi-Monthly`) and lists
//! every distribution in one table: Ex-Dividend Date, Pay Date, Frequency,
//! Amount, dates YYYY-MM-DD, amounts `$0.1690`. It states no record date and no
//! currency; `$` does not name one (its USD class writes `$` too), so a
//! distribution is in the currency the class trades in.

use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::html_tables;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{money_text, Distribution, Payer, Record};
use crate::reply::{day_from, Mismatch};
use crate::venue;

pub const SOURCE: &str = "hamilton";
pub const HOST: &str = "hamiltonetfs.com";
const HEADER: [&str; 4] = ["Ex-Dividend Date", "Pay Date", "Frequency", "Amount"];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word.trim() {
        "Monthly" => 12,
        "Semi-Monthly" => 24,
        "Quarterly" => 4,
        "Annually" => 1,
        _ => return None,
    })
}

fn mismatch(path: &str, why: String) -> Mismatch {
    Mismatch { path: path.into(), why }
}

pub fn parse_page(html: &str, currency: Currency) -> Outcome<Record> {
    let tables = html_tables(html);
    let Some(table) = tables.iter().find(|t| t.first().is_some_and(|h| h.iter().map(String::as_str).eq(HEADER.iter().copied()))) else {
        return Outcome::Mismatch(mismatch("table.tablepress", "the page carries no table of distributions".into()));
    };
    let per_year = match tables.iter().flatten().find(|r| r.len() == 2 && r[0] == "Distributions").map(|r| r[1].clone()) {
        Some(w) => match per_year(&w) {
            Some(n) => Some(n),
            None => return Outcome::Mismatch(mismatch("Distributions", format!("{w:?} is not a schedule this reader knows"))),
        },
        None => return Outcome::Mismatch(mismatch("Distributions", "the page states no schedule".into())),
    };
    let mut rows = Vec::new();
    for (n, r) in table.iter().enumerate().skip(1) {
        let path = format!("table row {n}");
        let [ex, pay, _frequency, amount] = r.as_slice() else {
            return Outcome::Mismatch(mismatch(&path, format!("{} cells, not 4", r.len())));
        };
        let (Ok(ex), Ok(pay)) = (day_from(ex.trim()), day_from(pay.trim())) else {
            return Outcome::Mismatch(mismatch(&path, format!("the dates {ex:?}, {pay:?} are not days")));
        };
        let Some(cash) = money_text(amount) else {
            return Outcome::Mismatch(mismatch(&path, format!("{amount:?} is not an amount")));
        };
        rows.push(Distribution { ex_date: ex, record_date: None, pay_date: Some(pay), cash, reinvested: None, currency });
    }
    Outcome::Answered(Record { rows, per_year })
}

pub struct Hamilton;

impl Payer for Hamilton {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["hamilton"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let slug = venue::root(&need.listing.symbol).to_ascii_lowercase().replace('.', "-");
        let url = format!("https://{HOST}/etf/{slug}/");
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
