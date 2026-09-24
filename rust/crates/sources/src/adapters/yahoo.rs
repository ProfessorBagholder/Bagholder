//! Yahoo's chart (`query1.finance.yahoo.com/v8/finance/chart/<symbol>`): daily
//! closes of listings, the closes, dividends and splits of each benchmark's
//! tracker, the dividend events of a US fund whose company's publication cannot
//! be read, and a US listing's quote (`docs/plans/stage-3a-sources.md`, research
//! 1; `docs/plans/stage-3a-brief-06.md`).
//!
//! Its closes are adjusted for later splits, and the same reply states those
//! splits when asked with `events=split`, so each close is stored as traded:
//! multiplied back by every split after its day (`numerator / denominator`, 10:1
//! is ×10). Its dividend amounts are adjusted the same way and are undone the
//! same way. A close is written as Yahoo writes it, binary leftovers and all
//! (`113.9010009765625`); a day it states no close for (null) is a day without
//! one. A bar is a closed session only once that session has ended: before the
//! day of the reply's current trading period, or that day after its regular end.
//! The quote is the reply's regular market price at its `regularMarketTime`.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, Rounding, SourceName};
use bagholder_net::{Ask, Net, Pace};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Keyed, Mismatch, Node, RecordedShape};

pub const SOURCE: &str = "yahoo";
pub const HOST: &str = "query1.finance.yahoo.com";
const CHART: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
const HEADERS: [(&str, &str); 2] = [("User-Agent", "Mozilla/5.0"), ("Accept", "application/json")];
/// Yahoo refuses bursts: one request every two seconds, and after a refusal
/// nothing for ten minutes.
pub const PACE: Pace = Pace { gap: std::time::Duration::from_secs(2), rest: std::time::Duration::from_secs(600) };
/// An unknown symbol is a 404.
const NOT_CARRIED: &[u16] = &[404];

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/yahoo-chart.paths"))
}

/// The chart's events are keyed by their instants.
pub const KEYED: &[Keyed] = &[Keyed { parent: "chart.result[].events.dividends", fields: &[] }, Keyed { parent: "chart.result[].events.splits", fields: &[] }];

/// A split the reply states, on the day it took effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub day: Date,
    pub numerator: Dec,
    pub denominator: Dec,
}

/// A US listing's quote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YahooQuote {
    pub price: Dec,
    pub at: Timestamp,
    /// The day's change in percent, as stated.
    pub change_pct: Option<Dec>,
}

/// What one chart reply states, read strictly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chart {
    pub symbol: String,
    pub currency: Currency,
    /// Each closed session's close, as traded, oldest first.
    pub closes: Vec<(Date, Dec)>,
    /// Each dividend by its ex-date, as declared (the split adjustment undone).
    pub dividends: Vec<(Date, Dec)>,
    pub splits: Vec<Split>,
    pub quote: YahooQuote,
}

fn instant(n: i64, path: &str) -> Result<Timestamp, Mismatch> {
    Timestamp::from_second(n).map_err(|_| Mismatch { path: path.into(), why: format!("{n} is not an instant") })
}

/// Multiply `v` back by every split after `day`.
fn as_traded(v: Dec, day: Date, splits: &[Split]) -> Result<Dec, String> {
    let mut out = v;
    for s in splits.iter().filter(|s| s.day > day) {
        out = out.checked_mul(s.numerator).map_err(|e| format!("{v} × {}: {e}", s.numerator))?;
        if s.denominator != Dec::ONE {
            // exact where the denominator divides it (a 1:5 consolidation); a
            // ratio like 3:2 leaves a repeating decimal, taken to twelve places
            out = out.div_rounded(s.denominator, 12, Rounding::HalfEven).map_err(|e| format!("{v} ÷ {}: {e}", s.denominator))?;
        }
    }
    Ok(out)
}

/// Read one chart reply for `symbol` at `now`.
pub fn parse(v: &Value, symbol: &str, now: Timestamp) -> Outcome<Chart> {
    parse_after(v, symbol, now, &[])
}

/// Read one chart reply, undoing also `later`: the splits after the span it
/// covers. Yahoo adjusts every close and dividend for every split up to now, but
/// a reply lists only the splits inside its own span.
pub fn parse_after(v: &Value, symbol: &str, now: Timestamp, later: &[Split]) -> Outcome<Chart> {
    match read(v, symbol, now, later) {
        Ok(Ok(c)) => Outcome::Answered(c),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read(v: &Value, symbol: &str, now: Timestamp, later: &[Split]) -> Result<Result<Chart, String>, Mismatch> {
    let chart = Node::root(v).obj("chart")?;
    let results = chart.list("result")?;
    let [r] = results.as_slice() else {
        return Ok(Err(format!("the chart holds {} results, not one", results.len())));
    };
    let meta = r.obj("meta")?;
    let answered = meta.text("symbol")?;
    if !answered.eq_ignore_ascii_case(symbol) {
        return Ok(Err(format!("the chart is {answered}'s, not {symbol}'s")));
    }
    let currency = match Currency::parse(meta.text("currency")?) {
        Ok(c) => c,
        Err(e) => return Ok(Err(format!("{symbol}'s currency: {e}"))),
    };
    let zone_name = meta.text("exchangeTimezoneName")?;
    let Ok(zone) = TimeZone::get(zone_name) else {
        return Ok(Err(format!("{symbol}'s exchange zone {zone_name} is not a zone")));
    };
    let day_of = |t: Timestamp| t.to_zoned(zone.clone()).date();
    // the session in progress or next: its day, and when its regular hours end
    let regular = meta.obj("currentTradingPeriod")?.obj("regular")?;
    let (period_start, period_end) = (instant(regular.int("start")?, "currentTradingPeriod.regular.start")?, instant(regular.int("end")?, "currentTradingPeriod.regular.end")?);
    let period_day = day_of(period_start);
    let closed = |d: Date| d < period_day || (d == period_day && now >= period_end);

    let price = meta.dec("regularMarketPrice")?;
    let at = instant(meta.int("regularMarketTime")?, "meta.regularMarketTime")?;
    let change_pct = match meta.field("regularMarketChangePercent") {
        Ok(n) => Some(n.as_dec()?),
        Err(_) => None,
    };
    if price <= Dec::ZERO {
        return Ok(Err(format!("{symbol}'s price is {price}")));
    }

    // the events: splits first, since they undo the rest
    let mut splits = Vec::new();
    let mut dividends = Vec::new();
    if let Ok(events) = r.obj("events") {
        if let Ok(s) = events.obj("splits") {
            for k in s.keys()? {
                let e = s.obj(k)?;
                let (n, d) = (e.dec("numerator")?, e.dec("denominator")?);
                if n <= Dec::ZERO || d <= Dec::ZERO {
                    return Ok(Err(format!("{symbol} states a split of {n}:{d}")));
                }
                splits.push(Split { day: day_of(instant(e.int("date")?, "events.splits.date")?), numerator: n, denominator: d });
            }
        }
        if let Ok(ds) = events.obj("dividends") {
            for k in ds.keys()? {
                let e = ds.obj(k)?;
                dividends.push((day_of(instant(e.int("date")?, "events.dividends.date")?), e.dec("amount")?));
            }
        }
    }
    for s in later {
        if !splits.iter().any(|x: &Split| x.day == s.day) {
            splits.push(*s);
        }
    }
    splits.sort_by_key(|s| s.day);
    let mut declared = Vec::new();
    for (d, amount) in dividends {
        if amount <= Dec::ZERO {
            return Ok(Err(format!("{symbol} states a dividend of {amount} on {d}")));
        }
        match as_traded(amount, d, &splits) {
            Ok(a) => declared.push((d, a)),
            Err(why) => return Ok(Err(why)),
        }
    }
    declared.sort_by_key(|(d, _)| *d);

    // a chart with no bars (a span with no session) carries no timestamp list
    let mut closes = Vec::new();
    if let Ok(stamps) = r.field("timestamp") {
        let stamps = stamps.as_list()?;
        let quote = r.obj("indicators")?.list("quote")?;
        let [q] = quote.as_slice() else {
            return Ok(Err(format!("{symbol}'s chart holds {} quote series", quote.len())));
        };
        let values = q.list("close")?;
        if values.len() != stamps.len() {
            return Ok(Err(format!("{symbol}'s chart has {} times and {} closes", stamps.len(), values.len())));
        }
        let mut last: Option<Date> = None;
        for (t, c) in stamps.iter().zip(&values) {
            let d = day_of(instant(t.as_int()?, t.path())?);
            if last.is_some_and(|l| d <= l) {
                return Ok(Err(format!("{symbol}'s chart repeats or reorders {d}")));
            }
            last = Some(d);
            // null: the source states no close that day
            if matches!(c.value(), Value::Null) || !closed(d) {
                continue;
            }
            let close = c.as_dec()?;
            if close <= Dec::ZERO {
                return Ok(Err(format!("{symbol}'s close on {d} is {close}")));
            }
            match as_traded(close, d, &splits) {
                Ok(t) => closes.push((d, t)),
                Err(why) => return Ok(Err(why)),
            }
        }
    }
    Ok(Ok(Chart { symbol: answered.to_string(), currency, closes, dividends: declared, splits, quote: YahooQuote { price, at, change_pct } }))
}

/// Epoch seconds at the start of `d` in UTC: the chart's `period1`/`period2`.
fn epoch(d: Date) -> i64 {
    d.to_zoned(TimeZone::UTC).map(|z| z.timestamp().as_second()).unwrap_or(0)
}

fn percent_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Ask for `symbol`'s chart from `from` through `to`, with its splits and
/// dividends. A span that ends before today is adjusted for the splits after it
/// too, which its reply does not list: those are asked first, as the monthly
/// chart from the day after the span to today, which lists them.
pub fn ask_span(net: &Net, symbol: &str, from: Date, to: Date, now: Timestamp) -> Noted<Chart> {
    let end = to.tomorrow().unwrap_or(to);
    let today = now.to_zoned(TimeZone::UTC).date();
    let mut later = Vec::new();
    if end < today {
        let url = format!("{CHART}{}?period1={}&period2={}&interval=1mo&events=split", percent_encode(symbol), epoch(end), epoch(today.tomorrow().unwrap_or(today)));
        let noted = ask_url(net, &url, symbol, now);
        match noted.outcome {
            Outcome::Answered(c) => later = c.splits,
            // without the later splits the span's closes cannot be undone
            other => return Noted { outcome: other, shape_change: noted.shape_change },
        }
    }
    let url = format!("{CHART}{}?period1={}&period2={}&interval=1d&events=div%7Csplit", percent_encode(symbol), epoch(from), epoch(end));
    ask_url_after(net, &url, symbol, now, &later)
}

/// Ask for `symbol`'s quote: the chart of its latest session.
pub fn ask_quote(net: &Net, symbol: &str, now: Timestamp) -> Noted<Chart> {
    let url = format!("{CHART}{}?range=1d&interval=1d", percent_encode(symbol));
    ask_url(net, &url, symbol, now)
}

fn ask_url(net: &Net, url: &str, symbol: &str, now: Timestamp) -> Noted<Chart> {
    ask_url_after(net, url, symbol, now, &[])
}

fn ask_url_after(net: &Net, url: &str, symbol: &str, now: Timestamp, later: &[Split]) -> Noted<Chart> {
    let reply = match ask::send(net, &Ask::get(url, &HEADERS), NOT_CARRIED) {
        Outcome::Answered(r) => r,
        other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
    };
    Noted { outcome: parse_after(&v, symbol, now, later), shape_change: ask::noticed(&v, &shape(), KEYED) }
}
