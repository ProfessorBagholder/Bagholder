import { describe, it, expect } from 'vitest'
import { previewRequest, view, plain, amt, sAmt, type Ticket, type ValsCtx } from './vals'
import type { Preview } from '../generated/orders'
import type { Dec } from '../dec'

// The ticket's figures are the server's (rust/crates/server/src/orders/preview.rs, its
// own tests hold the arithmetic). Here: what the page sends for them, and how it
// shows what comes back, without doing any money arithmetic itself.

function ticket(over: Partial<Ticket> = {}): Ticket {
  return {
    step: 'form', symbol: 'NVDA', securityId: '', exchange: 'NASDAQ',
    side: 'BUY', accountId: 'a', type: 'LIMIT', tif: 'DAY',
    qty: 10, limit: 100, stop: null,
    sl: { on: true, kind: 'stop', price: null, pct: 5, priceUnit: 'pct', trail: null, unit: 'pct' },
    tp: { on: true, price: null, pct: 10, unit: 'pct' },
    heldQty: null, text: {}, data: { quote: { last: 100, ask: 101, bid: 99, mid: 100, currency: 'USD', multiplier: 1 }, cash: 5000, fxUsdCad: 1.3712 },
    error: '', busy: false, submitError: '',
    ...over,
  }
}
const ctx: ValsCtx = { nav: '10000' as Dec, accounts: [{ id: 'a', name: 'A', margin: false, marginAccountId: 'm' }] }

describe('the ticket asks the server for its figures', () => {
  it('with what it holds as decimal text, never a figure of its own', () => {
    const r = previewRequest(ticket(), ctx)
    expect(r).toMatchObject({ side: 'BUY', type: 'LIMIT', quantity: '10', limit: '100', stop: null, nav: '10000', cash: '5000', fxUsdCad: '1.3712', margin: false, linkedMargin: true })
    expect(r.sl).toEqual({ on: true, kind: 'stop', priceUnit: 'pct', price: null, pct: '5', trail: null, unit: 'pct' })
    expect(r.quote).toEqual({ last: '100', ask: '101', bid: '99', multiplier: '1', currency: 'USD' })
  })

  it('a tiny number goes as plain decimal text, never exponent form', () => {
    expect(previewRequest(ticket({ limit: 0.0000001 }), ctx).limit).toBe('0.0000001')
  })

  it('an amount typed goes as typed, for the server to turn into units', () => {
    expect(previewRequest(ticket({ text: { amt: '1,055' } }), ctx).amount).toBe('1,055')
  })
})

describe('the ticket shows the answer', () => {
  const p = { entry: '100', limit: '100', stop: '102', quantity: '10', notional: '1000', stopLossOn: true, takeProfitOn: true, trailing: false, trail: null, trailDistance: null, stopLossPctIn: '5', stopLossPrice: '95', takeProfitPctIn: '10', takeProfitPrice: '110', risk: '50', gain: '100', stopLossPct: -0.05, takeProfitPct: 0.1, rewardToRisk: 2, cad: '1371.2', positionShare: 0.13712, marginAfter: null, after: '4000', maxQuantity: null } as unknown as Preview

  it('as the server stated it', () => {
    const v = view(ticket(), ctx, p)
    expect([v.entry, v.slPrice, v.tpPrice, v.risk, v.gain, v.rr, v.after]).toEqual(['100', '95', '110', '50', '100', 2, '4000'])
    expect(v.linkedMargin).toBe(true)
  })

  it('nothing before the first answer', () => {
    const v = view(ticket(), ctx, null)
    expect([v.entry, v.risk, v.after]).toEqual([null, null, null])
    expect(v.qty).toBe('10')
  })

  it('amounts whole without decimals, a percent as typed', () => {
    expect([amt('1000' as Dec), amt('1371.2' as Dec), sAmt('-50' as Dec), plain('5.50' as Dec), plain('10' as Dec)]).toEqual(['$1,000', '$1,371.20', '−$50', '5.5', '10'])
  })
})
