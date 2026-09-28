// The page's one way to the server.
//
// Every request goes through `request`: the header that marks a write as the
// page's own, the JSON body, and one way of failing. The server answers a failure
// with a status code and `{ ok: false, error }`; a request that never reached it,
// or an answer the page cannot read, is given the same shape here, so nothing
// throws. An answer is either the route's own (`ok` true) or a `Failure`, and none
// of the route's fields can be read before `ok` is checked: the type checker
// refuses a caller that reads the answer without first meeting the failure. (A
// caller that drops the answer altogether is refused by `no_dropped_failures.test.ts`.)
// What the page *shows* does not come from here at all -- it arrives on the event
// stream (live.ts); this is for what the person does, and for the few things looked
// up on demand.

import type { Routes } from './generated/routes'

/** A request that failed: the server's reason, or why it was never answered. Never empty. */
export interface Failure {
  ok: false
  error: string
}
/** A route's answer when it was taken: its own shape, with `ok` true (a declared `ok: false` form is the failure's). */
export type Success<T> = T extends { ok: false } ? never : T & { ok: true }
/** What a request comes back with: the route's answer, or a failure. */
export type Answer<T = Record<string, unknown>> = Success<T> | Failure

/** A failure in the page's own words, for what never reached the server or never came back readable. */
export function failure(error: string): Failure {
  return { ok: false, error }
}

// The server's answer, read strictly: an object whose `ok` is not false is the
// route's answer; `ok: false`, or a status that is not a success, is a failure with
// the server's reason (or the status, where it gave none); anything else is an answer
// the page cannot read, and is said as one.
async function answered<T>(r: Response): Promise<Answer<T>> {
  const text = await r.text()
  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    return failure(r.ok ? 'Bagholder answered in a form this page cannot read.' : 'Bagholder answered ' + r.status + (r.statusText ? ' ' + r.statusText : '') + '.')
  }
  if (typeof body !== 'object' || body === null || Array.isArray(body)) return failure('Bagholder answered in a form this page cannot read.')
  const o = body as Record<string, unknown>
  if (o.ok === false || !r.ok) {
    const why = typeof o.error === 'string' && o.error ? o.error : 'Bagholder answered ' + r.status + (r.statusText ? ' ' + r.statusText : '') + ' without a reason.'
    return failure(why)
  }
  return { ...o, ok: true } as unknown as Answer<T>
}

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
    .then((r) => answered<T>(r))
    .catch((e: unknown) => failure(String(e)))
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

/** The method, the URL and the body `key` is sent with, for `input`. */
function routed<K extends RouteKey>(key: K, input?: RouteInput<K>): { method: 'GET' | 'POST'; url: string; body?: unknown } {
  const [method, path] = key.split(' ', 2) as ['GET' | 'POST', string]
  const given = input as { query?: Record<string, Param>; body?: unknown } | undefined
  const q = given?.query ? query(given.query) : ''
  return { method, url: q ? path + '?' + q : path, body: given?.body }
}

export function call<K extends RouteKey>(key: K, input?: RouteInput<K>, signal?: AbortSignal): Promise<Answer<Routes[K]['answer']>> {
  const r = routed(key, input)
  return request<Routes[K]['answer']>(r.method, r.url, r.body, signal)
}
