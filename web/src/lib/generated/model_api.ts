// Generated from the server's http::model module. Do not edit: change the Rust type, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_model_api_types`.

import type { Leg, Fill, MarketDates, Position, PositionsSummary, Portfolio, Markets, Filters, Options, Kpi, EquityBlock, YearRow, BenchmarkRef, MonthlyBar, BySymbolRow, Grades, QueueRow, Trade, Cashflow, Unmatched, Account } from './wire'
import type { LegacyNote } from './book'
import type { Status } from './status'

export type JournalEntry = { thesis: string, 
/**
 * A page old enough to send tags as one comma-joined string still reads:
 * each piece around a comma is its own tag.
 */
tags: Array<string>, grade: string, };

export type TradeGroup = { id: string, 
/**
 * Read from the stored row, carried through unread by the model, which
 * treats every saved group as locked regardless.
 */
locked: boolean, members: Array<string>, };

export type TradeQuery = { id: string | null, };

export type TradeAnswer = { ok: boolean, id: string, legs: Array<Leg>, fills: Array<Fill>, };

export type DataSummary = { ok: boolean, path: string, activities: number, firstActivity: string, lastActivity: string, accounts: number, balances: number, navDays: number, securities: number, journal: number, fxDays: number, benchmarkDays: number, filings: number, syncedAt: string, sessionPresent: boolean, };

export type Clear = { journal?: boolean, market?: boolean, session?: boolean, };

export type JournalEntryRequest = { 
/**
 * The trade's or the group's id, as the figures name it.
 */
id: string, thesis: string, 
/**
 * `A`, `B`, `C`, `F`, or empty for none.
 */
grade: string, tags: Array<string>, };

export type JournalAnswer = { ok: true, };

export type Groups = { groups: Array<TradeGroup>, };

export type GroupsAnswer = { ok: boolean, groups: Array<TradeGroup>, };

export type Notes = { notes: { [key in string]: LegacyNote }, };

export type NotesAnswer = { ok: boolean, notes: { [key in string]: LegacyNote }, };

export type ModelQuery = { 
/**
 * the page's filters, as the JSON it keeps them in
 */
filters: string | null, 
/**
 * the trade whose detail to carry
 */
trade: string | null, 
/**
 * `live`: only what a price moves (the legacy page's quote tick)
 */
only: string | null, 
/**
 * with `only=live`: the heatmap is on screen and wants its tiles too
 */
markets: string | null, };

export type ModelLiveAnswer = { ok: boolean, today: string, currency: string, market: MarketDates, positions: Array<Position>, positionsSummary: PositionsSummary, portfolio: Portfolio, markets?: Markets, status: Status, };

export type ModelAnswer = { status: Status, ok: boolean, today: string, syncedAt: string, currency: string, market: MarketDates, filters: Filters, options: Options, kpi: Kpi, equity: EquityBlock, years: Array<YearRow>, benchmark: BenchmarkRef, monthly: Array<MonthlyBar>, bySymbol: Array<BySymbolRow>, grades: Grades, queue: Array<QueueRow>, trades: Array<Trade>, tradeCount: number, tradeTotal: number, positions: Array<Position>, positionsSummary: PositionsSummary, portfolio: Portfolio, markets: Markets, cashflow: Cashflow, unmatched: Array<Unmatched>, accounts: Array<Account>, activityCount: number, };

export type ModelViewAnswer = ModelLiveAnswer | ModelAnswer;

export type FiguresQuery = { 
/**
 * The page's filters, as the JSON it keeps them in (`wire::filters::Filters`).
 */
filters: string | null, };

export type Resync = { 
/**
 * the stream this page holds, from its `hello`
 */
id: number, };

export type EntryRequest = { "entry": "trade", account: string, instrument: string | null, symbol: string, currency: string, day: string, side: string, quantity: string, price: string, fee: string, } | { "entry": "cost-of-arrival", arrival: string, cost: string, acquired: string, } | { "entry": "spin-off", event: string, parent: string, children: Array<ChildShare>, } | { "entry": "return-of-capital", distribution: string, perUnit: string, };

export type ChildShare = { instrument: string, costShare: string, };

export type EntryAnswer = { ok: true, };

export type ImportRequest = { name: string, text: string, 
/**
 * The account its rows go to; empty is the Manual account. A row naming
 * an account of its own goes there.
 */
account: string, };

export type RowNote = { line: number, message: string, };

export type ImportReport = { file: string, layout: string, 
/**
 * The account its rows went to, by its name.
 */
account: string, rows: number, 
/**
 * Rows the book did not hold before.
 */
added: number, 
/**
 * Rows it held already (the same row in an earlier import).
 */
unchanged: number, 
/**
 * Rows linked to the broker's own row for the same fill.
 */
linked: number, 
/**
 * Rows with more than one broker row they could be: not linked.
 */
ambiguous: Array<RowNote>, 
/**
 * Rows kept with a problem, counted in no figure until it is resolved.
 */
problems: Array<RowNote>, };

export type WatchRequest = { path: string, account: string, };

export type WatchStatus = { path: string, watching: boolean, 
/**
 * The account its files go to; empty is the Manual account.
 */
account: string, lastScan: string, 
/**
 * Why the last scan of the folder failed, until one succeeds.
 */
scanError: string, files: Array<WatchedFile>, };

export type WatchedFile = { file: string, size: number, modified: string, scannedAt: string, read: FileOutcome, };

export type FileOutcome = { "outcome": "imported", report: ImportReport, } | { "outcome": "failed", error: string, };
