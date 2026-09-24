//! Global X Canada (research 2, 2026-09-24). Its fund pages answer only a
//! browser (a challenge keyed on the TLS handshake), so it is read from its own
//! monthly release on newswire.ca, "Global X announces … distributions for its
//! suite of ETFs": one table per schedule (`Table I A – Quarterly Distributions`,
//! `Table I B – Quarterly Reinvested Distributions`, `Table II – Monthly
//! Distributions`, `Table III – Semi-Monthly Distributions`), each row a ticker,
//! the fund's name, its ex-date (the record date), its pay date, the currency,
//! the amount per unit and the exchange; a second class of a fund shares the
//! first's dates and name (`DIVY.U`: ticker, currency, amount, exchange). The
//! schedule is the table the fund is in. The reinvested table's distributions
//! are paid in units.
//!
//! One read takes every distribution release the organization's page lists, so
//! the latest distribution of a quarterly fund is always among them.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::newswire::{self, Release};
use crate::payers::{money_text, Distribution, Payer, Record};
use crate::reply::Mismatch;
use crate::venue;

pub const SOURCE: &str = "global-x";
const ORGANIZATION: &str = "https://www.newswire.ca/news/global-x-investments-canada-inc/";

/// A table's schedule by its heading, and whether it is paid in units.
pub fn schedule(heading: &str) -> Option<(u32, bool)> {
    let h = heading.to_ascii_lowercase();
    if h.contains("quarterly reinvested") {
        Some((4, true))
    } else if h.contains("semi-monthly") {
        Some((24, false))
    } else if h.contains("monthly") {
        Some((12, false))
    } else if h.contains("quarterly") {
        Some((4, false))
    } else {
        None
    }
}

/// `09/29/2026` as a day.
fn mdy(s: &str) -> Option<Date> {
    let mut it = s.trim().split('/');
    let (m, d, y) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    Date::new(y, m, d).ok()
}

/// A release title that announces distributions.
pub fn announces_distributions(title: &str) -> bool {
    let t = title.to_ascii_uppercase();
    t.contains("ANNOUNCES") && t.contains("DISTRIBUTION")
}

/// The rows a release states for `ticker`, with the schedule of their table.
pub fn rows_for(r: &Release, ticker: &str) -> Result<Vec<(Distribution, u32)>, Mismatch> {
    let mut out = Vec::new();
    for (heading, table) in &r.tables {
        let mut dates: Option<(Date, Date)> = None;
        for (n, row) in table.iter().enumerate() {
            let m = |why: String| Mismatch { path: format!("{heading} row {n}"), why };
            if row.first().is_some_and(|c| c == "Ticker Symbol") {
                continue;
            }
            let (symbol, currency, amount) = match row.as_slice() {
                [symbol, _name, ex, pay, currency, amount, _exchange] => {
                    let (Some(ex), Some(pay)) = (mdy(ex), mdy(pay)) else { return Err(m(format!("the dates {ex:?}, {pay:?} are not days"))) };
                    dates = Some((ex, pay));
                    (symbol, currency, amount)
                }
                // a second class, beside the first's dates
                [symbol, currency, amount, _exchange] => (symbol, currency, amount),
                other => return Err(m(format!("{} cells", other.len()))),
            };
            if !symbol.trim().eq_ignore_ascii_case(ticker) {
                continue;
            }
            let Some((per_year, in_units)) = schedule(heading) else { return Err(m(format!("{heading:?} names no schedule this reader knows"))) };
            let Some((ex, pay)) = dates else { return Err(m("a second class with no first before it".into())) };
            let Ok(currency) = Currency::parse(currency.trim()) else { return Err(m(format!("{currency:?} is not a currency"))) };
            let Some(amount) = money_text(amount) else { return Err(m(format!("{amount:?} is not an amount"))) };
            let (cash, reinvested) = if in_units { (bagholder_core::Dec::ZERO, Some(amount)) } else { (amount, None) };
            out.push((Distribution { ex_date: ex, record_date: Some(ex), pay_date: Some(pay), cash, reinvested, currency }, per_year));
        }
    }
    Ok(out)
}

pub struct GlobalX;

impl Payer for GlobalX {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        newswire::HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        // Global X Canada was Horizons ETFs until 2024, and the broker still says so
        &["global x", "horizons"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let get = |url: &str| -> Result<String, Outcome<()>> {
            match ask::send(net, &Ask::get(url, &[]), &[]) {
                Outcome::Answered(r) => ask::text(&r.body).map(str::to_string).map_err(Outcome::Mismatch),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let page = match get(ORGANIZATION) {
            Ok(p) => p,
            Err(o) => return fail(o),
        };
        let listed = match newswire::listed(&page) {
            Ok(l) => l,
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let ticker = venue::root(&need.listing.symbol);
        let mut rows = Vec::new();
        let mut per_year = None;
        for l in listed.iter().filter(|l| announces_distributions(&l.title)) {
            let html = match get(&format!("https://{}{}", newswire::HOST, l.path)) {
                Ok(h) => h,
                Err(o) => return fail(o),
            };
            let found = match newswire::release(&html).and_then(|r| rows_for(&r, &ticker)) {
                Ok(f) => f,
                Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
            };
            for (row, n) in found {
                // the newest release's table states the schedule now
                per_year.get_or_insert(n);
                rows.push(row);
            }
        }
        if rows.is_empty() {
            return Noted { outcome: Outcome::NotCarried(format!("no Global X release lists {ticker}")), shape_change: None };
        }
        Noted { outcome: Outcome::Answered(Record { rows, per_year, by_record: vec![] }), shape_change: None }
    }
}
