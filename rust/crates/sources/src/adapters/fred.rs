//! FRED's S&P 500 series (`fredgraph.csv?id=SP500`): the index's daily level for
//! the trailing ten years, two decimals as written (research 5). A day FRED lists
//! with no value (a US market holiday) is the source stating no level for it.

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{day_from, Mismatch};

pub const SOURCE: &str = "fred";
pub const HOST: &str = "fred.stlouisfed.org";
const URL: &str = "https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500";
const HEADER: &str = "observation_date,SP500";
/// FRED never answers a request whose User-Agent is missing, a bare name it does
/// not know (`Bagholder/2.0`) or a browser's; it answers a known tool's
/// (`curl/8`) and a name with a contact URL, [`ask::USER_AGENT`] (observed
/// 2026-09-24 with curl over HTTP/1.1: the others hang until the timeout).
pub const HEADERS: [(&str, &str); 1] = [("User-Agent", ask::USER_AGENT)];

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

/// Every day FRED states a level for, oldest first.
pub fn parse(text: &str) -> Outcome<Vec<(Date, Dec)>> {
    let mut lines = text.lines();
    match lines.next().map(str::trim) {
        Some(HEADER) => {}
        other => return Outcome::Mismatch(Mismatch { path: "header".into(), why: format!("expected {HEADER:?}, found {other:?}") }),
    }
    let mut out: Vec<(Date, Dec)> = Vec::new();
    let mut last: Option<Date> = None;
    for (n, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let path = format!("row {}", n + 1);
        let Some((d, v)) = line.split_once(',') else {
            return Outcome::Mismatch(Mismatch { path, why: format!("{line:?} is not a day and a level") });
        };
        let d = match day_from(d) {
            Ok(d) => d,
            Err(why) => return Outcome::Mismatch(Mismatch { path, why }),
        };
        if last.is_some_and(|l| d <= l) {
            return Outcome::Meaning(format!("FRED lists {d} after {}", last.expect("checked")));
        }
        last = Some(d);
        if v.is_empty() {
            continue;
        }
        let level = match Dec::parse(v) {
            Ok(l) => l,
            Err(e) => return Outcome::Mismatch(Mismatch { path, why: format!("{v:?}: {e}") }),
        };
        if level <= Dec::ZERO {
            return Outcome::Meaning(format!("FRED states {level} for {d}"));
        }
        out.push((d, level));
    }
    if out.is_empty() {
        return Outcome::Meaning("FRED states no level at all".into());
    }
    Outcome::Answered(out)
}

pub fn ask(net: &Net) -> Noted<Vec<(Date, Dec)>> {
    let reply = match ask::send(net, &Ask::get(URL, &HEADERS), &[]) {
        Outcome::Answered(r) => r,
        other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
    };
    let outcome = match ask::text(&reply.body) {
        Ok(t) => parse(t),
        Err(m) => Outcome::Mismatch(m),
    };
    Noted { outcome, shape_change: None }
}
