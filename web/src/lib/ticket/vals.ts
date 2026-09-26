// The order ticket's form state and how it is shown. Its figures -- the working
// price, the stop loss and take profit, what is at risk and what the target gains,
// the order's value in CAD, the cash or margin after -- are the server's, worked out
// exactly from what the ticket holds (`POST /api/order/preview`,
// rust/crates/server/src/orders/preview.rs); the page does no money arithmetic.

import { money, signedMoney, px } from '../fmt'
import { dec, sign, type Dec } from '../dec'
import type { Preview, PreviewRequest } from '../generated/orders'

export interface Quote {
  symbol?: string
  last?: number | null; ask?: number | null; bid?: number | null; mid?: number | null
  change?: number | null; changePct?: number | null; currency?: string
  multiplier?: number | null; bidSize?: number | null; askSize?: number | null
  name?: string; exchange?: string; securityId?: string
}
export interface TicketData {
  quote?: Quote; orderTypes?: string[]; fxUsdCad?: number | null
  marginRate?: number | null; marginAvailable?: number | null; cash?: number | null
  buyingPower?: number | null; accounts?: TicketAccount[]
}
export interface TicketAccount { id: string; name: string; type?: string; margin: boolean; marginAccountId?: string | null; nav?: number | null }

export interface Ticket {
  step: 'form' | 'review'
  symbol: string; securityId: string; exchange: string
  side: 'BUY' | 'SELL'; accountId: string
  type: string; tif: string
  qty: number | null; limit: number | null; stop: number | null
  sl: { on: boolean; kind: string; price: number | null; pct: number | null; priceUnit: string; trail: number | null; unit: string }
  tp: { on: boolean; price: number | null; pct: number | null; unit: string }
  text: Record<string, string | null>
  data: TicketData | null
  error: string; busy: boolean; submitError: string
}

export interface ValsCtx { nav: Dec | null; accounts: TicketAccount[] }

// a number the ticket holds, as the decimal text the preview reads (never exponent form);
// the order API's own numbers stay numbers
const txt = (n: number | null | undefined): string | null => {
  if (n == null || !isFinite(n)) return null
  const s = String(n)
  return /e/i.test(s) ? n.toFixed(20).replace(/0+$/, '').replace(/\.$/, '') : s
}

/** What the server works the ticket's figures from (`POST /api/order/preview`). */
export function previewRequest(t: Ticket, ctx: ValsCtx): PreviewRequest {
  const d = t.data || {}
  const q = d.quote || {}
  const acct = ctx.accounts.find((a) => a.id === t.accountId) || null
  return {
    side: t.side, type: t.type,
    quantity: txt(t.qty), amount: t.text.amt ?? null, limit: txt(t.limit), stop: txt(t.stop),
    sl: { on: t.sl.on, kind: t.sl.kind, priceUnit: t.sl.priceUnit, price: txt(t.sl.price), pct: txt(t.sl.pct), trail: txt(t.sl.trail), unit: t.sl.unit },
    tp: { on: t.tp.on, unit: t.tp.unit, price: txt(t.tp.price), pct: txt(t.tp.pct) },
    quote: { last: txt(q.last), ask: txt(q.ask), bid: txt(q.bid), multiplier: txt(q.multiplier), currency: q.currency ?? '' },
    fxUsdCad: txt(d.fxUsdCad), marginRate: txt(d.marginRate), marginAvailable: txt(d.marginAvailable), cash: txt(d.cash), buyingPower: txt(d.buyingPower),
    margin: !!(acct && acct.margin), linkedMargin: !!(acct && !acct.margin && acct.marginAccountId), nav: ctx.nav,
  }
}

/** The ticket as the form and the review show it: its words, and the server's figures (`p`, none until the first answer). */
export function view(t: Ticket, ctx: ValsCtx, p: Preview | null) {
  const d = t.data || {}
  const q = d.quote || {}
  const buy = t.side === 'BUY'
  const acct = ctx.accounts.find((a) => a.id === t.accountId) || null
  const isMargin = !!(acct && acct.margin)
  const linkedMargin = !!(acct && !acct.margin && acct.marginAccountId) // collateral for a margin account: its margin moves too
  const typeWord = ({ MARKET: 'Market', LIMIT: 'Limit', STOP: 'Stop', STOP_LIMIT: 'Stop limit' } as Record<string, string>)[t.type]
  const tifWord = t.tif === 'DAY' ? 'Day' : 'GTC'
  const trail = p?.trail ?? null
  const trailWord = t.sl.unit === 'pct' ? plain(trail) + '%' : px(trail)
  return {
    buy, q, d, acct, isMargin, linkedMargin, typeWord, tifWord, trailWord,
    qty: p?.quantity ?? txtDec(t.qty),
    entry: p?.entry ?? null, limit: p?.limit ?? null, stop: p?.stop ?? null, notional: p?.notional ?? null,
    slOn: p ? p.stopLossOn : buy && t.sl.on, tpOn: p ? p.takeProfitOn : buy && t.tp.on, isTrail: t.sl.kind === 'trail',
    trail, trailDist: p?.trailDistance ?? null, slPctIn: p?.stopLossPctIn ?? null, slPrice: p?.stopLossPrice ?? null,
    tpPctIn: p?.takeProfitPctIn ?? null, tpPrice: p?.takeProfitPrice ?? null, risk: p?.risk ?? null, gain: p?.gain ?? null,
    slPct: p?.stopLossPct ?? null, tpPct: p?.takeProfitPct ?? null, rr: p?.rewardToRisk ?? null,
    cad: p?.cad ?? null, positionShare: p?.positionShare ?? null, marginAfter: p?.marginAfter ?? null, after: p?.after ?? null,
    maxQuantity: p?.maxQuantity ?? null,
  }
}

const txtDec = (n: number | null | undefined): Dec | null => {
  const s = txt(n)
  return s == null ? null : dec(s)
}

// whether exact decimal text is a whole number
const whole = (d: Dec) => !d.includes('.') || /\.0*$/.test(d)
// the text up to two decimals, trailing zeros dropped: a percent or a count as typed
const upTo2 = (d: Dec) => {
  const [i, f = ''] = d.replace(/^-/, '').split('.')
  const two = (f + '00').slice(0, 2).replace(/0+$/, '')
  return (sign(d) < 0 ? '-' : '') + i + (two ? '.' + two : '')
}
/** A whole number as it is, otherwise up to two decimals: a percent or a count as the ticket shows it. */
export function plain(n: Dec | number | null | undefined): string {
  if (n == null) return '—'
  if (typeof n === 'number') return Number.isInteger(n) ? String(n) : String(+n.toFixed(2))
  return whole(n) ? n.replace(/\.0*$/, '') : upTo2(n)
}
/** Money with no decimals when the amount is whole, two otherwise. */
export function amt(n: Dec | null | undefined): string {
  return n == null ? '—' : money(n, '', whole(n) ? 0 : 2)
}
export function sAmt(n: Dec | null | undefined): string {
  return n == null ? '—' : signedMoney(n, '', whole(n) ? 0 : 2)
}
/** The number typed into a field, punctuation stripped: what the ticket holds while it is typed. */
export function parseNum(v: string): number | null {
  const n = parseFloat(String(v).replace(/[^0-9.\-]/g, ''))
  return isNaN(n) ? null : n
}
