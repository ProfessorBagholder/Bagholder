//! An activity: one row of the account's history, first as it is stored
//! (`RawActivity`) and then as the model works with it (`Activity`).
//!
//! The stored rows are never rewritten. `normalize` makes the working copy: the
//! account name folded, crypto and option events expressed as trade fills, the
//! kind and the category decided once. Everything after it reads fields with
//! types; nothing looks a key up by name.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::lenient;
use crate::value::compact;

// --------------------------------------------------------------------------
// the small vocabularies
// --------------------------------------------------------------------------

/// What is traded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
pub enum Kind {
    Shares,
    Options,
    Crypto,
    Futures,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Shares, Kind::Options, Kind::Crypto, Kind::Futures];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Shares => "Shares",
            Kind::Options => "Options",
            Kind::Crypto => "Crypto",
            Kind::Futures => "Futures",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == text)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which way a fill goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
pub enum Side {
    #[serde(rename = "BUY")]
    Buy,
    #[serde(rename = "SELL")]
    Sell,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
    }

    /// The direction a fill on this side closes: a buy covers a short, a sale closes a long.
    pub fn closes(self) -> Direction {
        match self {
            Side::Buy => Direction::Short,
            Side::Sell => Direction::Long,
        }
    }
}

/// Which way a position faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
pub enum Direction {
    #[serde(rename = "LONG")]
    Long,
    #[serde(rename = "SHORT")]
    Short,
}

impl Direction {
    pub const BOTH: [Direction; 2] = [Direction::Long, Direction::Short];

    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Long => "LONG",
            Direction::Short => "SHORT",
        }
    }

    /// The side of the fill that closes a position facing this way.
    pub fn closed_by(self) -> Side {
        match self {
            Direction::Long => Side::Sell,
            Direction::Short => Side::Buy,
        }
    }

    /// The side of the fill that opens one.
    pub fn opened_by(self) -> Side {
        match self {
            Direction::Long => Side::Buy,
            Direction::Short => Side::Sell,
        }
    }
}

/// What a row is, as far as the model cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Trade,
    OptionEvent,
    Dividend,
    Interest,
    Other,
}

impl Category {
    pub fn parse(text: &str) -> Category {
        match text {
            "trade" => Category::Trade,
            "option_event" => Category::OptionEvent,
            "dividend" => Category::Dividend,
            "interest" => Category::Interest,
            _ => Category::Other,
        }
    }

    /// A row the matcher reads as a fill.
    pub fn is_fill(self) -> bool {
        matches!(self, Category::Trade | Category::OptionEvent)
    }
}

/// A mark the model puts on a row, a lot or a closed slice. They reach the page
/// as their text, sorted by it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Flag {
    Transfer,
    TransferOut,
    BasisUnknown,
    Reward,
    PendingDistribution,
    Assignment,
    AssumedExpiry,
    Rolled,
    RolledIn,
    RolledOut,
    /// `split N:1`: each share became N.
    Split(i64),
    /// `split 1:N`: N shares became one.
    ReverseSplit(i64),
    /// A mark this build does not make, carried as it came.
    Other(String),
}

impl Flag {
    pub fn text(&self) -> String {
        match self {
            Flag::Transfer => "transfer".into(),
            Flag::TransferOut => "transfer-out".into(),
            Flag::BasisUnknown => "basis-unknown".into(),
            Flag::Reward => "reward".into(),
            Flag::PendingDistribution => "pending-distribution".into(),
            Flag::Assignment => "assignment".into(),
            Flag::AssumedExpiry => "assumed-expiry".into(),
            Flag::Rolled => "rolled".into(),
            Flag::RolledIn => "rolled-in".into(),
            Flag::RolledOut => "rolled-out".into(),
            Flag::Split(n) => format!("split {}:1", n),
            Flag::ReverseSplit(n) => format!("split 1:{}", n),
            Flag::Other(t) => t.clone(),
        }
    }

    pub fn parse(text: &str) -> Flag {
        match text {
            "transfer" => Flag::Transfer,
            "transfer-out" => Flag::TransferOut,
            "basis-unknown" => Flag::BasisUnknown,
            "reward" => Flag::Reward,
            "pending-distribution" => Flag::PendingDistribution,
            "assignment" => Flag::Assignment,
            "assumed-expiry" => Flag::AssumedExpiry,
            "rolled" => Flag::Rolled,
            "rolled-in" => Flag::RolledIn,
            "rolled-out" => Flag::RolledOut,
            other => {
                let ratio = |t: &str| t.parse::<i64>().ok().filter(|n| *n > 0);
                match other.strip_prefix("split ").and_then(|r| r.split_once(':')) {
                    Some((n, "1")) if ratio(n).is_some() => Flag::Split(ratio(n).unwrap()),
                    Some(("1", n)) if ratio(n).is_some() => Flag::ReverseSplit(ratio(n).unwrap()),
                    _ => Flag::Other(other.to_string()),
                }
            }
        }
    }
}

impl PartialOrd for Flag {
    fn partial_cmp(&self, other: &Flag) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Flag {
    fn cmp(&self, other: &Flag) -> std::cmp::Ordering {
        self.text().cmp(&other.text())
    }
}

impl Serialize for Flag {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text())
    }
}

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Flag, D::Error> {
        Ok(Flag::parse(&lenient::text(d)?))
    }
}

/// Add a mark once.
pub fn mark(flags: &mut Vec<Flag>, flag: Flag) {
    if !flags.contains(&flag) {
        flags.push(flag);
    }
}

// --------------------------------------------------------------------------
// the stored row
// --------------------------------------------------------------------------

/// A row as the store (or a shared case, or a CSV) gives it. Every field has a
/// default, and text and numbers are read leniently (`lenient`), because the
/// cases every implementation runs leave most of them out.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawActivity {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub occurred_at: String,
    #[serde(deserialize_with = "lenient::text")]
    pub transaction_date: String,
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub book_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub fifo_id: String,
    /// The account's nickname, as Wealthsimple spells it.
    #[serde(deserialize_with = "lenient::text")]
    pub account_type: String,
    #[serde(deserialize_with = "lenient::text", alias = "activity_type")]
    pub activity_type: String,
    #[serde(deserialize_with = "lenient::text", alias = "activity_sub_type")]
    pub activity_sub_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub description: String,
    /// The broker's cash direction (`DEBIT`, `CREDIT`), not long or short.
    #[serde(deserialize_with = "lenient::text")]
    pub direction: String,
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::number")]
    pub quantity: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub unit_price: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub commission: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub net_cash_amount: f64,
    #[serde(deserialize_with = "lenient::text")]
    pub category: String,
    #[serde(deserialize_with = "lenient::text")]
    pub raw_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub aft_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
    /// A row entered by hand may say what it is; the store never does.
    #[serde(deserialize_with = "lenient::text")]
    pub kind: String,
}

// --------------------------------------------------------------------------
// the working row
// --------------------------------------------------------------------------

/// A row as the model works with it (`normalize::normalize`), or one the model
/// derived itself (`synth`). The matcher's inference writes what it decides --
/// the contract count behind a quantity-zero multileg, the price that implies,
/// which way it closes -- onto this row, and that is what a trade's fills print.
#[derive(Clone, Debug)]
pub struct Activity {
    pub id: String,
    pub occurred_at: String,
    pub transaction_date: String,
    pub account_id: String,
    pub book_id: String,
    pub fifo_id: String,
    /// The account's nickname with its spaces folded.
    pub account_name: String,
    pub activity_type: String,
    pub activity_sub_type: String,
    pub description: String,
    /// The broker's cash direction (`DEBIT`, `CREDIT`).
    pub cash_direction: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    /// Signed: negative for a sale.
    pub quantity: f64,
    pub unit_price: f64,
    pub commission: f64,
    pub net_cash_amount: f64,
    pub category: Category,
    pub raw_type: String,
    pub aft_type: String,
    pub security_id: String,
    pub kind: Kind,
    pub flags: Vec<Flag>,
}

impl Activity {
    /// The activity type upper-cased with its separators gone, as it is compared.
    pub fn type_c(&self) -> String {
        compact(&self.activity_type)
    }

    pub fn sub_type_c(&self) -> String {
        compact(&self.activity_sub_type)
    }

    pub fn raw_type_c(&self) -> String {
        compact(&self.raw_type)
    }

    pub fn has(&self, flag: &Flag) -> bool {
        self.flags.contains(flag)
    }

    /// When it happened, to the second where that is known, else the day.
    pub fn when(&self) -> &str {
        if self.occurred_at.is_empty() { &self.transaction_date } else { &self.occurred_at }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_flag_is_its_text_both_ways_and_sorts_by_it() {
        for text in ["transfer-out", "basis-unknown", "rolled-in", "split 4:1", "split 1:10", "something-new"] {
            assert_eq!(Flag::parse(text).text(), text);
        }
        assert_eq!(Flag::parse("split 4:1"), Flag::Split(4));
        assert_eq!(Flag::parse("split 1:10"), Flag::ReverseSplit(10));
        assert_eq!(Flag::parse("split 1:1"), Flag::Split(1), "one for one reads as the forward form");
        assert_eq!(Flag::parse("split x:1"), Flag::Other("split x:1".into()));
        let mut flags = vec![Flag::Rolled, Flag::Split(2), Flag::BasisUnknown, Flag::Assignment];
        flags.sort();
        assert_eq!(flags.iter().map(Flag::text).collect::<Vec<_>>(), ["assignment", "basis-unknown", "rolled", "split 2:1"]);
        assert_eq!(serde_json::to_string(&flags).unwrap(), r#"["assignment","basis-unknown","rolled","split 2:1"]"#);
    }

    #[test]
    fn test_a_stored_row_reads_whatever_its_fields_are() {
        let raw: RawActivity = serde_json::from_value(serde_json::json!({
            "id": 12, "quantity": "100", "unitPrice": 5, "netCashAmount": null, "securityId": null,
            "activity_sub_type": "BUY", "symbol": "QNC",
        }))
        .unwrap();
        assert_eq!((raw.id.as_str(), raw.quantity, raw.unit_price, raw.net_cash_amount), ("12", 100.0, 5.0, 0.0));
        assert_eq!((raw.security_id.as_str(), raw.activity_sub_type.as_str(), raw.currency.as_str()), ("", "BUY", ""));
    }
}
