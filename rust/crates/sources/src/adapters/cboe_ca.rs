//! Cboe Canada's quote for a listing on its own venue
//! (`www-api.cboe.com/ca/equities/securities-1/<symbol>/quote/`): the last price
//! at its `trade_time`, which carries its offset, and the day's change as stated.
//! An unknown symbol answers 404 with a page, not data.

use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Mismatch, Node, RecordedShape};

pub const SOURCE: &str = "cboe-canada";
pub const HOST: &str = "www-api.cboe.com";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/cboe-canada.paths"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CboeQuote {
    pub price: Dec,
    pub at: Timestamp,
    pub change: Option<Dec>,
    pub change_pct: Option<Dec>,
}

/// `2026-09-23 16:00:00-04:00` as an instant.
fn trade_time(s: &str) -> Option<Timestamp> {
    s.replacen(' ', "T", 1).parse().ok()
}

/// What Cboe Canada says of a listing: its last trade, or that it has not traded
/// in the session under way, with the close of the one before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CboeAnswer {
    Traded(CboeQuote),
    /// No trade yet this session (a day the market is shut, or before the first
    /// trade): the trade's fields are empty and only the previous close stands.
    NoTradeYet { prev_close: Dec },
}

pub fn parse(v: &Value, symbol: &str) -> Outcome<CboeAnswer> {
    let read = || -> Result<Result<CboeAnswer, String>, Mismatch> {
        let d = Node::root(v).obj("data")?;
        let answered = d.text("symbol_name")?;
        // the listing named (`symb_name`), with no trade: the trade's own name, time and
        // price blank, the previous close standing
        if answered.is_empty() && d.text("trade_time")?.is_empty() && d.text("symb_name")?.eq_ignore_ascii_case(symbol) {
            let last = d.dec_text("last")?;
            if !last.is_zero() {
                return Ok(Err(format!("{symbol} has a last price of {last} and no trade")));
            }
            let prev_close = d.dec_text("prev_close")?;
            if prev_close <= Dec::ZERO {
                return Ok(Err(format!("{symbol}'s previous close is {prev_close}")));
            }
            return Ok(Ok(CboeAnswer::NoTradeYet { prev_close }));
        }
        if !answered.eq_ignore_ascii_case(symbol) {
            return Ok(Err(format!("Cboe Canada answered {answered} for {symbol}")));
        }
        let price = d.dec_text("last")?;
        if price <= Dec::ZERO {
            return Ok(Err(format!("{symbol}'s last price is {price}")));
        }
        let time = d.text("trade_time")?;
        let Some(at) = trade_time(time) else {
            return Ok(Err(format!("{symbol}'s trade time {time:?} is not an instant")));
        };
        Ok(Ok(CboeAnswer::Traded(CboeQuote { price, at, change: d.opt_dec_text("change")?, change_pct: d.opt_dec_text("change_pct")? })))
    };
    match read() {
        Ok(Ok(q)) => Outcome::Answered(q),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub fn ask(net: &Net, symbol: &str) -> Noted<CboeAnswer> {
    let url = format!("https://{HOST}/ca/equities/securities-1/{symbol}/quote/");
    let reply = match ask::send(net, &Ask::get(&url, &HEADERS), &[404]) {
        Outcome::Answered(r) => r,
        other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
    };
    Noted { outcome: parse(&v, symbol), shape_change: ask::noticed(&v, &shape(), &[]) }
}
