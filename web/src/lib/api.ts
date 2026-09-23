// The page's one way to the server.
//
// Every request goes through `request`: the header that marks a write as the
// page's own, the JSON body, and one way of failing. The server answers a failure
// with a status code and `{ ok: false, error }`; a request that never reached it
// is given the same shape here, so a caller reads `ok` and `error` and nothing
// throws. What the page *shows* does not come from here at all -- it arrives on
// the event stream (live.ts); this is for what the person does, and for the few
// things looked up on demand.

import type { SymbolMatch } from './model'
import type { Routes } from './generated/routes'

export type Answer<T = Record<string, unknown>> = T & { ok?: boolean; error?: string }

type Param = string | number | boolean | null | undefined

/** `a=1&b=two`, leaving out what is empty. */
export function query(params: Record<string, Param>): string {
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '' && v !== false) q.set(k, v === true ? '1' : String(v))
  }
  return q.toString()
}

export function request<T = Record<string, unknown>>(method: 'GET' | 'POST', path: string, body?: unknown, signal?: AbortSignal): Promise<Answer<T>> {
  const headers: Record<string, string> = { 'X-Bagholder': '1' }
  const opts: RequestInit = { method, headers, signal }
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  return fetch(path, opts)
    .then((r) => r.json() as Promise<Answer<T>>)
    .catch((e) => ({ ok: false, error: String(e) }) as Answer<T>)
}

export function get<T = Record<string, unknown>>(path: string, params?: Record<string, Param>): Promise<Answer<T>> {
  const q = params ? query(params) : ''
  return request<T>('GET', q ? path + '?' + q : path)
}

export function post<T = Record<string, unknown>>(path: string, body?: unknown): Promise<Answer<T>> {
  return request<T>('POST', path, body ?? {})
}

// ---- the route table -----------------------------------------------------------
// `Routes` (generated/routes.ts) is the server's own account of every route: a
// key of `'METHOD /path'`, and the query, body and answer it carries. `call`
// splits the key back into a method and a path and sends what the route
// declares, so a route the server no longer answers this way, or answers
// differently, fails `npm run check` here rather than at the network.

type RouteKey = keyof Routes
/** What `key` takes besides its answer: `{ query }`, `{ body }`, both or neither. */
type RouteInput<K extends RouteKey> = Omit<Routes[K], 'answer'>

export function call<K extends RouteKey>(key: K, input?: RouteInput<K>, signal?: AbortSignal): Promise<Answer<Routes[K]['answer']>> {
  const [method, path] = key.split(' ', 2) as ['GET' | 'POST', string]
  const given = input as { query?: Record<string, Param>; body?: unknown } | undefined
  const q = given?.query ? query(given.query) : ''
  return request<Routes[K]['answer']>(method, q ? path + '?' + q : path, given?.body, signal)
}

// ---- what is looked up on demand ---------------------------------------------------
// A chart's bars, a listing's short interest, what is known of a listing, a symbol
// search: each is asked for when something on the page needs it, and they share one
// way of being asked. The same question asked twice while the first is in the air is
// one request; an answer worth keeping is kept (for good, or for `keepMs`); a failure is
// never kept, so it is asked again; and a request nobody is waiting for any more is
// dropped -- a reader gives the signal that says it has stopped waiting.

export interface Lookup<T> {
  /** The answer to `path`, kept under `key` (the path itself when none is given). */
  read(path: string, opts?: { key?: string; signal?: AbortSignal }): Promise<Answer<T>>
  /** The kept answer, if there is one and it is young enough. */
  peek(key: string): Answer<T> | undefined
  /** Drop what is kept (the server was started again; the answers were the old one's). */
  forget(): void
}

export function lookup<T = Record<string, unknown>>(rules: { keepMs?: number; keep?: (a: Answer<T>) => boolean } = {}): Lookup<T> {
  const kept = new Map<string, { at: number; answer: Answer<T> }>()
  const flying = new Map<string, { answer: Promise<Answer<T>>; readers: number; stop: AbortController }>()
  const peek = (key: string) => {
    const k = kept.get(key)
    if (!k) return undefined
    if (rules.keepMs != null && Date.now() - k.at >= rules.keepMs) return undefined
    return k.answer
  }
  function read(path: string, opts: { key?: string; signal?: AbortSignal } = {}): Promise<Answer<T>> {
    const key = opts.key ?? path
    const have = peek(key)
    if (have) return Promise.resolve(have)
    if (opts.signal?.aborted) return Promise.resolve({ ok: false, error: 'aborted' } as Answer<T>)
    let f = flying.get(key)
    if (!f) {
      const stop = new AbortController()
      const made = { readers: 0, stop, answer: request<T>('GET', path, undefined, stop.signal).then((a) => {
        if (flying.get(key) === made) flying.delete(key)
        if (a.ok && (rules.keep ? rules.keep(a) : true)) kept.set(key, { at: Date.now(), answer: a })
        return a
      }) }
      f = made
      flying.set(key, f)
    }
    const flight = f
    flight.readers++
    const signal = opts.signal
    if (!signal) return flight.answer
    // this reader may stop waiting; the request goes on while anyone else still is
    return new Promise((resolve) => {
      const gone = () => {
        resolve({ ok: false, error: 'aborted' } as Answer<T>)
        if (--flight.readers === 0 && flying.get(key) === flight) {
          flying.delete(key)
          flight.stop.abort()
        }
      }
      signal.addEventListener('abort', gone, { once: true })
      void flight.answer.then((a) => {
        signal.removeEventListener('abort', gone)
        resolve(a)
      })
    })
  }
  return { read, peek, forget: () => kept.clear() }
}

// ---- listings by name or ticker --------------------------------------------------
// Asked from the filter's symbol picker, the watchlist's add row and the news and
// short-interest lookups: one implementation, and an answer is kept for the
// session, since a listing does not change while the page is open.

const searches = lookup<{ matches?: SymbolMatch[] }>()

export async function searchSymbols(text: string): Promise<SymbolMatch[]> {
  const q = text.trim()
  if (!q) return []
  const r = await searches.read('/api/symbols/search?' + query({ q }), { key: q.toLowerCase() })
  return r.ok ? (r.matches ?? []) : [] // a failed lookup is not remembered
}
