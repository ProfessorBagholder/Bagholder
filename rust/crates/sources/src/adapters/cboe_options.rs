//! Cboe's delayed option chain for one US underlying
//! (`cdn.cboe.com/api/global/delayed_quotes/options/<symbol>.json`, the
//! listing's own symbol, `BRK.B` with its point). An unknown symbol answers 403.
//!
//! The chain states its own times, and nothing is inferred from when it was
//! published:
//! - `timestamp`, when Cboe made the chain, in UTC (its `Last-Modified` header
//!   agreed within seconds on each capture);
//! - `data.last_trade_time`, the underlying's last trade, Eastern
//!   (`2026-09-22T15:59:59`): the session the chain carries is that day;
//! - per contract (`option`, its OCC symbol), the closing `bid` and `ask`, and
//!   `last_trade_price` at `last_trade_time`, Eastern, null for a contract never
//!   traded.
//!
//! Prices are written as binary float leftovers (`0.370000004768372`); each is
//! kept as the decimal its digits spell.

use bagholder_core::instrument::OptionRight;
use bagholder_core::jiff::civil::{Date, DateTime};
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Mismatch, Node, RecordedShape};

pub const SOURCE: &str = "cboe-options";
pub const HOST: &str = "cdn.cboe.com";
const HEADERS: [(&str, &str); 1] = [("User-Agent", ask::USER_AGENT)];

/// How far behind the market the delayed chain runs, by Cboe's design.
pub const LATE_BY: std::time::Duration = std::time::Duration::from_secs(15 * 60);

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/cboe-options.paths"))
}

/// One contract as the chain states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainContract {
    /// Its OCC symbol (`BBAI280121C00010000`).
    pub occ: String,
    pub expiry: Date,
    pub right: OptionRight,
    pub strike: Dec,
    pub bid: Dec,
    pub ask: Dec,
    /// The last trade's price and its time, Eastern; none for a contract never traded.
    pub last: Option<(Dec, DateTime)>,
}

impl ChainContract {
    /// The bid/ask midpoint, where both sides are quoted.
    pub fn midpoint(&self) -> Option<Dec> {
        midpoint(self.bid, self.ask)
    }
}

/// A contract's bid/ask midpoint: only where both sides are quoted (each above
/// zero), and exact, `(bid + ask) / 2` in decimal.
pub fn midpoint(bid: Dec, ask: Dec) -> Option<Dec> {
    if !bid.is_positive() || !ask.is_positive() {
        return None;
    }
    bid.checked_add(ask).ok()?.checked_mul(Dec::parse("0.5").ok()?).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// When Cboe made it.
    pub made_at: Timestamp,
    /// The session it carries: the day of the underlying's last trade, Eastern.
    pub session: Date,
    pub contracts: Vec<ChainContract>,
}

/// `2026-09-23 03:30:07`, UTC.
fn made_at(s: &str) -> Option<Timestamp> {
    format!("{}Z", s.replacen(' ', "T", 1)).parse().ok()
}

/// `2026-09-22T15:59:59`, a wall time with no offset.
fn wall(s: &str) -> Option<DateTime> {
    s.parse().ok()
}

/// An OCC symbol's terms: the root, then `YYMMDD`, `C` or `P`, and the strike in
/// thousandths over eight digits.
pub fn occ_terms(occ: &str) -> Option<(Date, OptionRight, Dec)> {
    let tail = occ.get(occ.len().checked_sub(15)?..)?;
    if occ.len() == 15 || !tail.bytes().enumerate().all(|(i, b)| if i == 6 { b == b'C' || b == b'P' } else { b.is_ascii_digit() }) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| tail[r].parse::<i16>().ok();
    let expiry = Date::new(2000 + n(0..2)?, n(2..4)? as i8, n(4..6)? as i8).ok()?;
    let right = if &tail[6..7] == "C" { OptionRight::Call } else { OptionRight::Put };
    let strike = Dec::new(tail[7..].parse().ok()?, 3).ok()?;
    Some((expiry, right, strike))
}

pub fn parse(v: &Value, symbol: &str) -> Outcome<Chain> {
    let read = || -> Result<Result<Chain, String>, Mismatch> {
        let root = Node::root(v);
        let d = root.obj("data")?;
        let answered = d.text("symbol")?;
        if !answered.eq_ignore_ascii_case(symbol) {
            return Ok(Err(format!("Cboe answered the chain of {answered} for {symbol}")));
        }
        let stamp = root.text("timestamp")?;
        let Some(made_at) = made_at(stamp) else {
            return Ok(Err(format!("{symbol}'s chain timestamp {stamp:?} is not a time")));
        };
        let Some(underlying) = d.opt_text("last_trade_time")? else {
            return Ok(Err(format!("{symbol}'s chain states no last trade of the underlying, so not its session")));
        };
        let Some(session) = wall(underlying).map(|t| t.date()) else {
            return Ok(Err(format!("{symbol}'s last trade time {underlying:?} is not a time")));
        };
        let mut contracts = Vec::new();
        for c in d.list("options")? {
            let occ = c.text("option")?;
            let Some((expiry, right, strike)) = occ_terms(occ) else {
                return Err(c.field("option")?.mismatch(format!("{occ:?} is not an OCC symbol")));
            };
            let (bid, ask) = (c.dec("bid")?, c.dec("ask")?);
            if bid.is_negative() || ask.is_negative() {
                return Ok(Err(format!("{occ} is quoted {bid} / {ask}")));
            }
            let last = match c.opt_text("last_trade_time")? {
                Some(t) => {
                    let Some(at) = wall(t) else { return Ok(Err(format!("{occ}'s last trade time {t:?} is not a time"))) };
                    let price = c.dec("last_trade_price")?;
                    if !price.is_positive() {
                        return Ok(Err(format!("{occ} last traded at {price}")));
                    }
                    Some((price, at))
                }
                None => None,
            };
            if contracts.iter().any(|k: &ChainContract| k.occ == occ) {
                return Ok(Err(format!("{occ} is listed twice in {symbol}'s chain")));
            }
            contracts.push(ChainContract { occ: occ.to_string(), expiry, right, strike, bid, ask, last });
        }
        Ok(Ok(Chain { made_at, session, contracts }))
    };
    match read() {
        Ok(Ok(c)) => Outcome::Answered(c),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub fn ask(net: &Net, symbol: &str) -> Noted<Chain> {
    let url = format!("https://{HOST}/api/global/delayed_quotes/options/{symbol}.json");
    let reply = match ask::send(net, &Ask::get(&url, &HEADERS), &[403]) {
        Outcome::Answered(r) => r,
        other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
    };
    Noted { outcome: parse(&v, symbol), shape_change: ask::noticed(&v, &shape(), &[]) }
}
