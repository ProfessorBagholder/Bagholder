import { afterEach, describe, expect, it, vi } from 'vitest'
import { askAgain, held, read } from './reads.svelte'

afterEach(() => vi.unstubAllGlobals())

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
const sq = (symbol: string) => ({ query: { symbol, exchange: '', currency: '', name: '', trend: false } })
const final = (a: { ok: boolean }) => a.ok && !('pending' in a && (a as { pending?: boolean }).pending)

describe('the one read layer', () => {
  it('asks once for the same thing asked twice at once, holds a good answer, and asks again when told to', async () => {
    const calls = slowFetch()
    const a = read('GET /api/history', hq('1'))
    const b = read('GET /api/history', hq('1'))
    expect(calls.length).toBe(1)
    calls[0].answer({ ok: true, n: 3 })
    expect(await a).toEqual({ ok: true, n: 3 })
    expect(await b).toEqual({ ok: true, n: 3 })
    // held at once from now on, and given without asking again
    expect(held('GET /api/history', hq('1'))).toEqual({ ok: true, n: 3 })
    expect(await read('GET /api/history', hq('1'))).toEqual({ ok: true, n: 3 })
    expect(calls.length).toBe(1)
    // the server started again: asked again, and what was held stands until it answers
    askAgain('GET /api/history')
    void read('GET /api/history', hq('1'))
    expect(calls.length).toBe(2)
    expect(held('GET /api/history', hq('1'))).toBeUndefined() // not in memory, and this test browser keeps nothing
  })

  it('never holds a failure, nor an answer its rule says is not final', async () => {
    const calls = slowFetch()
    const first = read('GET /api/history', hq('p'), { final })
    calls[0].answer({ ok: false, error: 'no' })
    expect((await first).ok).toBe(false)
    expect(held('GET /api/history', hq('p'))).toBeUndefined()
    const second = read('GET /api/history', hq('p'), { final })
    calls[1].answer({ ok: true, pending: true })
    await second
    expect(held('GET /api/history', hq('p'))).toBeUndefined()
    void read('GET /api/history', hq('p'), { final })
    expect(calls.length).toBe(3)
  })

  it('asks again once an answer is older than it is good for', async () => {
    vi.useFakeTimers()
    try {
      const calls = slowFetch()
      const first = read('GET /api/shorts', sq('s'), { maxAgeMs: 30 * 60_000 })
      calls[0].answer({ ok: true })
      await first
      vi.advanceTimersByTime(29 * 60_000)
      await read('GET /api/shorts', sq('s'), { maxAgeMs: 30 * 60_000 })
      expect(calls.length).toBe(1)
      vi.advanceTimersByTime(2 * 60_000)
      void read('GET /api/shorts', sq('s'), { maxAgeMs: 30 * 60_000 })
      expect(calls.length).toBe(2)
    } finally {
      vi.useRealTimers()
    }
  })

  it('drops a request when the last reader stops waiting, and not before', async () => {
    const calls = slowFetch()
    const one = new AbortController()
    const two = new AbortController()
    const a = read('GET /api/history', hq('h'), { signal: one.signal })
    const b = read('GET /api/history', hq('h'), { signal: two.signal })
    one.abort()
    expect(await a).toEqual({ ok: false, error: 'aborted' })
    expect(calls[0].signal.aborted).toBe(false)
    two.abort()
    expect(await b).toEqual({ ok: false, error: 'aborted' })
    expect(calls[0].signal.aborted).toBe(true)
    void read('GET /api/history', hq('h'))
    expect(calls.length).toBe(2)
  })

  it('asks an action and the glance quote of the server each time, and holds neither', async () => {
    const calls = slowFetch()
    const q = { query: { symbol: 'Q', exchange: '', currency: '', name: '' } }
    const a = read('GET /api/symbols/quote', q)
    calls[0].answer({ ok: true, price: '1', percentChange: 0 })
    await a
    expect(held('GET /api/symbols/quote', q)).toBeUndefined()
    void read('GET /api/symbols/quote', q)
    expect(calls.length).toBe(2)
  })
})
