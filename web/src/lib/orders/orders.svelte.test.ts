import { describe, it, expect, beforeEach } from 'vitest'
import { ordersStore, bracketLegs, orderDetailLine, type Order, type Bracket } from './orders.svelte'

// SPEC.md §4, Orders: what a card's second line and a bracket's leg rows say in each
// state the server can send. The rules, not examples: every ended state and every
// state with a request out is walked.

const order = (over: Partial<Order>): Order => ({
  id: 'o', createdAt: '2026-09-18T14:31:00Z', accountId: 'a', account: 'A', securityId: 's', symbol: 'X', currency: 'CAD',
  side: 'BUY', type: 'LIMIT', quantity: 10, limitPrice: 2, stopPrice: null, tif: 'DAY', stopLoss: null, takeProfit: null,
  status: 'pending', wsOrderId: '', error: '', updatedAt: '', source: 'bagholder', wsStatus: '', filledQty: null, avgFill: null,
  submittedAt: '', expiresAt: '', parentId: '', role: 'entry', fillBookedQty: null, exchange: '', ...over,
} as Order)

const bracket = (over: Partial<Bracket>): Bracket => ({
  id: 'b', orderId: 'e', createdAt: '', accountId: 'a', securityId: 's', symbol: 'X', currency: 'CAD', quantity: 10, tif: 'UNTIL_CANCEL',
  slKind: 'stop', slPrice: 1.8, slTrail: null, slTrailUnit: 'pct', slOrderId: '', slNative: true, slMode: 'native', highWater: null,
  tpPrice: 2.4, tpOrderId: '', status: 'armed', outcome: '', error: '', attempts: 0, movedAt: '', armedAt: '2026-09-18T15:00:00Z',
  seenHeld: false, missedAt: '', updatedAt: '', ...over,
} as Bracket)

const words = (b: Bracket) => Object.fromEntries(bracketLegs(b).map((l) => [l.key, l.note]))

const entry = order({ id: 'e', status: 'filled', filledQty: 10, avgFill: 2 })

beforeEach(() => {
  ordersStore.data = { ok: true, live: true, refreshedAt: '', orders: [entry], brackets: [] }
})

describe('an order whose cancel is out', () => {
  it('reads Cancelling on its second line, after its terms', () => {
    expect(orderDetailLine(order({ status: 'cancelling' }))).toBe('10 at 2.00 limit · Day · Cancelling')
  })
  it('and no other live state says it', () => {
    for (const status of ['sent', 'pending'] as const) expect(orderDetailLine(order({ status }))).not.toContain('Cancelling')
  })
})

describe('a bracket\'s stop while the limit sell at the target rests', () => {
  it('reads Watching while the target order rests at Wealthsimple', () => {
    for (const status of ['sent', 'pending'] as const) {
      ordersStore.data!.orders = [entry, order({ id: 't', role: 'target', parentId: 'e', side: 'SELL', status })]
      expect(words(bracket({ status: 'target_placed', tpOrderId: 't' }))).toEqual({ sl: 'Watching', tp: '' })
    }
  })
  it('then Placing once the stop level is hit and the market sell is under way', () => {
    expect(words(bracket({ status: 'stopping' })).sl).toBe('Placing')
  })
  it('says nothing of watching while no limit sell rests (the target being placed again)', () => {
    expect(words(bracket({ status: 'target_placed', tpOrderId: '' })).sl).toBe('')
  })
  it('a stop resting at Wealthsimple, or one watched because Wealthsimple takes none, has no word', () => {
    ordersStore.data!.orders = [entry, order({ id: 's1', role: 'stop', parentId: 'e', side: 'SELL', type: 'STOP', status: 'pending' })]
    expect(words(bracket({ slOrderId: 's1' })).sl).toBe('')
    expect(words(bracket({ slMode: 'watched' })).sl).toBe('')
  })
})

describe('a bracket that ended', () => {
  const ENDED_WITHOUT_EXIT = ['cancelled by the user', 'both legs removed', 'sold from the ticket', 'stop cancelled at Wealthsimple by hand', 'the shares are not there', 'sold: 10 shares in the activity feed']
  it('without exiting reads Off on both legs, however it ended', () => {
    for (const outcome of ENDED_WITHOUT_EXIT) {
      for (const status of ['done', 'cancelled'] as const) expect(words(bracket({ status, outcome })), outcome + ' / ' + status).toEqual({ sl: 'Off', tp: 'Off' })
    }
  })
  it('without exiting, while its resting leg\'s cancel is out, reads Cancelling on that leg and Off on the other', () => {
    ordersStore.data!.orders = [entry, order({ id: 's1', role: 'stop', parentId: 'e', side: 'SELL', type: 'STOP', status: 'cancelling' })]
    for (const outcome of ENDED_WITHOUT_EXIT) expect(words(bracket({ status: 'closing', outcome }))).toEqual({ sl: 'Cancelling', tp: 'Off' })
  })
  it('by an exit reads the fill on the leg that exited and Cancelled on the other', () => {
    ordersStore.data!.orders = [entry, order({ id: 's1', role: 'stop', parentId: 'e', side: 'SELL', type: 'STOP', status: 'filled', filledQty: 10, avgFill: 1.79 })]
    const legs = bracketLegs(bracket({ status: 'done', outcome: 'stopped' }))
    expect(legs.map((l) => [l.key, l.line, l.note])).toEqual([['sl', 'Filled 10 at 1.79', ''], ['tp', '10 at 2.40', 'Cancelled']])
    ordersStore.data!.orders = [entry, order({ id: 't', role: 'target', parentId: 'e', side: 'SELL', status: 'filled', filledQty: 10, avgFill: 2.4 })]
    const up = bracketLegs(bracket({ status: 'done', outcome: 'target' }))
    expect(up.map((l) => [l.key, l.line, l.note])).toEqual([['sl', '10 at 1.80', 'Cancelled'], ['tp', 'Filled 10 at 2.40', '']])
  })
  it('by an exit, while the other leg\'s cancel is out, reads Cancelling on it, then Cancelled', () => {
    ordersStore.data!.orders = [entry, order({ id: 't', role: 'target', parentId: 'e', side: 'SELL', status: 'cancelling' })]
    expect(words(bracket({ status: 'closing', outcome: 'stopped' }))).toEqual({ sl: 'Filled', tp: 'Cancelling' })
    ordersStore.data!.orders = [entry]
    expect(words(bracket({ status: 'closing', outcome: 'stopped' }))).toEqual({ sl: 'Filled', tp: 'Cancelled' })
  })
})
