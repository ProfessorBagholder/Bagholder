import { describe, it, expect, afterEach } from 'vitest'
import {
  orderDetailLine, orderLine, orderFillLine, orderValue, orderEndWord, orderUnconfirmed, listing,
  legRow, bracketEditor, tabCards, typedDec, draftCard,
  type OrderCard, type BracketCard, type Leg,
} from './orders.svelte'
import { filters } from '../filters.svelte'
import { store } from '../state.svelte'
import type { TicketDraft } from '../ticket/ticket.svelte'
import type { OrdersDoc, Preview } from '../generated/orders'
import type { Dec } from '../dec'
import type { Model } from '../model'

// SPEC.md §4, Orders: the card grammar written out from the cards the server builds.
// The server states every amount, every leg's word, each card's tab and whether it
// acts; these walk every kind, time in force and state the page writes words for.

const d = (s: string) => s as Dec

const order = (over: Partial<OrderCard> = {}): OrderCard => ({
  id: 'o', account: 'A1', exchange: 'TSX', symbol: 'X', side: 'buy', kind: 'limit', tif: 'day',
  quantity: d('10'), limitPrice: d('2'), stopPrice: null, state: 'pending', filled: d('0'), average: null, why: null,
  value: d('20'), approx: false, tab: 'pending', at: '2026-09-18T14:31:00Z', live: true, editable: true, legs: [], ...over,
})

const leg = (over: Partial<Leg> = {}): Leg => ({
  key: 'sl', quantity: d('10'), level: d('1.8'), trailPct: null, trailAmount: null, filled: null, note: '', amount: d('18'), ...over,
})

const bracket = (over: Partial<BracketCard> = {}): BracketCard => ({
  id: 'b', account: 'A1', exchange: 'TSX', symbol: 'X', tab: 'pending', at: '2026-09-18T15:00:00Z', live: true,
  value: d('20'), legs: [leg(), leg({ key: 'tp', level: d('2.4'), amount: d('24') })], endWord: null,
  stopLevel: d('1.8'), trailPct: null, trailAmount: null, target: d('2.4'), ...over,
})

describe('row two of an order', () => {
  it('names the quantity, how it works and its time in force, for every kind', () => {
    expect(orderDetailLine(order())).toBe('10 at 2.00 limit · Day')
    expect(orderDetailLine(order({ kind: 'market', limitPrice: null }))).toBe('10 at market')
    expect(orderDetailLine(order({ kind: 'stop', limitPrice: null, stopPrice: d('1.6') }))).toBe('10 at 1.60 stop · Day')
    expect(orderDetailLine(order({ kind: 'stop-limit', stopPrice: d('1.6'), limitPrice: d('1.55'), tif: 'until-cancel' }))).toBe('10 stop 1.60 · limit 1.55 · GTC')
  })
  it('reads Cancelling after its terms while its cancel is out, and no other state says it', () => {
    expect(orderDetailLine(order({ state: 'cancelling' }))).toBe('10 at 2.00 limit · Day · Cancelling')
    for (const state of ['pending', 'partly-filled', 'sending', 'unconfirmed']) expect(orderDetailLine(order({ state }))).not.toContain('Cancelling')
  })
  it('names how it went once filled', () => {
    expect(orderDetailLine(order({ state: 'filled', tab: 'filled', filled: d('10'), average: d('1.75') }))).toBe('Filled 10 at 1.75')
  })
})

describe('an order as a sentence', () => {
  it('reads side, quantity, symbol and terms', () => {
    expect(orderLine(order({ symbol: 'QNC.V', quantity: d('100'), limitPrice: d('1.75') }))).toBe('Buy 100 QNC at 1.75 limit')
    expect(orderLine(order({ side: 'sell', kind: 'market' }))).toBe('Sell 10 X at market')
    expect(orderLine(order({ side: 'sell', kind: 'stop', stopPrice: d('150') }))).toBe('Sell 10 X at 150.00 stop')
    expect(orderLine(order({ kind: 'stop-limit', stopPrice: d('1.6'), limitPrice: d('1.55') }))).toBe('Buy 10 X stop 1.60 · limit 1.55')
  })
})

describe('row one', () => {
  it('names the listing, the symbol alone with no exchange', () => {
    expect(listing(order({ symbol: 'QNC.V', exchange: 'TSX-V' }))).toBe('TSX-V: QNC')
    expect(listing(order({ exchange: '' }))).toBe('X')
  })
  it('shows the server\'s value, ≈ on a guessed fill, nothing where it has none, the dash and word where it waits', () => {
    expect(orderValue(order({ value: d('4135') }))).toBe('$4,135.00')
    expect(orderValue(order({ kind: 'market', value: d('812.5'), approx: true }))).toBe('≈ $812.50')
    expect(orderValue(order({ kind: 'market', value: null }))).toBe('')
    expect(orderValue(order({ value: { gaps: ['multiplier-unstated'] } }))).toBe('— size')
  })
})

describe('the third line', () => {
  it('names the fill so far on a pending order partly filled', () => {
    expect(orderFillLine(order({ state: 'partly-filled', quantity: d('100'), filled: d('40'), average: d('64.5') }))).toBe('40 of 100 filled at 64.50')
  })
  it('names the reason on a rejected or failed order', () => {
    expect(orderFillLine(order({ state: 'rejected', tab: 'cancelled', why: 'Insufficient funds.' }))).toBe('Insufficient funds.')
    expect(orderFillLine(order({ state: 'failed', tab: 'cancelled', why: 'No record.' }))).toBe('No record.')
  })
  it('is absent otherwise', () => {
    expect(orderFillLine(order())).toBe('')
    expect(orderFillLine(order({ state: 'cancelled', tab: 'cancelled', filled: d('4') }))).toBe('')
    expect(orderFillLine(order({ state: 'unconfirmed', why: 'The answer could not be read.' }))).toBe('')
  })
})

describe('the foot', () => {
  it('a finished order names its state, red for a rejection or a failure', () => {
    expect(['cancelled', 'expired', 'rejected', 'failed', 'dry'].map((state) => orderEndWord(order({ state })))).toEqual([
      ['Cancelled', false], ['Expired', false], ['Rejected', true], ['Failed', true], ['Not sent', false],
    ])
  })
  it('an order sent and not answered for yet reads as sent, not confirmed', () => {
    expect(['sending', 'unconfirmed', 'pending'].map((state) => orderUnconfirmed(order({ state })))).toEqual([true, true, false])
  })
})

describe('a leg row', () => {
  it('reads label, quantity and level, and the server\'s word and amount', () => {
    expect(legRow(leg({ note: 'Placing' }))).toEqual({ key: 'sl', label: 'Stop loss', tone: 'neg', line: '10 at 1.80', note: 'Placing', amount: '$18.00' })
    expect(legRow(leg({ key: 'tp', level: d('2.4'), amount: d('24') }))).toMatchObject({ label: 'Take profit', tone: 'pos', line: '10 at 2.40', amount: '$24.00' })
  })
  it('a trailing stop names its trail, as a percent or an amount', () => {
    expect(legRow(leg({ level: d('1.62'), trailPct: d('5') })).line).toBe('10 at 1.62 · trailing 5%')
    expect(legRow(leg({ level: d('1.62'), trailAmount: d('0.1') })).line).toBe('10 at 1.62 · trailing 0.10')
  })
  it('the leg that exited reads its fill instead of its level', () => {
    expect(legRow(leg({ filled: { quantity: d('10'), average: d('1.64') }, amount: d('16.4') }))).toMatchObject({ line: 'Filled 10 at 1.64', amount: '$16.40' })
  })
  it('an amount that waits shows the dash and its word', () => {
    expect(legRow(leg({ amount: { gaps: ['multiplier-unstated'] } })).amount).toBe('— size')
  })
})

describe('the bracket editor', () => {
  it('starts from the stop price, the trail or the trail percent, and the target', () => {
    expect(bracketEditor(bracket())).toEqual({ sl: { label: 'Stop price', start: '1.80', field: 'price', cur: '1.8' }, tp: { start: '2.40', cur: '2.4' } })
    expect(bracketEditor(bracket({ trailPct: d('5') })).sl).toMatchObject({ label: 'Trail %', start: '5', field: 'trail' })
    expect(bracketEditor(bracket({ trailAmount: d('0.25') })).sl).toMatchObject({ label: 'Trail', start: '0.25', field: 'trail' })
    expect(bracketEditor(bracket({ stopLevel: null, target: null }))).toEqual({ sl: null, tp: null })
  })
})

describe('what is typed into an editor', () => {
  it('is sent as the decimal it states, the formatting taken off', () => {
    expect(['150', '1,500', '$1.75', ' 2.5 '].map(typedDec)).toEqual(['150', '1500', '1.75', '2.5'])
  })
  it('is none when it is not a positive decimal', () => {
    expect(['', '0', 'abc', '-3', '1.2.3'].map(typedDec)).toEqual([null, null, null, null, null])
  })
})

describe('a tab\'s cards', () => {
  const doc = (orders: OrderCard[], brackets: BracketCard[]): OrdersDoc => ({ ok: true, live: true, refreshedAt: null, orders, brackets, error: null })
  afterEach(() => { filters.lists.account = []; store.model = null })
  it('are the orders and brackets the server put on it, newest first together', () => {
    const cards = tabCards(doc(
      [order({ id: 'o1', at: '2026-09-18T10:00:00Z' }), order({ id: 'o2', tab: 'filled' }), order({ id: 'o3', at: '2026-09-18T12:00:00Z' })],
      [bracket({ id: 'b1', at: '2026-09-18T11:00:00Z' }), bracket({ id: 'b2', tab: 'cancelled' })],
    ), 'pending')
    expect(cards.map((c) => (c.kind === 'order' ? c.o.id : c.b.id))).toEqual(['o3', 'b1', 'o1'])
  })
  it('leave out every card outside the accounts in scope', () => {
    store.model = { accounts: [{ id: 'acct-1', brokerAccount: 'A1', name: 'TFSA' }, { id: 'acct-2', brokerAccount: 'A2', name: 'Margin' }] } as unknown as Model
    filters.lists.account = ['acct-2']
    const cards = tabCards(doc([order({ id: 'o1' }), order({ id: 'o2', account: 'A2' })], [bracket({ id: 'b1' }), bracket({ id: 'b2', account: 'A2' })]), 'pending')
    expect(cards.map((c) => (c.kind === 'order' ? c.o.id : c.b.id)).sort()).toEqual(['b2', 'o2'])
  })
})

describe('the draft card', () => {
  // the server's figures for what the draft holds (POST /api/order/preview)
  const preview = (over: Record<string, unknown> = {}) => ({
    entry: '165.4', limit: '165.4', stop: '168.71', quantity: '25', notional: '4135', stopLossOn: true, takeProfitOn: true, trailing: false,
    trail: '5', trailDistance: '8.27', stopLossPctIn: '5', stopLossPrice: '157.13', takeProfitPctIn: '10', takeProfitPrice: '181.94',
    stopLossValue: '3928.25', takeProfitValue: '4548.5', ...over,
  }) as unknown as Preview
  const draft = (over: Partial<TicketDraft> = {}): TicketDraft => ({
    symbol: 'X', side: 'BUY', exchange: 'EX', at: '2026-09-19T16:00:00Z', accountId: 'a', type: 'LIMIT', tif: 'DAY',
    qty: 1, limit: null, stop: null,
    sl: { on: true, kind: 'stop', price: null, pct: null, priceUnit: 'amt', trail: null, unit: 'pct' },
    tp: { on: true, price: null, pct: null, unit: 'amt' }, text: {}, preview: preview(), ...over,
  })
  it('shows the server\'s amount, quantity, price and legs, though nothing was typed', () => {
    const c = draftCard(draft())
    expect([c.value, c.line]).toEqual(['$4,135.00', 'Buy 25 at 165.40 limit · Day'])
    expect(c.legs.map((l) => [l.label, l.value, l.amount])).toEqual([['Stop loss', '25 at 157.13', '$3,928.25'], ['Take profit', '25 at 181.94', '$4,548.50']])
  })
  it('a trailing stop names its trail beside its starting level', () => {
    const c = draftCard(draft({ preview: preview({ trailing: true, trail: '5' }) }))
    expect(c.legs[0].value).toBe('25 at 157.13 · trailing 5%')
  })
  it('a leg that is off, or a sale, has no leg row', () => {
    expect(draftCard(draft({ preview: preview({ stopLossOn: false, stopLossValue: null }) })).legs.map((l) => l.label)).toEqual(['Take profit'])
    expect(draftCard(draft({ side: 'SELL', preview: preview({ stopLossOn: false, takeProfitOn: false }) })).legs).toEqual([])
  })
  it('a market order holds no price: no amount', () => {
    const c = draftCard(draft({ type: 'MARKET', preview: preview({ entry: '165.42' }) }))
    expect([c.value, c.line]).toEqual(['', 'Buy 25 at market'])
  })
  it('without the server\'s figures it shows no figure it would have to work out', () => {
    const c = draftCard(draft({ preview: null, limit: 1.72 }))
    expect([c.value, c.line, c.legs]).toEqual(['', 'Buy 1 at 1.72 limit · Day', []])
  })
})
