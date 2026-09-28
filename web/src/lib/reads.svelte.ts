// The one way the page reads from the server (docs/architecture.md §13; brief 13).
//
// The screens' documents arrive over the stream (live.svelte.ts). Everything else
// the page reads (a trade's executions, a chart's bars, short interest, what is known
// of a listing, the watched folder) is read here, and nowhere else: a scan fails on a
// read made anywhere else (reads.test.ts). The pattern is stale-while-revalidate (RFC
// 5861): the last answer held is given at once, from memory or from the browser's
// store, and the server is asked in the background; its answer replaces the one shown
// only where it differs. A placeholder is only ever for a question never answered.
//
// An answer is kept in the browser under the book shown (kept.ts), so it is drawn at
// once on the next open and after a restart, and Clear data drops it with the rest.
// A few answers must not outlive the moment and are never kept: they are named at
// the route, below.

import { call, type Answer } from './api'
import type { Routes } from './generated/routes'
import type { SymbolMatch } from './model'
import { kept, keptRead, readKept, save } from './kept'
import { bookShown } from './live.svelte'

type GetKey = Extract<keyof Routes, `GET ${string}`>
type Input<K extends GetKey> = Omit<Routes[K], 'answer'>

/**
 * The reads whose answer must not outlive the moment: never kept in the browser, and
 * never given from memory to a later ask.
 * - a search the person types, and a match's glance quote (SPEC.md §4 Markets,
 *   Watchlist: a minute's memory, nothing stored);
 * - what is asked as an action, not to be shown: the news read for a ticker typed, a
 *   forced re-read of a listing's disclosures.
 */
export const NEVER_KEPT: readonly GetKey[] = [
  'GET /api/symbols/search',
  'GET /api/symbols/quote',
  'GET /api/news/symbol',
  'GET /api/filings',
]
/**
 * The ones asked of the server each time, never answered from this page load's memory:
 * an action, the glance quote, whose minute the watchlist keeps, and every trade (the
 * export and the rows a trade page is drawn from, each wanting the book as it is now;
 * the rows are kept, so a trade page opens on them at once).
 */
const ASKED_EACH_TIME: readonly GetKey[] = ['GET /api/symbols/quote', 'GET /api/figures/trades', 'GET /api/news/symbol', 'GET /api/filings']

/** The page's own reads in the air: what is read ahead waits for none (`whenIdle`). */
let pageReads = 0
let idle: (() => void)[] = []
/** Once the page has no read of its own in the air: what is read ahead never holds up a screen's. */
export function whenIdle(): Promise<void> {
  return pageReads ? new Promise((r) => idle.push(r)) : Promise.resolve()
}

/** An answer asked this page load: given again from memory while it is young enough. */
const asked = new Map<string, { at: number; answer: Answer<unknown> }>()
/** The same question in the air: one request, however many ask. */
const flying = new Map<string, { answer: Promise<Answer<unknown>>; readers: number; stop: AbortController }>()

function path<K extends GetKey>(route: K, input?: Input<K>): string {
  const [, p] = route.split(' ', 2)
  const q = new URLSearchParams()
  const given = (input as { query?: Record<string, unknown> } | undefined)?.query ?? {}
  for (const [k, v] of Object.entries(given)) if (v !== undefined && v !== null && v !== '' && v !== false) q.set(k, v === true ? '1' : String(v))
  const qs = q.toString()
  return qs ? p + '?' + qs : p
}
// filed under the book shown, so nothing held of a book cleared since is ever given
const keyOf = (route: GetKey, input: unknown, key?: string) => route + ' ' + (key ?? path(route, input as never)) + ' ' + bookShown()
const storeKey = (k: string) => 'get:' + k

/**
 * The last answer held to this question, at once: from this page load, else from what
 * the browser kept. Undefined only for a question never answered (or one never kept).
 */
export function held<K extends GetKey>(route: K, input?: Input<K>, opts: { key?: string } = {}): Answer<Routes[K]['answer']> | undefined {
  const k = keyOf(route, input, opts.key)
  const mem = ASKED_EACH_TIME.includes(route) ? undefined : asked.get(k)
  if (mem) return mem.answer as Answer<Routes[K]['answer']>
  const book = bookShown()
  if (NEVER_KEPT.includes(route) || !keptRead(book)) return undefined
  const b = kept(book, storeKey(k), {})
  return b ? (b.data as Answer<Routes[K]['answer']>) : undefined
}

/** What `held` gives, once what the browser kept has been read (as the page opens, a moment after). */
export async function heldOnceRead<K extends GetKey>(route: K, input?: Input<K>, opts: { key?: string } = {}): Promise<Answer<Routes[K]['answer']> | undefined> {
  await readKept(bookShown())
  return held(route, input, opts)
}

/**
 * The server's answer to the question: asked once however many ask at once, given
 * from this page load's memory while younger than `maxAgeMs` (for good when none is
 * given). An answer `final` accepts (all, when none is given) is kept; a failure never
 * is, so it is asked again. A reader that gives a signal may stop waiting; the request
 * goes on while anyone else still is.
 */
export function read<K extends GetKey>(
  route: K,
  input?: Input<K>,
  opts: { key?: string; signal?: AbortSignal; maxAgeMs?: number; final?: (a: Answer<Routes[K]['answer']>) => boolean; ahead?: boolean } = {},
): Promise<Answer<Routes[K]['answer']>> {
  type A = Answer<Routes[K]['answer']>
  const ephemeral = NEVER_KEPT.includes(route)
  const k = keyOf(route, input, opts.key)
  const mem = asked.get(k)
  if (!ASKED_EACH_TIME.includes(route) && mem && (opts.maxAgeMs == null || Date.now() - mem.at < opts.maxAgeMs)) return Promise.resolve(mem.answer as A)
  if (opts.signal?.aborted) return Promise.resolve({ ok: false, error: 'aborted' } as A)
  let f = flying.get(k)
  if (!f) {
    const stop = new AbortController()
    // an answer's age runs from when it was asked for: what it says was true then
    const askedAt = Date.now()
    const own = !opts.ahead
    if (own) pageReads++
    const made = {
      readers: 0,
      stop,
      answer: call(route, input as never, stop.signal).then((a) => {
        if (own && --pageReads === 0) idle.splice(0).forEach((r) => r())
        if (flying.get(k) === made) flying.delete(k)
        if (a.ok && (!opts.final || opts.final(a as A))) {
          if (!ASKED_EACH_TIME.includes(route)) asked.set(k, { at: askedAt, answer: a })
          if (!ephemeral) void save(bookShown(), [{ key: storeKey(k), params: {}, data: structuredClone(a), v: '' }])
        }
        return a as Answer<unknown>
      }),
    }
    f = made
    flying.set(k, f)
  }
  const flight = f
  flight.readers++
  const signal = opts.signal
  if (!signal) return flight.answer as Promise<A>
  return new Promise((resolve) => {
    const gone = () => {
      resolve({ ok: false, error: 'aborted' } as A)
      if (--flight.readers === 0 && flying.get(k) === flight) {
        flying.delete(k)
        flight.stop.abort()
      }
    }
    signal.addEventListener('abort', gone, { once: true })
    void flight.answer.then((a) => {
      signal.removeEventListener('abort', gone)
      resolve(a as A)
    })
  })
}

/**
 * What a screen last drew from its reads, kept as it drew it: the chart's timeframe and
 * bars, which several reads settle between them. Drawn again as it was on the next open.
 */
const drawn = new Map<string, unknown>()
export function recall<T>(key: string): T | undefined {
  const k = 'drawn:' + key + ' ' + bookShown()
  if (drawn.has(k)) return drawn.get(k) as T
  const book = bookShown()
  if (!keptRead(book)) return undefined
  return kept(book, k, {})?.data as T | undefined
}
export function remember(key: string, value: unknown): void {
  const k = 'drawn:' + key + ' ' + bookShown()
  drawn.set(k, value)
  void save(bookShown(), [{ key: k, params: {}, data: structuredClone(value), v: '' }])
}

/**
 * The server was started again: what this page load asked is asked again at the next
 * read. What was kept still stands on screen until those answers arrive.
 */
export function askAgain(prefix?: GetKey): void {
  for (const k of [...asked.keys()]) if (!prefix || k.startsWith(prefix + ' ')) asked.delete(k)
}

/**
 * Listings by name or ticker, asked from the filter's symbol picker, the watchlist's
 * add row and the news and short-interest lookups. A search the person types is
 * never kept.
 */
export async function searchSymbols(text: string): Promise<SymbolMatch[]> {
  const q = text.trim()
  if (!q) return []
  const r = await read('GET /api/symbols/search', { query: { q } }, { key: q.toLowerCase() })
  return r.ok ? r.matches : [] // a failed lookup is not remembered
}
