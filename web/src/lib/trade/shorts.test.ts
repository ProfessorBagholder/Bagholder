import { describe, it, expect, vi, afterEach } from 'vitest'
import type { Trade } from '../model'
import { ensureShorts, reportsShorts } from './shorts.svelte'

const trade = (kind: string, symbol: string) =>
  ({ kind, symbol, underlying: symbol.split(' ')[0], exchange: 'NASDAQ', currency: 'USD' }) as unknown as Trade

afterEach(() => vi.unstubAllGlobals())

describe('short interest', () => {
  // every kind the server names that is not a share listing
  const unreported = ['Options', 'Crypto', 'Futures', 'Indices', 'Rates', 'Currencies', 'Commodities', 'Event contracts']

  it('is reported for a share listing and for nothing else', () => {
    expect(reportsShorts({ kind: 'Shares' })).toBe(true)
    for (const kind of unreported) expect(reportsShorts({ kind }), kind).toBe(false)
  })

  it('is never asked for what no one reports, an option through its underlying included', async () => {
    const fetched = vi.fn(async () => new Response(JSON.stringify({ ok: true, covered: false })))
    vi.stubGlobal('fetch', fetched)
    for (const kind of unreported) await ensureShorts(trade(kind, kind === 'Options' ? 'ABC 20NOV26 3.00 CALL' : 'ABC'))
    expect(fetched).not.toHaveBeenCalled()
    await ensureShorts(trade('Shares', 'ABC'))
    expect(fetched).toHaveBeenCalledTimes(1)
  })
})
