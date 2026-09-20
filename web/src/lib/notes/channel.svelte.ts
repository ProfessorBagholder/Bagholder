// Notifications: the server tells, and something shows. Where the computer the server
// runs on has a desktop the server posts them itself ("native"); only where it has none
// is this browser asked to, under the browser's own permission, which it gives only on
// a click. The kinds are the server's settings; the permission is this browser's.

import { post } from '../api'
import { store } from '../state.svelte'
import { ui } from '../ui.svelte'
import { panel } from '../orders/orders.svelte'
import { goSub } from '../router.svelte'
import type { Note } from './notes.svelte'

export type Channel = 'native' | 'granted' | 'denied' | 'default' | 'unavailable'

// the browser's answer, re-read after asking (it does not announce a change)
const asked = $state({ permission: typeof Notification === 'undefined' ? '' : Notification.permission })

const settings = () => (store.model?.status?.notify ?? {}) as Record<string, unknown>

export function channel(): Channel {
  if (settings().native) return 'native'
  return typeof Notification === 'undefined' ? 'unavailable' : (asked.permission as Channel)
}

/** Whether a kind is on and there is something to show it. */
export function notifyOn(kind: string): boolean {
  const c = channel()
  return (c === 'native' || c === 'granted') && !!settings()[kind]
}

/** Nothing can be shown: the switches are dimmed. */
export const notifyDead = (): boolean => channel() === 'unavailable' || channel() === 'denied'

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

// shown at once; the server's own account of its settings follows on the stream
function save(patch: Record<string, boolean>): void {
  const cur = store.model?.status?.notify
  if (cur) Object.assign(cur, patch)
  void post('/api/notifications/settings', patch)
}

export async function notifyToggle(kind: string): Promise<void> {
  const c = channel()
  if (c === 'unavailable' || c === 'denied') return
  if (c === 'default') {
    if (await ask()) save({ [kind]: true })
    return
  }
  save({ [kind]: !notifyOn(kind) })
}

export async function notifyTest(): Promise<void> {
  const c = channel()
  if (c === 'unavailable' || c === 'denied') return
  if (c === 'default' && !(await ask())) return
  void post('/api/notifications/test')
}

/** What a notification leads to when it is about an order or a listing the book holds. */
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
    const held = store.model?.positions.find((p) => p.symbol === symbol)
    if (held) {
      goSub('portfolio', held.id)
      return true
    }
  }
  return false
}

/** A notification that has just arrived. Where the page is the channel it shows the banner, once. */
export function arrived(n: Note): void {
  if (ui.notesOpen) {
    // the panel is open: it is read as it lands
    n.readAt = new Date().toISOString()
    void post('/api/notifications/read', { ids: [n.id] })
  }
  if (channel() !== 'granted' || n.seenAt) return
  n.seenAt = new Date().toISOString()
  void post('/api/notifications/seen', { ids: [n.id] })
  let banner: Notification
  try {
    banner = new Notification(n.title, { body: n.body || '', icon: '/favicon.png', tag: 'bh-' + n.id })
  } catch {
    return
  }
  banner.onclick = () => {
    window.focus()
    openNoteTarget(n)
    banner.close()
  }
}
