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
use crate::reply::{Mismatch, Node, Shape};

pub const SOURCE: &str = "cboe-canada";
pub const HOST: &str = "www-api.cboe.com";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn shape() -> Shape {
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

pub fn parse(v: &Value, symbol: &str) -> Outcome<CboeQuote> {
    let read = || -> Result<Result<CboeQuote, String>, Mismatch> {
        let d = Node::root(v).obj("data")?;
        let answered = d.text("symbol_name")?;
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
        Ok(Ok(CboeQuote { price, at, change: d.opt_dec_text("change")?, change_pct: d.opt_dec_text("change_pct")? }))
    };
    match read() {
        Ok(Ok(q)) => Outcome::Answered(q),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub fn ask(net: &Net, symbol: &str) -> Noted<CboeQuote> {
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
