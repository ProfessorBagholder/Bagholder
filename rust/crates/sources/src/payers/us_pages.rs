//! Two US fund companies whose own page lists every distribution
//! (research 2, 2026-09-24):
//!
//! - **YieldMax** (`yieldmaxetfs.com/our-etfs/<ticker>/`): Distribution per
//!   Share, Declared Date, Ex Date, Record Date, Payable Date, ROC, dates
//!   MM/DD/YYYY, newest first, declared ones included. Amounts are as paid per
//!   share at the time, not restated for a later consolidation (MSTY's 1:5 of
//!   2025-12-08). The schedule is the fund's own `distribution-frequency` term in
//!   the site's data (`/wp-json/wp/v2/etf?slug=<ticker>`, terms at
//!   `/wp-json/wp/v2/distribution-frequency`): `Weekly` for MSTY. A fund the data
//!   does not list is not carried.
//! - **Defiance** (`defianceetfs.com/<ticker>/`): Declaration Date, Ex-Div Date,
//!   Record Date, Payable Date, Amount ($); a scheduled row with no amount yet
//!   (`—`) is a date, not a distribution. The page states no schedule; the
//!   company states it only in a PDF this reader does not read, so none is stored.
//!
//! Both are US funds paying in US dollars.

use bagholder_core::jiff::civil::Date;
use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::html_tables;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{money_text, Distribution, Payer, Record};
use crate::reply::{Mismatch, Node};
use crate::venue;

const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

/// `09/24/2026` as a day.
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

fn page_text(net: &Net, url: &str) -> Result<String, Outcome<()>> {
    match ask::send(net, &Ask::get(url, &HEADERS), &[404]) {
        Outcome::Answered(r) => ask::text(&r.body).map(str::to_string).map_err(Outcome::Mismatch),
        other => Err(other.failed().expect("not answered")),
    }
}

// -- YieldMax -----------------------------------------------------------------

pub const YIELDMAX: &str = "yieldmax";
pub const YIELDMAX_HOST: &str = "yieldmaxetfs.com";
const YIELDMAX_HEADER: [&str; 6] = ["DISTRIBUTION PER SHARE", "DECLARED DATE", "EX DATE", "RECORD DATE", "PAYABLE DATE", "ROC"];

pub fn yieldmax_per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Weekly" => 52,
        "Monthly" => 12,
        "Quarterly" => 4,
        _ => return None,
    })
}

/// The fund's schedule from the site's data: its term's name.
pub fn yieldmax_schedule(etf: &Value, terms: &Value) -> Result<Option<Option<u32>>, Mismatch> {
    let list = Node::root(etf).as_list()?;
    let Some(fund) = list.first() else { return Ok(None) };
    let ids: Vec<i64> = fund.list("distribution-frequency")?.iter().map(|n| n.as_int()).collect::<Result<_, _>>()?;
    let [id] = ids.as_slice() else {
        return Ok(Some(None));
    };
    for t in Node::root(terms).as_list()? {
        if t.int("id")? == *id {
            let name = t.text("name")?;
            return yieldmax_per_year(name).map(|n| Some(Some(n))).ok_or_else(|| t.mismatch(format!("{name:?} is not a schedule this reader knows")));
        }
    }
    Err(fund.mismatch(format!("its schedule term {id} is not among the site's terms")))
}

pub fn yieldmax_page(html: &str) -> Outcome<Vec<Distribution>> {
    let tables = html_tables(html);
    let Some(table) = tables.iter().find(|t| t.first().is_some_and(|h| h.iter().map(String::as_str).eq(YIELDMAX_HEADER.iter().copied()))) else {
        return Outcome::Mismatch(mismatch("table.distributions-table", "the page carries no table of distributions".into()));
    };
    let mut rows = Vec::new();
    for (n, r) in table.iter().enumerate().skip(1) {
        let path = format!("table row {n}");
        let [amount, _declared, ex, record, pay, _roc] = r.as_slice() else {
            return Outcome::Mismatch(mismatch(&path, format!("{} cells, not 6", r.len())));
        };
        let (Some(ex), Some(record), Some(pay)) = (mdy(ex), mdy(record), mdy(pay)) else {
            return Outcome::Mismatch(mismatch(&path, format!("the dates {ex:?}, {record:?}, {pay:?} are not days")));
        };
        let Some(cash) = money_text(amount) else {
            return Outcome::Mismatch(mismatch(&path, format!("{amount:?} is not an amount")));
        };
        rows.push(Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash, reinvested: None, currency: Currency::USD });
    }
    Outcome::Answered(rows)
}

pub struct YieldMax;

impl Payer for YieldMax {
    fn source(&self) -> SourceName {
        SourceName::named(YIELDMAX)
    }

    fn host(&self) -> &'static str {
        YIELDMAX_HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["yieldmax"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::US
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let ticker = venue::root(&need.listing.symbol).to_ascii_lowercase();
        let json = |url: &str| page_text(net, url).and_then(|t| crate::reply::parse(&t).map_err(Outcome::Mismatch));
        let etf = match json(&format!("https://{YIELDMAX_HOST}/wp-json/wp/v2/etf?slug={ticker}")) {
            Ok(v) => v,
            Err(o) => return fail(o),
        };
        let terms = match json(&format!("https://{YIELDMAX_HOST}/wp-json/wp/v2/distribution-frequency")) {
            Ok(v) => v,
            Err(o) => return fail(o),
        };
        let per_year = match yieldmax_schedule(&etf, &terms) {
            Ok(Some(p)) => p,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("YieldMax lists no fund {ticker}")), shape_change: None },
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let outcome = match page_text(net, &format!("https://{YIELDMAX_HOST}/our-etfs/{ticker}/")) {
            Ok(h) => yieldmax_page(&h).map(|rows| Record { form: bagholder_core::distribution::Form::Stated, rows, per_year, by_record: vec![] }),
            Err(o) => o.failed().expect("not answered"),
        };
        Noted { outcome, shape_change: None }
    }
}

// -- Defiance -------------------------------------------------------------------

pub const DEFIANCE: &str = "defiance";
pub const DEFIANCE_HOST: &str = "www.defianceetfs.com";
const DEFIANCE_HEADER: [&str; 5] = ["Declaration Date", "Ex-Div Date", "Record Date", "Payable Date", "Amount ($)"];

pub fn defiance_page(html: &str) -> Outcome<Vec<Distribution>> {
    let tables = html_tables(html);
    let Some(table) = tables.iter().find(|t| t.first().is_some_and(|h| h.iter().map(|c| c.trim()).eq(DEFIANCE_HEADER.iter().copied()))) else {
        return Outcome::Mismatch(mismatch("table.def-dist-table", "the page carries no table of distributions".into()));
    };
    let mut rows = Vec::new();
    for (n, r) in table.iter().enumerate().skip(1) {
        let path = format!("table row {n}");
        let [_declared, ex, record, pay, amount] = r.as_slice() else {
            return Outcome::Mismatch(mismatch(&path, format!("{} cells, not 5", r.len())));
        };
        // a date on the schedule with no amount yet
        if amount.trim() == "\u{2014}" {
            continue;
        }
        let (Some(ex), Some(record), Some(pay)) = (mdy(ex), mdy(record), mdy(pay)) else {
            return Outcome::Mismatch(mismatch(&path, format!("the dates {ex:?}, {record:?}, {pay:?} are not days")));
        };
        let Some(cash) = money_text(amount) else {
            return Outcome::Mismatch(mismatch(&path, format!("{amount:?} is not an amount")));
        };
        rows.push(Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash, reinvested: None, currency: Currency::USD });
    }
    Outcome::Answered(rows)
}

pub struct Defiance;

impl Payer for Defiance {
    fn source(&self) -> SourceName {
        SourceName::named(DEFIANCE)
    }

    fn host(&self) -> &'static str {
        DEFIANCE_HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["defiance"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::US
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let ticker = venue::root(&need.listing.symbol).to_ascii_lowercase();
        let outcome = match page_text(net, &format!("https://{DEFIANCE_HOST}/{ticker}/")) {
            Ok(h) => defiance_page(&h).map(|rows| Record { form: bagholder_core::distribution::Form::Stated, rows, per_year: None, by_record: vec![] }),
            Err(o) => o.failed().expect("not answered"),
        };
        Noted { outcome, shape_change: None }
    }
}
