// Notifications are the browser's (`docs/plans/notifications-through-the-browser.md`):
// this page's service worker (/sw.js) shows each one, from this page while it is
// open, and from a push the server sends through the browser's own push service
// while no Bagholder tab is. The browser asks the person once, from a click, and
// never on its own. The kinds are the server's settings; the permission and the
// subscription are this browser's.

import { status } from '../subs.svelte'
import { call } from '../api'
import { quiet, tell, ui } from '../ui.svelte'
import { resyncAll } from '../live.svelte'
import { panel } from '../orders/orders.svelte'
import { goSub } from '../router.svelte'
import { rememberListing } from '../listing.svelte'
import { keyBytes } from './keybytes'
import type { Note } from './notes.svelte'

export type Channel = 'granted' | 'denied' | 'default' | 'unavailable'

// the browser's answer, re-read after asking (it does not announce a change)
const asked = $state({ permission: typeof Notification === 'undefined' ? '' : Notification.permission, subscribed: false })

const settings = () => (status.data?.notify ?? {}) as Record<string, unknown>

/** The service worker, registered once: none where the browser has none. */
const worker: Promise<ServiceWorkerRegistration | null> =
  typeof navigator !== 'undefined' && 'serviceWorker' in navigator
    ? navigator.serviceWorker.register('/sw.js').then(
        (r) => r,
        (e) => {
          quiet('the notifications worker could not be registered', String(e))
          return null
        },
      )
    : Promise.resolve(null)

// whether this browser holds a subscription to this copy, read once the worker is ready
worker.then((r) => r?.pushManager?.getSubscription().then((s) => (asked.subscribed = !!s)))

// a click on a notification, while this page is open: its row's own page
if (typeof navigator !== 'undefined' && 'serviceWorker' in navigator) {
  navigator.serviceWorker.addEventListener('message', (e: MessageEvent) => {
    const m = e.data as { type?: string; row?: Note } | null
    if (m?.type === 'bh-note' && m.row) openNoteTarget(m.row)
  })
}

export function channel(): Channel {
  return typeof Notification === 'undefined' || typeof navigator === 'undefined' || !('serviceWorker' in navigator) ? 'unavailable' : (asked.permission as Channel)
}

/** Whether this browser is subscribed, so a row reaches it with no tab open. */
export const notifySubscribed = (): boolean => asked.subscribed

/** Whether a kind is on and this browser can show it. */
export function notifyOn(kind: string): boolean {
  return channel() === 'granted' && !!settings()[kind]
}

/** The key a browser subscribes with, once the server's status has brought it. */
const pushKey = (): string => (settings().push as { key?: string } | undefined)?.key ?? ''

/** Nothing can be shown, or not yet: the switches are dimmed until the server's key is here. */
export const notifyDead = (): boolean => channel() === 'unavailable' || channel() === 'denied' || !pushKey()

// the permission prompt, whichever way this browser answers it (a promise, or the older callback)
function ask(): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false
    const settle = (r: string) => {
      if (done) return
      done = true
      asked.permission = Notification.permission
      resolve(r === 'granted')
    }
    try {
      const p = Notification.requestPermission(settle)
      if (p && p.then) p.then(settle)
    } catch {
      settle('')
    }
  })
}

/**
 * Turn on notifications, from a click: the browser's permission, then this
 * browser's subscription to this copy's pushes, kept by the server. Each step
 * that fails is said.
 */
export async function turnOn(): Promise<boolean> {
  if (channel() === 'unavailable' || channel() === 'denied') return false
  if (channel() === 'default' && !(await ask())) return false
  const reg = await worker
  const key = pushKey()
  if (!reg) {
    tell('Notifications could not be turned on: this browser did not start the page\'s notifications worker', 'err')
    return false
  }
  if (!key) return false
  try {
    const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) as BufferSource })
    const r = await call('POST /api/push/subscribe', { body: sub.toJSON() as never })
    if (!r.ok) {
      tell('Notifications could not be turned on: ' + r.error, 'err')
      return false
    }
    asked.subscribed = true
    return true
  } catch (e) {
    tell('This browser could not subscribe to notifications: ' + String(e), 'err')
    return false
  }
}

// shown at once; the server's own account of its settings follows on the stream, and
// a change it refused is said in the header, the switches put back as the server has them
function save(patch: Record<string, boolean>): void {
  const cur = status.data?.notify
  if (cur) Object.assign(cur, patch)
  call('POST /api/notifications/settings', { body: patch }).then((r) => {
    if (r.ok) return
    tell('Could not save the notification settings: ' + r.error, 'err')
    resyncAll()
  })
}

export async function notifyToggle(kind: string): Promise<void> {
  const c = channel()
  if (c === 'unavailable' || c === 'denied') return
  if (c === 'default' || !asked.subscribed) {
    if (await turnOn()) save({ [kind]: true })
    return
  }
  save({ [kind]: !notifyOn(kind) })
}

export async function notifyTest(): Promise<void> {
  const c = channel()
  if (c === 'unavailable' || c === 'denied') return
  if ((c === 'default' || !asked.subscribed) && !(await turnOn())) return
  call('POST /api/notifications/test').then((r) => {
    if (!r.ok) tell('Could not send a test notification: ' + r.error, 'err')
  })
}

/**
 * What a banner leads to: an order's opens the Orders panel at its tab; a disclosure's
 * or a release's opens the instrument's page, the listing's, which is the holding's
 * where the book holds it (the server says which, by the listing, never a symbol
 * matched across accounts here).
 */
export function openNoteTarget(n: Note): boolean {
  if (n.kind === 'fills' || n.kind === 'problems') {
    ui.menuOpen = false
    ui.notesOpen = false
    panel.tab = n.kind === 'fills' ? 'filled' : 'cancelled'
    ui.ordersOpen = true
    return true
  }
  const symbol = n.extra?.symbol
  if ((n.kind === 'disclosures' || n.kind === 'releases') && symbol) {
    ui.menuOpen = false
    ui.notesOpen = false
    goSub('markets', rememberListing({ symbol: String(symbol), exchange: n.extra?.exchange, listing: String(n.extra?.listing || '') }))
    return true
  }
  return false
}

/** A notification that has just arrived. This page shows its banner, once, through the worker. */
export function arrived(n: Note): void {
  if (ui.notesOpen) {
    // the panel is open: it is read as it lands
    n.readAt = new Date().toISOString()
    call('POST /api/notifications/read', { body: { ids: [n.id] } }).then((r) => {
      if (r.ok) return
      // the panel marks it again as it is read again (resyncAll)
      quiet('a notification could not be marked read', r.error)
      resyncAll()
    })
  }
  if (channel() !== 'granted' || n.seenAt) return
  n.seenAt = new Date().toISOString()
  // not marked seen, another page (or this one, opened again) would show the banner a second time: said
  call('POST /api/notifications/seen', { body: { ids: [n.id] } }).then((r) => {
    // its banner may show once more; nothing the person sees is wrong
    if (!r.ok) quiet('a notification could not be marked shown', r.error)
  })
  // the same tag as a push of it would carry: the browser shows one banner for both
  worker.then((reg) => reg?.showNotification(n.title, { body: n.body || '', icon: '/favicon.png', tag: 'bh-' + n.id, data: n }))
}

/** Opened from a notification while no tab was: the row's own page, once the history has it. */
export function openFromLink(rows: readonly Note[]): void {
  if (typeof location === 'undefined') return
  const want = new URLSearchParams(location.search).get('note')
  if (!want) return
  const n = rows.find((r) => String(r.id) === want)
  if (!n) return
  history.replaceState(null, '', location.pathname + location.hash)
  openNoteTarget(n)
}
