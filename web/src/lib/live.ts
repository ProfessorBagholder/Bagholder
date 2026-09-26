// The page's one connection to the server, and the only way its data changes
// (docs/architecture.md, rules 0 and 2).
//
// The server sends the whole view once, then only what moves in it: the entities
// and the fields that differ, keyed by the entity's own identity
// (rust/crates/model/src/patch.rs). Every entity on the page -- a position, a trade,
// a tile, a headline -- is one object that lives as long as the entity does, and a
// change is written *into* it. Svelte's reactivity is per field, so writing one
// field updates the one element bound to it and nothing else: no list is replaced,
// no card re-rendered, no model refetched. A price moving on one holding writes
// that holding's figures and the totals that include it.
//
// Nothing here polls. There is no timer in this file.

import type { Model } from './model'

import { post } from './api'
import { PROTOCOL } from './protocol'
import { ROW_KEYS } from './generated/keys'

export type Step = string | { k: string; v: string }
export type Op = ['set', Step[], unknown] | ['del', Step[]] | ['rows', Step[], string, string[], Record<string, unknown>]

type Obj = Record<string, unknown>

const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v)
const keyText = (v: unknown): string | null => (typeof v === 'string' ? v : typeof v === 'number' ? String(v) : null)


function walk(root: unknown, path: Step[]): unknown {
  let at: unknown = root
  for (const s of path) {
    if (at == null) return undefined
    if (typeof s === 'string') at = (at as Obj)[s]
    else at = Array.isArray(at) ? at.find((r) => isObj(r) && keyText(r[s.k]) === s.v) : undefined
  }
  return at
}

/** Write a change into the objects already held. Returns the ids of the rows it touched. */
export function applyOps(root: unknown, ops: Op[]): Set<string> {
  const touched = new Set<string>()
  for (const op of ops) {
    const path = op[1]
    for (const s of path) if (typeof s !== 'string') touched.add(s.v)
    if (op[0] === 'rows') {
      const list = walk(root, path)
      if (!Array.isArray(list)) continue
      const [, , key, order, added] = op
      const have = new Map<string, unknown>()
      for (const r of list) have.set(keyText((r as Obj)[key]) ?? '', r)
      // the rows that stay are the same objects, moved; only a new row is a new object
      const next = order.map((id) => have.get(id) ?? added[id]).filter((r) => r !== undefined)
      list.splice(0, list.length, ...next)
      continue
    }
    const last = path[path.length - 1]
    const parent = walk(root, path.slice(0, -1))
    if (parent == null) continue
    if (typeof last === 'string') {
      if (op[0] === 'set') (parent as Obj)[last] = op[2]
      else delete (parent as Obj)[last]
    } else if (op[0] === 'set' && Array.isArray(parent)) {
      const at = parent.findIndex((r) => isObj(r) && keyText(r[last.k]) === last.v)
      if (at >= 0) parent[at] = op[2]
    }
  }
  return touched
}

function same(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => same(x, b[i]))
  if (isObj(a) && isObj(b)) {
    const ka = Object.keys(a)
    return ka.length === Object.keys(b).length && ka.every((k) => k in b && same(a[k], b[k]))
  }
  return false
}

/** The document kind a key names: `model`, `orders`, or the part before `:` (`quote:…`). */
export const docKind = (doc: string): string => doc.split(':')[0]

/** The field that tells the rows of the list at `path` apart, as the server's differ keys it (`generated/keys.ts`). */
function keyAt(keys: Record<string, string>, path: string[]): string | null {
  for (const [p, field] of Object.entries(keys)) {
    const steps = p.split('.')
    if (steps.length === path.length && steps.every((s, i) => s === '*' || s === path[i])) return field
  }
  return null
}

/** Each row's key, when every row has one and no two share it: as the server's differ reads them. */
function keysOf(rows: unknown[], key: string): string[] | null {
  const seen = new Set<string>()
  const out: string[] = []
  for (const r of rows) {
    const t = isObj(r) ? keyText(r[key]) : null
    if (t === null || seen.has(t)) return null
    seen.add(t)
    out.push(t)
  }
  return out
}

/**
 * Make `target` say what `source` says, keeping every object that is still there.
 * For a whole view arriving over one already shown (the filters changed, the
 * connection was lost and made again): a trade in both is the same object
 * afterwards, with only its differing fields written, so its row is not rebuilt.
 * A list's rows are matched by the field its row type declares (`keys`, the
 * document's entry of `ROW_KEYS`); a list with none is written whole when it differs.
 */
export function reconcile(target: Obj, source: Obj, keys: Record<string, string>, path: string[] = []): void {
  for (const k of Object.keys(target)) if (!(k in source)) delete target[k]
  for (const [k, v] of Object.entries(source)) {
    const was = target[k]
    const at = [...path, k]
    if (isObj(was) && isObj(v)) reconcile(was, v, keys, at)
    else if (Array.isArray(was) && Array.isArray(v)) reconcileRows(was, v, keys, at)
    else if (!same(was, v)) target[k] = v
  }
}

function reconcileRows(target: unknown[], source: unknown[], keys: Record<string, string>, path: string[]): void {
  const key = keyAt(keys, path)
  const now = key ? keysOf(source, key) : null
  const was = key ? keysOf(target, key) : null
  if (!key || !now || !was) {
    if (!same(target, source)) target.splice(0, target.length, ...source)
    return
  }
  const have = new Map<string, Obj>()
  target.forEach((r, i) => have.set(was[i], r as Obj))
  const rows = [...path, '*']
  const next = source.map((r, i) => {
    const held = have.get(now[i])
    if (!held) return r
    reconcile(held, r as Obj, keys, rows)
    return held
  })
  if (next.length !== target.length || next.some((r, i) => r !== target[i])) target.splice(0, target.length, ...next)
}

// --- the connection ---------------------------------------------------------------

export interface Sink {
  model: Model | null
  error: string | null
  loading: boolean
}

/** Where a document the page is showing is kept: `data` is written into, never replaced. */
export interface Holder<T> {
  data: T | null
}

/**
 * The order a stream's messages are numbered in, from 1: `seen` takes each
 * message's number and calls `gap` when one is not the next, so a message lost
 * on the way is made good by the whole state (`POST /api/events/resync`).
 */
export function numbering(gap: () => void): (id: string) => void {
  let last = 0
  return (id: string) => {
    const n = Number(id)
    if (!Number.isInteger(n) || n <= 0) return
    if (n === 1) last = 0 // a new connection counts from the start
    if (last && n !== last + 1) gap()
    last = n
  }
}

let source: EventSource | null = null
let url = ''
let streamId = 0
const wanted = new Map<string, { params: unknown; holder: Holder<unknown>; changed?: () => void }>()

/** What runs after a change to the model is written, with the ids it touched, or 'all' for the whole view again (the open trade refreshes its fills). */
let afterChange: (touched: Set<string> | 'all') => void = () => {}
export function onChange(fn: (touched: Set<string> | 'all') => void): void {
  afterChange = fn
}

/** Which server answered: when it started, the version it runs and the protocol it speaks (from its status). */
export interface ServerStamp {
  startedAt: string
  version: string
  protocol: string
}
function stamp(m: unknown): ServerStamp | null {
  if (!isObj(m) || !isObj(m.status) || typeof m.status.startedAt !== 'string' || !m.status.startedAt) return null
  const { startedAt, version, protocol } = m.status
  return { startedAt, version: typeof version === 'string' ? version : '', protocol: typeof protocol === 'string' ? protocol : '' }
}

/**
 * Whether the server answering after a restart runs other code than this page: a
 * version other than the one the page last heard, or a protocol other than the one
 * the page was built to speak. The page is then the old build and loads itself again
 * (SPEC §2, Versions). The page that loads is the new build, and its first answer is
 * no restart, so it does not load again.
 */
export function isUpdate(was: ServerStamp, now: ServerStamp, built: string = PROTOCOL): boolean {
  return now.version !== was.version || (!!now.protocol && now.protocol !== built)
}

/**
 * What runs when the view arrives from a server started since the one that sent the
 * last: what was kept of its answers is no longer its word. After an update it runs
 * before the view is taken, which the old build then does not take at all.
 */
let afterRestart: (was: ServerStamp, now: ServerStamp) => void = () => {}
export function onRestart(fn: (was: ServerStamp, now: ServerStamp) => void): void {
  afterRestart = fn
}

// Tell the server what this page is showing beyond the model: once per turn of the
// page however many things opened and closed in it, and not at all when the set is
// what was last said (a card that closes and opens again in one update says nothing).
let saidFor = 0
let said = ''
let saying = false
function sayWanted(): void {
  if (saying) return
  saying = true
  queueMicrotask(() => {
    saying = false
    if (!streamId) return
    const docs: Record<string, unknown> = {}
    for (const key of [...wanted.keys()].sort()) docs[key] = wanted.get(key)!.params ?? {}
    const now = JSON.stringify(docs)
    if (saidFor === streamId && now === said) return
    saidFor = streamId
    said = now
    void post('/api/events/watch', { id: streamId, docs })
  })
}

/**
 * Show a document for as long as something on the page needs it: the orders while
 * their panel is open, the short-interest table while its card is. It arrives whole
 * once and then by change, written into `holder.data`; `changed` runs after each.
 * Returns what stops it.
 */
export function watchDoc<T>(key: string, params: unknown, holder: Holder<T>, changed?: () => void): () => void {
  wanted.set(key, { params, holder: holder as Holder<unknown>, changed })
  sayWanted()
  return () => {
    if (wanted.get(key)?.holder === holder) {
      wanted.delete(key)
      sayWanted()
    }
  }
}

/**
 * Connect, or connect again under other filters. The first message is the whole
 * view; when one is already shown it is reconciled into the objects on screen.
 */
export function connect(sink: Sink, filters: unknown): void {
  // the browser's time zone: the person's days, months and "today" are in it
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const next = '/api/events?filters=' + encodeURIComponent(JSON.stringify(filters)) + '&zone=' + encodeURIComponent(zone)
  if (source && next === url && source.readyState !== EventSource.CLOSED) return
  source?.close()
  url = next
  streamId = 0
  if (!sink.model) sink.loading = true
  const es = new EventSource(next)
  source = es
  const seen = numbering(() => {
    if (streamId) void post('/api/events/resync', { id: streamId })
  })
  es.addEventListener('hello', (e) => {
    seen((e as MessageEvent).lastEventId)
    streamId = (JSON.parse((e as MessageEvent).data) as { id: number }).id
    sayWanted() // a new stream knows nothing of what this page shows
  })
  es.addEventListener('snapshot', (e) => {
    seen((e as MessageEvent).lastEventId)
    const { doc, data } = JSON.parse((e as MessageEvent).data) as { doc: string; data: unknown }
    if (doc === 'model') {
      const shown = !!sink.model
      const was = stamp(sink.model)
      const now = stamp(data)
      const restarted = was && now && was.startedAt !== now.startedAt
      if (restarted && isUpdate(was, now)) {
        afterRestart(was, now) // the page loads itself again: this build does not take the new server's view
        return
      }
      if (sink.model) reconcile(sink.model as unknown as Obj, data as Obj, ROW_KEYS.model)
      else sink.model = data as Model
      if (restarted) afterRestart(was, now)
      if (shown) afterChange('all') // a view over the one shown: anything in it may have moved while away
      sink.error = null
      sink.loading = false
      return
    }
    const w = wanted.get(doc)
    if (!w) return
    if (isObj(w.holder.data) && isObj(data)) reconcile(w.holder.data, data, ROW_KEYS[docKind(doc)] ?? {})
    else w.holder.data = data
    w.changed?.()
  })
  es.addEventListener('patch', (e) => {
    seen((e as MessageEvent).lastEventId)
    const { doc, ops } = JSON.parse((e as MessageEvent).data) as { doc: string; ops: Op[] }
    if (doc === 'model') {
      if (sink.model) afterChange(applyOps(sink.model, ops))
      return
    }
    const w = wanted.get(doc)
    if (w?.holder.data == null) return
    applyOps(w.holder.data, ops)
    w.changed?.()
  })
  // the browser connects again by itself; the server then sends the whole view, which
  // is reconciled, so whatever was missed while away is made good
  es.onerror = () => {
    streamId = 0
    if (!sink.model) sink.error = 'Could not reach Bagholder.'
  }
}

export function disconnect(): void {
  source?.close()
  source = null
  url = ''
  streamId = 0
}
