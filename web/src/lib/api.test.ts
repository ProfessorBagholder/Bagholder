import { afterEach, describe, expect, it, vi } from 'vitest'
import { call, get, post, query } from './api'
import { searchSymbols } from './reads.svelte'

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
    expect(streams).toEqual(['/src/lib/live.svelte.ts'])
  })

  // brief 13: every read goes through the one read layer, which keeps each answer and
  // draws it at once the next time; a read made anywhere else would open empty
  it('reads only through the read layer: no GET is made anywhere else', () => {
    const outside = Object.entries(sources)
      .filter(([path]) => !path.endsWith('.test.ts') && path !== '/src/lib/reads.svelte.ts' && path !== '/src/lib/api.ts' && !path.startsWith('/src/lib/generated/'))
      .flatMap(([path, text]) => [...text.matchAll(/\b(?:call|request|get)(?:<[^>]*>)?\(\s*['"`](?:GET\b|\/api)/g)].map((m) => path + ': ' + m[0]))
    expect(outside).toEqual([])
    const lookups = Object.entries(sources).filter(([path, text]) => !path.endsWith('.test.ts') && /\blookup\(/.test(text)).map(([path]) => path)
    expect(lookups).toEqual([])
  })

  // A timer is kept only where there is nothing to wait for instead, and each is argued
  // for in docs/architecture.md under "Timers that remain". None of these asks the server
  // anything: the page is told (live.ts), it does not look.
  it('waits on a clock only where that is accounted for', () => {
    const allowed: Record<string, number> = {
      '/src/lib/ui.svelte.ts': 1, // how long a notice stays in the header (the sign-in's deadline is the server's)
      '/src/lib/clock.svelte.ts': 1, // the minute, while something on screen says "… ago"
      '/src/lib/scrollbars.ts': 1, // a scrollbar lingers after its list stops: Safari has no scrollend
      '/src/lib/TradeDetail.svelte': 1, // the thesis is saved a moment after typing stops
      '/src/lib/FilterPopover.svelte': 2, // a lookup and a range wait for the typing to pause
      '/src/lib/markets/News.svelte': 1, // a lookup waits for the typing to pause
      '/src/lib/markets/Shorts.svelte': 1, // a lookup waits for the typing to pause
      '/src/lib/markets/Watchlist.svelte': 2, // a lookup waits for the typing to pause; the added row's tint decays
      '/src/lib/trade/Disclosures.svelte': 1, // "Opening…" on a row whose document opens in another tab, which tells the page nothing
      '/src/lib/heatmap/Heatmap.svelte': 1, // the slideshow's dwell: the reader's own figure
    }
    const found: Record<string, number> = {}
    for (const [path, text] of Object.entries(sources)) {
      if (path.endsWith('.test.ts')) continue
      const n = (text.match(/\b(setTimeout|setInterval)\(/g) || []).length
      if (n) found[path] = n
    }
    expect(found).toEqual(allowed)
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

  it('reads an answer strictly: a refusal without a reason, a failed status and an unreadable body are failures', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: false }), { status: 500, statusText: 'Internal Server Error' })))
    expect(await get('/api/status')).toEqual({ ok: false, error: 'Bagholder answered 500 Internal Server Error without a reason.' })
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ path: '/x' }), { status: 404, statusText: 'Not Found' })))
    expect(await get('/api/watch')).toEqual({ ok: false, error: 'Bagholder answered 404 Not Found without a reason.' })
    vi.stubGlobal('fetch', vi.fn(async () => new Response('<html>busy</html>', { status: 502, statusText: 'Bad Gateway' })))
    expect(await get('/api/status')).toEqual({ ok: false, error: 'Bagholder answered 502 Bad Gateway.' })
    vi.stubGlobal('fetch', vi.fn(async () => new Response('[1,2]')))
    expect(await get('/api/status')).toEqual({ ok: false, error: 'Bagholder answered in a form this page cannot read.' })
    // an answer that carries no `ok` of its own (a route's plain state) is the route's answer
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ path: '/in', account: '' }))))
    expect(await get('/api/watch')).toEqual({ ok: true, path: '/in', account: '' })
  })

  it('lets no field of an answer be read before its failure is met', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: true, id: 'w1' }))))
    const r = await call('POST /api/watchlist/add', { body: { symbol: 'QNC', exchange: 'TSXV', name: '', currency: 'CAD' } })
    // @ts-expect-error -- a failure has no id: the type checker refuses this read until `ok` is checked
    expect(r.id).toBe('w1')
    if (!r.ok) throw new Error(r.error)
    expect(r.id).toBe('w1')
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
