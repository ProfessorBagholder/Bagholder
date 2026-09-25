//! The figures document: everything the page shows of the book, for one set of
//! filters, built from the engine's figures (`build`).
//!
//! Only what the page reads is here, and what the page used to work out for
//! itself from amounts (an "Other" slice, income a month, a month's net, the
//! total of the accounts' values) is worked out here, exactly: the page does no
//! money arithmetic. Money and quantities are [`Dec`]; ratios, counts and days
//! are numbers. A per-instrument amount is in the instrument's currency, named on
//! its row; an aggregate is in CAD.

use serde::Serialize;
use ts_rs::TS;

use super::{Dec, Fig};

/// A total of amounts some of which may not be stated: the stated ones added, and
/// how many were left out.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Partial {
    pub total: Fig<Dec>,
    pub left_out: usize,
}

// --------------------------------------------------------------------------
// trades and holdings
// --------------------------------------------------------------------------

/// Whether any of a trade is still held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Closed,
}

/// A trade: a position in one account and instrument from its first fill until
/// it is flat, or a saved group of them.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    /// The trade's id (a saved group's for a group).
    pub id: String,
    pub status: Status,
    /// While open, the holding it is (`Position::id`): opening it opens that page.
    pub position: Option<String>,
    pub symbol: String,
    /// The symbol of what a contract is written on; the symbol itself otherwise.
    pub underlying: String,
    pub name: String,
    pub exchange: String,
    pub kind: String,
    pub currency: String,
    pub account: String,
    pub account_id: String,
    pub instrument: String,
    /// The broker's id for the instrument, which an order names (stage 4 moves orders onto the instrument).
    pub security: String,
    /// `SELL` for a long, `COVER` for a short.
    pub side: String,
    pub qty: Fig<Dec>,
    pub entry: Fig<Dec>,
    /// None before the first sale.
    pub exit: Option<Fig<Dec>>,
    pub entry_date: String,
    /// The last close; none while open.
    pub exit_date: Option<String>,
    /// The day of its latest fill: the list's order, newest activity first.
    pub last_date: String,
    pub hold_days: i64,
    pub pnl: Fig<Dec>,
    pub pnl_cad: Fig<Dec>,
    pub pnl_pct: Option<f64>,
    pub fees: Fig<Dec>,
    pub flags: Vec<String>,
    pub grade: String,
    pub thesis: String,
    pub tags: Vec<String>,
}

/// A holding: what one account holds of one instrument, one way.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub id: String,
    /// The trade it is, whose journal it shows.
    pub trade: String,
    pub symbol: String,
    pub underlying: String,
    pub name: String,
    pub exchange: String,
    pub kind: String,
    pub currency: String,
    pub account: String,
    pub account_id: String,
    pub instrument: String,
    pub security: String,
    pub short: bool,
    pub qty: Fig<Dec>,
    pub avg: Fig<Dec>,
    /// What the units held cost (long) or brought in (short).
    pub cost: Fig<Dec>,
    /// The price marked at.
    pub last: Fig<Dec>,
    pub mv: Fig<Dec>,
    pub unreal: Fig<Dec>,
    pub unreal_pct: Option<f64>,
    pub day_change: Fig<Option<Dec>>,
    /// The day's move of the price, as a fraction.
    pub percent_change: Option<f64>,
    pub opened: String,
    pub held: Fig<i64>,
    pub grade: String,
    pub thesis: String,
    pub tags: Vec<String>,
    /// What the holding waits on.
    pub gaps: Vec<String>,
    /// Its lots' marks (`entered`, `deposited`, …).
    pub flags: Vec<String>,
}

/// One broker fill of a trade or holding, for its page.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub id: String,
    /// The instant, where the record states one.
    pub when: Option<String>,
    pub date: String,
    /// `BUY`, `SELL`, or empty where the record does not say.
    pub side: String,
    /// What the fill did in the trade (`BUY TO OPEN`, `SELL TO CLOSE`).
    pub sub: String,
    /// Signed: negative for a sale.
    pub qty: Fig<Dec>,
    pub price: Fig<Dec>,
    pub amount: Fig<Dec>,
    pub currency: String,
    pub flags: Vec<String>,
}

/// A trade's or holding's fills, for its page.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct Detail {
    pub id: String,
    pub fills: Vec<Fill>,
}

// --------------------------------------------------------------------------
// the dashboard
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Kpi {
    /// Realized P&L in the dates chosen, every sale's; open trades' included.
    pub realized: Fig<Dec>,
    pub realized_left_out: usize,
    /// Closed trades scored.
    pub count: usize,
    pub left_out: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakeven: usize,
    pub win_rate: Option<f64>,
    pub gross_win: Fig<Dec>,
    pub gross_loss: Fig<Dec>,
    pub profit_factor: Fig<Option<f64>>,
    pub profit_factor_infinite: bool,
    pub expectancy: Fig<Option<Dec>>,
    pub avg_win: Fig<Option<Dec>>,
    pub avg_loss: Fig<Option<Dec>>,
}

/// A day of the account value series.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = d)]
pub struct Point {
    pub d: String,
    pub v: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Drawdown {
    pub pct: Option<f64>,
    pub abs: Option<Dec>,
    pub at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct Annualized {
    pub rate: Option<f64>,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Equity {
    /// The accounts' value by day: what Returns, the years and Max drawdown read.
    pub series: Vec<Point>,
    pub drawdown: Drawdown,
    pub annualized: Annualized,
    /// What the series waits on.
    pub gaps: Vec<String>,
    /// The filters set that the value series does not read.
    pub skipped_filters: Vec<String>,
    /// The realized P&L in scope, a running total by day.
    pub pnl: PnlCurve,
}

/// The realized P&L in scope, CAD, the total to the end of each day a part was realized.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct PnlCurve {
    pub series: Vec<Point>,
    /// Parts whose P&L waits on something, left out of every total.
    pub left_out: u32,
    /// What the series waits on: a day whose total does not fit is not drawn.
    pub gaps: Vec<String>,
}

/// One year's return beside the benchmark's over the same span.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = year)]
#[serde(rename_all = "camelCase")]
pub struct YearRow {
    pub year: String,
    pub r: f64,
    pub sp_r: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct BenchmarkRef {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = key)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyBar {
    /// `YYYY-MM`.
    pub key: String,
    pub label: String,
    pub value: Fig<Dec>,
    pub count: usize,
    pub trade_ids: Vec<String>,
}

/// The realized P&L of what one underlying's trades made.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct BySymbolRow {
    /// The underlying instrument.
    pub id: String,
    pub symbol: String,
    pub pnl: Fig<Dec>,
    pub n: usize,
    pub win_rate: Option<f64>,
    pub avg_hold: Option<f64>,
    pub trade_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = grade)]
#[serde(rename_all = "camelCase")]
pub struct GradeBucket {
    pub grade: String,
    pub n: usize,
    pub pnl: Fig<Dec>,
    pub trade_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct Grades {
    pub buckets: Vec<GradeBucket>,
    pub graded: usize,
}

/// A closed trade missing a grade or a thesis.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
pub struct QueueRow {
    pub id: String,
    pub symbol: String,
    pub date: String,
    pub pnl: Fig<Dec>,
    pub missing: String,
}

// --------------------------------------------------------------------------
// the portfolio
// --------------------------------------------------------------------------

/// A slice of a donut, as the page draws it: the largest first, the rest folded
/// into one `Other (n)`.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = label)]
pub struct Slice {
    pub label: String,
    pub value: Fig<Dec>,
    pub share: f64,
    /// The one holding it is, where it is one.
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Portfolio {
    pub position_count: usize,
    pub market_value: Partial,
    pub cost_basis: Partial,
    pub unrealized: Partial,
    pub unrealized_pct: Fig<Option<f64>>,
    pub nav: Option<Fig<Dec>>,
    pub nav_accounts: usize,
    pub has_margin: bool,
    pub margin_used: Fig<Dec>,
    pub margin_used_pct: Fig<Option<f64>>,
    pub available_margin: Option<Fig<Dec>>,
    /// The margin accounts whose buying power the broker did not state.
    pub available_margin_unavailable: Vec<String>,
    pub cash: Fig<Dec>,
    pub cash_pct: Fig<Option<f64>>,
    pub day_change: Option<Partial>,
    pub day_change_pct: Fig<Option<f64>>,
    pub allocation: Vec<Slice>,
}

/// An account, for the ticket and Add trade.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    /// The broker's own id for it, which an order names; none for an account
    /// kept by hand.
    pub broker_account: Option<String>,
    pub status: String,
    /// Whether the person trades in it: a self-directed account of cash or
    /// margin, not one the broker manages.
    pub tradable: bool,
    pub margin: bool,
    /// Its value as the broker states it.
    pub nav: Option<Dec>,
}

// --------------------------------------------------------------------------
// cashflow
// --------------------------------------------------------------------------

/// A tile over the cashflow chart, by what it shows.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = label)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CashflowTile {
    /// What was paid over the span, and that over the months that paid.
    Paid { label: String, total: Partial, per_month: Fig<Option<Dec>> },
    /// Margin drawn and what it costs a month.
    Margin { label: String, margin_used: Fig<Dec>, interest_per_month: Fig<Option<Dec>> },
    /// Projected income over cost, and a month of it.
    Yield {
        label: String,
        #[serde(rename = "yield")]
        yield_on_cost: Fig<Option<f64>>,
        projected: Fig<Dec>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = key)]
pub struct CashflowMonth {
    /// `YYYY-MM`.
    pub key: String,
    pub label: String,
    /// Distributions paid.
    pub value: Fig<Dec>,
    pub count: usize,
    /// Interest charged, shown positive.
    pub interest: Fig<Dec>,
    /// Distributions less interest charged.
    pub net: Fig<Dec>,
}

/// A holding that pays: its rate and what it has paid, in its own currency.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct CashflowHolding {
    /// The holding's id.
    pub id: String,
    pub symbol: String,
    pub account: String,
    pub currency: String,
    pub qty: Fig<Dec>,
    pub avg: Fig<Dec>,
    pub cost: Fig<Dec>,
    pub mv: Fig<Dec>,
    /// Cash per unit of the latest distribution gone ex.
    pub per: Fig<Dec>,
    pub ytd: Partial,
    pub all: Partial,
    pub next_ex_date: Option<String>,
    pub next_pay_date: Option<String>,
    pub ex_past: bool,
    pub pay_past: bool,
    /// Per unit × payments a year × units.
    pub annual: Fig<Dec>,
    /// That, a month.
    pub per_month: Fig<Dec>,
    pub yoc: Fig<f64>,
    pub current_yield: Fig<f64>,
}

/// A dividend paid.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
pub struct CashflowRow {
    pub id: String,
    pub date: String,
    pub symbol: String,
    pub account: String,
    pub qty: Option<Dec>,
    pub per: Option<Fig<Dec>>,
    pub amount: Dec,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Cashflow {
    pub tiles: Vec<CashflowTile>,
    pub months: Vec<CashflowMonth>,
    pub holdings: Vec<CashflowHolding>,
    /// Projected income a month in CAD, by holding, as the pie draws it.
    pub income: Vec<Slice>,
    pub income_total: Partial,
    pub rows: Vec<CashflowRow>,
    /// The filters in force that the cashflow does not read.
    pub skipped_filters: Vec<String>,
}

// --------------------------------------------------------------------------
// what the filters can be set to
// --------------------------------------------------------------------------

/// A fact only the person can give (`SPEC.md` §2, What you enter), and what it is
/// about: the forms offer these.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = transaction)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    /// The transaction an entry is made against.
    pub transaction: String,
    /// `cost-of-arrival`: what units that arrived cost; `event`: what a corporate
    /// event did to cost.
    pub what: String,
    pub account: String,
    pub account_name: String,
    pub instrument: String,
    pub symbol: String,
    pub currency: String,
    pub day: String,
    /// The units the transaction moved, as the broker states them.
    pub units: Option<Dec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
pub struct AccountOption {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
pub struct InstrumentOption {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub exchange: String,
    pub kind: String,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct Options {
    pub accounts: Vec<AccountOption>,
    pub instruments: Vec<InstrumentOption>,
    pub tags: Vec<String>,
    pub exchanges: Vec<String>,
    pub kinds: Vec<String>,
    pub grades: Vec<String>,
    pub sides: Vec<String>,
    pub results: Vec<String>,
    pub years: Vec<String>,
}

// --------------------------------------------------------------------------
// the document
// --------------------------------------------------------------------------

/// Everything the page shows of the book, for one set of filters.
#[derive(Clone, Debug, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Figures {
    /// Today, in the person's zone.
    pub today: String,
    /// Transactions on the record: none is the first-run page.
    pub activity_count: usize,
    pub options: Options,
    pub kpi: Kpi,
    pub equity: Equity,
    pub years: Vec<YearRow>,
    pub benchmark: BenchmarkRef,
    pub monthly: Vec<MonthlyBar>,
    pub by_symbol: Vec<BySymbolRow>,
    pub grades: Grades,
    pub queue: Vec<QueueRow>,
    pub trades: Vec<Trade>,
    pub positions: Vec<Position>,
    pub portfolio: Portfolio,
    pub cashflow: Cashflow,
    pub accounts: Vec<Account>,
    /// Σ the accounts' values, for the ticket's share of it.
    pub nav_total: Option<Fig<Dec>>,
    /// What waits on the person, whatever the filters.
    pub waiting: Vec<Waiting>,
    /// The market around the book, from its readers (`context`).
    pub markets: super::context::Markets,
    pub sectors: Vec<super::context::ExposureSlice>,
    pub regions: Vec<super::context::ExposureSlice>,
}
