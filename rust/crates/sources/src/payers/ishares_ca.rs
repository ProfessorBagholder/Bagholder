//! iShares Canada (research 2, 2026-09-24): BlackRock Canada's product
//! screener maps a ticker (`localExchangeTicker`) to its fund's id; the fund's
//! page states the schedule (its `Distribution Frequency` item); the fund's
//! distributions come from the data its page reads (`…/fund/1464253357804.ajax?
//! tab=distributions&fileType=json&subtab=all.table`, which begins with a byte
//! order mark): per row the ex-date, pay date and record date, then the cash
//! amount, the total and the reinvested part, each written with its currency
//! (`CAD 0.10200`). Declared distributions not yet paid are included. A total
//! that is not the two parts is a meaning failure.

use bagholder_core::jiff::civil::Date;
use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::unescape;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::{self, Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "ishares-canada";
pub const HOST: &str = "www.blackrock.com";
const BASE: &str = "https://www.blackrock.com/ca/investors/en";
const SCREENER: &str = "https://www.blackrock.com/ca/investors/en/product-screener/product-screener-v3.1.jsn?dcrPath=/templatedata/config/product-screener-v3/data/en/ca-one/product-screener-backend-config&siteEntryPassthrough=true";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annual" | "Semi-Annually" => 2,
        "Annual" | "Annually" => 1,
        _ => return None,
    })
}

/// A reply's JSON, its byte order mark left out.
pub fn json(body: &[u8]) -> Result<Value, Mismatch> {
    let text = ask::text(body)?;
    reply::parse(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// A fund's id and its page's path, by its ticker.
pub fn fund_of(v: &Value, ticker: &str) -> Result<Option<(String, String)>, Mismatch> {
    let root = Node::root(v);
    for id in root.keys()? {
        let fund = root.obj(id)?;
        if fund.text("localExchangeTicker")?.eq_ignore_ascii_case(ticker) {
            return Ok(Some((id.to_string(), fund.text("productPageUrl")?.to_string())));
        }
    }
    Ok(None)
}

/// The schedule the fund's page states.
pub fn schedule_word(html: &str) -> Option<String> {
    let i = html.find("col-distributionFrequency")?;
    let j = html[i..].find("<div class=\"data\">")? + i + "<div class=\"data\">".len();
    let end = html[j..].find("</div>")? + j;
    Some(unescape(html[j..end].trim()))
}

fn day(n: &Node) -> Result<Date, Mismatch> {
    let raw = n.int("raw")?;
    let (y, m, d) = (raw / 10000, raw / 100 % 100, raw % 100);
    Date::new(y as i16, m as i8, d as i8).map_err(|_| n.mismatch(format!("{raw} is not a day")))
}

/// `CAD 0.10200` as its currency and amount.
fn amount(n: &Node) -> Result<(Currency, Dec), Mismatch> {
    let display = n.text("display")?;
    let (c, a) = display.split_once(' ').ok_or_else(|| n.mismatch(format!("{display:?} is not a currency and an amount")))?;
    let currency = Currency::parse(c).map_err(|e| n.mismatch(e.to_string()))?;
    let amount = Dec::parse(a).map_err(|e| n.mismatch(format!("{a:?}: {e}")))?;
    Ok((currency, amount))
}

pub fn parse_history(v: &Value) -> Outcome<Vec<Distribution>> {
    let read = || -> Result<Result<Vec<Distribution>, String>, Mismatch> {
        let mut out = Vec::new();
        for row in Node::root(v).obj("all.table")?.list("aaData")? {
            let cells = row.as_list()?;
            let [ex, pay, record, cash, total, units] = cells.as_slice() else {
                return Err(row.mismatch(format!("{} cells, not 6", cells.len())));
            };
            let (ex, pay, record) = (day(ex)?, day(pay)?, day(record)?);
            let ((c1, cash), (c2, total), (c3, units)) = (amount(cash)?, amount(total)?, amount(units)?);
            if c1 != c2 || c2 != c3 {
                return Ok(Err(format!("the distribution going ex {ex} is in more than one currency")));
            }
            if cash.checked_add(units).ok() != Some(total) {
                return Ok(Err(format!("the distribution going ex {ex} totals {total}, its parts {cash} and {units}")));
            }
            out.push(Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash, reinvested: (!units.is_zero()).then_some(units), currency: c1 });
        }
        Ok(Ok(out))
    };
    match read() {
        Ok(Ok(d)) => Outcome::Answered(d),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub struct ISharesCanada;

impl Payer for ISharesCanada {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["ishares"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let get = |url: &str| -> Result<bagholder_net::Reply, Outcome<()>> {
            match ask::send(net, &Ask::get(url, &HEADERS), &[]) {
                Outcome::Answered(r) => Ok(r),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let mismatch = |m: Mismatch| Noted { outcome: Outcome::Mismatch(m), shape_change: None };
        let screener = match get(SCREENER) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let ticker = venue::root(&need.listing.symbol);
        let (id, path) = match json(&screener.body).and_then(|v| fund_of(&v, &ticker)) {
            Ok(Some(f)) => f,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("iShares Canada lists no fund {ticker}")), shape_change: None },
            Err(m) => return mismatch(m),
        };
        let page = match get(&format!("https://{HOST}{path}")) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let per_year = match ask::text(&page.body).ok().and_then(schedule_word) {
            Some(w) => match per_year(&w) {
                Some(n) => Some(n),
                None => return mismatch(Mismatch { path: "Distribution Frequency".into(), why: format!("{w:?} is not a schedule this reader knows") }),
            },
            None => return mismatch(Mismatch { path: "Distribution Frequency".into(), why: "the page states no schedule".into() }),
        };
        let history = match get(&format!("{BASE}/products/{id}/fund/1464253357804.ajax?tab=distributions&fileType=json&subtab=all.table")) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let outcome = match json(&history.body) {
            Ok(v) => parse_history(&v).map(|rows| Record { rows, per_year }),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
