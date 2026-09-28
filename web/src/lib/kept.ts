// What the page keeps of each screen between opens, in the browser's own database
// (docs/architecture.md §13, "What was loaded is kept between opens").
//
// Each document the page showed is kept with its version, filed under the book's
// id and the protocol the page speaks, so one book's figures are never shown for
// another's and a build that speaks differently starts afresh. On opening, the page
// draws each screen from what it kept at once and tells the server the version it
// holds; a screen that has not changed is not sent again.
//
// What is kept is a copy that makes an open instant, never the only copy of
// anything: the browser may clear it whenever it likes (it is best-effort storage),
// and then the page loads as it would the first time. It never holds a credential.

import { PROTOCOL } from './protocol'

const DB = 'bagholder'
const STORE = 'docs'
const BOOK = 'bh2.book'

export interface Kept {
  data: unknown
  v: string
}

let db: Promise<IDBDatabase | null> | null = null

/** The database, or null where the browser keeps none (a private window, storage turned off). */
function open(): Promise<IDBDatabase | null> {
  if (db) return db
  db = new Promise((resolve) => {
    let req: IDBOpenDBRequest
    try {
      req = indexedDB.open(DB, 1)
    } catch {
      resolve(null)
      return
    }
    req.onupgradeneeded = () => {
      req.result.createObjectStore(STORE)
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => resolve(null)
    req.onblocked = () => resolve(null)
  })
  return db
}

/** The book the page last showed, as the server named it: what it drew from before the server answers. */
export function lastBook(): string {
  try {
    return localStorage.getItem(BOOK) || ''
  } catch {
    return ''
  }
}

function rememberBook(book: string): void {
  try {
    localStorage.setItem(BOOK, book)
  } catch {
    /* the next open draws nothing before the server answers: slower, not wrong */
  }
}

const prefix = (book: string) => PROTOCOL + '|' + book + '|'
const name = (book: string, key: string, params: unknown) => prefix(book) + key + '|' + JSON.stringify(params ?? {})

/**
 * Everything kept for the book, read into memory once as the page opens: a screen
 * opened later is drawn from it in the same moment, never after a read of the
 * browser's database that would let a frame of the placeholder through.
 */
const held = new Map<string, Kept>()
let heldFor = ''
let reading: Promise<void> | null = null
let read = false

/** Read what was kept for `book` into memory, once; later calls wait on the same read. */
export function readKept(book: string): Promise<void> {
  if (book === heldFor && reading) return reading
  heldFor = book
  held.clear()
  read = false
  reading = (async () => {
    const d = book ? await open() : null
    if (d) {
      await new Promise<void>((resolve) => {
        try {
          const from = prefix(book)
          const req = d.transaction(STORE, 'readonly').objectStore(STORE).openCursor(IDBKeyRange.bound(from, from + '\uffff'))
          req.onsuccess = () => {
            const c = req.result
            if (!c) return resolve()
            const k = c.value as Kept | undefined
            if (k && typeof k.v === 'string' && heldFor === book) held.set(String(c.key), k)
            c.continue()
          }
          req.onerror = () => resolve()
        } catch {
          resolve()
        }
      })
    }
    if (heldFor === book) read = true
  })()
  return reading
}

/** Whether what was kept for `book` is in memory. */
export function keptRead(book: string): boolean {
  return read && heldFor === book
}

/** What was kept of `key` under `params` for `book`, from memory (a copy of it), or null. */
export function kept(book: string, key: string, params: unknown): Kept | null {
  if (!keptRead(book)) return null
  const k = held.get(name(book, key, params))
  return k ? { data: structuredClone(k.data), v: k.v } : null
}

/**
 * The last thing kept of `key` for `book` under any parameters (a copy of it), or
 * null: what a screen whose filters or sort changed shows until the server answers
 * it. Its version is not the one asked for, so it is never named to the server.
 */
export function keptLatest(book: string, key: string): Kept | null {
  if (!keptRead(book)) return null
  const of = prefix(book) + key + '|'
  let last: Kept | undefined
  for (const [k, v] of held) if (k.startsWith(of)) last = v
  return last ? { data: structuredClone(last.data), v: '' } : null
}

/** Keep each document as it stands now. */
export async function save(book: string, docs: { key: string; params: unknown; data: unknown; v: string }[]): Promise<void> {
  if (!book || !docs.length) return
  // the last kept of each is the newest in memory: what a screen under new parameters shows first
  if (book === heldFor)
    for (const x of docs) {
      const n = name(book, x.key, x.params)
      held.delete(n)
      held.set(n, { data: x.data, v: x.v })
    }
  const d = await open()
  if (!d) return
  try {
    const tx = d.transaction(STORE, 'readwrite')
    const store = tx.objectStore(STORE)
    for (const x of docs) store.put({ data: x.data, v: x.v } satisfies Kept, name(book, x.key, x.params))
  } catch {
    /* kept at the next change, or not at all: the next open loads afresh */
  }
}

/**
 * The server named its book: remembered, and anything kept under another book or
 * another protocol dropped, so nothing of the book as it was (before Clear data,
 * or another data folder) is shown again.
 */
export async function bookIs(book: string): Promise<void> {
  const was = lastBook()
  rememberBook(book)
  if (was === book) return
  const d = await open()
  if (!d) return
  try {
    const store = d.transaction(STORE, 'readwrite').objectStore(STORE)
    const req = store.openCursor()
    req.onsuccess = () => {
      const c = req.result
      if (!c) return
      if (!String(c.key).startsWith(prefix(book))) c.delete()
      c.continue()
    }
  } catch {
    /* left for the next time the book is named */
  }
}
