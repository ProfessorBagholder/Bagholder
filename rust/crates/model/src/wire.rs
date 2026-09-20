//! What the page is sent: the view and every row in it, as types.
//!
//! The order of a struct's fields is the order of its keys on the wire, and
//! `tests/wire` holds every one of them to what the page was sent before the
//! model had types. A field that may have nothing to say is an `Option`: `None`
//! is `null` on the wire unless the field says it is left out.

use serde::Serialize;
use std::sync::Arc;

use crate::activity::{Direction, Flag, Kind, Side};

// --------------------------------------------------------------------------
// trades
// --------------------------------------------------------------------------

/// How a trade was closed: a long is sold, a short is covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ExitSide {
    #[serde(rename = "SELL")]
    Sell,
    #[serde(rename = "COVER")]
    Cover,
}

impl ExitSide {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitSide::Sell => "SELL",
            ExitSide::Cover => "COVER",
        }
    }
}

/// One closed piece of a trade: a lot against the fill that closed it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Leg {
    /// What a saved group names this member by.
    pub key: String,
    pub qty: f64,
    pub entry: f64,
    pub exit: f64,
    pub entry_date: String,
    pub exit_date: String,
    pub pnl: f64,
    pub pnl_cad: f64,
    pub fees: f64,
    pub buy_activity_id: String,
    pub sell_activity_id: String,
    pub flags: Vec<Flag>,
}

/// One broker fill as the page prints it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub id: String,
    pub when: String,
    pub date: String,
    pub time: String,
    /// `BUY`, `SELL`, or nothing when the row does not say.
    #[serde(serialize_with = "side_or_blank")]
    pub side: Option<Side>,
    /// Under a trade, what the fill did in it (`BUY TO OPEN`, `SELL (close +
    /// open)`); under a holding, the broker's own sub-type.
    pub sub: String,
    /// Signed: negative for a sale.
    pub qty: f64,
    pub price: f64,
    pub amount: f64,
    pub fees: f64,
    pub currency: String,
    pub flags: Vec<Flag>,
}

fn side_or_blank<S: serde::Serializer>(side: &Option<Side>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(side.map_or("", Side::as_str))
}

/// How much was opened or closed, at what average, in how many fills.
#[derive(Clone, Debug, Serialize)]
pub struct Tally {
    pub qty: f64,
    pub avg: f64,
    pub fills: usize,
}

/// A round trip: a position going from flat to open and back to flat.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    pub id: String,
    pub status: &'static str,
    /// Grouped by hand, so not regrouped by the model.
    pub locked: bool,
    pub symbol: String,
    pub underlying: String,
    pub name: String,
    pub exchange: String,
    pub kind: Kind,
    pub currency: String,
    pub account: String,
    pub account_id: String,
    pub security_id: String,
    pub side: ExitSide,
    pub open_direction: Direction,
    pub qty: f64,
    pub mult: i64,
    pub entry: f64,
    pub exit: f64,
    pub entry_date: String,
    pub exit_date: String,
    pub entry_when: String,
    pub exit_when: String,
    pub hold_days: i64,
    pub pnl: f64,
    pub pnl_cad: f64,
    pub fees: f64,
    pub fees_cad: f64,
    /// Nothing when there is no basis to measure against.
    pub pnl_pct: Option<f64>,
    /// Sent only for the trade whose page is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legs: Option<Arc<Vec<Leg>>>,
    pub leg_count: usize,
    /// Sent only for the trade whose page is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<Arc<Vec<Fill>>>,
    pub opened: Tally,
    pub closed: Tally,
    pub net_cash: f64,
    pub flags: Vec<Flag>,
    pub grade: String,
    pub thesis: String,
    pub tags: Vec<String>,
}

// --------------------------------------------------------------------------
// holdings
// --------------------------------------------------------------------------

/// What a holding is marked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    /// Its own last fill.
    Fill,
    /// A stored quote.
    Quote,
}

/// One opening fill's share of a holding.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenLot {
    pub opened: String,
    pub qty: f64,
    pub price: f64,
    pub basis: f64,
    pub held: i64,
    pub flags: Vec<Flag>,
    pub activity_id: String,
}

/// An open position: the open lots of one symbol in one account, currency and
/// direction.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// The round trip that opened it, which is the id the trade will have when
    /// it closes, so the two share a journal entry.
    pub id: String,
    pub symbol: String,
    pub underlying: String,
    pub name: String,
    pub exchange: String,
    pub kind: Kind,
    pub account: String,
    pub account_id: String,
    pub currency: String,
    pub security_id: String,
    pub short: bool,
    pub qty: f64,
    pub mult: i64,
    pub avg: f64,
    pub cost: f64,
    pub fees: f64,
    pub last: f64,
    pub price_source: Mark,
    pub price_change: Option<f64>,
    pub percent_change: Option<f64>,
    /// The day's move on the whole position, in its own currency.
    pub day_change: Option<f64>,
    pub mv: f64,
    pub unreal: f64,
    pub unreal_pct: Option<f64>,
    /// Days held, weighted by quantity.
    pub held: i64,
    pub opened: String,
    /// What Wealthsimple says the account holds, where it says.
    pub ws_qty: Option<f64>,
    pub rt: Option<String>,
    pub lots: Vec<OpenLot>,
    /// Sent only for the holding whose page is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<Arc<Vec<Fill>>>,
    pub grade: String,
    pub thesis: String,
    pub tags: Vec<String>,
    /// Its share of the book's cost.
    pub alloc: f64,
}

// --------------------------------------------------------------------------
// cashflow
// --------------------------------------------------------------------------

/// What kind of payment a cashflow row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Payment {
    Dividend,
    Interest,
    #[serde(rename = "Withholding tax")]
    WithholdingTax,
    #[serde(rename = "Interest charge")]
    InterestCharge,
}

impl Payment {
    pub fn as_str(self) -> &'static str {
        match self {
            Payment::Dividend => "Dividend",
            Payment::Interest => "Interest",
            Payment::WithholdingTax => "Withholding tax",
            Payment::InterestCharge => "Interest charge",
        }
    }
}

/// One payment into or out of the book that is not a trade.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashflowRow {
    pub id: String,
    pub date: String,
    pub time: String,
    /// The ticker; `Cash` for interest; `—` when the row names nothing.
    pub symbol: String,
    pub name: String,
    pub kind: Payment,
    pub account: String,
    pub account_id: String,
    /// Shares paid on; nothing when the row does not say.
    pub qty: Option<f64>,
    /// Paid per share; nothing when the row does not say.
    pub per: Option<f64>,
    pub amount: f64,
    pub currency: String,
    pub amount_cad: f64,
}
