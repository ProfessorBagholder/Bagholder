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
exitDate: string | null, holdDays: number, pnl: Fig<Dec>, pnlCad: Fig<Dec>, pnlPct: number | null, fees: Fig<Dec>, flags: Array<string>, grade: string, thesis: string, tags: Array<string>, };

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
gaps: Array<string>, };

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

export type Equity = { series: Array<Point>, drawdown: Drawdown, annualized: Annualized, 
/**
 * What the series waits on.
 */
gaps: Array<string>, };

export type YearRow = { year: string, r: number, spR: number | null, };

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

export type Slice = { label: string, value: Dec, share: number, 
/**
 * The one holding it is, where it is one.
 */
id: string | null, };

export type Portfolio = { positionCount: number, marketValue: Partial, costBasis: Partial, unrealized: Partial, unrealizedPct: Fig<number | null>, nav: Fig<Dec> | null, navAccounts: number, hasMargin: boolean, marginUsed: Fig<Dec>, marginUsedPct: Fig<number | null>, availableMargin: Fig<Dec> | null, 
/**
 * The margin accounts whose buying power the broker did not state.
 */
availableMarginUnavailable: Array<string>, cash: Fig<Dec>, cashPct: Fig<number | null>, dayChange: Partial | null, dayChangePct: Fig<number | null>, allocation: Array<Slice>, };

export type Account = { id: string, name: string, status: string, 
/**
 * Whether the person trades in it: a self-directed account of cash or
 * margin, not one the broker manages.
 */
tradable: boolean, margin: boolean, 
/**
 * Its value as the broker states it.
 */
nav: Dec | null, };

export type CashflowTile = { label: string, 
/**
 * What was paid over the span (a paid tile).
 */
total: Partial | null, 
/**
 * That, over the months that paid.
 */
perMonth: Fig<Dec | null> | null, 
/**
 * Margin drawn and what it costs a month (the margin tile).
 */
marginUsed: Fig<Dec> | null, interestPerMonth: Fig<Dec | null> | null, 
/**
 * Projected income over cost, and a month of it (the yield tile).
 */
yield: Fig<number | null> | null, projected: Fig<Dec> | null, };

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
income: Array<Slice>, incomeTotal: Fig<Dec>, rows: Array<CashflowRow>, 
/**
 * The filters in force that the cashflow does not read.
 */
skippedFilters: Array<string>, };

export type AccountOption = { id: string, name: string, };

export type InstrumentOption = { id: string, symbol: string, name: string, exchange: string, kind: string, currency: string, };

export type Options = { accounts: Array<AccountOption>, instruments: Array<InstrumentOption>, tags: Array<string>, exchanges: Array<string>, kinds: Array<string>, grades: Array<string>, sides: Array<string>, results: Array<string>, years: Array<string>, };

export type Figures = { 
/**
 * Today, in the person's zone.
 */
today: string, 
/**
 * Transactions on the record: none is the first-run page.
 */
activityCount: number, options: Options, kpi: Kpi, equity: Equity, years: Array<YearRow>, benchmark: BenchmarkRef, monthly: Array<MonthlyBar>, bySymbol: Array<BySymbolRow>, grades: Grades, queue: Array<QueueRow>, trades: Array<Trade>, positions: Array<Position>, portfolio: Portfolio, cashflow: Cashflow, accounts: Array<Account>, 
/**
 * Σ the accounts' values, for the ticket's share of it.
 */
navTotal: Fig<Dec> | null, };

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
