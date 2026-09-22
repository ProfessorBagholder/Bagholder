//! The rows the model is given besides the activities: what the store holds
//! about accounts, balances, listings, prices and the journal. Each is read
//! leniently (`lenient`) once, here, and has its types from then on.

use serde::Deserialize;
use std::collections::HashMap;

use crate::activity::Kind;
use crate::lenient;
use crate::value::norm_account_name;

/// A Wealthsimple account.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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

/// A listing the person watches.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WatchRow {
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub exchange: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
}

/// A stored news item, as read for one listing (or for the market: `*` on `MARKET`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewsRow {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub exchange: String,
    #[serde(deserialize_with = "lenient::text")]
    pub headline: String,
    /// The wire that carried it, which is the source the page names.
    #[serde(deserialize_with = "lenient::text")]
    pub wire: String,
    #[serde(deserialize_with = "lenient::text")]
    pub url: String,
    #[serde(deserialize_with = "lenient::text")]
    pub published_at: String,
    #[serde(deserialize_with = "lenient::text")]
    pub kind: String,
}

/// One constituent of a market universe (the heatmaps beyond the book).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UniverseRow {
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::number")]
    pub value: f64,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub percent_change: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub sector: String,
    #[serde(deserialize_with = "lenient::text")]
    pub country: String,
}

/// A tile the person put in the Markets tab's row.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TileRef {
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub exchange: String,
}

/// A declared distribution of a fund.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Distribution {
    #[serde(deserialize_with = "lenient::text")]
    pub ex_date: String,
    #[serde(deserialize_with = "lenient::text")]
    pub pay_date: String,
    #[serde(deserialize_with = "lenient::number")]
    pub amount: f64,
}

/// A listing as the market readers are asked for it: what a quote, a chart or a
/// record source needs to find it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
    /// `Shares`, `Options`, `Crypto`, `Futures`, or `Instrument` for an index, a
    /// future, a rate or a currency pair from the directory. Left out where the
    /// reader is not told (a payer's record is asked for by symbol and venue).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Where its quote is kept, when that is not under its symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_key: Option<String>,
    /// A directory instrument's Yahoo symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yahoo: Option<String>,
    /// The earliest day its bars are wanted from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
}

impl Listing {
    /// The listing as the JSON the market readers take.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a listing is plain data")
    }
}

/// Listings as the JSON the market readers take.
pub fn listings_json(listings: &[Listing]) -> Vec<serde_json::Value> {
    listings.iter().map(Listing::to_value).collect()
}
