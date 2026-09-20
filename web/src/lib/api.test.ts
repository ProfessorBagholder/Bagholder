import { afterEach, describe, expect, it, vi } from 'vitest'
import { get, post, query, searchSymbols } from './api'

const sources = import.meta.glob('/src/**/*.{ts,svelte}', { query: '?raw', import: 'default', eager: true }) as Record<string, string>

afterEach(() => vi.unstubAllGlobals())

describe('the one way to the server', () => {
  it('is the only place the page calls fetch or opens a stream of its own', () => {
    const callers = Object.entries(sources)
      .filter(([path, text]) => !path.endsWith('.test.ts') && /\bfetch\(/.test(text))
      .map(([path]) => path)
    expect(callers).toEqual(['/src/lib/api.ts'])
    const streams = Object.entries(sources)
      .filter(([path, text]) => !path.endsWith('.test.ts') && /new EventSource\(/.test(text))
      .map(([path]) => path)
    expect(streams).toEqual(['/src/lib/live.ts'])
  })

  it('leaves out of a query what is empty', () => {
    expect(query({ symbol: 'QNC', exchange: '', currency: undefined, trend: true, refresh: false, n: 0 })).toBe('symbol=QNC&trend=1&n=0')
  })

  it('marks a write as the page\'s own and sends JSON', async () => {
    const fetched = vi.fn(async () => new Response(JSON.stringify({ ok: true, id: 7 })))
    vi.stubGlobal('fetch', fetched)
    expect(await post('/api/journal', { id: 't1' })).toEqual({ ok: true, id: 7 })
    const [path, opts] = fetched.mock.calls[0] as unknown as [string, RequestInit]
    expect(path).toBe('/api/journal')
    expect(opts.method).toBe('POST')
    expect(opts.headers).toMatchObject({ 'X-Bagholder': '1', 'Content-Type': 'application/json' })
    expect(opts.body).toBe('{"id":"t1"}')
  })

  it('answers a failure in the server\'s own shape, and never throws', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: false, error: 'symbol required' }), { status: 400 })))
    expect(await get('/api/filings')).toEqual({ ok: false, error: 'symbol required' })
    vi.stubGlobal('fetch', vi.fn(async () => { throw new TypeError('Failed to fetch') }))
    expect(await get('/api/status')).toEqual({ ok: false, error: 'TypeError: Failed to fetch' })
  })

  it('asks for a listing once, and does not remember a lookup that failed', async () => {
    const fetched = vi.fn(async () => new Response(JSON.stringify({ ok: false, error: 'busy' }), { status: 500 }))
    vi.stubGlobal('fetch', fetched)
    expect(await searchSymbols('plt')).toEqual([])
    fetched.mockImplementation(async () => new Response(JSON.stringify({ ok: true, matches: [{ symbol: 'PLTR', exchange: 'NASDAQ', name: 'Palantir', currency: 'USD' }] })))
    expect((await searchSymbols('plt')).map((m) => m.symbol)).toEqual(['PLTR'])
    expect((await searchSymbols(' PLT ')).map((m) => m.symbol)).toEqual(['PLTR'])
    expect(fetched).toHaveBeenCalledTimes(2)
  })
})
