//! The Bank of Canada's holiday schedule (`docs/plans/stage-3a-sources.md`,
//! "Holidays"): the page `/press/upcoming-events/bank-of-canada-holiday-schedule/`
//! lists the closures still to come, each an article with its date ("September
//! 30, 2026") and its name, so a closure is known before its day passes. Past
//! closures need no page: a completed read of the rates that skipped them says so.
//!
//! Every article's date and name are read; a date on a weekend is kept as the page
//! states it. An article missing either, a date that does not read, or a page
//! with no articles at all is a mismatch, never an empty schedule.

use bagholder_core::jiff::civil::Date;
use bagholder_core::SourceName;
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::Mismatch;

pub const SOURCE: &str = "bank-of-canada-holidays";
pub const HOST: &str = "www.bankofcanada.ca";
const PAGE: &str = "https://www.bankofcanada.ca/press/upcoming-events/bank-of-canada-holiday-schedule/";

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// "September 30, 2026" as a day.
fn long_date(text: &str) -> Option<Date> {
    let (month, rest) = text.trim().split_once(' ')?;
    let (d, y) = rest.split_once(", ")?;
    let m = MONTHS.iter().position(|n| *n == month)? as i8 + 1;
    Date::new(y.trim().parse().ok()?, m, d.trim().parse().ok()?).ok()
}

/// The text of the element whose opening tag holds `open`: from the tag's end to the next `<`.
fn text_after<'a>(s: &'a str, open: &str) -> Option<&'a str> {
    let i = s.find(open)? + open.len();
    let tail = &s[i..];
    let tail = &tail[tail.find('>')? + 1..];
    Some(&tail[..tail.find('<')?])
}

fn unescape(s: &str) -> String {
    s.trim().replace("&amp;", "&").replace("&#8217;", "\u{2019}").replace("&#039;", "'").replace("&nbsp;", " ")
}

/// The closures the page lists: each date and its name.
pub fn parse(html: &str) -> Result<Vec<(Date, String)>, Mismatch> {
    let mismatch = |why: String| Mismatch { path: "article.media".into(), why };
    let mut out = Vec::new();
    for (n, article) in html.split("<article class=\"media\"").skip(1).enumerate() {
        let article = article.split("</article>").next().unwrap_or("");
        let date_text = text_after(article, "media-date").ok_or_else(|| mismatch(format!("article {n} has no date")))?;
        let day = long_date(&unescape(date_text)).ok_or_else(|| mismatch(format!("article {n}'s date {date_text:?} is not a day")))?;
        let heading = &article[article.find("media-heading").ok_or_else(|| mismatch(format!("article {n} has no heading")))?..];
        let name = text_after(heading, "<a ").map(unescape).filter(|s| !s.is_empty()).ok_or_else(|| mismatch(format!("article {n} has no name")))?;
        out.push((day, name));
    }
    if out.is_empty() {
        return Err(mismatch("the page lists no closures".into()));
    }
    Ok(out)
}

/// Whether the closures read are a current schedule on `today`: the page lists
/// the closures still to come, so a page none of whose closures is on or after
/// the day it is read is not current (last year's), and is a meaning failure.
pub fn current(closures: &[(Date, String)], today: Date) -> Result<(), String> {
    if closures.iter().any(|(d, _)| *d >= today) {
        return Ok(());
    }
    let last = closures.iter().map(|c| c.0).max().map_or(String::new(), |d| d.to_string());
    Err(format!("the holiday page lists no closure on or after {today} (its last is {last}): it is not current"))
}

/// Read the page on `today`, the Bank's day.
pub fn ask(net: &Net, today: Date) -> Noted<Vec<(Date, String)>> {
    let reply = match ask::send(net, &Ask::get(PAGE, &[]), &[]) {
        Outcome::Answered(r) => r,
        other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
    };
    let outcome = match ask::text(&reply.body).and_then(parse) {
        Ok(h) => match current(&h, today) {
            Ok(()) => Outcome::Answered(h),
            Err(why) => Outcome::Meaning(why),
        },
        Err(m) => Outcome::Mismatch(m),
    };
    Noted { outcome, shape_change: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_date_reads_as_its_day() {
        assert_eq!(long_date("December 28, 2026"), Some(bagholder_core::jiff::civil::date(2026, 12, 28)));
        assert_eq!(long_date("Smarch 1, 2026"), None);
        assert_eq!(long_date("February 30, 2026"), None);
    }
}
