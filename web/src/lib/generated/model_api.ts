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

export type Clear = { kinds: Array<Kind>, };

export type ClearAnswer = { ok: boolean, };

export type Kind = "broker" | "entries" | "journal" | "market" | "orders" | "settings" | "login";

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

export type Resync = { 
/**
 * the stream this page holds, from its `hello`
 */
id: number, };

export type TradesQuery = { 
/**
 * The page's filters, as the JSON it keeps them in (`wire::filters::Filters`).
 */
filters: string | null, 
/**
 * The column the list is sorted by, as the page's header names it.
 */
sort: string | null, 
/**
 * `asc` or `desc`.
 */
dir: string | null, };

export type ViewQuery = { 
/**
 * The subscription's key: `book`, `dashboard`, `positions`, `trades`,
 * `cashflow`, `exposure`, `markets`, `trade:<id>`.
 */
key: string | null, 
/**
 * Its parameters, as the JSON a subscription is asked with (`views::Params`).
 */
params: string | null, };

export type ViewAnswer = unknown;

export type EntryRequest = { "entry": "trade", account: string, instrument: string | null, symbol: string, currency: string, day: string, side: string, quantity: string, price: string, fee: string, } | { "entry": "cost-of-arrival", arrival: string, cost: string, acquired: string, } | { "entry": "spin-off", event: string, parent: string, children: Array<ChildShare>, } | { "entry": "return-of-capital", distribution: string, perUnit: string, };

export type ChildShare = { instrument: string, costShare: string, };

export type EntryAnswer = { ok: true, };

export type ImportQuery = { name: string, 
/**
 * Empty is the Manual account.
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
 * Rows with more than one broker row they could be: not linked. The first
 * `REPORT_NOTES` by line, and how many there are.
 */
ambiguous: Array<RowNote>, ambiguousRows: number, 
/**
 * Rows kept with a problem, counted in no figure until it is resolved: the first
 * `REPORT_NOTES` by line, and how many there are. Every one is in the book, and
 * the header says them (`status::unread_rows`).
 */
problems: Array<RowNote>, problemRows: number, 
/**
 * The person stopped it after `rows` of the file's rows: those are kept, and
 * importing the file again goes on from them (a row kept already is unchanged).
 */
stopped: boolean, };

export type ImportAccepted = { id: string, };

export type ImportTold = { id: string, };

export type WatchRequest = { path: string, account: string, };

export type WatchStatus = { path: string, watching: boolean, 
/**
 * The account its files go to; empty is the Manual account.
 */
account: string, lastScan: string, 
/**
 * Why the last scan of the folder failed, until one succeeds.
 */
scanError: string, 
/**
 * The rows the last scan's files added that the book did not hold.
 */
lastScanAdded: number, files: Array<WatchedFile>, };

export type WatchedFile = { file: string, size: number, modified: string, scannedAt: string, read: FileOutcome, };

export type FileOutcome = { "outcome": "imported", report: ImportReport, } | { "outcome": "failed", error: string, };
