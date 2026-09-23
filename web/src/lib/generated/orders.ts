// Generated from rust/crates/store/src/orders/types.rs and the server's orders document. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`.

import type { OkOr } from './common'

export type Side = "" | "BUY" | "SELL";

export type OrderType = "" | "MARKET" | "LIMIT" | "STOP" | "STOP_LIMIT";

export type OrderStatus = "" | "dry" | "sending" | "sent" | "pending" | "cancelling" | "filled" | "cancelled" | "expired" | "rejected" | "failed";

export type Role = "" | "entry" | "stop" | "target";

export type Source = "" | "bagholder" | "wealthsimple" | "manual" | "csv";

export type BracketStatus = "" | "waiting" | "armed" | "firing" | "target_placed" | "stopping" | "closing" | "done" | "cancelled";

export type SlKind = "" | "stop" | "trail";

export type TrailUnit = "" | "pct" | "amt";

export type SlMode = "" | "native" | "watched";

export type StopLoss = { kind: SlKind, price: number | null, trail: number | null, trailUnit: TrailUnit, };

export type TakeProfit = { price: number | null, };

export type Order = { id: string, createdAt: string, accountId: string, account: string, securityId: string, symbol: string, currency: string, side: Side, type: OrderType, quantity: number | null, limitPrice: number | null, stopPrice: number | null, tif: string, 
/**
 * The exits asked for with an entry, as the ticket gave them.
 */
stopLoss: StopLoss | null, takeProfit: TakeProfit | null, status: OrderStatus, wsOrderId: string, error: string, updatedAt: string, source: Source, wsStatus: string, filledQty: number | null, avgFill: number | null, submittedAt: string, expiresAt: string, parentId: string, role: Role, fillBookedQty: number | null, };

export type OrderCard = { exchange: string, id: string, createdAt: string, accountId: string, account: string, securityId: string, symbol: string, currency: string, side: Side, type: OrderType, quantity: number | null, limitPrice: number | null, stopPrice: number | null, tif: string, 
/**
 * The exits asked for with an entry, as the ticket gave them.
 */
stopLoss: StopLoss | null, takeProfit: TakeProfit | null, status: OrderStatus, wsOrderId: string, error: string, updatedAt: string, source: Source, wsStatus: string, filledQty: number | null, avgFill: number | null, submittedAt: string, expiresAt: string, parentId: string, role: Role, fillBookedQty: number | null, };

export type Bracket = { id: string, orderId: string, createdAt: string, accountId: string, securityId: string, symbol: string, currency: string, quantity: number | null, tif: string, slKind: SlKind, slPrice: number | null, slTrail: number | null, slTrailUnit: TrailUnit, slOrderId: string, slNative: boolean, slMode: SlMode, highWater: number | null, tpPrice: number | null, tpOrderId: string, status: BracketStatus, outcome: string, error: string, attempts: number, movedAt: string, armedAt: string, seenHeld: boolean, missedAt: string, updatedAt: string, };

export type OrdersDoc = { ok: boolean, orders: Array<OrderCard>, brackets: Array<Bracket>, 
/**
 * Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
 */
live: boolean, refreshedAt: string, };

export type OrderActionAnswer = { ok: boolean, error?: string, id?: string, status?: string, unchanged?: boolean, };

export type RefreshOrdersAnswer = { ok: boolean, skipped?: string, read?: number, added?: number, failed?: number, };

export type Named = { id: string, };

export type Modify = { id: string, quantity: number | string | null, limitPrice: number | string | null, };

export type Adjust = { id: string, leg: string, price?: number | string | null, trail?: number | string | null, remove?: boolean, };

export type RefreshAndOrders = { ok: boolean, skipped?: string, read?: number, added?: number, failed?: number, orders: Array<OrderCard>, brackets: Array<Bracket>, 
/**
 * Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
 */
live: boolean, refreshedAt: string, };

export type QuoteOf = { symbol: string, security: string, account: string, exchange: string, };

export type OrderAccount = { id: string, name: string, type: string, margin: boolean, currency: string, marginAccountId: string, };

export type TicketQuoteDetail = { securityId: string, symbol: string, name: string, exchange: string, currency: string, securityType: string, buyable: boolean, sellable: boolean, tradeEligible: boolean, status: string, last: number | null, bid: number | null, ask: number | null, bidSize: number | null, askSize: number | null, mid: number | null, change: number | null, changePct: number | null, marketStatus: string, quotedAsOf: string, multiplier: number | null, };

export type TicketQuoteOk = { ok: true, quote: TicketQuoteDetail, orderTypes: Array<string>, marginRate: number | null, accounts: Array<OrderAccount>, account: OrderAccount | null, buyingPower: number | null, cash: number | null, marginAvailable: number | null, fxUsdCad: number | null, live: boolean, };

export type TicketQuote = TicketQuoteOk | OkOr;

export type TicketStop = { kind: string | null, price: number | null, trail: number | null, trailUnit: string | null, };

export type TicketTarget = { price: number | null, };

export type Ticket = { symbol: string, securityId: string, accountId: string, side: string, type: string, tif: string | null, quantity: number | null, limitPrice: number | null, stopPrice: number | null, currency: string | null, stopLoss: TicketStop | null, takeProfit: TicketTarget | null, };

export type PlaceTicketAnswer = { ok: boolean, error?: string, id?: string, status?: string, order?: Order, wsOrderId?: string, bracketId?: string, };
