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

export function request<T = Record<string, unknown>>(method: 'GET' | 'POST', path: string, body?: unknown): Promise<Answer<T>> {
  const headers: Record<string, string> = { 'X-Bagholder': '1' }
  const opts: RequestInit = { method, headers }
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

// ---- listings by name or ticker --------------------------------------------------
// Asked from the filter's symbol picker, the watchlist's add row and the news and
// short-interest lookups: one implementation, and an answer is kept for the
// session, since a listing does not change while the page is open.

const found = new Map<string, SymbolMatch[]>()

export async function searchSymbols(text: string): Promise<SymbolMatch[]> {
  const q = text.trim()
  if (!q) return []
  const kept = found.get(q.toLowerCase())
  if (kept) return kept
  const r = await get<{ matches?: SymbolMatch[] }>('/api/symbols/search', { q })
  if (!r.ok) return [] // a failed lookup is not remembered
  const matches = r.matches ?? []
  found.set(q.toLowerCase(), matches)
  return matches
}
