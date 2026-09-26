import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Fill, Trade } from '../model'
import { fillPoints, loadHistory } from './chart'

let n = 0
const trade = () => ({ id: 't' + ++n, symbol: 'ABC', exchange: 'NASDAQ', currency: 'USD', kind: 'Shares', entryDate: '2026-01-05', exitDate: '2026-02-05' }) as unknown as Trade

afterEach(() => vi.unstubAllGlobals())

describe("a trade's bars", () => {
  it('that could not be asked for say the failure in the chart’s place', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => { throw new TypeError('Failed to fetch') }))
    const h = await loadHistory(trade(), '1d')
    expect(h.bars).toEqual([])
    expect(h.reason).toContain('Failed to fetch')
  })

  it('that the server refused say its refusal, in its words', async () => {
    const error = 'a refusal ' + Math.random()
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: false, error }), { status: 500 })))
    expect((await loadHistory(trade(), '1d')).reason).toBe(error)
  })

  it('never read as a span with no bars when the request failed', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: false }), { status: 502 })))
    expect((await loadHistory(trade(), '1d')).reason).not.toBe('')
  })
})

describe('executions on their own time axis', () => {
  const c = { text: '', grid: '', up: 'green', down: 'red', line: '' }
  const fill = (o: Record<string, unknown>) => ({ id: String(Math.random()), side: 'BUY', qty: '10', price: '2.50', date: '2026-01-05', when: null, flags: [], ...o }) as unknown as Fill

  it('stand at their own prices in time order, a buy below its price and a sell above, with nothing unpriced', () => {
    const pts = fillPoints([fill({ side: 'SELL', price: '3.10', date: '2026-01-09' }), fill({ price: '2.50' }), fill({ price: '0', flags: ['reward'] })], c)
    expect(pts.map((p) => [p.time, p.price, p.position, p.shape])).toEqual([
      ['2026-01-05', 2.5, 'atPriceBottom', 'arrowUp'],
      ['2026-01-09', 3.1, 'atPriceTop', 'arrowDown'],
    ])
  })

  it('stand at their times when every one was recorded, on their days when any was not', () => {
    const timed = [fill({ when: '2026-01-05T15:00:00Z' }), fill({ side: 'SELL', when: '2026-01-05T15:30:00Z', price: '2.60' })]
    expect(fillPoints(timed, c).every((p) => typeof p.time === 'number')).toBe(true)
    expect(fillPoints([...timed, fill({ date: '2026-01-06' })], c).map((p) => p.time)).toEqual(['2026-01-05', '2026-01-05', '2026-01-06'])
  })
})
