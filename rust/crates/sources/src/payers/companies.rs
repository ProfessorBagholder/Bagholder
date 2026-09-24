//! Companies that pay a dividend declare each one in a release (research 2,
//! 2026-09-24), and every one of the nine reaches newswire.ca. Each company's
//! sentence is its own ("declared a quarterly dividend of $0.4375 per common
//! share, payable on October 15, 2026 to shareholders of record at the close of
//! business on September 15, 2026"; TD's "one dollar and twelve cents ($1.12)";
//! Couche-Tard's "CA 21.5¢"), so each company is a row of [`COMPANIES`]: its
//! organization page, which releases declare, what precedes the amount, the pay
//! date and the record date in its sentence, and where it states its schedule.
//! A declaring release in which any of them cannot be read is a mismatch naming
//! it, never a skipped release.
//!
//! A company states its record date, and only TD its ex-date elsewhere, so the
//! ex-date is the exchange's rule from the record date: the same day since
//! 2024-05-27 (settlement in one day), the business day before until then.
//! Amounts are in Canadian dollars unless the sentence writes `US$`. A special
//! dividend declared beside the regular one (Alvopetro's) is not read.

use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::unescape;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::newswire::{self, Release};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::Mismatch;
use crate::venue;

pub const SOURCE: &str = "newswire";

/// Where a company states its schedule.
#[derive(Clone, Copy, Debug)]
pub enum Schedule {
    /// In the declaring release, by a phrase: each phrase and its payments a year.
    InRelease(&'static [(&'static str, u32)]),
    /// On a page of its own site, by a sentence.
    OnPage { url: &'static str, phrase: &'static str, per_year: u32 },
}

#[derive(Clone, Copy, Debug)]
pub struct Company {
    pub ticker: &'static str,
    pub mic: &'static str,
    /// Its organization page on newswire.ca: `/news/<organization>/`.
    pub organization: &'static str,
    /// A release that declares has a title that begins with `title_starts`,
    /// contains every one of `title_has` and ends with `title_ends`, case aside.
    pub title_starts: &'static str,
    pub title_has: &'static [&'static str],
    pub title_ends: &'static str,
    /// What precedes the common share dividend's amount in its sentence.
    pub amount_after: &'static [&'static str],
    pub pay_after: &'static [&'static str],
    pub record_after: &'static [&'static str],
    pub schedule: Schedule,
}

const QUARTERLY: &[(&str, u32)] = &[("quarterly dividend", 4)];

pub const COMPANIES: &[Company] = &[
    Company {
        ticker: "BNS",
        mic: "XTSE",
        organization: "scotiabank",
        title_starts: "",
        title_has: &["Announces Dividend on Outstanding Common Shares"],
        title_ends: "",
        amount_after: &["Common Shares Dividend No."],
        pay_after: &["payable on "],
        record_after: &["of record at the close of business on "],
        schedule: Schedule::OnPage {
            url: "https://www.scotiabank.com/ca/en/about/investors-shareholders/equity-investors/common-share-data.html",
            phrase: "pay common share dividends on a quarterly basis",
            per_year: 4,
        },
    },
    Company {
        ticker: "TD",
        mic: "XTSE",
        organization: "td-bank-group",
        title_starts: "",
        title_has: &["DECLARES DIVIDENDS"],
        title_ends: "",
        amount_after: &["dividend in an amount of"],
        pay_after: &["payable on and after "],
        record_after: &["of record at the close of business on "],
        schedule: Schedule::InRelease(&[("declared for the quarter ending", 4)]),
    },
    Company {
        ticker: "BCE",
        mic: "XTSE",
        organization: "bce-inc.",
        title_starts: "BCE reports",
        title_has: &["results"],
        title_ends: "",
        amount_after: &["declared a quarterly dividend of"],
        pay_after: &["payable on "],
        record_after: &["of record at the close of business on "],
        schedule: Schedule::InRelease(QUARTERLY),
    },
    Company {
        ticker: "ENB",
        mic: "XTSE",
        organization: "enbridge-inc",
        title_starts: "",
        title_has: &["Declares Quarterly Dividends"],
        title_ends: "",
        amount_after: &["declared a quarterly dividend of"],
        pay_after: &["payable on "],
        record_after: &["of record on "],
        schedule: Schedule::InRelease(QUARTERLY),
    },
    Company {
        ticker: "T",
        mic: "XTSE",
        organization: "telus-corporation",
        title_starts: "",
        title_has: &["NOTICE OF CASH DIVIDEND"],
        title_ends: "",
        amount_after: &["declared a quarterly dividend of"],
        pay_after: &["payable on "],
        record_after: &["of record at the close of business on "],
        schedule: Schedule::InRelease(QUARTERLY),
    },
    Company {
        ticker: "CP",
        mic: "XTSE",
        organization: "cpkc",
        title_starts: "",
        title_has: &["declares dividend"],
        title_ends: "",
        amount_after: &["declared a quarterly dividend of"],
        pay_after: &["payable on "],
        record_after: &["of record at the close of business on "],
        schedule: Schedule::InRelease(QUARTERLY),
    },
    Company {
        ticker: "ATD",
        mic: "XTSE",
        organization: "alimentation-couche-tard-inc",
        title_starts: "",
        title_has: &["ANNOUNCES ITS RESULTS"],
        title_ends: "",
        amount_after: &["declared a quarterly dividend of"],
        pay_after: &["approved its payment effective "],
        record_after: &["on record as at "],
        schedule: Schedule::InRelease(QUARTERLY),
    },
    Company {
        ticker: "ALV",
        mic: "XTSX",
        organization: "alvopetro-energy-ltd",
        title_starts: "",
        title_has: &["Dividend of US$"],
        title_ends: "",
        amount_after: &["declared a quarterly dividend of"],
        pay_after: &["payable in cash on "],
        record_after: &["of record at the close of business on "],
        schedule: Schedule::InRelease(QUARTERLY),
    },
    Company {
        ticker: "DE",
        mic: "XTSX",
        organization: "decisive-dividend-corporation",
        // its name carries "Dividend": the declaration is "Announces October 2026 Dividend"
        title_starts: "Decisive Dividend Corporation Announces",
        title_has: &[],
        title_ends: " Dividend",
        amount_after: &["declared a dividend of"],
        pay_after: &["payable on "],
        record_after: &["of record at the close of business "],
        schedule: Schedule::InRelease(&[("monthly dividend policy", 12)]),
    },
];

/// The company a listing is, by its ticker and venue.
pub fn company_of(need: &PayerNeed) -> Option<&'static Company> {
    let ticker = venue::root(&need.listing.symbol);
    let mic = need.listing.venue_mic.as_deref()?;
    COMPANIES.iter().find(|c| c.ticker == ticker && c.mic == mic)
}

const MONTHS: [(&str, i8); 21] = [
    ("January", 1), ("February", 2), ("March", 3), ("April", 4), ("May", 5), ("June", 6), ("July", 7), ("August", 8), ("September", 9), ("October", 10), ("November", 11), ("December", 12),
    ("Jan.", 1), ("Feb.", 2), ("Aug.", 8), ("Sept.", 9), ("Sep.", 9), ("Oct.", 10), ("Nov.", 11), ("Dec.", 12), ("Apr.", 4),
];

/// A day written `October 15, 2026` at the start of `s`.
pub fn long_date(s: &str) -> Option<Date> {
    let s = s.trim_start();
    let (month, rest) = MONTHS.iter().find_map(|(name, m)| s.strip_prefix(name).map(|r| (*m, r)))?;
    let rest = rest.trim_start();
    let day_end = rest.find(|c: char| !c.is_ascii_digit())?;
    let day: i8 = rest[..day_end].parse().ok()?;
    let rest = rest[day_end..].strip_prefix(',')?.trim_start();
    let year: i16 = rest.get(..4)?.parse().ok()?;
    Date::new(year, month, day).ok()
}

/// The amount at the start of `s`: `$0.4375`, `US$0.12`, `CA 21.5¢`,
/// `one dollar and twelve cents ($1.12)`, `No. 629 of $1.14`; the text between
/// the marker and the first `$` or number is skipped only when it is words.
pub fn amount(s: &str) -> Option<(Dec, Currency)> {
    // the amount follows its marker closely: nothing further on is read for it
    let mut cut = s.len().min(80);
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let s = &s[..cut];
    let usd = {
        let head = &s[..s.find(|c: char| c.is_ascii_digit()).unwrap_or(0)];
        head.contains("US$")
    };
    // the first figure written with a dollar sign, or in cents
    let dollar = s.find('$');
    let start = match dollar {
        Some(i) => i + 1,
        None => s.find(|c: char| c.is_ascii_digit())?,
    };
    let rest = &s[start..];
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(rest.len());
    let number = rest[..end].trim_end_matches(['.', ',']).replace(',', "");
    let mut value = Dec::parse(&number).ok()?;
    if dollar.is_none() && rest[end..].trim_start().starts_with('¢') {
        value = value.checked_mul(Dec::parse("0.01").ok()?).ok()?;
    }
    Some((value, if usd { Currency::USD } else { Currency::CAD }))
}

/// The text right after the first of `markers` in `body`.
fn after<'a>(body: &'a str, markers: &[&str]) -> Option<&'a str> {
    markers.iter().filter_map(|m| body.find(m).map(|i| &body[i + m.len()..])).next()
}

/// The first record date on which the ex-date is the record date: settlement in
/// one business day (T+1) from 2024-05-27.
pub const T_PLUS_ONE: Date = date(2024, 5, 27);

/// The ex-date the exchange's rule gives a record date: the record date itself
/// since T+1. Before it the ex-date was one business day before the record date
/// (two before 2017-09-05), counted on the exchange's own calendar of sessions,
/// which this reader does not hold: none is stated rather than one guessed.
pub fn ex_date(record: Date) -> Option<Date> {
    (record >= T_PLUS_ONE).then_some(record)
}

/// The common share dividend a declaring release states; none for a record date
/// before T+1, whose ex-date is not stated here (see [`ex_date`]).
pub fn declared(c: &Company, r: &Release) -> Result<Option<Distribution>, Mismatch> {
    let m = |what: &str| Mismatch { path: format!("{} release of {}", c.ticker, r.at), why: format!("its sentence gives no {what}") };
    let (cash, currency) = after(&r.body, c.amount_after).and_then(amount).ok_or_else(|| m("amount"))?;
    let pay = after(&r.body, c.pay_after).and_then(long_date).ok_or_else(|| m("pay date"))?;
    let record = after(&r.body, c.record_after).and_then(long_date).ok_or_else(|| m("record date"))?;
    Ok(ex_date(record).map(|ex_date| Distribution { ex_date, record_date: Some(record), pay_date: Some(pay), cash, reinvested: None, currency }))
}

/// The schedule a release states, where the company states it there.
pub fn schedule_in(c: &Company, r: &Release) -> Option<u32> {
    match c.schedule {
        Schedule::InRelease(phrases) => phrases.iter().find(|(p, _)| r.body.to_lowercase().contains(&p.to_lowercase())).map(|(_, n)| *n),
        Schedule::OnPage { .. } => None,
    }
}

/// The schedule a company's own page states.
pub fn schedule_on_page(c: &Company, html: &str) -> Option<u32> {
    match c.schedule {
        Schedule::OnPage { phrase, per_year, .. } => unescape(html).contains(phrase).then_some(per_year),
        Schedule::InRelease(_) => None,
    }
}

/// The mismatch when a company's own statement of its schedule is not found
/// where it makes it.
pub fn schedule_missing(c: &Company) -> Mismatch {
    match c.schedule {
        Schedule::InRelease(phrases) => Mismatch { path: format!("{} releases", c.ticker), why: format!("none states its schedule ({})", phrases.iter().map(|(p, _)| format!("{p:?}")).collect::<Vec<_>>().join(", ")) },
        Schedule::OnPage { url, phrase, .. } => Mismatch { path: url.to_string(), why: format!("the page no longer states {phrase:?}") },
    }
}

/// Whether a release's title is one of the company's declarations.
pub fn declares(c: &Company, title: &str) -> bool {
    let t = title.to_lowercase();
    t.starts_with(&c.title_starts.to_lowercase()) && c.title_has.iter().all(|p| t.contains(&p.to_lowercase())) && t.trim_end().ends_with(&c.title_ends.to_lowercase())
}

pub struct Companies;

impl Payer for Companies {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        newswire::HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &[]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn serves(&self, need: &PayerNeed) -> bool {
        company_of(need).is_some()
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let Some(c) = company_of(need) else {
            return Noted { outcome: Outcome::NotCarried(format!("{} is not a company this reader knows", need.listing.symbol)), shape_change: None };
        };
        let get = |url: &str| -> Result<String, Outcome<()>> {
            match ask::send(net, &Ask::get(url, &[]), &[]) {
                Outcome::Answered(r) => ask::text(&r.body).map(str::to_string).map_err(Outcome::Mismatch),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let mismatch = |m: Mismatch| Noted { outcome: Outcome::Mismatch(m), shape_change: None };
        let page = match get(&format!("https://{}/news/{}/", newswire::HOST, c.organization)) {
            Ok(p) => p,
            Err(o) => return fail(o),
        };
        let listed = match newswire::listed(&page) {
            Ok(l) => l,
            Err(m) => return mismatch(m),
        };
        let mut rows = Vec::new();
        let mut per_year = None;
        for l in listed.iter().filter(|l| declares(c, &l.title)) {
            let html = match get(&format!("https://{}{}", newswire::HOST, l.path)) {
                Ok(h) => h,
                Err(o) => return fail(o),
            };
            let release = match newswire::release(&html) {
                Ok(r) => r,
                Err(m) => return mismatch(m),
            };
            match declared(c, &release) {
                Ok(Some(d)) => rows.push(d),
                // a declaration from before T+1: its ex-date is not known here
                Ok(None) => {}
                Err(m) => return mismatch(m),
            }
            if per_year.is_none() {
                per_year = schedule_in(c, &release);
            }
        }
        if rows.is_empty() {
            return Noted { outcome: Outcome::NotCarried(format!("no release of {} declares a dividend", c.ticker)), shape_change: None };
        }
        if let Schedule::OnPage { url, .. } = c.schedule {
            per_year = match get(url) {
                Ok(h) => schedule_on_page(c, &h),
                Err(o) => return fail(o),
            };
        }
        // each company here states its schedule, in its releases or on its page:
        // a statement gone is a change of its wording, not a schedule unstated
        let Some(per_year) = per_year else { return mismatch(schedule_missing(c)) };
        Noted { outcome: Outcome::Answered(Record { rows, per_year: Some(per_year) }), shape_change: None }
    }
}
