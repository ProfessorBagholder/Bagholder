// Generated from the server's http::model module. Do not edit: change the Rust type, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_model_api_types`.

import type { Leg, Fill } from './wire'
import type { LegacyNote } from './book'

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

export type JournalEntryRequest = { id: string | null, thesis: string, 
/**
 * A page old enough to send tags as one comma-joined string still reads:
 * each piece around a comma is its own tag.
 */
tags: Array<string>, grade: string, };

export type JournalAnswer = { ok: boolean, journal: { [key in string]: JournalEntry }, };

export type Groups = { groups: Array<TradeGroup>, };

export type GroupsAnswer = { ok: boolean, groups: Array<TradeGroup>, };

export type Notes = { notes: { [key in string]: LegacyNote }, };

export type NotesAnswer = { ok: boolean, notes: { [key in string]: LegacyNote }, };

export type Import = { text: string, name: string, };
