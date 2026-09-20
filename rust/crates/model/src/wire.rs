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

// --------------------------------------------------------------------------
// a map that keeps the order it was filled in
// --------------------------------------------------------------------------

/// Keys to values in the order they were first put in, which is the order the
/// page is sent them in: the symbol picker lists listings as the book met them.
#[derive(Clone, Debug)]
pub struct Ordered<V>(pub Vec<(String, V)>);

impl<V> Default for Ordered<V> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<V> Ordered<V> {
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The value under `key`, put there by `make` when there was none.
    pub fn entry(&mut self, key: &str, make: impl FnOnce() -> V) -> &mut V {
        if let Some(i) = self.0.iter().position(|(k, _)| k == key) {
            return &mut self.0[i].1;
        }
        self.0.push((key.to_string(), make()));
        &mut self.0.last_mut().unwrap().1
    }
}

impl<V: Serialize> Serialize for Ordered<V> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

// --------------------------------------------------------------------------
// the dashboard's figures
// --------------------------------------------------------------------------

/// The KPI tiles: what one filtered list of trades adds up to, in CAD.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kpi {
    pub realized: f64,
    pub count: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakeven: usize,
    pub win_rate: Option<f64>,
    pub gross_win: f64,
    pub gross_loss: f64,
    /// Nothing when there are wins and no losses: see `profit_factor_infinite`.
    pub profit_factor: Option<f64>,
    pub profit_factor_infinite: bool,
    pub expectancy: Option<f64>,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub fees: f64,
    pub avg_hold: Option<f64>,
    pub open_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BySymbolRow {
    /// The underlying, so a chain of contracts sits under the name it is written on.
    pub symbol: String,
    pub pnl: f64,
    pub n: usize,
    pub legs: usize,
    pub win_rate: f64,
    pub avg_hold: f64,
    pub trade_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyBar {
    /// `YYYY-MM`.
    pub key: String,
    pub label: String,
    pub value: f64,
    pub count: usize,
    pub trade_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeBucket {
    pub grade: &'static str,
    pub n: usize,
    pub pnl: f64,
    pub trade_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Grades {
    pub buckets: Vec<GradeBucket>,
    pub ungraded: usize,
    pub graded: usize,
}

/// A closed trade still missing a grade or a thesis.
#[derive(Clone, Debug, Serialize)]
pub struct QueueRow {
    pub id: String,
    pub symbol: String,
    pub date: String,
    pub pnl: f64,
    pub currency: &'static str,
    pub missing: &'static str,
}

// --------------------------------------------------------------------------
// returns
// --------------------------------------------------------------------------

/// One year's time-weighted return beside the benchmark's over the same days.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearRow {
    pub year: String,
    pub r: f64,
    pub days: i64,
    pub from: String,
    pub to: String,
    /// Net deposits over the year; nothing when the record does not carry them.
    pub flow: Option<f64>,
    pub end_v: Option<f64>,
    pub sp_r: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Annualized {
    pub rate: Option<f64>,
    pub years: f64,
    pub count: usize,
    pub first: String,
    pub last: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drawdown {
    pub pct: Option<f64>,
    pub abs: Option<f64>,
    pub at: String,
    pub peak_at: String,
}

// --------------------------------------------------------------------------
// portfolio
// --------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Allocation {
    pub id: String,
    pub symbol: String,
    pub account: String,
    pub value: f64,
    pub share: f64,
}

/// One slice of the book by sector or by country.
#[derive(Clone, Debug, Serialize)]
pub struct ExposureSlice {
    pub name: String,
    pub value: f64,
    pub share: f64,
}

/// The Portfolio tiles: CAD aggregates over the accounts in scope.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Portfolio {
    pub allocation: Vec<Allocation>,
    pub sectors: Vec<ExposureSlice>,
    pub regions: Vec<ExposureSlice>,
    pub market_value: f64,
    pub cost_basis: f64,
    pub unrealized: f64,
    pub unrealized_pct: Option<f64>,
    pub position_count: usize,
    pub account_count: usize,
    pub nav: Option<f64>,
    pub nav_accounts: usize,
    pub margin_used: f64,
    /// By currency, to the cent.
    pub margin_used_by: std::collections::BTreeMap<String, f64>,
    pub margin_used_pct: Option<f64>,
    pub available_margin: Option<f64>,
    /// The margin accounts whose buying power Wealthsimple did not give.
    pub available_margin_unavailable: Vec<String>,
    pub has_margin: bool,
    pub cash: f64,
    pub cash_pct: Option<f64>,
    pub day_change: Option<f64>,
    pub day_change_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub currency: String,
    pub status: String,
    pub nav: Option<f64>,
}

// --------------------------------------------------------------------------
// cashflow
// --------------------------------------------------------------------------

/// A tile over the cashflow chart. Three kinds share the row.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum CashflowTile {
    /// What was paid over a span.
    #[serde(rename_all = "camelCase")]
    Paid { label: String, total: f64, per_month: f64, count: usize },
    /// Margin drawn (the Portfolio tab's figure) and what it costs a month.
    #[serde(rename_all = "camelCase")]
    Margin { label: &'static str, margin_used: f64, interest_per_month: f64, interest_months: usize },
    /// The declared rate of what is held, over its cost.
    #[serde(rename_all = "camelCase")]
    Yield { label: &'static str, r#yield: Option<f64>, projected: f64, earned: f64, book: f64 },
}

#[derive(Clone, Debug, Serialize)]
pub struct CashflowMonth {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub count: usize,
}

/// Where a payer's rate came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RateSource {
    /// The fund's own declared record.
    Declared,
    /// Read off the payments the book received.
    Payments,
    #[serde(rename = "")]
    Unknown,
}

/// What a payer is priced at in the cashflow table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priced {
    Close,
    Fill,
}

/// A holding that pays, with its rate and what it has paid.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashflowHolding {
    pub id: String,
    pub symbol: String,
    pub account: String,
    pub qty: f64,
    pub per: Option<f64>,
    /// Payments a year, read from the record, never assumed.
    pub freq: Option<i64>,
    pub freq_verified: bool,
    pub rate_source: RateSource,
    pub cost: f64,
    pub avg: f64,
    pub last: f64,
    pub price_source: Priced,
    pub ytd: f64,
    pub ttm: f64,
    pub all: f64,
    pub next_ex_date: String,
    pub next_pay_date: String,
    pub ex_past: bool,
    pub pay_past: bool,
    /// Projected income a payment.
    pub yob: Option<f64>,
    pub annual: Option<f64>,
    pub yoc: Option<f64>,
    pub current_yield: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cashflow {
    pub tiles: Vec<CashflowTile>,
    pub months: Vec<CashflowMonth>,
    pub holdings: Vec<CashflowHolding>,
    pub rows: Vec<CashflowRow>,
    pub other: Vec<CashflowRow>,
    pub total: f64,
    pub count: usize,
    /// The filters in force that the cashflow does not read.
    pub skipped_filters: Vec<&'static str>,
    pub interest: f64,
    pub withholding: f64,
}

// --------------------------------------------------------------------------
// markets
// --------------------------------------------------------------------------

/// One tile of the heatmap of what the book holds.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldTile {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub value: f64,
    pub percent_change: Option<f64>,
    pub sector: String,
}

/// One tile of a market universe's heatmap.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniverseTile {
    /// A universe's tile is no holding: always nothing.
    pub id: Option<String>,
    pub symbol: String,
    pub name: String,
    pub value: f64,
    pub percent_change: Option<f64>,
    pub sector: String,
    pub country: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchItem {
    pub symbol: String,
    pub exchange: String,
    pub name: String,
    pub currency: String,
    pub last: Option<f64>,
    pub price_change: Option<f64>,
    pub percent_change: Option<f64>,
    pub sector: String,
    pub kind: String,
    pub position_id: Option<String>,
}

/// A listing a news item was read for.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsTag {
    pub symbol: String,
    pub exchange: String,
    pub held: bool,
    pub watched: bool,
    pub percent_change: Option<f64>,
    pub position_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: String,
    pub headline: String,
    pub source: String,
    pub url: String,
    pub published_at: String,
    /// From the market's own feed rather than a listing's.
    pub market: bool,
    pub tags: Vec<NewsTag>,
    /// `story`, or `release` for a company's own.
    pub kind: String,
}

/// One tile of the Markets tab's row.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketTile {
    pub symbol: &'static str,
    pub exchange: &'static str,
    pub label: String,
    pub name: &'static str,
    pub kind: &'static str,
    pub last: Option<f64>,
    pub change: Option<f64>,
    pub percent_change: Option<f64>,
    pub decimals: i64,
    /// A contract quoted as 100 minus a rate carries that rate, and its move,
    /// beside the price: both keys, or neither.
    #[serde(flatten)]
    pub implied: Option<ImpliedRate>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpliedRate {
    pub rate: f64,
    pub rate_change: Option<f64>,
}

/// An entry of the directory the tile picker searches.
#[derive(Clone, Debug, Serialize)]
pub struct MarketInstrument {
    pub symbol: &'static str,
    pub label: String,
    pub name: &'static str,
    pub exchange: &'static str,
    pub kind: &'static str,
    pub aliases: &'static [&'static str],
}

#[derive(Clone, Debug, Serialize)]
pub struct Markets {
    pub holdings: Vec<HeldTile>,
    pub watchlist: Vec<WatchItem>,
    pub news: Vec<NewsItem>,
    pub universes: Ordered<Vec<UniverseTile>>,
    pub tiles: Vec<MarketTile>,
    pub instruments: Vec<MarketInstrument>,
}

// --------------------------------------------------------------------------
// the view
// --------------------------------------------------------------------------

/// What the symbol picker shows beside a symbol.
#[derive(Clone, Debug, Serialize)]
pub struct ListingInfo {
    pub name: String,
    pub exchange: String,
    pub kind: Kind,
    pub currency: String,
}

/// What the filters can be set to, from the whole book.
#[derive(Clone, Debug, Serialize)]
pub struct Options {
    pub accounts: Vec<String>,
    pub symbols: Vec<String>,
    pub listings: Ordered<ListingInfo>,
    pub tags: Vec<String>,
    pub exchanges: Vec<String>,
    pub kinds: Vec<Kind>,
    pub grades: Vec<&'static str>,
    pub sides: [&'static str; 2],
    pub results: [&'static str; 3],
    pub years: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDates {
    pub fx_last: String,
    pub benchmark_last: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EquityBlock {
    pub label: String,
    pub series: Vec<crate::nav::Point>,
    pub drawdown: Drawdown,
    pub annualized: Annualized,
}

#[derive(Clone, Debug, Serialize)]
pub struct BenchmarkRef {
    pub key: String,
    pub label: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct PositionsSummary {
    pub count: usize,
    pub book: f64,
    pub mv: f64,
    pub unreal: f64,
}

/// Everything the page shows, for one set of filters.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub ok: bool,
    pub today: String,
    pub synced_at: String,
    pub currency: &'static str,
    pub market: MarketDates,
    pub filters: crate::filters::Filters,
    pub options: Options,
    pub kpi: Kpi,
    pub equity: EquityBlock,
    pub years: Vec<YearRow>,
    pub benchmark: BenchmarkRef,
    pub monthly: Vec<MonthlyBar>,
    pub by_symbol: Vec<BySymbolRow>,
    pub grades: Grades,
    pub queue: Vec<QueueRow>,
    pub trades: Vec<Trade>,
    pub trade_count: usize,
    pub trade_total: usize,
    pub positions: Vec<Position>,
    pub positions_summary: PositionsSummary,
    pub portfolio: Portfolio,
    pub markets: Markets,
    pub cashflow: Cashflow,
    pub unmatched: Vec<crate::fifo::Unmatched>,
    pub accounts: Vec<Account>,
    pub activity_count: usize,
}

/// The legs and fills of one trade or holding.
#[derive(Clone, Debug, Serialize)]
pub struct TradeDetail {
    pub id: String,
    pub legs: Arc<Vec<Leg>>,
    pub fills: Arc<Vec<Fill>>,
}

impl View {
    /// The view as the JSON the page is sent: what the stream compares and the
    /// shared cases are written in.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a view is plain data")
    }
}
