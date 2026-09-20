// The order ticket's derived figures — entry, notional, the stop-loss/take-profit
// prices, the risk/gain/reward, cash-or-margin-after — ported verbatim from the
// legacy tkVals so the numbers match. Pure and testable: it takes the ticket and
// a context (nav, the chosen account) rather than reading global state, so the
// risk math can be verified without a live quote.

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
}
export interface TicketAccount { id: string; name: string; margin: boolean; marginAccountId?: string | null; nav?: number | null }

export interface Ticket {
  step: 'form' | 'review'
  symbol: string; securityId: string; exchange: string
  side: 'BUY' | 'SELL'; accountId: string
  type: string; tif: string
  qty: number | null; limit: number | null; stop: number | null
  sl: { on: boolean; kind: string; price: number | null; pct: number | null; priceUnit: string; trail: number | null; unit: string }
  tp: { on: boolean; price: number | null; pct: number | null; unit: string }
  heldQty: number | null
  text: Record<string, string | null>
  data: TicketData | null
  error: string; busy: boolean; submitError: string
}

export interface ValsCtx { nav: number; accounts: TicketAccount[] }

export function tick(p: number | null): number | null {
  return p == null || !isFinite(p) ? p : +p.toFixed(p >= 1 ? 2 : 4)
}

export function computeVals(t: Ticket, ctx: ValsCtx) {
  const d = t.data || {}
  const q = d.quote || {}
  const buy = t.side === 'BUY'
  const dir = buy ? 1 : -1
  const last = q.last != null ? q.last : null
  const mult = q.multiplier || 1
  const qtyN = t.qty != null ? t.qty : 0
  const limit = t.limit != null ? t.limit : tick(last)
  const stop = t.stop != null ? t.stop : last != null ? +(last * (buy ? 1.02 : 0.98)).toFixed(2) : null
  const entry = t.type === 'MARKET' ? (buy ? (q.ask != null ? q.ask : last) : q.bid != null ? q.bid : last) : t.type === 'STOP' ? stop : limit
  const notional = entry != null ? qtyN * entry * mult : null
  const brackets = buy
  const slOn = brackets && t.sl.on
  const tpOn = brackets && t.tp.on
  const isTrail = t.sl.kind === 'trail'
  const trail = t.sl.trail != null ? t.sl.trail : t.sl.unit === 'pct' ? 5 : entry != null ? +(entry * 0.05).toFixed(2) : null
  const trailDist = entry == null || trail == null ? null : t.sl.unit === 'pct' ? (entry * trail) / 100 : trail
  const slPctIn = t.sl.pct != null ? t.sl.pct : 5
  const slPrice = isTrail
    ? entry != null && trailDist != null ? +(entry - dir * trailDist).toFixed(2) : null
    : t.sl.priceUnit === 'pct'
      ? entry != null ? +(entry * (1 - (dir * slPctIn) / 100)).toFixed(2) : null
      : t.sl.price != null ? t.sl.price : entry != null ? +(entry * (1 - dir * 0.05)).toFixed(2) : null
  const tpPctIn = t.tp.pct != null ? t.tp.pct : 10
  const tpPrice = t.tp.unit === 'pct'
    ? entry != null ? +(entry * (1 + (dir * tpPctIn) / 100)).toFixed(2) : null
    : t.tp.price != null ? t.tp.price : entry != null ? +(entry * (1 + dir * 0.1)).toFixed(2) : null
  const risk = entry != null && slPrice != null ? (isTrail && trailDist != null ? trailDist : dir * (entry - slPrice)) * qtyN * mult : null
  const gain = entry != null && tpPrice != null ? dir * (tpPrice - entry) * qtyN * mult : null
  const slPct = entry ? (isTrail && trailDist != null ? -trailDist / entry : (dir * (slPrice! - entry)) / entry) : null
  const tpPct = entry ? (dir * (tpPrice! - entry)) / entry : null
  const rr = slOn && tpOn && risk != null && risk > 0 && gain != null ? gain / risk : null
  const fx = q.currency === 'USD' && d.fxUsdCad ? d.fxUsdCad : 1
  const cad = notional != null ? notional * fx : null
  const acct = ctx.accounts.find((a) => a.id === t.accountId) || null
  const isMargin = !!(acct && acct.margin)
  const linkedMargin = !!(acct && !acct.margin && acct.marginAccountId)
  const rate = d.marginRate != null ? d.marginRate : 1
  const marginAfter = (isMargin || linkedMargin) && d.marginAvailable != null && cad != null ? d.marginAvailable - dir * cad * rate : null
  const after = isMargin ? marginAfter : d.cash != null && notional != null ? d.cash - dir * notional : null
  const typeWord = ({ MARKET: 'Market', LIMIT: 'Limit', STOP: 'Stop', STOP_LIMIT: 'Stop limit' } as Record<string, string>)[t.type]
  const tifWord = t.tif === 'DAY' ? 'Day' : 'GTC'
  return {
    buy, dir, last, mult, qtyN, limit, stop, entry, notional, slOn, tpOn, isTrail, trail, trailDist, slPctIn,
    slPrice, tpPctIn, tpPrice, risk, gain, slPct, tpPct, rr, cad, nav: ctx.nav, acct, isMargin, linkedMargin,
    marginAfter, after, typeWord, tifWord, q, d,
  }
}
