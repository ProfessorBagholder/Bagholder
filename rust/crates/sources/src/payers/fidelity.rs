//! Fidelity Canada (research 2, 2026-09-24): its fund data file
//! (`www.fidelity.ca/content/fidelity-data/pnp-cached-en.json`, every fund)
//! states each series' codes and schedule (`distribution_frequency`). Its
//! distributions are stated in its own releases on newswire.ca, as Global X's
//! are: its history service lists one date per distribution, names none of them
//! the ex-date and leaves out reinvested distributions, so it is not read.
//!
//! Two kinds of release state a distribution, and only their final forms are
//! read (an "estimated" one is replaced by its final one):
//! - "Announces Cash Distributions …" and "Announces Final December … Cash
//!   Distributions …": tables of name, ticker(s) (`FCUD/FCUD.U`, one row for
//!   both classes), cash per unit in Canadian dollars (`-` where the fund pays
//!   none that period), CUSIP, ISIN, payment frequency and exchange, each table
//!   after the sentence stating its record and pay dates ("unitholders of record
//!   as of September 28, 2026, will receive a per-unit cash distribution payable
//!   on September 30, 2026"); a table with no sentence of its own shares the
//!   one before it. The ex-date is the exchange's rule from the record date.
//! - "Announces Final … Annual Reinvested Capital Gains Distributions …":
//!   tables whose sixth column is the capital gain per unit, reinvested and
//!   consolidated (paid in units), under a sentence stating the ex-date and
//!   record date ("The ex-dividend date and the record date for the 2025 annual
//!   distributions is today, December 29, 2025, and those distributions will be
//!   payable on December 31, 2025").
//!
//! The special distributions of a fund being terminated come with its
//! termination, which the broker's record carries; those releases are not read.
//!
//! The organization's page lists 25 releases a page, newest first. One read
//! takes every distribution release listed from 400 days before today, paging
//! back until a page reaches past that day, so an annual payer's latest
//! distribution is always among them.

use bagholder_core::json::Value;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::companies::{after, long_date, sessions_before};
use crate::payers::newswire::{self, Listed, Release};
use crate::payers::{money_text, ByRecord, Distribution, Payer, Record};
use crate::reply::{Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "fidelity-canada";
pub const HOST: &str = "www.fidelity.ca";
const FUNDS: &str = "https://www.fidelity.ca/content/fidelity-data/pnp-cached-en.json";
pub const ORGANIZATION: &str = "https://www.newswire.ca/news/fidelity-investments-canada-ulc/";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];
/// How far back the releases are read.
pub const WINDOW_DAYS: i64 = 400;
/// More pages than this within the window is a change of the page, not a year
/// of releases.
const MOST_PAGES: u32 = 12;

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

/// The series whose code is the ticker: its stated schedule.
pub fn schedule_of(v: &Value, ticker: &str) -> Result<Option<Option<u32>>, Mismatch> {
    for fund in Node::root(v).list("funds")? {
        for series in fund.list("series")? {
            let codes: Vec<String> = series.list("codes")?.iter().map(|c| c.text("code").map(str::to_string)).collect::<Result<_, _>>()?;
            if codes.iter().any(|c| c.eq_ignore_ascii_case(ticker)) {
                let word = series.text("distribution_frequency")?;
                return match per_year(word) {
                    Some(n) => Ok(Some(Some(n))),
                    None if word == "-" => Ok(Some(None)),
                    None => Err(series.mismatch(format!("{word:?} is not a schedule this reader knows"))),
                };
            }
        }
    }
    Ok(None)
}

/// The kind of distribution release a title announces, if it is one read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cash,
    AnnualReinvested,
}

pub fn kind_of(title: &str) -> Option<Kind> {
    let t = title.to_ascii_lowercase();
    if !t.contains("announces") || !t.contains("distributions") || t.contains("estimated") || t.contains("terminat") || t.contains("special") {
        return None;
    }
    if t.contains("annual reinvested capital gains") {
        t.contains("final").then_some(Kind::AnnualReinvested)
    } else if t.contains("cash distributions") {
        Some(Kind::Cash)
    } else {
        None
    }
}

/// Whether a ticker cell (`FCUD/FCUD.U`, `FCAB/ FCAB.U`) names `ticker`.
fn names(cell: &str, ticker: &str) -> bool {
    cell.split('/').any(|t| t.trim().eq_ignore_ascii_case(ticker))
}

/// The rows a release states for `ticker`: dated ones, and ones by record date
/// still to be dated on the exchange's sessions.
pub fn rows_for(r: &Release, kind: Kind, ticker: &str) -> Result<(Vec<Distribution>, Vec<ByRecord>), Mismatch> {
    let (mut dated, mut by_record) = (Vec::new(), Vec::new());
    let m = |path: String, why: String| Mismatch { path, why };
    // an annual release states its dates once, in its body
    let annual = match kind {
        Kind::AnnualReinvested => {
            let ex = after(&r.body, &["the record date for the "]).and_then(|s| s.find(" is today, ").map(|i| &s[i + " is today, ".len()..])).and_then(long_date);
            let pay = after(&r.body, &["will be payable on "]).and_then(long_date);
            match (ex, pay) {
                (Some(ex), Some(pay)) => Some((ex, pay)),
                _ => return Err(m("release".into(), "its sentence gives no ex-date, record date and pay date".into())),
            }
        }
        Kind::Cash => None,
    };
    let mut dates: Option<(Date, Date)> = None;
    for (t, (heading, table)) in r.tables.iter().enumerate() {
        if kind == Kind::Cash {
            if heading.contains("of record as of ") {
                let record = after(heading, &["of record as of "]).and_then(long_date);
                let pay = after(heading, &["payable on "]).and_then(long_date);
                match (record, pay) {
                    (Some(rd), Some(pd)) => dates = Some((rd, pd)),
                    _ => return Err(m(format!("table {t}"), "its sentence gives no record date and pay date".into())),
                }
            }
        }
        let Some(header) = table.first() else { continue };
        let amount_at = match kind {
            Kind::Cash => {
                let known = header.len() == 7 && header[1] == "Ticker Symbol" && header[2].to_ascii_lowercase().starts_with("cash distribution per unit (") && header[5] == "Payment Frequency";
                if !known {
                    return Err(m(format!("table {t}"), format!("its columns {header:?} are not a cash distribution table this reader knows")));
                }
                if !matches!(header[2].to_ascii_lowercase().as_str(), "cash distribution per unit (c$)" | "cash distribution per unit ($)") {
                    return Err(m(format!("table {t}"), format!("{:?} is not in Canadian dollars", header[2])));
                }
                2
            }
            Kind::AnnualReinvested => {
                let known = header.len() == 7 && header[1] == "Ticker Symbol" && header[5].to_ascii_lowercase().starts_with("annual capital gain per unit");
                if !known {
                    return Err(m(format!("table {t}"), format!("its columns {header:?} are not an annual distribution table this reader knows")));
                }
                5
            }
        };
        for (n, row) in table.iter().enumerate().skip(1) {
            let at = format!("table {t} row {n}");
            if row.len() != header.len() {
                return Err(m(at, format!("{} cells", row.len())));
            }
            if !names(&row[1], ticker) {
                continue;
            }
            let cell = row[amount_at].trim();
            // the fund pays none this period
            if cell == "-" || cell == "–" {
                continue;
            }
            let Some(amount) = money_text(cell) else { return Err(m(at, format!("{cell:?} is not an amount"))) };
            match (kind, annual, dates) {
                (Kind::AnnualReinvested, Some((ex, pay)), _) => dated.push(Distribution { ex_date: ex, record_date: Some(ex), pay_date: Some(pay), cash: Dec::ZERO, reinvested: Some(amount), currency: Currency::CAD }),
                (Kind::Cash, _, Some((record, pay))) => {
                    let row = ByRecord { record_date: record, pay_date: Some(pay), cash: amount, reinvested: None, currency: Currency::CAD };
                    if sessions_before(record) == 0 {
                        dated.push(row.with_ex(record));
                    } else {
                        by_record.push(row);
                    }
                }
                _ => return Err(m(at, "no sentence before its table states its record and pay dates".into())),
            }
        }
    }
    Ok((dated, by_record))
}

/// The first day whose releases are read, `now` in Eastern time.
pub fn window_start(now: Timestamp) -> Date {
    let today = now.to_zoned(TimeZone::get("America/Toronto").unwrap_or(TimeZone::UTC)).date();
    today.checked_sub(SignedDuration::from_hours(24 * WINDOW_DAYS)).unwrap_or(Date::MIN)
}

pub struct FidelityCanada;

impl Payer for FidelityCanada {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["fidelity"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, now: Timestamp) -> Noted<Record> {
        let get = |url: &str| -> Result<bagholder_net::Reply, Outcome<()>> {
            match ask::send(net, &Ask::get(url, &HEADERS), &[]) {
                Outcome::Answered(r) => Ok(r),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let get_text = |url: &str| -> Result<String, Outcome<()>> { get(url).and_then(|r| ask::text(&r.body).map(str::to_string).map_err(Outcome::Mismatch)) };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let mismatch = |m: Mismatch| Noted { outcome: Outcome::Mismatch(m), shape_change: None };
        let funds = match get(FUNDS) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let ticker = venue::root(&need.listing.symbol);
        let per_year = match ask::json(&funds.body).and_then(|v| schedule_of(&v, &ticker)) {
            Ok(Some(p)) => p,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("Fidelity lists no series {ticker}")), shape_change: None },
            Err(m) => return mismatch(m),
        };
        // the distribution releases of the window, paging back until one reaches past it
        let from = window_start(now);
        let mut releases: Vec<(Listed, Kind)> = Vec::new();
        let mut page = 1;
        loop {
            let url = if page == 1 { ORGANIZATION.to_string() } else { format!("{ORGANIZATION}?page={page}&pagesize=25") };
            let listed = match get_text(&url).map(|h| newswire::listed(&h)) {
                Ok(Ok(l)) => l,
                Ok(Err(m)) => return mismatch(m),
                Err(o) => return fail(o),
            };
            let mut past = false;
            for l in listed {
                let Some(day) = l.day else { return mismatch(Mismatch { path: format!("{url} {}", l.path), why: "the page gives the release no day".into() }) };
                if day < from {
                    past = true;
                    continue;
                }
                if let Some(k) = kind_of(&l.title) {
                    if !releases.iter().any(|(r, _)| r.path == l.path) {
                        releases.push((l, k));
                    }
                }
            }
            if past {
                break;
            }
            if page == MOST_PAGES {
                return mismatch(Mismatch { path: url, why: format!("{MOST_PAGES} pages do not reach back to {from}") });
            }
            page += 1;
        }
        let (mut rows, mut by_record) = (Vec::new(), Vec::new());
        for (l, kind) in &releases {
            let html = match get_text(&format!("https://{}{}", newswire::HOST, l.path)) {
                Ok(h) => h,
                Err(o) => return fail(o),
            };
            match newswire::release(&html).and_then(|r| rows_for(&r, *kind, &ticker)) {
                Ok((d, b)) => {
                    rows.extend(d);
                    by_record.extend(b);
                }
                Err(m) => return mismatch(Mismatch { path: format!("{} {}", l.path, m.path), why: m.why }),
            }
        }
        Noted { outcome: Outcome::Answered(Record { rows, per_year, by_record }), shape_change: None }
    }
}
