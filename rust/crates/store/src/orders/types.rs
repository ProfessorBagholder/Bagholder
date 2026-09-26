//! What an order and a bracket are, as the store keeps them and the page is sent them.
//!
//! The words Bagholder itself writes -- a side, an order type, a status, a role, where
//! a row came from, how a stop is held -- are enums; a word of Wealthsimple's (its own
//! status, a time in force it names) stays the text it sent. A field order here is the
//! wire's key order. On the wire an absent text is `""` and an absent number `null`,
//! which is what the page has always been sent.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An enum of words written as text: `""` when it is not set, and a word this build
/// does not know (a row a later build wrote) read as not set rather than refused.
macro_rules! words {
    ($(#[$doc:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        words!($(#[$doc])* $name, Unset { $($variant = $text),+ });
    };
    // `$first` is what a row that does not say is: the word the store itself would write
    ($(#[$doc:meta])* $name:ident, $first:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ts_rs::TS, bagholder_diff_derive::Diff)]
        pub enum $name {
            #[ts(rename = "")]
            Unset,
            $(#[ts(rename = $text)] $variant),+
        }
        impl Default for $name {
            fn default() -> $name { $name::$first }
        }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $name::Unset => "", $($name::$variant => $text),+ }
            }
            pub fn parse(s: &str) -> $name {
                match s { $($text => $name::$variant,)+ _ => $name::Unset }
            }
            pub fn is_set(self) -> bool { self != $name::Unset }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(self.as_str()) }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<$name, D::Error> {
                Ok(match Option::<String>::deserialize(d)? { Some(s) => $name::parse(&s), None => $name::Unset })
            }
        }
        impl rusqlite::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> { Ok(self.as_str().into()) }
        }
    };
}

words!(
    /// Which way an order trades.
    Side { Buy = "BUY", Sell = "SELL" }
);
words!(
    /// How an order is priced.
    OrderType { Market = "MARKET", Limit = "LIMIT", Stop = "STOP", StopLimit = "STOP_LIMIT" }
);
words!(
    /// Where an order stands, in Bagholder's words. `Dry`: written and not sent, orders being
    /// off. `Sending`: written, and Wealthsimple not yet heard from -- what a crash between the
    /// two leaves behind.
    OrderStatus {
        Dry = "dry", Sending = "sending", Sent = "sent", Pending = "pending", Cancelling = "cancelling",
        Filled = "filled", Cancelled = "cancelled", Expired = "expired", Rejected = "rejected", Failed = "failed",
    }
);
words!(
    /// What an order is to its bracket: the entry, or one of the two exits placed for it.
    Role, Entry { Entry = "entry", Stop = "stop", Target = "target" }
);
words!(
    /// Where an order row came from.
    Source, Bagholder { Bagholder = "bagholder", Wealthsimple = "wealthsimple", Manual = "manual", Csv = "csv" }
);
words!(
    /// Where a bracket stands.
    BracketStatus {
        Waiting = "waiting", Armed = "armed", Firing = "firing", TargetPlaced = "target_placed",
        Stopping = "stopping", Closing = "closing", Done = "done", Cancelled = "cancelled",
    }
);
words!(
    /// A stop at a price, or one that trails the high.
    SlKind { Stop = "stop", Trail = "trail" }
);
words!(
    /// A trail measured in percent of the high, or in the listing's currency.
    TrailUnit, Pct { Pct = "pct", Amt = "amt" }
);
words!(
    /// A stop Wealthsimple holds as a resting order, or one Bagholder watches the price for.
    SlMode { Native = "native", Watched = "watched" }
);

impl OrderStatus {
    /// Still with the broker: it can fill, and it can be cancelled.
    pub fn is_live(self) -> bool {
        matches!(self, OrderStatus::Sent | OrderStatus::Pending | OrderStatus::Cancelling)
    }
}
impl BracketStatus {
    /// Still has work to do.
    pub fn is_live(self) -> bool {
        matches!(self, BracketStatus::Waiting | BracketStatus::Armed | BracketStatus::Firing | BracketStatus::TargetPlaced | BracketStatus::Stopping | BracketStatus::Closing)
    }
}

fn day() -> String { "DAY".into() }

/// A number as the page or a test row gives it: a number, or the text of one; nothing
/// for null, `""` and what is not a number.
pub(crate) fn lenient_num<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// One order: a ticket written here before it was sent, or an order read back from
/// Wealthsimple that was placed elsewhere.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase", default)]
#[diff(key = id)]
pub struct Order {
    pub id: String,
    pub created_at: String,
    pub account_id: String,
    pub account: String,
    pub security_id: String,
    pub symbol: String,
    pub currency: String,
    pub side: Side,
    #[serde(rename = "type")]
    pub kind: OrderType,
    #[serde(deserialize_with = "lenient_num")]
    pub quantity: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub limit_price: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub stop_price: Option<f64>,
    pub tif: String,
    /// The exits asked for with an entry, as the ticket gave them.
    pub stop_loss: Option<StopLoss>,
    pub take_profit: Option<TakeProfit>,
    pub status: OrderStatus,
    pub ws_order_id: String,
    pub error: String,
    /// What was sent to Wealthsimple, kept as sent. Stored, but never sent to
    /// the page: the ticket's own request body has no fixed shape, and the
    /// page has no use for it.
    #[serde(skip_serializing)]
    #[ts(skip)]
    pub request: Value,
    pub updated_at: String,
    pub source: Source,
    pub ws_status: String,
    #[serde(deserialize_with = "lenient_num")]
    pub filled_qty: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub avg_fill: Option<f64>,
    pub submitted_at: String,
    pub expires_at: String,
    pub parent_id: String,
    pub role: Role,
    #[serde(deserialize_with = "lenient_num")]
    pub fill_booked_qty: Option<f64>,
}

/// The stop an entry asked for.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase", default)]
pub struct StopLoss {
    pub kind: SlKind,
    #[serde(deserialize_with = "lenient_num")]
    pub price: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub trail: Option<f64>,
    pub trail_unit: TrailUnit,
}

/// The target an entry asked for.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase", default)]
pub struct TakeProfit {
    #[serde(deserialize_with = "lenient_num")]
    pub price: Option<f64>,
}

/// The fields of an order that change after it is written. `None` leaves a field as it
/// is; a number set to `Some(None)` is cleared. A key a patch does not name is `None`;
/// one it names `null` clears a number, and a key the store does not read is ignored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrderPatch {
    pub status: Option<OrderStatus>,
    pub ws_order_id: Option<String>,
    pub error: Option<String>,
    pub ws_status: Option<String>,
    pub submitted_at: Option<String>,
    pub expires_at: Option<String>,
    pub tif: Option<String>,
    pub currency: Option<String>,
    pub symbol: Option<String>,
    pub filled_qty: Option<Option<f64>>,
    pub avg_fill: Option<Option<f64>>,
    pub quantity: Option<Option<f64>>,
    pub limit_price: Option<Option<f64>>,
    pub stop_price: Option<Option<f64>>,
}

impl OrderPatch {
    pub fn is_empty(&self) -> bool { *self == OrderPatch::default() }
    /// The order as it reads with this patch applied.
    pub fn apply(&self, o: &mut Order) {
        macro_rules! take { ($($f:ident),+) => { $(if let Some(v) = &self.$f { o.$f = v.clone(); })+ } }
        take!(status, ws_order_id, error, ws_status, submitted_at, expires_at, tif, currency, symbol, filled_qty, avg_fill, quantity, limit_price, stop_price);
    }
}

/// The exits held for an entry: a stop, a target, or both, placed once the entry fills
/// and kept to the shares it filled.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase", default)]
#[diff(key = id)]
pub struct Bracket {
    pub id: String,
    pub order_id: String,
    pub created_at: String,
    pub account_id: String,
    pub security_id: String,
    pub symbol: String,
    pub currency: String,
    #[serde(deserialize_with = "lenient_num")]
    pub quantity: Option<f64>,
    #[serde(default = "day")]
    pub tif: String,
    pub sl_kind: SlKind,
    #[serde(deserialize_with = "lenient_num")]
    pub sl_price: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub sl_trail: Option<f64>,
    pub sl_trail_unit: TrailUnit,
    pub sl_order_id: String,
    pub sl_native: bool,
    pub sl_mode: SlMode,
    #[serde(deserialize_with = "lenient_num")]
    pub high_water: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub tp_price: Option<f64>,
    pub tp_order_id: String,
    pub status: BracketStatus,
    pub outcome: String,
    pub error: String,
    pub attempts: i64,
    pub moved_at: String,
    pub armed_at: String,
    pub seen_held: bool,
    pub missed_at: String,
    pub updated_at: String,
}

/// The fields of a bracket that change after it is written; as `OrderPatch`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BracketPatch {
    pub symbol: Option<String>,
    pub currency: Option<String>,
    pub tif: Option<String>,
    pub sl_kind: Option<SlKind>,
    pub sl_trail_unit: Option<TrailUnit>,
    pub sl_order_id: Option<String>,
    pub tp_order_id: Option<String>,
    pub status: Option<BracketStatus>,
    pub outcome: Option<String>,
    pub error: Option<String>,
    pub moved_at: Option<String>,
    pub armed_at: Option<String>,
    pub sl_mode: Option<SlMode>,
    pub missed_at: Option<String>,
    pub quantity: Option<Option<f64>>,
    pub sl_price: Option<Option<f64>>,
    pub sl_trail: Option<Option<f64>>,
    pub high_water: Option<Option<f64>>,
    pub tp_price: Option<Option<f64>>,
    pub attempts: Option<i64>,
    pub sl_native: Option<bool>,
    pub seen_held: Option<bool>,
}

impl BracketPatch {
    pub fn is_empty(&self) -> bool { *self == BracketPatch::default() }
    pub fn apply(&self, b: &mut Bracket) {
        macro_rules! take { ($($f:ident),+) => { $(if let Some(v) = &self.$f { b.$f = v.clone(); })+ } }
        take!(symbol, currency, tif, sl_kind, sl_trail_unit, sl_order_id, tp_order_id, status, outcome, error, moved_at, armed_at, sl_mode, missed_at, quantity, sl_price, sl_trail, high_water, tp_price, attempts, sl_native, seen_held);
    }
}
