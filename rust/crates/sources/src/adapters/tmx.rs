//! TMX Money's GraphQL service (`app-money.tmx.com/graphql`): the quote of a
//! Canadian listing, the S&P/TSX Composite and S&P/TSX 60 daily levels, and the
//! exchange-side record of distributions for a fund whose company's own
//! publication cannot be read (Mackenzie's QCN and QUU; the owner's exception of
//! 2026-09-24).
//!
//! The service wants its own site's origin and a locale. An unknown symbol
//! answers with an error naming code 404 and no data: not carried. A form of the
//! symbol answered on another venue than the one asked is another listing, and is
//! refused by the caller against the venue the book names.
//!
//! **Distributions.** TMX states each distribution's ex-date and amount, and its
//! record, pay and declaration dates where it has them. It does not say whether a
//! distribution was paid in cash or in units. Its rows with no pay date are, for
//! Mackenzie's funds, the year-end distributions paid in units: four of the five
//! QCN lists (2018, 2021, 2022, 2023) equal the difference between Mackenzie's own
//! yearly tax totals and its cash rows; the fifth (2021-03-22) is a cash payment
//! TMX lists without any of its dates. A cash payment has a pay date, so a row
//! with one is read as cash, and a row with none as paying no cash (its amount
//! kept as the part paid in units). Only the latest distribution gone ex feeds a
//! figure, and a year-end distribution in units is exactly the row that would
//! otherwise stand as the latest for three months.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{day_from, Mismatch, Node, RecordedShape};

pub const SOURCE: &str = "tmx";
pub const HOST: &str = "app-money.tmx.com";
const URL: &str = "https://app-money.tmx.com/graphql";
const HEADERS: [(&str, &str); 5] = [("Content-Type", "application/json"), ("locale", "en"), ("Origin", "https://money.tmx.com"), ("Referer", "https://money.tmx.com/"), ("User-Agent", "Mozilla/5.0")];

const QUOTE: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName exchangeCode price priceChange percentChange prevClose currency datetime dividendFrequency } }";
const DIVIDENDS: &str = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate recordDate payableDate declarationDate amount currency } } }";
const SERIES: &str = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime close } }";
/// Rows per page of distributions; a full page means another is asked.
const BATCH: usize = 100;

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn quote_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-quote.paths"))
}

pub fn dividends_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-dividends.paths"))
}

pub fn series_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-series.paths"))
}

/// A listing's quote as TMX states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmxQuote {
    pub symbol: String,
    /// The venue the answer is for, as TMX names it.
    pub exchange_name: String,
    pub price: Dec,
    pub change: Option<Dec>,
    pub change_pct: Option<Dec>,
    pub currency: Currency,
    pub datetime: Timestamp,
    /// How often the listing pays, where TMX states it.
    pub per_year: Option<u32>,
}

/// A distribution as TMX lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TmxDistribution {
    pub ex_date: Date,
    pub record_date: Option<Date>,
    pub pay_date: Option<Date>,
    /// The cash paid per unit: the amount of a row with a pay date, else none.
    pub cash: Dec,
    /// The amount of a row with no pay date: paid in units.
    pub in_units: Option<Dec>,
    pub currency: Currency,
}

/// TMX's words for how often a listing pays, and how many times a year each is.
/// A word not here is a mismatch naming it.
pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Weekly" => 52,
        "Semi-Monthly" | "Bi-Monthly" => 24,
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annual" | "Semi-Annually" => 2,
        "Annual" | "Annually" => 1,
        _ => return None,
    })
}

fn escaped(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A GraphQL request body: the operation, its variables (already JSON) and its query.
fn body(operation: &str, variables: &str, query: &str) -> Vec<u8> {
    format!("{{\"operationName\":{},\"variables\":{variables},\"query\":{}}}", escaped(operation), escaped(query)).into_bytes()
}

/// The `errors` of a reply that answers "not found" for the symbol.
fn not_found(root: &Node) -> bool {
    root.list("errors").is_ok_and(|errs| errs.iter().any(|e| e.text("code").is_ok_and(|c| c == "404")))
}

fn post(net: &Net, body: &[u8]) -> Result<Value, Outcome<()>> {
    let reply = match ask::send(net, &Ask::post(URL, &HEADERS, body), &[]) {
        Outcome::Answered(r) => r,
        other => return Err(other.failed().expect("not answered")),
    };
    ask::json(&reply.body).map_err(Outcome::Mismatch)
}

/// Read a quote reply for `form`.
pub fn parse_quote(v: &Value, form: &str) -> Outcome<TmxQuote> {
    let root = Node::root(v);
    if not_found(&root) {
        return Outcome::NotCarried(format!("TMX does not know {form}"));
    }
    match read_quote(&root, form) {
        Ok(Ok(q)) => Outcome::Answered(q),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_quote(root: &Node, form: &str) -> Result<Result<TmxQuote, String>, Mismatch> {
    let q = root.obj("data")?.obj("getQuoteBySymbol")?;
    let symbol = q.text("symbol")?;
    let bare = form.split(':').next().unwrap_or(form);
    if !symbol.eq_ignore_ascii_case(bare) {
        return Ok(Err(format!("TMX answered {symbol} for {form}")));
    }
    let price = q.dec("price")?;
    if price <= Dec::ZERO {
        return Ok(Err(format!("{form}'s price is {price}")));
    }
    let currency = match Currency::parse(q.text("currency")?) {
        Ok(c) => c,
        Err(e) => return Ok(Err(format!("{form}'s currency: {e}"))),
    };
    let datetime_text = q.text("datetime")?;
    let Ok(datetime) = datetime_text.parse::<Timestamp>() else {
        return Ok(Err(format!("{form}'s time {datetime_text:?} is not an instant")));
    };
    let per_year = match q.opt_text("dividendFrequency")? {
        None => None,
        Some(w) => match self::per_year(w) {
            Some(n) => Some(n),
            None => return Err(q.field("dividendFrequency")?.mismatch(format!("{w:?} is not a schedule this reader knows"))),
        },
    };
    Ok(Ok(TmxQuote {
        symbol: symbol.to_string(),
        exchange_name: q.text("exchangeName")?.to_string(),
        price,
        change: q.opt_dec("priceChange")?,
        change_pct: q.opt_dec("percentChange")?,
        currency,
        datetime,
        per_year,
    }))
}

/// Read one page of a distributions reply.
pub fn parse_dividends(v: &Value, form: &str) -> Outcome<Vec<TmxDistribution>> {
    match read_dividends(v, form) {
        Ok(Ok(d)) => Outcome::Answered(d),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_dividends(v: &Value, form: &str) -> Result<Result<Vec<TmxDistribution>, String>, Mismatch> {
    let rows = Node::root(v).obj("data")?.obj("dividends")?.list("dividends")?;
    let mut out = Vec::new();
    for r in rows {
        let ex_date = r.day("exDate")?;
        let record_date = r.opt_day("recordDate")?;
        let pay_date = r.opt_day("payableDate")?;
        // declared or not, the date is not what a figure reads; it must still be a day
        r.opt_day("declarationDate")?;
        let amount = r.dec("amount")?;
        if amount < Dec::ZERO {
            return Ok(Err(format!("{form} lists a distribution of {amount} going ex {ex_date}")));
        }
        if pay_date.is_some_and(|p| p < ex_date) {
            return Ok(Err(format!("{form} lists a distribution paid {} before it goes ex {ex_date}", pay_date.expect("checked"))));
        }
        let currency = match Currency::parse(r.text("currency")?) {
            Ok(c) => c,
            Err(e) => return Ok(Err(format!("{form}'s distribution currency: {e}"))),
        };
        let (cash, in_units) = match pay_date {
            Some(_) => (amount, None),
            None if amount.is_zero() => (Dec::ZERO, None),
            None => (Dec::ZERO, Some(amount)),
        };
        out.push(TmxDistribution { ex_date, record_date, pay_date, cash, in_units, currency });
    }
    Ok(Ok(out))
}

/// Read a daily series reply: each session's close, oldest first.
pub fn parse_series(v: &Value, symbol: &str, span: (Date, Date)) -> Outcome<Vec<(Date, Dec)>> {
    match read_series(v, symbol, span) {
        Ok(Ok(d)) => Outcome::Answered(d),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_series(v: &Value, symbol: &str, span: (Date, Date)) -> Result<Result<Vec<(Date, Dec)>, String>, Mismatch> {
    let rows = Node::root(v).obj("data")?.list("getTimeSeriesData")?;
    let mut out: Vec<(Date, Dec)> = Vec::new();
    for r in rows {
        let text = r.text("dateTime")?;
        // the session's day is the day the row is stamped in its own offset
        let d = match text.get(..10).map(day_from) {
            Some(Ok(d)) => d,
            _ => return Err(r.field("dateTime")?.mismatch(format!("{text:?} does not begin with a day"))),
        };
        if d < span.0 || d > span.1 {
            return Ok(Err(format!("{symbol} answered {d}, outside the {} to {} asked", span.0, span.1)));
        }
        // newest first
        if out.last().is_some_and(|(l, _)| d >= *l) {
            return Ok(Err(format!("{symbol}'s series repeats or reorders {d}")));
        }
        let close = r.dec("close")?;
        if close <= Dec::ZERO {
            return Ok(Err(format!("{symbol}'s level on {d} is {close}")));
        }
        out.push((d, close));
    }
    out.reverse();
    Ok(Ok(out))
}

/// Ask for `form`'s quote.
pub fn ask_quote(net: &Net, form: &str) -> Noted<TmxQuote> {
    let b = body("getQuoteBySymbol", &format!("{{\"symbol\":{},\"locale\":\"en\"}}", escaped(form)), QUOTE);
    match post(net, &b) {
        Ok(v) => Noted { outcome: parse_quote(&v, form), shape_change: ask::noticed(&v, &quote_shape(), &[]) },
        Err(o) => Noted { outcome: o.failed().unwrap_or(Outcome::Unreachable("no reply".into())), shape_change: None },
    }
}

/// Ask for every distribution TMX lists for `form`, page by page.
pub fn ask_dividends(net: &Net, form: &str) -> Noted<Vec<TmxDistribution>> {
    let mut all = Vec::new();
    let mut shape_change = None;
    for page in 1.. {
        let b = body("getDividendsForSymbol", &format!("{{\"symbol\":{},\"page\":{page},\"batch\":{BATCH}}}", escaped(form)), DIVIDENDS);
        let v = match post(net, &b) {
            Ok(v) => v,
            Err(o) => return Noted { outcome: o.failed().unwrap_or(Outcome::Unreachable("no reply".into())), shape_change },
        };
        shape_change = shape_change.or(ask::noticed(&v, &dividends_shape(), &[]));
        match parse_dividends(&v, form) {
            Outcome::Answered(rows) => {
                let full = rows.len() == BATCH;
                all.extend(rows);
                if !full {
                    break;
                }
            }
            other => return Noted { outcome: other, shape_change },
        }
    }
    Noted { outcome: Outcome::Answered(all), shape_change }
}

/// Ask for an index's daily levels from `from` through `to`.
pub fn ask_series(net: &Net, symbol: &str, from: Date, to: Date) -> Noted<Vec<(Date, Dec)>> {
    let b = body("getTimeSeriesData", &format!("{{\"symbol\":{},\"freq\":\"day\",\"interval\":1,\"start\":\"{from}\",\"end\":\"{to}\"}}", escaped(symbol)), SERIES);
    match post(net, &b) {
        Ok(v) => Noted { outcome: parse_series(&v, symbol, (from, to)), shape_change: ask::noticed(&v, &series_shape(), &[]) },
        Err(o) => Noted { outcome: o.failed().unwrap_or(Outcome::Unreachable("no reply".into())), shape_change: None },
    }
}
