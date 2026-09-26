// Generated from the server's orders document and routes (rust/crates/server/src/orders). Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`.

import type { OkOr } from './common'
import type { Dec } from '../dec'
import type { Fig } from './figures'

export type Filled = { quantity: Dec, average: Dec | null, };

export type Leg = { 
/**
 * `sl` or `tp`.
 */
key: string, quantity: Dec, 
/**
 * The level: the stop's (a trailing stop's current one) or the target.
 */
level: Dec, 
/**
 * A trailing stop's trail: a percent, or an amount per share.
 */
trailPct: Dec | null, trailAmount: Dec | null, 
/**
 * The exit's fill, when this leg is how the bracket ended.
 */
filled: Filled | null, 
/**
 * `Placing`, `Cancelling`, `Retrying · <reason>`, `Watching`, `Filled`,
 * `Cancelled`, `Off`, or nothing.
 */
note: string, 
/**
 * Quantity × level × the contract size; a fill's own amount once it filled.
 */
amount: Fig<Dec>, };

export type OrderCard = { id: string, 
/**
 * The broker's id for the account.
 */
account: string, exchange: string, symbol: string, 
/**
 * `buy` or `sell`.
 */
side: string, 
/**
 * `market`, `limit`, `stop` or `stop-limit`.
 */
kind: string, 
/**
 * `day` or `until-cancel`; none when Wealthsimple has not said.
 */
tif: string | null, quantity: Dec, limitPrice: Dec | null, stopPrice: Dec | null, 
/**
 * Where it stands (`bagholder_core::order::OrderState`).
 */
state: string, filled: Dec, average: Dec | null, 
/**
 * Why it was refused, failed or is not confirmed, in the words recorded.
 */
why: string | null, 
/**
 * Its value in the instrument's currency: the quantity at its price, what it
 * filled for once filled; none for a market order not filled.
 */
value: Fig<Dec> | null, 
/**
 * The value is the fill's price guessed at: a market order's.
 */
approx: boolean, 
/**
 * `pending`, `filled` or `cancelled`.
 */
tab: string, at: string, 
/**
 * Edit and Cancel act on it; Edit is dimmed where Wealthsimple takes no change.
 */
live: boolean, editable: boolean, 
/**
 * The legs of the bracket waiting for this order to fill.
 */
legs: Array<Leg>, };

export type BracketCard = { id: string, account: string, exchange: string, symbol: string, tab: string, 
/**
 * When it armed while live; when it ended after.
 */
at: string, live: boolean, 
/**
 * What was paid for the shares under it.
 */
value: Fig<Dec>, legs: Array<Leg>, 
/**
 * On the Cancelled tab: `Cancelled` when the person ended it, `Off` otherwise.
 */
endWord: string | null, 
/**
 * What its editor starts from.
 */
stopLevel: Dec | null, trailPct: Dec | null, trailAmount: Dec | null, target: Dec | null, };

export type OrdersDoc = { ok: boolean, 
/**
 * Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
 */
live: boolean, refreshedAt: string | null, orders: Array<OrderCard>, brackets: Array<BracketCard>, 
/**
 * Why the document could not be read, when it could not.
 */
error: string | null, };

export type PageDec = number | string | null;

export type OrderActionAnswer = { ok: boolean, error?: string, id?: string, status?: string, unchanged?: boolean, };

export type RefreshOrdersAnswer = { ok: boolean, skipped?: string, read: number, failed: number, };

export type Named = { id: string, };

export type Modify = { id: string, quantity: PageDec, limitPrice: PageDec, };

export type Adjust = { id: string, leg: string, price: PageDec, trail: PageDec, remove?: boolean, };

export type RefreshAndOrders = { read: RefreshOrdersAnswer, orders: OrdersDoc, };

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

export type TicketStop = { 
/**
 * `stop` or `trail`.
 */
kind: string | null, price: PageDec, trail: PageDec, 
/**
 * `pct` or `amt`.
 */
trailUnit: string | null, };

export type TicketTarget = { price: PageDec, };

export type Ticket = { symbol: string, securityId: string, accountId: string, 
/**
 * `BUY` or `SELL`.
 */
side: string, 
/**
 * `MARKET`, `LIMIT`, `STOP` or `STOP_LIMIT`.
 */
type: string, 
/**
 * `DAY` or `UNTIL_CANCEL`; a day order when absent.
 */
tif: string | null, quantity: PageDec, limitPrice: PageDec, stopPrice: PageDec, currency: string | null, stopLoss: TicketStop | null, takeProfit: TicketTarget | null, };

export type PlaceTicketAnswer = { ok: boolean, error?: string, id?: string, 
/**
 * Where the order stands after the answer: `dry`, `unconfirmed`, `pending`, …
 */
status?: string, bracketId?: string, };

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
 * Each leg's amount, the quantity at its price, in the instrument's currency: what
 * a leg row shows beside its level (the Orders panel's draft card).
 */
stopLossValue: Dec | null, takeProfitValue: Dec | null, 
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
