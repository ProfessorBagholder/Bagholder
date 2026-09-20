// Notifications: loaded once, then kept live over the SSE stream. Newest first.

// A notification's `extra`: the symbol/exchange it is about, the moment it
// happened (`at`, a day or a full timestamp), and its link (a filed document id
// with its source, or an external url). Drives the timestamp word and the click
// target, exactly as ledger.html's noteWhenWord/noteOpen read n.extra.
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
  id: string
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

export const notesStore = $state<{ rows: Note[]; unread: number; loaded: boolean }>({ rows: [], unread: 0, loaded: false })

let source: EventSource | null = null

export async function loadNotes(): Promise<void> {
  try {
    const r = await fetch('/api/notifications')
    const d = await r.json()
    if (d.ok) {
      notesStore.rows = d.rows ?? []
      notesStore.unread = d.unread ?? notesStore.rows.filter((n) => !n.readAt).length
    }
  } catch {
    /* leave */
  }
  notesStore.loaded = true
}

// The stream keeps the bell live without polling — one event, one row prepended.
export function startNotesStream(): () => void {
  loadNotes()
  try {
    source = new EventSource('/api/notifications/stream')
    source.onmessage = (e) => {
      try {
        const n = JSON.parse(e.data) as Note
        if (!n || !n.id) return
        if (notesStore.rows.some((x) => x.id === n.id)) return
        notesStore.rows = [n, ...notesStore.rows]
        if (!n.readAt) notesStore.unread++
      } catch {
        /* ignore malformed frame */
      }
    }
    source.onerror = () => { /* EventSource auto-reconnects */ }
  } catch {
    /* SSE unavailable */
  }
  return () => { source?.close(); source = null }
}

export async function markAllRead(): Promise<void> {
  if (!notesStore.rows.some((n) => !n.readAt)) return
  const now = new Date().toISOString()
  notesStore.rows.forEach((n) => { if (!n.readAt) n.readAt = now })
  notesStore.unread = 0
  try { await fetch('/api/notifications/read', { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' }) } catch { /* ignore */ }
}

export async function clearNotes(): Promise<void> {
  notesStore.rows = []
  notesStore.unread = 0
  try { await fetch('/api/notifications/clear', { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' }) } catch { /* ignore */ }
}
