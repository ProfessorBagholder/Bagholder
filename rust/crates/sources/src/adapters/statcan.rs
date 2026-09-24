//! Statistics Canada's archive of the Bank of Canada's noon spot rate, table
//! 10-10-0008 (`docs/plans/stage-3a-sources.md`, research 3): the only source of
//! the Bank's published rate before 2007-05-01, for the twelve currencies still
//! in use it holds. The archive ended on 2017-04-28 and never changes; it stands
//! only until the Bank's own noon series begins, so its era ends 2007-04-30.
//!
//! Its Web Data Service writes each day of a vector: a rate on a business day; on
//! a weekend a value of 0 (status 0), and on a holiday no value (status 1), both
//! the archive stating no rate for that day. Anything else (a zero on a weekday,
//! a rate on a weekend, another status) is a meaning failure. Values are written
//! with up to eight decimals (`2.96799999`) and kept exactly as written.

use bagholder_core::jiff::civil::{date, Date, Weekday};
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Mismatch, Node, Shape};

pub const SOURCE: &str = "statistics-canada";
pub const HOST: &str = "www150.statcan.gc.ca";
const WDS: &str = "https://www150.statcan.gc.ca/t1/wds/rest/getDataFromVectorByReferencePeriodRange";

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn shape() -> Shape {
    ask::recorded_shape(include_str!("../../shapes/statistics-canada.paths"))
}

/// One currency's noon spot series in table 10-10-0008.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveSeries {
    pub currency: &'static str,
    pub vector: u32,
    /// The table's coordinate for the series (its member in "Type of currency").
    pub coordinate: &'static str,
    pub first: Date,
}

const fn arch(currency: &'static str, vector: u32, coordinate: &'static str, first: Date) -> ArchiveSeries {
    ArchiveSeries { currency, vector, coordinate, first }
}

/// The twelve, from the table's own metadata and series (2026-09-24).
pub const ARCHIVE: [ArchiveSeries; 12] = [
    arch("USD", 121716, "1.1.0.0.0.0.0.0.0.0", date(1950, 10, 2)),
    arch("NOK", 121717, "1.10.0.0.0.0.0.0.0.0", date(1950, 10, 2)),
    arch("SEK", 121718, "1.11.0.0.0.0.0.0.0.0", date(1950, 10, 2)),
    arch("CHF", 121719, "1.12.0.0.0.0.0.0.0.0", date(1950, 10, 2)),
    arch("GBP", 121720, "1.13.0.0.0.0.0.0.0.0", date(1950, 10, 2)),
    arch("AUD", 121729, "1.21.0.0.0.0.0.0.0.0", date(1979, 1, 2)),
    arch("HKD", 121732, "1.24.0.0.0.0.0.0.0.0", date(1986, 1, 2)),
    arch("NZD", 121733, "1.25.0.0.0.0.0.0.0.0", date(1986, 1, 2)),
    arch("MXN", 121739, "1.30.0.0.0.0.0.0.0.0", date(1993, 1, 4)),
    arch("EUR", 121742, "1.33.0.0.0.0.0.0.0.0", date(1999, 1, 4)),
    arch("DKK", 121743, "1.4.0.0.0.0.0.0.0.0", date(1950, 10, 2)),
    arch("JPY", 121747, "1.8.0.0.0.0.0.0.0.0", date(1952, 1, 14)),
];

/// The last day of the archive's era: the day before the Bank's noon series.
pub const ERA_END: Date = date(2007, 4, 30);

pub fn series_of(c: Currency) -> Option<ArchiveSeries> {
    ARCHIVE.iter().copied().find(|s| s.currency == c.as_str())
}

fn is_weekend(d: Date) -> bool {
    matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// The rates one vector's reply states over the span asked, read strictly.
pub fn parse(v: &Value, series: &ArchiveSeries, span: (Date, Date)) -> Outcome<Vec<(Date, Dec)>> {
    match read(v, series, span) {
        Ok(Ok(r)) => Outcome::Answered(r),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read(v: &Value, series: &ArchiveSeries, span: (Date, Date)) -> Result<Result<Vec<(Date, Dec)>, String>, Mismatch> {
    let items = Node::root(v).as_list()?;
    let [item] = items.as_slice() else {
        return Ok(Err(format!("the reply holds {} series, not the one asked", items.len())));
    };
    let status = item.text("status")?;
    if status != "SUCCESS" {
        return Ok(Err(format!("the service answered {status}")));
    }
    let o = item.obj("object")?;
    let vector = o.int("vectorId")?;
    let coordinate = o.text("coordinate")?;
    if vector != i64::from(series.vector) || coordinate != series.coordinate {
        return Ok(Err(format!("the reply is vector {vector} at {coordinate}, not {} at {}", series.vector, series.coordinate)));
    }
    let mut out: Vec<(Date, Dec)> = Vec::new();
    let mut last: Option<Date> = None;
    for p in o.list("vectorDataPoint")? {
        let d = p.day("refPer")?;
        let code = p.int("statusCode")?;
        let value = p.opt_dec("value")?;
        if d < span.0 || d > span.1 {
            return Ok(Err(format!("vector {} answered {d}, outside the {} to {} asked", series.vector, span.0, span.1)));
        }
        if last.is_some_and(|l| d <= l) {
            return Ok(Err(format!("vector {} answered {d} out of order", series.vector)));
        }
        last = Some(d);
        match (value, code) {
            // a holiday: no value, status 1
            (None, 1) => {}
            // a weekend: stated as 0
            (Some(r), 0) if r == Dec::ZERO && is_weekend(d) => {}
            (Some(r), 0) if r > Dec::ZERO && !is_weekend(d) => out.push((d, r)),
            (value, code) => return Ok(Err(format!("vector {} states {value:?} with status {code} for {d}, which is neither a rate nor a day without one", series.vector))),
        }
    }
    Ok(Ok(out))
}

/// Ask for one vector's days over `span`.
pub fn ask(net: &Net, series: &ArchiveSeries, span: (Date, Date)) -> Noted<Vec<(Date, Dec)>> {
    let url = format!("{WDS}?vectorIds=%22{}%22&startRefPeriod={}&endReferencePeriod={}", series.vector, span.0, span.1);
    let reply = match ask::send(net, &Ask::get(&url, &[]), &[]) {
        Outcome::Answered(r) => r,
        other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
    };
    // the service answers a burst with an empty body: a refusal, not an answer
    if reply.body.is_empty() {
        net.limiter().refused(HOST, reply.received_at, None);
        return Noted { outcome: Outcome::Refused { status: Some(reply.status), retry_after: None }, shape_change: None };
    }
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
    };
    Noted { outcome: parse(&v, series, span), shape_change: ask::noticed(&v, &shape(), &[]) }
}
