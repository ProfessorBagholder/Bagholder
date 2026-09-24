//! Ninepoint (research 2, 2026-09-24): its fund list
//! (`/api/funds/getallfundswithpriceandperformance/en?showAllSeries=true`) maps a
//! ticker to its fund's page; the page, asked for its ETF series, states the
//! schedule (`Payment Frequency`, or `Distribution Frequency`) and lists every
//! distribution in one table: Record Date, Ex-dividend Date, Payment Date,
//! Payment Amount, Frequency, Currency, dates M/D/YYYY, amounts `$0.13500`,
//! newest first. A page that carries no such table (a money market fund's shows
//! only its latest payment, with no ex-date) is a mismatch naming what is
//! missing.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::html_tables;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{money_text, Distribution, Payer, Record};
use crate::reply::{Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "ninepoint";
pub const HOST: &str = "www.ninepoint.com";
const FUNDS: &str = "https://www.ninepoint.com/api/funds/getallfundswithpriceandperformance/en?showAllSeries=true";
const HEADER: [&str; 6] = ["Record Date", "Ex-dividend Date", "Payment Date", "Payment Amount", "Frequency", "Currency"];

/// Ninepoint's words for a schedule.
pub fn per_year(word: &str) -> Option<u32> {
    Some(match word.trim() {
        "Twice-monthly" | "Twice-Monthly" => 24,
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annual" | "Semi-annual" => 2,
        "Annual" | "Annually" => 1,
        _ => return None,
    })
}

/// A fund's page path in the fund list, by its ETF series' ticker.
pub fn page_of(v: &Value, ticker: &str) -> Result<Option<String>, Mismatch> {
    for f in Node::root(v).as_list()? {
        if f.text("fundSeriesCode")?.eq_ignore_ascii_case(ticker) && f.bool("fundSeriesIsEtf")? {
            return Ok(Some(f.text("fundPageUrl")?.to_string()));
        }
    }
    Ok(None)
}

/// `9/15/2026` as a day.
fn mdy(s: &str) -> Option<Date> {
    let mut it = s.trim().split('/');
    let (m, d, y) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() {
        return None;
    }
    Date::new(y, m, d).ok()
}

fn mismatch(path: &str, why: String) -> Mismatch {
    Mismatch { path: path.into(), why }
}

/// A fund's page: its schedule and its distributions.
pub fn parse_page(html: &str) -> Outcome<Record> {
    let tables = html_tables(html);
    let Some(table) = tables.iter().find(|t| t.first().is_some_and(|h| h.iter().map(String::as_str).eq(HEADER.iter().copied()))) else {
        return Outcome::Mismatch(mismatch("table#page-length-distributions", "the page carries no table of distributions".into()));
    };
    // the schedule: a two-cell row naming it
    let word = tables.iter().flatten().find(|r| r.len() == 2 && matches!(r[0].as_str(), "Payment Frequency" | "Distribution Frequency")).map(|r| r[1].clone());
    let per_year = match word {
        Some(w) => match per_year(&w) {
            Some(n) => Some(n),
            None => return Outcome::Mismatch(mismatch("Payment Frequency", format!("{w:?} is not a schedule this reader knows"))),
        },
        None => return Outcome::Mismatch(mismatch("Payment Frequency", "the page states no schedule".into())),
    };
    let mut rows = Vec::new();
    for (n, r) in table.iter().enumerate().skip(1) {
        let path = format!("table#page-length-distributions row {n}");
        let [record, ex, pay, amount, _frequency, currency] = r.as_slice() else {
            return Outcome::Mismatch(mismatch(&path, format!("{} cells, not 6", r.len())));
        };
        let (Some(record), Some(ex), Some(pay)) = (mdy(record), mdy(ex), mdy(pay)) else {
            return Outcome::Mismatch(mismatch(&path, format!("the dates {record:?}, {ex:?}, {pay:?} are not days")));
        };
        let Some(cash) = money_text(amount) else {
            return Outcome::Mismatch(mismatch(&path, format!("{amount:?} is not an amount")));
        };
        let Ok(currency) = Currency::parse(currency.trim()) else {
            return Outcome::Mismatch(mismatch(&path, format!("{currency:?} is not a currency")));
        };
        rows.push(Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash, reinvested: None, currency });
    }
    Outcome::Answered(Record { rows, per_year })
}

pub struct Ninepoint;

impl Payer for Ninepoint {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["ninepoint"]
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let failed = |o: Outcome<bagholder_net::Reply>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let list = match ask::send(net, &Ask::get(FUNDS, &[]), &[]) {
            Outcome::Answered(r) => r,
            other => return failed(other),
        };
        let v = match ask::json(&list.body) {
            Ok(v) => v,
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let ticker = venue::root(&need.listing.symbol);
        let path = match page_of(&v, &ticker) {
            Ok(Some(p)) => p,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("Ninepoint lists no ETF {ticker}")), shape_change: None },
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let url = format!("https://{HOST}{path}?series=ETF");
        let page = match ask::send(net, &Ask::get(&url, &[]), &[]) {
            Outcome::Answered(r) => r,
            other => return failed(other),
        };
        let outcome = match ask::text(&page.body) {
            Ok(h) => parse_page(h),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
