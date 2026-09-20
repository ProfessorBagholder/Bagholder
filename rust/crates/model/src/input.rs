//! The rows the model is given besides the activities: what the store holds
//! about accounts, balances, listings, prices and the journal. Each is read
//! leniently (`lenient`) once, here, and has its types from then on.

use serde::Deserialize;
use std::collections::HashMap;

use crate::activity::Kind;
use crate::lenient;
use crate::value::norm_account_name;

/// A Wealthsimple account.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountRow {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub nickname: String,
    #[serde(deserialize_with = "lenient::text")]
    pub unified_account_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(deserialize_with = "lenient::text", rename = "type")]
    pub kind: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub net_liquidation_value: Option<f64>,
}

impl AccountRow {
    /// What the account is called everywhere: its nickname, else its type, with
    /// the spaces folded.
    pub fn name(&self) -> String {
        [&self.nickname, &self.unified_account_type, &self.kind].into_iter().find(|v| !v.is_empty()).map(|v| norm_account_name(v)).unwrap_or_default()
    }
}

/// What Wealthsimple says an account holds of one security.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BalanceRow {
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
    #[serde(deserialize_with = "lenient::number")]
    pub quantity: f64,
}

/// An account's buying power.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginRow {
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub buying_power: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
}

/// Where a price came from, as far as it decides what the price may price.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteSource {
    /// No source stated: taken as the kind's own.
    Unstated,
    Coinbase,
    CboeOptions,
    /// A listing's venue (TMX, Yahoo, …).
    Listing,
}

/// A stored price.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Quote {
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub price: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub price_change: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub percent_change: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub source: String,
    #[serde(deserialize_with = "lenient::text")]
    pub ex_dividend_date: String,
}

impl Quote {
    pub fn source(&self) -> QuoteSource {
        match self.source.as_str() {
            "" => QuoteSource::Unstated,
            "coinbase" => QuoteSource::Coinbase,
            "cboe_options" => QuoteSource::CboeOptions,
            _ => QuoteSource::Listing,
        }
    }

    /// A quote prices a position only when its source is the kind's. The coin
    /// BTC's Coinbase price must never price a share called BTC, and a listing's
    /// TMX price never a coin.
    pub fn fits(&self, kind: Kind) -> bool {
        match (self.source(), kind) {
            (QuoteSource::Unstated, _) => true,
            (source, Kind::Crypto) => source == QuoteSource::Coinbase,
            (source, Kind::Options) => source == QuoteSource::CboeOptions,
            (source, _) => source == QuoteSource::Listing,
        }
    }
}

/// Prices by key. One map, two kinds of key: a holding's own symbol, and
/// `SYMBOL@EXCHANGE` for a watched listing or a tile.
pub type Quotes = HashMap<String, Quote>;

/// What the person wrote about a trade.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct JournalEntry {
    #[serde(deserialize_with = "lenient::text")]
    pub thesis: String,
    #[serde(deserialize_with = "lenient::texts")]
    pub tags: Vec<String>,
    #[serde(deserialize_with = "lenient::text")]
    pub grade: String,
}

/// The journal, by trade id (a round trip's `rt:…`, a saved group's id) or
/// position id.
pub type Journal = HashMap<String, JournalEntry>;

/// Trades the person grouped by hand, named by their members' keys.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TradeGroup {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::texts")]
    pub members: Vec<String>,
}

/// The newest fill that carried a price, for a symbol.
#[derive(Clone, Debug, PartialEq)]
pub struct LastFill {
    pub price: f64,
    pub date: String,
}

/// The journal as the store keeps it, read.
pub fn journal_from(map: &serde_json::Map<String, serde_json::Value>) -> Journal {
    map.iter().filter(|(_, v)| v.is_object()).filter_map(|(k, v)| Some((k.clone(), JournalEntry::deserialize(v).ok()?))).collect()
}
