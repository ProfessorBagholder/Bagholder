// Generated from rust/crates/server/src/wire. Do not edit: change the Rust type, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_figures_types`.

import type { Dec } from '../dec'

export type Fig<T> = T | { 
/**
 * Each thing the figure waits on, as the word `SPEC.md` lists for it.
 */
gaps: Array<string>, };

export type Partial = { total: Fig<Dec>, leftOut: number, };

export type Status = "open" | "closed";

export type Trade = { 
/**
 * The trade's id (a saved group's for a group).
 */
id: string, status: Status, 
/**
 * While open, the holding it is (`Position::id`): opening it opens that page.
 */
position: string | null, symbol: string, 
/**
 * The symbol of what a contract is written on; the symbol itself otherwise.
 */
underlying: string, name: string, exchange: string, kind: string, currency: string, account: string, accountId: string, instrument: string, 
/**
 * The broker's id for the instrument, which an order names (stage 4 moves orders onto the instrument).
 */
security: string, 
/**
 * `SELL` for a long, `COVER` for a short.
 */
side: string, qty: Fig<Dec>, entry: Fig<Dec>, 
/**
 * None before the first sale.
 */
exit: Fig<Dec> | null, entryDate: string, 
/**
 * The last close; none while open.
 */
exitDate: string | null, 
/**
 * The day of its latest fill: the list's order, newest activity first.
 */
lastDate: string, holdDays: number, pnl: Fig<Dec>, pnlCad: Fig<Dec>, pnlPct: number | null, fees: Fig<Dec>, flags: Array<string>, grade: string, thesis: string, tags: Array<string>, };

export type Position = { id: string, 
/**
 * The trade it is, whose journal it shows.
 */
trade: string, symbol: string, underlying: string, name: string, exchange: string, kind: string, currency: string, account: string, accountId: string, instrument: string, security: string, short: boolean, qty: Fig<Dec>, avg: Fig<Dec>, 
/**
 * What the units held cost (long) or brought in (short).
 */
cost: Fig<Dec>, 
/**
 * The price marked at.
 */
last: Fig<Dec>, mv: Fig<Dec>, unreal: Fig<Dec>, unrealPct: number | null, dayChange: Fig<Dec | null>, 
/**
 * The day's move of the price, as a fraction.
 */
percentChange: number | null, opened: string, held: Fig<number>, grade: string, thesis: string, tags: Array<string>, 
/**
 * What the holding waits on.
 */
gaps: Array<string>, 
/**
 * Its lots' marks (`entered`, `deposited`, …).
 */
flags: Array<string>, };

export type Fill = { id: string, 
/**
 * The instant, where the record states one.
 */
when: string | null, date: string, 
/**
 * `BUY`, `SELL`, or empty where the record does not say.
 */
side: string, 
/**
 * What the fill did in the trade (`BUY TO OPEN`, `SELL TO CLOSE`).
 */
sub: string, 
/**
 * Signed: negative for a sale.
 */
qty: Fig<Dec>, price: Fig<Dec>, amount: Fig<Dec>, currency: string, flags: Array<string>, };

export type Detail = { id: string, fills: Array<Fill>, };

export type Kpi = { 
/**
 * Realized P&L in the dates chosen, every sale's; open trades' included.
 */
realized: Fig<Dec>, realizedLeftOut: number, 
/**
 * Closed trades scored.
 */
count: number, leftOut: number, wins: number, losses: number, breakeven: number, winRate: number | null, grossWin: Fig<Dec>, grossLoss: Fig<Dec>, profitFactor: Fig<number | null>, profitFactorInfinite: boolean, expectancy: Fig<Dec | null>, avgWin: Fig<Dec | null>, avgLoss: Fig<Dec | null>, };

export type Point = { d: string, v: Dec, };

export type Drawdown = { pct: number | null, abs: Dec | null, at: string | null, };

export type Annualized = { rate: number | null, count: number, };

export type PnlCurve = { series: Array<Point>, 
/**
 * Parts whose P&L waits on something, left out of every total.
 */
leftOut: number, 
/**
 * What the series waits on: a day whose total does not fit is not drawn.
 */
gaps: Array<string>, };

export type Equity = { 
/**
 * The accounts' value by day: what Returns, the years and Max drawdown read.
 */
series: Array<Point>, drawdown: Drawdown, annualized: Annualized, 
/**
 * What the series waits on.
 */
gaps: Array<string>, 
/**
 * The realized P&L in scope, a running total by day.
 */
pnl: PnlCurve, };

export type YearRow = { year: string, r: number, spR: number | null, 
/**
 * How far the year's return was over (positive) or under (negative) the
 * benchmark's, in percentage points as a fraction: `r` less `sp_r`.
 */
vs: number | null, };

export type BenchmarkRef = { key: string, label: string, };

export type MonthlyBar = { 
/**
 * `YYYY-MM`.
 */
key: string, label: string, value: Fig<Dec>, count: number, tradeIds: Array<string>, };

export type BySymbolRow = { 
/**
 * The underlying instrument.
 */
id: string, symbol: string, pnl: Fig<Dec>, n: number, winRate: number | null, avgHold: number | null, tradeIds: Array<string>, };

export type GradeBucket = { grade: string, n: number, pnl: Fig<Dec>, tradeIds: Array<string>, };

export type Grades = { buckets: Array<GradeBucket>, graded: number, };

export type QueueRow = { id: string, symbol: string, date: string, pnl: Fig<Dec>, missing: string, };

export type Slice = { label: string, value: Fig<Dec>, share: number, 
/**
 * The one holding it is, where it is one.
 */
id: string | null, };

export type Portfolio = { positionCount: number, marketValue: Partial, costBasis: Partial, unrealized: Partial, unrealizedPct: Fig<number | null>, nav: Fig<Dec> | null, navAccounts: number, hasMargin: boolean, marginUsed: Fig<Dec>, marginUsedPct: Fig<number | null>, availableMargin: Fig<Dec> | null, 
/**
 * The margin accounts whose buying power the broker did not state.
 */
availableMarginUnavailable: Array<string>, cash: Fig<Dec>, cashPct: Fig<number | null>, dayChange: Partial | null, dayChangePct: Fig<number | null>, allocation: Array<Slice>, };

export type Account = { id: string, name: string, 
/**
 * The broker's own id for it, which an order names; none for an account
 * kept by hand.
 */
brokerAccount: string | null, status: string, 
/**
 * Whether the person trades in it: a self-directed account of cash or
 * margin, not one the broker manages.
 */
tradable: boolean, margin: boolean, 
/**
 * Its value as the broker states it.
 */
nav: Dec | null, };

export type CashflowTile = { "kind": "paid", label: string, total: Partial, perMonth: Fig<Dec | null>, } | { "kind": "margin", label: string, marginUsed: Fig<Dec>, interestPerMonth: Fig<Dec | null>, } | { "kind": "yield", label: string, yield: Fig<number | null>, projected: Fig<Dec>, };

export type CashflowMonth = { 
/**
 * `YYYY-MM`.
 */
key: string, label: string, 
/**
 * Distributions paid.
 */
value: Fig<Dec>, count: number, 
/**
 * Interest charged, shown positive.
 */
interest: Fig<Dec>, 
/**
 * Distributions less interest charged.
 */
net: Fig<Dec>, };

export type CashflowHolding = { 
/**
 * The holding's id.
 */
id: string, symbol: string, account: string, currency: string, qty: Fig<Dec>, avg: Fig<Dec>, cost: Fig<Dec>, mv: Fig<Dec>, 
/**
 * Cash per unit of the latest distribution gone ex.
 */
per: Fig<Dec>, ytd: Partial, all: Partial, nextExDate: string | null, nextPayDate: string | null, exPast: boolean, payPast: boolean, 
/**
 * Per unit × payments a year × units.
 */
annual: Fig<Dec>, 
/**
 * That, a month.
 */
perMonth: Fig<Dec>, yoc: Fig<number>, currentYield: Fig<number>, };

export type CashflowRow = { id: string, date: string, symbol: string, account: string, qty: Dec | null, per: Fig<Dec> | null, amount: Dec, currency: string, };

export type Cashflow = { tiles: Array<CashflowTile>, months: Array<CashflowMonth>, holdings: Array<CashflowHolding>, 
/**
 * Projected income a month in CAD, by holding, as the pie draws it.
 */
income: Array<Slice>, incomeTotal: Partial, rows: Array<CashflowRow>, };

export type Waiting = { 
/**
 * The transaction an entry is made against.
 */
transaction: string, 
/**
 * `cost-of-arrival`: what units that arrived cost; `event`: what a corporate
 * event did to cost.
 */
what: string, account: string, accountName: string, instrument: string, symbol: string, currency: string, day: string, 
/**
 * The units the transaction moved, as the broker states them.
 */
units: Dec | null, };

export type AccountOption = { id: string, name: string, };

export type InstrumentOption = { id: string, symbol: string, name: string, exchange: string, kind: string, currency: string, 
/**
 * The broker's id for it, which an order names; empty where none is known.
 */
security: string, 
/**
 * The accounts that traded or hold it, by id.
 */
accounts: Array<string>, };

export type Options = { accounts: Array<AccountOption>, instruments: Array<InstrumentOption>, tags: Array<string>, exchanges: Array<string>, kinds: Array<string>, grades: Array<string>, sides: Array<string>, results: Array<string>, years: Array<string>, };

export type BookDoc = { 
/**
 * Today, in the person's zone.
 */
today: string, 
/**
 * Transactions on the record: none is the first-run page.
 */
activityCount: number, options: Options, accounts: Array<Account>, 
/**
 * Σ the accounts' values, for the ticket's share of it.
 */
navTotal: Fig<Dec> | null, 
/**
 * What waits on the person, whatever the filters.
 */
waiting: Array<Waiting>, };

export type DashboardDoc = { kpi: Kpi, equity: Equity, years: Array<YearRow>, benchmark: BenchmarkRef, monthly: Array<MonthlyBar>, bySymbol: Array<BySymbolRow>, grades: Grades, queue: Array<QueueRow>, };

export type PositionsDoc = { portfolio: Portfolio, positions: Array<Position>, };

export type TradesDoc = { 
/**
 * How many trades the filters show, all of them.
 */
total: number, trades: Array<Trade>, };

export type CashflowDoc = { cashflow: Cashflow, 
/**
 * How many dividends the filters show, all of them.
 */
rowsTotal: number, };

export type TradeDoc = { 
/**
 * The id asked for: a document for it with neither is one the figures do not have.
 */
id: string, trade: Trade | null, position: Position | null, };

export type MarketTile = { id: string, symbol: string, exchange: string, 
/**
 * What people call it (`GOLD`, `10Y`), else its symbol.
 */
label: string, name: string, kind: string, last: Dec | null, 
/**
 * The day's change in points.
 */
change: Dec | null, 
/**
 * The day's change, as a fraction.
 */
percentChange: number | null, 
/**
 * Its prices' scale.
 */
decimals: number, 
/**
 * Quoted as 100 minus the rate it settles against: the tile shows the rate.
 */
pricedAsRate: boolean, 
/**
 * That rate, 100 − the price; none for any other instrument, or before a quote.
 */
rate: Dec | null, 
/**
 * The rate's move, against the price's; none where the price's is not known.
 */
rateChange: Dec | null, };

export type WatchItem = { 
/**
 * The instrument, which removing it names.
 */
id: string, symbol: string, exchange: string, name: string, 
/**
 * What its prices are in.
 */
currency: string, last: Dec | null, 
/**
 * The day's change in points.
 */
change: Dec | null, 
/**
 * The day's change, as a fraction.
 */
percentChange: number | null, sector: string, 
/**
 * `Shares`, `Crypto`, or the directory's kind (`Index`, `Future`, …).
 */
kind: string, 
/**
 * The holding it is, where the book holds it: a click opens that page.
 */
positionId: string | null, };

export type DirectoryEntry = { 
/**
 * `SYMBOL@VENUE`.
 */
key: string, symbol: string, label: string, name: string, exchange: string, kind: string, aliases: Array<string>, };

export type MarketsDoc = { tiles: Array<MarketTile>, watchlist: Array<WatchItem>, directory: Array<DirectoryEntry>, };

export type HeatTile = { 
/**
 * Its own in the heatmap: its symbol, told apart from another of the same by
 * its venue, a folded remainder by its sector. A tile keeps it from one
 * universe or sizing to the next, so it travels.
 */
key: string, 
/**
 * The holding it is, where it is one: a click opens that page.
 */
id: string | null, symbol: string, exchange: string, currency: string, name: string, 
/**
 * What it is sized by: its value in CAD, its index weight or market cap, or
 * one under `Equal`.
 */
value: Dec, 
/**
 * The day's change, as a fraction; for `Other (N)`, its tiles' value-weighted.
 */
percentChange: number | null, 
/**
 * Several small tiles folded into one, which opens nothing.
 */
other: boolean, };

export type HeatBlock = { label: string, 
/**
 * Σ its tiles' values.
 */
value: Dec, 
/**
 * Its tiles' value-weighted day change, as a fraction.
 */
percentChange: number | null, tiles: Array<HeatTile>, };

export type HeatCounts = { holdings: number, watchlist: number, both: number, ca: number, us: number, intl: number, };

export type HeatmapDoc = { universe: string, size: string, blocks: Array<HeatBlock>, counts: HeatCounts, };

export type ExposureDoc = { sectors: Array<Slice>, regions: Array<Slice>, };

export type NewsTag = { 
/**
 * Its bare ticker.
 */
symbol: string, exchange: string, held: boolean, watched: boolean, 
/**
 * The listing's day change, as a fraction: the holding's where the book
 * holds it, the watched listing's otherwise.
 */
percentChange: number | null, 
/**
 * The holding it is, where the book holds it.
 */
positionId: string | null, };

export type Filed = { id: string, symbol: string, source: string, url: string, 
/**
 * Its title is still being read.
 */
pending: boolean, };

export type Headline = { id: string, headline: string, 
/**
 * The wire or publisher, or the regulator for a filed release.
 */
source: string, url: string, publishedAt: string, 
/**
 * From the market's own feed rather than a listing's.
 */
market: boolean, 
/**
 * `story`, or `release` for a company's own.
 */
kind: string, tags: Array<NewsTag>, 
/**
 * A release as the issuer filed it, beside the wires'.
 */
filed: Filed | null, };

export type ChipKinds = { stories: number, releases: number, };

export type HeadlinesDoc = { items: Array<Headline>, total: number, 
/**
 * Under a chip: how many of its items each tab holds.
 */
chip: ChipKinds | null, 
/**
 * Why the issuers' filed releases are not on the Releases tab, when they
 * could not be read.
 */
filedFailed: string | null, };

export type Range = { 
/**
 * `>` or `<`.
 */
op: string, 
/**
 * The bound as decimal text, or none.
 */
v: Dec | null, };

export type Filters = { 
/**
 * `account` (account ids), `symbol` (instrument ids), `grade`, `tag`, `kind`,
 * `exchange`, `side`, `result`.
 */
lists: { [key in string]: Array<string> }, 
/**
 * `price`, `hold`, `pnl`, `qty`.
 */
ranges: { [key in string]: Range }, preset: string, years: Array<string>, from: string, to: string, search: string, benchmark: string, };

export type Dir = "asc" | "desc";

export type Sort = { key: string, dir: Dir, };

export type Params = { 
/**
 * The page's filters: every kind but `book` and `trade:` reads them.
 */
filters: Filters | null, 
/**
 * The trades' or the dividends' order.
 */
sort: Sort | null, 
/**
 * How many rows of a long list the page shows: as far as it has scrolled.
 */
limit: number | null, 
/**
 * The heatmap's universe: `holdings`, `watchlist`, `both`, `ca`, `us`, `intl`.
 */
universe: string | null, 
/**
 * The heatmap's sizing: `value` or `equal`.
 */
size: string | null, 
/**
 * The News card's scope: `all`, `holdings`, `watchlist`.
 */
scope: string | null, 
/**
 * The News card's tab: `stories` or `releases`.
 */
kind: string | null, 
/**
 * The News card's chip: a listing's bare ticker and its venue.
 */
symbol: string | null, exchange: string | null, 
/**
 * What is typed in the News card's box.
 */
query: string | null, };
