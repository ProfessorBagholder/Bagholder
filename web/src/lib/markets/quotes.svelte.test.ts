import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { sugKey, sugQuotes, sugQuoteSchedule } from './quotes.svelte'

// SPEC §4 Markets, Watchlist: a match's quote is read for the glance, with a minute's memory.

let price = 1
const fetched = vi.fn(async () => new Response(JSON.stringify({ ok: true, price: price, percentChange: 0.5 })))
beforeEach(() => {
  vi.useFakeTimers()
  fetched.mockClear()
  vi.stubGlobal('fetch', fetched)
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe("a match's quote", () => {
  it('is remembered for a minute and read again when wanted after it', async () => {
    const row = { symbol: 'QQQM' + Math.random().toString(36).slice(2, 5).toUpperCase(), exchange: 'NASDAQ' }
    sugQuoteSchedule([row])
    await vi.advanceTimersByTimeAsync(0)
    expect(fetched).toHaveBeenCalledTimes(1)
    expect(sugQuotes[sugKey(row)].last).toBe(1)
    await vi.advanceTimersByTimeAsync(59_000)
    sugQuoteSchedule([row])
    await vi.advanceTimersByTimeAsync(0)
    expect(fetched).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(2_000)
    price = 2
    sugQuoteSchedule([row])
    // the one shown stands until the new one lands
    expect(sugQuotes[sugKey(row)].last).toBe(1)
    await vi.advanceTimersByTimeAsync(0)
    expect(fetched).toHaveBeenCalledTimes(2)
    expect(sugQuotes[sugKey(row)].last).toBe(2)
  })

  it('is never read for a row that carries its own price', async () => {
    sugQuoteSchedule([{ symbol: 'ZZWL', exchange: 'TSX', last: 3 }])
    await vi.advanceTimersByTimeAsync(0)
    expect(fetched).not.toHaveBeenCalled()
  })
})
