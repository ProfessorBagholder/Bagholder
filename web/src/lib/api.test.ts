import { afterEach, describe, expect, it, vi } from 'vitest'
import { get, lookup, post, query, searchSymbols } from './api'

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

  // A timer is kept only where there is nothing to wait for instead, and each is argued
  // for in docs/architecture.md under "Timers that remain". None of these asks the server
  // anything: the page is told (live.ts), it does not look.
  it('waits on a clock only where that is accounted for', () => {
    const allowed: Record<string, number> = {
      '/src/lib/ui.svelte.ts': 2, // how long a notice stays in the header; the sign-in's three-minute deadline
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

describe('what is looked up on demand', () => {
  // a fetch that answers when told to, and knows when it was dropped
  function slowFetch() {
    const calls: { path: string; signal: AbortSignal; answer: (body: unknown) => void }[] = []
    vi.stubGlobal('fetch', (path: string, opts: RequestInit) =>
      new Promise<Response>((resolve, reject) => {
        const signal = opts.signal as AbortSignal
        signal.addEventListener('abort', () => reject(new DOMException('dropped', 'AbortError')))
        calls.push({ path, signal, answer: (body) => resolve(new Response(JSON.stringify(body))) })
      }))
    return calls
  }

  const hq = (symbol: string) => ({ query: { symbol, exchange: '', currency: '', kind: '', from: '', to: '', tf: '' } })

  it('asks once for the same thing asked twice at once, and keeps a good answer', async () => {
    const calls = slowFetch()
    const bars = lookup('GET /api/history')
    const a = bars.read(hq('1'))
    const b = bars.read(hq('1'))
    expect(calls.length).toBe(1)
    calls[0].answer({ ok: true, n: 3 })
    expect(await a).toEqual({ ok: true, n: 3 })
    expect(await b).toEqual({ ok: true, n: 3 })
    expect(await bars.read(hq('1'))).toEqual({ ok: true, n: 3 })
    expect(calls.length).toBe(1)
    bars.forget()
    void bars.read(hq('1'))
    expect(calls.length).toBe(2)
  })

  it('never keeps a failure, nor an answer its rule says is not final', async () => {
    const calls = slowFetch()
    const bars = lookup('GET /api/history', { keep: (a) => !('pending' in a && a.pending) })
    const first = bars.read(hq('p'))
    calls[0].answer({ ok: false, error: 'no' })
    expect((await first).ok).toBe(false)
    const second = bars.read(hq('p'))
    calls[1].answer({ ok: true, pending: true })
    await second
    void bars.read(hq('p'))
    expect(calls.length).toBe(3)
  })

  it('keeps an answer only as long as it is good for', async () => {
    vi.useFakeTimers()
    try {
      const calls = slowFetch()
      const shorts = lookup('GET /api/shorts', { keepMs: 30 * 60_000 })
      const first = shorts.read({ query: { symbol: 's', exchange: '', currency: '', name: '', trend: false } })
      calls[0].answer({ ok: true })
      await first
      vi.advanceTimersByTime(29 * 60_000)
      await shorts.read({ query: { symbol: 's', exchange: '', currency: '', name: '', trend: false } })
      expect(calls.length).toBe(1)
      vi.advanceTimersByTime(2 * 60_000)
      void shorts.read({ query: { symbol: 's', exchange: '', currency: '', name: '', trend: false } })
      expect(calls.length).toBe(2)
    } finally {
      vi.useRealTimers()
    }
  })

  it('drops a request when the last reader stops waiting, and not before', async () => {
    const calls = slowFetch()
    const bars = lookup('GET /api/history')
    const one = new AbortController()
    const two = new AbortController()
    const a = bars.read(hq('h'), { signal: one.signal })
    const b = bars.read(hq('h'), { signal: two.signal })
    one.abort()
    expect(await a).toEqual({ ok: false, error: 'aborted' })
    expect(calls[0].signal.aborted).toBe(false)
    two.abort()
    expect(await b).toEqual({ ok: false, error: 'aborted' })
    expect(calls[0].signal.aborted).toBe(true)
    // and the next reader asks afresh
    void bars.read(hq('h'))
    expect(calls.length).toBe(2)
  })
})
