//! Coinbase: a coin's quote and its daily closes (`SPEC.md` §2, Coinbase and
//! Coinbase Exchange; `docs/plans/stage-3a-sources.md`, "Every quote has a time").
//!
//! - **The spot price** (`api.coinbase.com/v2/prices/<base>-<currency>/spot`) in
//!   the coin's own currency. It states no time: it is stamped with the reply's
//!   `Date`, less the age its `Cache-Control` allows it (`max-age`), that allowance
//!   stored with it (`no-store`: none).
//! - **The Exchange ticker** (`api.exchange.coinbase.com/products/<pair>/ticker`),
//!   which carries its trade's time, for a pair the Exchange lists (USD pairs;
//!   the CAD pairs answer 404).
//! - **Daily candles** (`…/products/<pair>/candles?granularity=86400`): a coin's
//!   close is its pair's close for the UTC day, and where the pair has no Exchange
//!   market (every CAD pair), its USD market's, stored in USD as stated and
//!   converted at that day's rate where a figure uses it. Only a UTC day that has
//!   ended is a close.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net, Reply};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Mismatch, Node, RecordedShape};

pub const SPOT_SOURCE: &str = "coinbase";
pub const SPOT_HOST: &str = "api.coinbase.com";
pub const EXCHANGE_SOURCE: &str = "coinbase-exchange";
pub const EXCHANGE_HOST: &str = "api.exchange.coinbase.com";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];
/// The Exchange answers at most 300 candles a request.
pub const CANDLES_PER_ASK: i64 = 300;

pub fn spot_source() -> SourceName {
    SourceName::named(SPOT_SOURCE)
}

pub fn exchange_source() -> SourceName {
    SourceName::named(EXCHANGE_SOURCE)
}

pub fn spot_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/coinbase-spot.paths"))
}

pub fn ticker_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/coinbase-exchange-ticker.paths"))
}

/// A price with the time it was current, and how much older it may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub price: Dec,
    pub at: Timestamp,
    pub allowance: std::time::Duration,
}

/// The age a `Cache-Control` value allows: its `max-age`, else none.
pub fn allowance(cache_control: Option<&str>) -> std::time::Duration {
    cache_control
        .and_then(|v| v.split(',').map(str::trim).find_map(|d| d.strip_prefix("max-age=")))
        .and_then(|s| s.parse::<u64>().ok())
        .map_or(std::time::Duration::ZERO, std::time::Duration::from_secs)
}

/// A spot reply for `base` in `currency`, stamped from the reply's headers.
pub fn parse_spot(v: &Value, base: &str, currency: Currency, date: Option<&str>, cache_control: Option<&str>) -> Outcome<Spot> {
    let read = || -> Result<Result<Spot, String>, Mismatch> {
        let d = Node::root(v).obj("data")?;
        let (b, c) = (d.text("base")?, d.text("currency")?);
        if b != base || c != currency.as_str() {
            return Ok(Err(format!("the spot price answered is {b}-{c}, not {base}-{}", currency.as_str())));
        }
        let price = d.dec_text("amount")?;
        if price <= Dec::ZERO {
            return Ok(Err(format!("{base}-{c}'s spot price is {price}")));
        }
        let Some(stamp) = date.and_then(|t| bagholder_core::jiff::fmt::rfc2822::parse(t).ok()).map(|z| z.timestamp()) else {
            return Ok(Err(format!("{base}-{c}'s spot price came with no time: its reply has no date")));
        };
        let allow = allowance(cache_control);
        let at = stamp - SignedDuration::try_from(allow).unwrap_or(SignedDuration::ZERO);
        Ok(Ok(Spot { price, at, allowance: allow }))
    };
    match read() {
        Ok(Ok(s)) => Outcome::Answered(s),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

/// An Exchange ticker reply: the last trade's price and time.
pub fn parse_ticker(v: &Value, pair: &str) -> Outcome<Spot> {
    let read = || -> Result<Result<Spot, String>, Mismatch> {
        let r = Node::root(v);
        let price = r.dec_text("price")?;
        let time = r.text("time")?;
        let Ok(at) = time.parse::<Timestamp>() else {
            return Ok(Err(format!("{pair}'s trade time {time:?} is not an instant")));
        };
        if price <= Dec::ZERO {
            return Ok(Err(format!("{pair}'s price is {price}")));
        }
        Ok(Ok(Spot { price, at, allowance: std::time::Duration::ZERO }))
    };
    match read() {
        Ok(Ok(s)) => Outcome::Answered(s),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

/// A candles reply: each ended UTC day's close, oldest first. Each candle is
/// `[time, low, high, open, close, volume]`, newest first.
pub fn parse_candles(v: &Value, pair: &str, now: Timestamp) -> Outcome<Vec<(Date, Dec)>> {
    let read = || -> Result<Result<Vec<(Date, Dec)>, String>, Mismatch> {
        let rows = Node::root(v).as_list()?;
        let mut out: Vec<(Date, Dec)> = Vec::new();
        for row in rows {
            let cells = row.as_list()?;
            if cells.len() != 6 {
                return Err(row.mismatch(format!("a candle of {} values, not 6", cells.len())));
            }
            let start = cells[0].as_int()?;
            let Ok(start) = Timestamp::from_second(start) else {
                return Err(cells[0].mismatch(format!("{start} is not an instant")));
            };
            let close = cells[4].as_dec()?;
            let day = start.to_zoned(TimeZone::UTC).date();
            if out.last().is_some_and(|(d, _)| day >= *d) {
                return Ok(Err(format!("{pair}'s candles repeat or reorder {day}")));
            }
            if close <= Dec::ZERO {
                return Ok(Err(format!("{pair}'s close on {day} is {close}")));
            }
            // a day still under way has no close yet
            if start + SignedDuration::from_hours(24) <= now {
                out.push((day, close));
            }
        }
        out.reverse();
        Ok(Ok(out))
    };
    match read() {
        Ok(Ok(c)) => Outcome::Answered(c),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn sent(net: &Net, url: &str) -> Result<Reply, Outcome<()>> {
    match ask::send(net, &Ask::get(url, &HEADERS), &[404]) {
        Outcome::Answered(r) => Ok(r),
        other => Err(other.failed().expect("not answered")),
    }
}

/// Ask for `base`'s spot price in `currency`.
pub fn ask_spot(net: &Net, base: &str, currency: Currency) -> Noted<Spot> {
    let url = format!("https://{SPOT_HOST}/v2/prices/{base}-{}/spot", currency.as_str());
    let reply = match sent(net, &url) {
        Ok(r) => r,
        Err(o) => return Noted { outcome: o.failed().expect("not answered"), shape_change: None },
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
    };
    Noted { outcome: parse_spot(&v, base, currency, reply.header("date"), reply.header("cache-control")), shape_change: ask::noticed(&v, &spot_shape(), &[]) }
}

/// Ask the Exchange for `pair`'s ticker.
pub fn ask_ticker(net: &Net, pair: &str) -> Noted<Spot> {
    let url = format!("https://{EXCHANGE_HOST}/products/{pair}/ticker");
    let reply = match sent(net, &url) {
        Ok(r) => r,
        Err(o) => return Noted { outcome: o.failed().expect("not answered"), shape_change: None },
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
    };
    Noted { outcome: parse_ticker(&v, pair), shape_change: ask::noticed(&v, &ticker_shape(), &[]) }
}

/// Ask the Exchange for `pair`'s daily candles from `from` through `to`, in
/// requests of at most 300 days.
pub fn ask_candles(net: &Net, pair: &str, from: Date, to: Date, now: Timestamp) -> Noted<Vec<(Date, Dec)>> {
    let mut all: Vec<(Date, Dec)> = Vec::new();
    let mut start = from;
    while start <= to {
        let end = start.checked_add(SignedDuration::from_hours(24 * (CANDLES_PER_ASK - 1))).unwrap_or(to).min(to);
        let url = format!("https://{EXCHANGE_HOST}/products/{pair}/candles?granularity=86400&start={start}T00:00:00Z&end={end}T00:00:00Z");
        let reply = match sent(net, &url) {
            Ok(r) => r,
            Err(o) => return Noted { outcome: o.failed().expect("not answered"), shape_change: None },
        };
        let v = match ask::json(&reply.body) {
            Ok(v) => v,
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        match parse_candles(&v, pair, now) {
            Outcome::Answered(days) => all.extend(days),
            other => return Noted { outcome: other, shape_change: None },
        }
        start = match end.tomorrow() {
            Ok(d) => d,
            Err(_) => break,
        };
    }
    Noted { outcome: Outcome::Answered(all), shape_change: None }
}
