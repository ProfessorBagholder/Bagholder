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
//!
//! The page lists a month's distribution only days after Harvest announces it
//! (on 2026-10-23 the October release was out and every fund page still ended
//! at September's). The announcement itself is on Harvest's press pages, one
//! for the ETFs and one for the High Income Shares, each linking its month's
//! release (`…-announces-october-2026-distributions/`): one sentence states the
//! days for every fund in it ("paid on or about November 6, 2026 to
//! unitholders of record on October 30, 2026, with an ex-dividend date of
//! October 30, 2026"), then each fund's ticker and amount (`RDDY $0.1500 per
//! unit`). The newest release is read beside the page, and what it announces
//! for the fund is its newest distribution where the page has not listed it.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::{html_tables, text};
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

/// A page's text, or why it could not be had.
fn read_text(net: &Net, url: &str) -> Outcome<String> {
    match ask::send(net, &Ask::get(url, &HEADERS), &[]) {
        Outcome::Answered(r) => match ask::text(&r.body) {
            Ok(h) => Outcome::Answered(h.to_string()),
            Err(m) => Outcome::Mismatch(m),
        },
        other => other.failed().expect("not answered"),
    }
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
    Outcome::Answered(Record { form: bagholder_core::distribution::Form::Stated, rows, per_year, by_record: vec![] })
}

/// The press page a fund's releases are on, and the start of its monthly
/// releases' addresses: the High Income Shares' for a page of that layout.
fn press_of(income_shares: bool) -> (&'static str, &'static str) {
    if income_shares {
        ("high-income-shares/press/", "harvest-high-income-shares-etfs-announces-")
    } else {
        ("press-releases/", "harvest-etfs-announces-")
    }
}

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

/// The newest monthly release a press page links whose address starts with
/// `prefix` (`…/harvest-etfs-announces-october-2026-distributions/`).
pub fn newest_release(html: &str, prefix: &str) -> Option<String> {
    let mut best: Option<((i16, usize), String)> = None;
    for part in html.split("href=\"").skip(1) {
        let Some(url) = part.split('"').next() else { continue };
        let Some(slug) = url.strip_prefix(&format!("https://{HOST}/")).map(|s| s.trim_end_matches('/')) else { continue };
        let Some(rest) = slug.strip_prefix(prefix).and_then(|r| r.strip_suffix("-distributions")) else { continue };
        let Some((month, year)) = rest.split_once('-') else { continue };
        let (Some(m), Ok(y)) = (MONTHS.iter().position(|n| *n == month), year.parse::<i16>()) else { continue };
        if best.as_ref().is_none_or(|(at, _)| (y, m) > *at) {
            best = Some(((y, m), url.to_string()));
        }
    }
    best.map(|(_, url)| url)
}

/// `html` without its `<tag …>…</tag>` elements.
fn without(html: &str, tag: &str) -> String {
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

/// `November 6, 2026` as a day.
fn long_day(s: &str) -> Option<Date> {
    let mut it = s.split_whitespace();
    let word = it.next()?;
    let m = MONTHS.iter().position(|n| n.eq_ignore_ascii_case(word))? as i8 + 1;
    let d: i8 = it.next()?.trim_end_matches(',').parse().ok()?;
    let y: i16 = it.next()?.trim_end_matches(['.', ',']).parse().ok()?;
    Date::new(y, m, d).ok()
}

/// The words after `after` up to `until`, in `t`.
fn between<'a>(t: &'a str, after: &str, until: &str) -> Option<&'a str> {
    let from = t.find(after)? + after.len();
    let len = t[from..].find(until)?;
    Some(t[from..from + len].trim())
}

/// What a monthly release announces for `ticker`: its amount in the month's
/// table, on the days the release states for the month. None where the
/// release does not name the fund.
pub fn announced(html: &str, ticker: &str, currency: Currency) -> Outcome<Option<Distribution>> {
    // the page's scripts carry the release's first sentence again, in its metadata
    let t = text(&without(&without(html, "script"), "style"));
    // the month's distributions: from the release's first sentence to the
    // twice-monthly or estimated ones it may announce after them
    let Some(start) = t.find("announces the following") else {
        return Outcome::Mismatch(mismatch("release", "the release announces no distributions".into()));
    };
    let month = &t[start..];
    let month = &month[..["Harvest also announces", "Note:"].iter().filter_map(|w| month.find(w)).min().unwrap_or(month.len())];
    let days = (between(month, "paid on or about ", " to "), between(month, "of record on ", ", with"), between(month, "ex-dividend date of ", "."));
    let (Some(pay), Some(record), Some(ex)) = (days.0.and_then(long_day), days.1.and_then(long_day), days.2.and_then(long_day)) else {
        return Outcome::Mismatch(mismatch("release", format!("the release's days {days:?} are not days")));
    };
    let words: Vec<&str> = month.split_whitespace().collect();
    let Some(at) = words.windows(2).position(|w| w[0] == ticker && w[1].starts_with('$')) else { return Outcome::Answered(None) };
    let Some(cash) = money_text(words[at + 1]) else {
        return Outcome::Mismatch(mismatch("release", format!("{:?} is not an amount", words[at + 1])));
    };
    Outcome::Answered(Some(Distribution { ex_date: ex, record_date: Some(record), pay_date: Some(pay), cash, reinvested: None, currency }))
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
        let html = match ask::text(&page.body) {
            Ok(h) => h,
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let mut record = match parse_page(html, need.listing.currency) {
            Outcome::Answered(r) => r,
            other => return Noted { outcome: other, shape_change: None },
        };
        // the month's announcement, where the page has not listed it yet
        let income_shares = html_tables(html).iter().any(|t| t.first().is_some_and(|h| h.iter().map(|c| c.trim()).eq(INCOME_SHARES.iter().copied())));
        let (press, prefix) = press_of(income_shares);
        let listing = match read_text(net, &format!("https://{HOST}/{press}")) {
            Outcome::Answered(p) => p,
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
        };
        let Some(url) = newest_release(&listing, prefix) else {
            return Noted { outcome: Outcome::Mismatch(mismatch(press, format!("the press page links no release starting {prefix:?}"))), shape_change: None };
        };
        let release = match read_text(net, &url) {
            Outcome::Answered(r) => r,
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
        };
        let currency = record.rows.first().map(|r| r.currency).unwrap_or(need.listing.currency);
        match announced(&release, &venue::root(&need.listing.symbol), currency) {
            Outcome::Answered(Some(d)) => {
                if record.rows.iter().all(|r| r.ex_date < d.ex_date) {
                    record.rows.insert(0, d);
                }
            }
            Outcome::Answered(None) => {}
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
        }
        Noted { outcome: Outcome::Answered(record), shape_change: None }
    }
}
