// Bagholder's service worker (`docs/plans/notifications-through-the-browser.md`):
// what shows a notification, whether a Bagholder tab is open or not. Served as
// /sw.js, its scope the page's whole origin. It holds no state and caches nothing:
// it shows what it is sent, takes a click to the row's own page, and subscribes
// again when the browser renews a subscription. Its handlers are plain functions
// over the worker's scope, so a test drives them with a stand-in.

/** A notification row as the server sends it, whole or by its id alone. */
export interface Row {
  id: number
  title?: string
  body?: string
  kind?: string
  extra?: Record<string, unknown>
}

interface Shown {
  close(): void
  data?: unknown
}

interface Tab {
  url: string
  focus(): Promise<unknown>
  postMessage(message: unknown): void
}

interface Subscription {
  endpoint: string
  toJSON(): unknown
}

/** What the handlers use of the worker's scope. */
export interface Scope {
  location: { origin: string }
  registration: {
    showNotification(title: string, options: { body?: string; tag?: string; data?: unknown; icon?: string }): Promise<void>
    pushManager: { subscribe(options: { userVisibleOnly: boolean; applicationServerKey: Uint8Array }): Promise<Subscription> }
  }
  clients: {
    matchAll(options: { type: 'window'; includeUncontrolled: boolean }): Promise<readonly Tab[]>
    openWindow(url: string): Promise<unknown>
  }
  fetch(url: string, init?: { method?: string; headers?: Record<string, string>; body?: string }): Promise<{ ok: boolean; json(): Promise<unknown> }>
}

const WRITE = { 'Content-Type': 'application/json', 'X-Bagholder': '1' }

/** A base64url key as the bytes `pushManager.subscribe` takes. */
export function keyBytes(b64url: string): Uint8Array {
  const b64 = b64url.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (b64url.length % 4)) % 4)
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))
}

/** The history's rows, for a push that carried a row's id alone. */
async function rowById(scope: Scope, id: number): Promise<Row | null> {
  try {
    const r = await scope.fetch('/api/notifications')
    if (!r.ok) return null
    const doc = (await r.json()) as { rows?: Row[] }
    return (doc.rows ?? []).find((n) => n.id === id) ?? null
  } catch {
    return null
  }
}

/**
 * A push: always shown, never silent (Safari allows no push that shows nothing).
 * A row too long for a push came as its id alone and is fetched; one that cannot
 * be read at all still shows, as Bagholder's.
 */
export async function onPush(scope: Scope, data: { json(): unknown } | null): Promise<void> {
  let row: Row | null = null
  try {
    row = (data?.json() as Row) ?? null
  } catch {
    row = null
  }
  if (row && typeof row.id === 'number' && !row.title) row = (await rowById(scope, row.id)) ?? row
  const heading = row?.title || 'Bagholder'
  const tag = row && typeof row.id === 'number' ? 'bh-' + row.id : 'bh'
  return scope.registration.showNotification(heading, { body: row?.body || '', tag, data: row, icon: '/favicon.png' })
}

/**
 * A click: the notification goes, and the row's own page is shown. A Bagholder
 * tab is brought forward and told which row, so it opens the row's target as a
 * click in its history would (`openNoteTarget`); with none open, one is opened
 * on the row.
 */
export async function onClick(scope: Scope, shown: Shown): Promise<unknown> {
  shown.close()
  const row = (shown.data ?? null) as Row | null
  const tabs = await scope.clients.matchAll({ type: 'window', includeUncontrolled: true })
  const tab = tabs.find((t) => t.url.startsWith(scope.location.origin))
  if (tab) {
    tab.postMessage({ type: 'bh-note', row })
    return tab.focus()
  }
  return scope.clients.openWindow(row && typeof row.id === 'number' ? '/?note=' + row.id : '/')
}

/** The browser renewed a subscription: subscribe again with this copy's key, and say so. */
export async function onSubscriptionChange(scope: Scope, old: { endpoint: string } | null): Promise<void> {
  const r = await scope.fetch('/api/notifications')
  if (!r.ok) return
  const key = ((await r.json()) as { settings?: { push?: { key?: string } } }).settings?.push?.key
  if (!key) return
  const sub = await scope.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) })
  await scope.fetch('/api/push/subscribe', { method: 'POST', headers: WRITE, body: JSON.stringify(sub.toJSON()) })
  if (old && old.endpoint !== sub.endpoint) await scope.fetch('/api/push/unsubscribe', { method: 'POST', headers: WRITE, body: JSON.stringify({ endpoint: old.endpoint }) })
}

// bound to the worker's own events: `sw-worker.ts`, the file built as /sw.js, calls this
interface WaitingEvent {
  waitUntil(p: Promise<unknown>): void
}
type Worker = Scope & { addEventListener(type: string, f: (e: WaitingEvent & Record<string, unknown>) => void): void }

export function bind(worker: Worker): void {
  worker.addEventListener('push', (e) => e.waitUntil(onPush(worker, (e.data as { json(): unknown }) ?? null)))
  worker.addEventListener('notificationclick', (e) => e.waitUntil(onClick(worker, e.notification as Shown)))
  worker.addEventListener('pushsubscriptionchange', (e) => e.waitUntil(onSubscriptionChange(worker, (e.oldSubscription as { endpoint: string }) ?? null)))
}
