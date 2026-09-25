// Generated from rust/crates/store/src/activities.rs, book.rs,
// broker.rs and the server's book-append answer. Do not edit: change the Rust type, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_book_types`.

import type { TradeGroup } from './model_api'

export type ActivityRow = { id: string, canonicalId: string | null, occurredAt: string, transactionDate: string, settlementDate: string, accountId: string, bookId: string, fifoId: string, accountType: string, activityType: string, activitySubType: string, description: string, direction: string, symbol: string, name: string, currency: string, quantity: number, unitPrice: number, commission: number, netCashAmount: number, category: string, balance: number | null, source: string, rawType: string, aftType: string, counterSymbol: string, securityId: string | null, };

export type Appended = { ok: boolean, added: number, duplicates: number, activity: ActivityRow | null, activities: Array<ActivityRow> | null, };

export type BookAppend = { activities: Array<ActivityRow>, activity: ActivityRow | null, side: string, qty: number | null, quantity: number | null, price: number | null, unitPrice: number | null, date: string, transactionDate: string, occurredAt: string, symbol: string, currency: string, accountId: string, account: string, accountType: string, commission: number | null, };

export type LegacyNote = { thesis: string, tag: string, grade: string, tradeId: string, };

export type BrokerAccount = { id: string, nickname: string, unifiedAccountType: string, currency: string, status: string, type: string, netLiquidationValue: number | null, 
/**
 * The margin account this one backs, when it is Margin Boost collateral.
 */
marginAccountId: string, };

export type Security = { id: string, symbol: string, name: string, primaryExchange: string, primaryMic: string, currency: string, 
/**
 * An option's record points at what it is an option on.
 */
underlyingId: string, };

export type BookBalance = { accountId: string | null, custodianAccountId: string | null, securityId: string | null, quantity: number | null, };

export type BookNav = { date: string, equity: number | null, currency: string, netDeposits?: number, };

export type Book = { ok: boolean, activities: Array<ActivityRow>, accounts: Array<BrokerAccount>, balances: Array<BookBalance>, navHistory: Array<BookNav>, navByAccount: { [key in string]: Array<BookNav> }, syncedAt: string, tradeGroups: Array<TradeGroup>, notes: { [key in string]: LegacyNote }, securities: Array<Security>, };
