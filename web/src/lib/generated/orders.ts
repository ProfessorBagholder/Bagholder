// Generated from rust/crates/store/src/orders/types.rs and the server's orders document. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`.

import type { OkOr } from './common'
import type { Dec } from '../dec'
import type { Fig } from './figures'

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

export type OrderAccount = { id: string, name: string, margin: boolean, 
/**
 * The margin account whose margin an order here moves: its own for a margin
 * account, the margin account it is linked to for one that is collateral.
 */
marginAccountId: string, };

export type TicketQuoteDetail = { securityId: string, symbol: string, name: string, exchange: string, currency: string, securityType: string, buyable: boolean, sellable: boolean, tradeEligible: boolean, status: string, last: number | null, bid: number | null, ask: number | null, bidSize: number | null, askSize: number | null, mid: number | null, change: number | null, changePct: number | null, marketStatus: string, quotedAsOf: string, multiplier: number | null, };

export type TicketQuoteOk = { ok: true, quote: TicketQuoteDetail, orderTypes: Array<string>, marginRate: number | null, accounts: Array<OrderAccount>, account: OrderAccount | null, buyingPower: number | null, cash: number | null, marginAvailable: number | null, live: boolean, };

export type TicketQuote = TicketQuoteOk | OkOr;

export type TicketStop = { kind: string | null, price: number | null, trail: number | null, trailUnit: string | null, };

export type TicketTarget = { price: number | null, };

export type Ticket = { symbol: string, securityId: string, accountId: string, side: string, type: string, tif: string | null, quantity: number | null, limitPrice: number | null, stopPrice: number | null, currency: string | null, stopLoss: TicketStop | null, takeProfit: TicketTarget | null, };

export type PlaceTicketAnswer = { ok: boolean, error?: string, id?: string, status?: string, order?: Order, wsOrderId?: string, bracketId?: string, };

export type StopInput = { on: boolean, 
/**
 * `stop` or `trail`.
 */
kind: string, 
/**
 * `amt` or `pct`: a fixed stop typed as a price, or as a percent below the working price.
 */
priceUnit: string, price: string | null, pct: string | null, trail: string | null, 
/**
 * `pct` or `amt`: the trail's distance as a percent or an amount.
 */
unit: string, };

export type TargetInput = { on: boolean, 
/**
 * `amt` or `pct`.
 */
unit: string, price: string | null, pct: string | null, };

export type QuoteInput = { last: string | null, ask: string | null, bid: string | null, 
/**
 * Shares a unit: a contract's size, 1 otherwise.
 */
multiplier: string | null, currency: string, };

export type PreviewRequest = { 
/**
 * `BUY` or `SELL`.
 */
side: string, 
/**
 * `MARKET`, `LIMIT`, `STOP` or `STOP_LIMIT`.
 */
type: string, quantity: string | null, 
/**
 * The order's value typed in Amount: the quantity becomes the whole units it buys.
 */
amount: string | null, limit: string | null, stop: string | null, sl: StopInput, tp: TargetInput, quote: QuoteInput, marginRate: string | null, marginAvailable: string | null, cash: string | null, buyingPower: string | null, 
/**
 * The account borrows; `linked_margin`, it backs a margin account.
 */
margin: boolean, linkedMargin: boolean, 
/**
 * The accounts' value, CAD.
 */
nav: string | null, };

export type Preview = { 
/**
 * The price the order works at: the ask or bid for a market order, the stop
 * for a stop order, the limit otherwise.
 */
entry: Dec | null, 
/**
 * The limit as the order carries it: typed, else the last price at an order's tick.
 */
limit: Dec | null, 
/**
 * The stop as the order carries it: typed, else 2% through the last price.
 */
stop: Dec | null, quantity: Dec, notional: Dec | null, stopLossOn: boolean, takeProfitOn: boolean, trailing: boolean, 
/**
 * The trail's distance as typed (a percent or an amount), and in price.
 */
trail: Dec | null, trailDistance: Dec | null, stopLossPctIn: Dec, stopLossPrice: Dec | null, takeProfitPctIn: Dec, takeProfitPrice: Dec | null, 
/**
 * What the stop loss loses and the target gains, in the instrument's currency.
 */
risk: Dec | null, gain: Dec | null, stopLossPct: number | null, takeProfitPct: number | null, rewardToRisk: number | null, 
/**
 * The order's value in CAD, and its share of the accounts' value: waiting where
 * the quote's currency has no rate.
 */
cad: Fig<Dec> | null, positionShare: Fig<number> | null, 
/**
 * The margin account's available margin after the order.
 */
marginAfter: Fig<Dec> | null, 
/**
 * What the review's last line shows: available margin after on a margin
 * account, cash after on any other.
 */
after: Fig<Dec> | null, 
/**
 * The whole units the buying power covers at the working price (a Buy).
 */
maxQuantity: Dec | null, };
