// Notifications, newest first.

// A notification's `extra`: the symbol/exchange it is about, the moment it
// happened (`at`, a day or a full timestamp), and its link (a filed document id
// with its source, or an external url). Drives the timestamp word and the click
// target, exactly as ledger.html's noteWhenWord/noteOpen read n.extra.
import { call } from '../api'
import { watchDoc, type Holder } from '../live'
import { arrived } from './channel.svelte'

export interface NoteExtra {
  symbol?: string
  exchange?: string
  at?: string
  doc?: string
  source?: string
  url?: string
  [k: string]: unknown
}

export interface Note {
  id: number
  kind: string
  key?: string
  title: string
  body?: string
  at: string
  readAt?: string | null
  seenAt?: string | null
  extra?: NoteExtra
  href?: string
}

// The bell is a document on the page's one stream (live.ts): it arrives whole once,
// then a new notification is a row inserted and "mark all read" is `readAt` set on
// the rows it touched. There is no second connection and nothing is asked again.
const doc = $state<Holder<{ rows: Note[]; unread: number }>>({ data: null })

export const notesStore = {
  get rows(): Note[] {
    return doc.data?.rows ?? []
  },
  get unread(): number {
    return doc.data?.unread ?? 0
  },
  get loaded(): boolean {
    return doc.data !== null
  },
}

/** Keep the bell current for as long as the page is open. Returns what stops it. */
export function showNotifications(): () => void {
  // a row this page has not had before has just arrived -- except the first time, when
  // the rows are the history
  let known: Set<Note['id']> | null = null
  return watchDoc('notifications', {}, doc, () => {
    const rows = doc.data?.rows ?? []
    if (known) for (const n of rows) if (!known.has(n.id)) arrived(n)
    known = new Set(rows.map((n) => n.id))
  })
}

// What the person does shows at once; the server's own account of it follows on
// the stream and is written over the same rows.
export async function markAllRead(): Promise<void> {
  if (!doc.data || !doc.data.rows.some((n) => !n.readAt)) return
  const now = new Date().toISOString()
  doc.data.rows.forEach((n) => { if (!n.readAt) n.readAt = now })
  doc.data.unread = 0
  await call('POST /api/notifications/read', { body: { ids: null } })
}

export async function clearNotes(): Promise<void> {
  if (doc.data) {
    doc.data.rows.splice(0)
    doc.data.unread = 0
  }
  await call('POST /api/notifications/clear')
}
