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

import { post } from './api'
import { PROTOCOL } from './protocol'
import { ROW_KEYS } from './generated/keys'
import { bookIs, lastBook, load, save } from './kept'

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

/** Where a document the page is showing is kept: `data` is written into, never replaced; `v` is the version of it held. */
export interface Holder<T> {
  data: T | null
  v?: string
  error?: string
}

/** The connection itself: whether it is reaching the server. */
export const conn = $state<{ error: string; open: boolean }>({ error: '', open: false })

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
let streamId = 0
/** The book the page shows: the one it last showed until the server names it. */
let book = lastBook()
/** What is being drawn from what was kept: the page says what it shows once these are in. */
const drawing = new Set<Promise<void>>()
const wanted = new Map<string, { params: unknown; holder: Holder<unknown>; changed?: () => void }>()

/** Which server answered: when it started, the version it runs and the protocol it speaks (from its status). */
export interface ServerStamp {
  startedAt: string
  version: string
  protocol: string
}
function stamp(s: unknown): ServerStamp | null {
  if (!isObj(s) || typeof s.startedAt !== 'string' || !s.startedAt) return null
  const { startedAt, version, protocol } = s
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
 * What runs when the status arrives from a server started since the one that sent the
 * last: what was kept of its answers is no longer its word. After an update it runs
 * before anything is taken, which the old build then does not take at all.
 */
let afterRestart: (was: ServerStamp, now: ServerStamp) => void = () => {}
export function onRestart(fn: (was: ServerStamp, now: ServerStamp) => void): void {
  afterRestart = fn
}

// Tell the server what this page is showing: once per turn of the page however many
// things opened and closed in it, and not at all when the set is what was last said
// (a card that closes and opens again in one update says nothing). Each carries the
// version of it the page holds, so what has not changed is not sent again.
let saidFor = 0
let said = ''
let saying = false
/** What the stream was opened naming: said already, so a hello is not followed by the same again. */
let connectedWith = ''

/** What the page shows now, and the version of each it holds, as the server is told it. */
function shownNow(): { docs: Record<string, unknown>; have: Record<string, string> } {
  const docs: Record<string, unknown> = {}
  const have: Record<string, string> = {}
  for (const key of [...wanted.keys()].sort()) {
    const w = wanted.get(key)!
    docs[key] = w.params ?? {}
    if (w.holder.v && w.holder.data != null) have[key] = w.holder.v
  }
  return { docs, have }
}
function sayWanted(): void {
  if (saying) return
  saying = true
  queueMicrotask(async () => {
    // a screen drawn from what was kept says the version it holds
    while (drawing.size) await Promise.all([...drawing])
    saying = false
    if (!streamId) return
    const { docs, have } = shownNow()
    const now = JSON.stringify(docs)
    if (saidFor === streamId && now === said) return
    saidFor = streamId
    said = now
    post('/api/events/watch', { id: streamId, docs, have }).then((r) => {
      if (!r.ok) conn.error = 'Bagholder did not take what this page shows: ' + r.error
    })
  })
}

/**
 * Show a document for as long as something on the page needs it: a screen's
 * figures while it is open, the orders while their panel is. It arrives whole once
 * and then by change, written into `holder.data`; `changed` runs after each.
 * Returns what stops it.
 */
export function watchDoc<T>(key: string, params: unknown, holder: Holder<T>, changed?: () => void): () => void {
  // this watch's own entry: a later watch of the key into the same slot (the
  // filters changed) replaces it, and stopping this one then leaves that alone
  const entry = { params, holder: holder as Holder<unknown>, changed }
  wanted.set(key, entry)
  if (holder.data == null) {
    // drawn at once from what was kept, before the server answers
    const p = load(book, key, params).then((k) => {
      if (k && holder.data == null && wanted.get(key) === entry) {
        holder.data = k.data as T
        holder.v = k.v
        changed?.()
      }
    })
    drawing.add(p)
    p.finally(() => drawing.delete(p))
  }
  sayWanted()
  return () => {
    if (wanted.get(key) === entry) {
      wanted.delete(key)
      sayWanted()
    }
  }
}

/**
 * The server this page load has heard answer: never one drawn from what was kept.
 * A kept status is the word of a server heard on an earlier load; compared with it,
 * the new build a reload fetched would read as the old one and load itself again,
 * for ever.
 */
let heard: ServerStamp | null = null

/** A new state of the header's status: from a server started again, and was it an update? */
function restarted(now: unknown): boolean {
  const a = heard
  const b = stamp(now)
  if (b) heard = b
  if (!a || !b || a.startedAt === b.startedAt) return false
  if (isUpdate(a, b)) {
    afterRestart(a, b) // the page loads itself again: this build takes nothing more from the new server
    return true
  }
  afterRestart(a, b)
  return false
}

/**
 * Connect: one stream, over which every document the page shows arrives. The
 * browser connects again by itself when it drops; the page then says again what
 * it shows, with the version of each it holds, and is sent what changed.
 */
let connecting = false
export async function connect(): Promise<void> {
  if (connecting || (source && source.readyState !== EventSource.CLOSED)) return
  connecting = true
  // what the screens on show read is registered as they start, and what was kept of
  // them drawn: then the stream is opened naming them, with the version of each held,
  // so its first message already answers them
  await Promise.resolve()
  while (drawing.size) await Promise.all([...drawing])
  connecting = false
  if (source && source.readyState !== EventSource.CLOSED) return
  // the browser's time zone: the person's days, months and "today" are in it
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const { docs, have } = shownNow()
  connectedWith = JSON.stringify(docs)
  const es = new EventSource('/api/events?zone=' + encodeURIComponent(zone) + '&docs=' + encodeURIComponent(connectedWith) + '&have=' + encodeURIComponent(JSON.stringify(have)))
  source = es
  streamId = 0
  const seen = numbering(() => {
    if (streamId) post('/api/events/resync', { id: streamId }).then((r) => {
      if (!r.ok) conn.error = 'Bagholder did not send this page what it missed: ' + r.error
    })
  })
  es.addEventListener('hello', (e) => {
    seen((e as MessageEvent).lastEventId)
    const hello = JSON.parse((e as MessageEvent).data) as { id: number; book: string }
    streamId = hello.id
    conn.open = true
    conn.error = ''
    // the stream was opened naming what the page showed then: only a difference is said
    saidFor = streamId
    said = connectedWith
    if (hello.book && hello.book !== book) {
      // another book than the one drawn: nothing kept of that one stands
      if (book) for (const w of wanted.values()) {
        w.holder.data = null
        w.holder.v = undefined
      }
      book = hello.book
      void bookIs(book)
      saidFor = 0 // said again, holding nothing of this book
    }
    sayWanted()
  })
  es.addEventListener('snapshot', (e) => {
    seen((e as MessageEvent).lastEventId)
    const { doc, data, v } = JSON.parse((e as MessageEvent).data) as { doc: string; data: unknown; v: string }
    const w = wanted.get(doc)
    if (!w) return
    if (doc === 'status' && restarted(data)) return
    if (isObj(w.holder.data) && isObj(data)) reconcile(w.holder.data, data, ROW_KEYS[docKind(doc)] ?? {})
    else w.holder.data = data
    w.holder.v = v
    w.holder.error = ''
    w.changed?.()
  })
  es.addEventListener('same', (e) => {
    seen((e as MessageEvent).lastEventId)
    const { doc, v } = JSON.parse((e as MessageEvent).data) as { doc: string; v: string }
    const w = wanted.get(doc)
    if (w) w.holder.v = v
  })
  es.addEventListener('patch', (e) => {
    seen((e as MessageEvent).lastEventId)
    const { doc, ops, v } = JSON.parse((e as MessageEvent).data) as { doc: string; ops: Op[]; v: string }
    const w = wanted.get(doc)
    if (w?.holder.data == null) return
    applyOps(w.holder.data, ops)
    w.holder.v = v
    w.changed?.()
  })
  es.addEventListener('refused', (e) => {
    seen((e as MessageEvent).lastEventId)
    const { doc, error } = JSON.parse((e as MessageEvent).data) as { doc: string; error: string }
    const w = wanted.get(doc)
    if (w) w.holder.error = error
    else if (!doc) {
      // the stream could not read what the page shows: said, and said again the other way
      conn.error = error
      saidFor = 0
      sayWanted()
    }
  })
  es.onerror = () => {
    streamId = 0
    conn.open = false
    conn.error = 'Could not reach Bagholder.'
  }
}

/** The server's word over what the page holds: every document sent whole again, reconciled into what is shown. */
export function resyncAll(): void {
  if (!streamId) return
  post('/api/events/resync', { id: streamId }).then((r) => {
    if (!r.ok) conn.error = 'Bagholder did not send this page its state again: ' + r.error
  })
}

/** Keep every document shown as it stands: when the page is hidden or left, the moment it may not come back. */
export function keepShown(): void {
  const docs = [...wanted.entries()].filter(([, w]) => w.holder.data != null && w.holder.v).map(([key, w]) => ({ key, params: w.params, data: $state.snapshot(w.holder.data), v: w.holder.v! }))
  void save(book, docs)
}
if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') keepShown()
  })
  window.addEventListener('pagehide', keepShown)
}

export function disconnect(): void {
  source?.close()
  source = null
  streamId = 0
  conn.open = false
}
