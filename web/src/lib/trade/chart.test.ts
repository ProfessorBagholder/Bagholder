import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Trade } from '../model'
import { loadHistory } from './chart'

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
