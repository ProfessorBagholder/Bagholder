import { describe, it, expect } from 'vitest'
import { computeVals, type Ticket, type ValsCtx } from './vals'

function ticket(over: Partial<Ticket> = {}): Ticket {
  return {
    step: 'form', symbol: 'NVDA', securityId: '', exchange: 'NASDAQ',
    side: 'BUY', accountId: 'a', type: 'LIMIT', tif: 'DAY',
    qty: 10, limit: 100, stop: null,
    sl: { on: true, kind: 'stop', price: null, pct: 5, priceUnit: 'pct', trail: null, unit: 'pct' },
    tp: { on: true, price: null, pct: 10, unit: 'pct' },
    heldQty: null, text: {}, data: { quote: { last: 100, ask: 101, bid: 99, mid: 100, currency: 'USD', multiplier: 1 } },
    error: '', busy: false, submitError: '',
    ...over,
  }
}
const ctx: ValsCtx = { nav: 10000, accounts: [{ id: 'a', name: 'A', margin: false }] }

describe('order ticket risk math', () => {
  it('a BUY limit with a 5% stop and 10% target computes risk, gain and 1:2 R:R', () => {
    const v = computeVals(ticket(), ctx)
    expect(v.entry).toBe(100)
    expect(v.notional).toBe(1000)
    expect(v.slPrice).toBe(95)
    expect(v.tpPrice).toBe(110)
    expect(v.risk).toBe(50)
    expect(v.gain).toBe(100)
    expect(v.rr).toBe(2) // 1:2
    expect(v.slPct).toBeCloseTo(-0.05, 6)
    expect(v.tpPct).toBeCloseTo(0.1, 6)
  })

  it('a trailing stop loses exactly its distance', () => {
    const v = computeVals(ticket({ sl: { on: true, kind: 'trail', price: null, pct: null, priceUnit: 'amt', trail: 5, unit: 'pct' } }), ctx)
    // 5% trail off a $100 entry = $5 distance, stop starts at 95, risk = 5 * 10 shares
    expect(v.isTrail).toBe(true)
    expect(v.slPrice).toBe(95)
    expect(v.risk).toBe(50)
  })

  it('a SELL shows no brackets and no risk', () => {
    const v = computeVals(ticket({ side: 'SELL' }), ctx)
    expect(v.slOn).toBe(false)
    expect(v.tpOn).toBe(false)
    expect(v.rr).toBe(null)
  })

  it('cash after a buy subtracts the notional', () => {
    const v = computeVals(ticket(), { ...ctx, accounts: [{ id: 'a', name: 'A', margin: false }] })
    const t = ticket()
    t.data!.cash = 5000
    expect(computeVals(t, ctx).after).toBe(4000) // 5000 - 1000
  })
})
