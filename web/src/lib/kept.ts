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

/** What was kept of `key` under `params` for `book`, or null. */
export async function load(book: string, key: string, params: unknown): Promise<Kept | null> {
  if (!book) return null
  const d = await open()
  if (!d) return null
  return new Promise((resolve) => {
    try {
      const req = d.transaction(STORE, 'readonly').objectStore(STORE).get(name(book, key, params))
      req.onsuccess = () => {
        const k = req.result as Kept | undefined
        resolve(k && typeof k.v === 'string' ? k : null)
      }
      req.onerror = () => resolve(null)
    } catch {
      resolve(null)
    }
  })
}

/** Keep each document as it stands now. */
export async function save(book: string, docs: { key: string; params: unknown; data: unknown; v: string }[]): Promise<void> {
  if (!book || !docs.length) return
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
