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

export type Step = string | { k: string; v: string }
export type Op = ['set', Step[], unknown] | ['del', Step[]] | ['rows', Step[], string, string[], Record<string, unknown>]

type Obj = Record<string, unknown>

// The fields a row may be told apart by, tried in this order: the same list, in the
// same order, as the server's (patch.rs KEYS).
const KEYS = ['id', 'key', 'd', 'year', 'symbol', 'grade', 'label', 'date', 'name']

const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v)
const keyText = (v: unknown): string | null => (typeof v === 'string' ? v : typeof v === 'number' ? String(v) : null)

/** The field that tells this list's rows apart: every row has it, no two share it. */
export function rowKey(rows: unknown[]): string | null {
  if (!rows.length || !rows.every(isObj)) return null
  for (const k of KEYS) {
    const seen = new Set<string>()
    if (rows.every((r) => { const t = keyText((r as Obj)[k]); return t !== null && !seen.has(t) && !!seen.add(t) })) return k
  }
  return null
}

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

/**
 * Make `target` say what `source` says, keeping every object that is still there.
 * For a whole view arriving over one already shown (the filters changed, the
 * connection was lost and made again): a trade in both is the same object
 * afterwards, with only its differing fields written, so its row is not rebuilt.
 */
export function reconcile(target: Obj, source: Obj): void {
  for (const k of Object.keys(target)) if (!(k in source)) delete target[k]
  for (const [k, v] of Object.entries(source)) {
    const was = target[k]
    if (isObj(was) && isObj(v)) reconcile(was, v)
    else if (Array.isArray(was) && Array.isArray(v)) reconcileRows(was, v)
    else if (!same(was, v)) target[k] = v
  }
}

function reconcileRows(target: unknown[], source: unknown[]): void {
  const key = rowKey(source)
  if (!key || (target.length > 0 && rowKey(target) !== key)) {
    if (!same(target, source)) target.splice(0, target.length, ...source)
    return
  }
  const have = new Map<string, Obj>()
  for (const r of target) have.set(keyText((r as Obj)[key]) ?? '', r as Obj)
  const next = source.map((r) => {
    const was = have.get(keyText((r as Obj)[key]) ?? '')
    if (!was) return r
    reconcile(was, r as Obj)
    return was
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

let source: EventSource | null = null
let url = ''
let streamId = 0
const wanted = new Map<string, { params: unknown; holder: Holder<unknown>; changed?: () => void }>()

/** What runs after a change to the model is written, with the ids it touched (the open trade refreshes its fills). */
let afterChange: (touched: Set<string>) => void = () => {}
export function onChange(fn: (touched: Set<string>) => void): void {
  afterChange = fn
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
    fetch('/api/events/watch', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Bagholder': '1' }, body: JSON.stringify({ id: streamId, docs }) }).catch(() => {})
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
  const next = '/api/events?filters=' + encodeURIComponent(JSON.stringify(filters))
  if (source && next === url && source.readyState !== EventSource.CLOSED) return
  source?.close()
  url = next
  streamId = 0
  if (!sink.model) sink.loading = true
  const es = new EventSource(next)
  source = es
  es.addEventListener('hello', (e) => {
    streamId = (JSON.parse((e as MessageEvent).data) as { id: number }).id
    sayWanted() // a new stream knows nothing of what this page shows
  })
  es.addEventListener('snapshot', (e) => {
    const { doc, data } = JSON.parse((e as MessageEvent).data) as { doc: string; data: unknown }
    if (doc === 'model') {
      if (sink.model) reconcile(sink.model as unknown as Obj, data as Obj)
      else sink.model = data as Model
      sink.error = null
      sink.loading = false
      return
    }
    const w = wanted.get(doc)
    if (!w) return
    if (isObj(w.holder.data) && isObj(data)) reconcile(w.holder.data, data)
    else w.holder.data = data
    w.changed?.()
  })
  es.addEventListener('patch', (e) => {
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
