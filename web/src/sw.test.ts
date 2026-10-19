import { describe, expect, it } from 'vitest'
import { keyBytes as pageKeyBytes } from './lib/notes/keybytes'
import { keyBytes, onClick, onPush, onSubscriptionChange, type Row, type Scope } from './sw'

// the service worker's handlers, over a stand-in for its scope
// (`docs/plans/notifications-through-the-browser.md`)
function scope(opts: { tabs?: { url: string }[]; rows?: Row[]; key?: string } = {}) {
  const shown: { title: string; options: Record<string, unknown> }[] = []
  const posted: { tab: string; message: unknown }[] = []
  const focused: string[] = []
  const opened: string[] = []
  const fetched: { url: string; init?: { method?: string; body?: string } }[] = []
  const subscribed: Uint8Array[] = []
  const s: Scope = {
    location: { origin: 'http://127.0.0.1:8765' },
    registration: {
      showNotification: async (title, options) => {
        shown.push({ title, options: options as Record<string, unknown> })
      },
      pushManager: {
        subscribe: async (o) => {
          subscribed.push(o.applicationServerKey)
          return { endpoint: 'https://push.example/new', toJSON: () => ({ endpoint: 'https://push.example/new', keys: { p256dh: 'k', auth: 'a' } }) }
        },
      },
    },
    clients: {
      matchAll: async () =>
        (opts.tabs ?? []).map((t) => ({
          url: t.url,
          focus: async () => focused.push(t.url),
          postMessage: (message: unknown) => posted.push({ tab: t.url, message }),
        })),
      openWindow: async (url) => opened.push(url),
    },
    fetch: async (url, init) => {
      fetched.push({ url, init })
      const body = url === '/api/notifications' ? { rows: opts.rows ?? [], settings: { push: { key: opts.key ?? 'BAAA' } } } : { ok: true }
      return { ok: true, json: async () => body }
    },
  }
  return { s, shown, posted, focused, opened, fetched, subscribed }
}

const row: Row = { id: 7, title: 'Order filled · QNC', body: 'Bought 5 at 1.75', kind: 'fills' }

describe('the service worker', () => {
  it('shows every push, tagged by its row, with the row for the click', async () => {
    const t = scope()
    await onPush(t.s, { json: () => row })
    expect(t.shown).toEqual([{ title: 'Order filled · QNC', options: { body: 'Bought 5 at 1.75', tag: 'bh-7', data: row, icon: '/favicon.png' } }])
  })

  it("fetches a row that came by its id alone, as one too long for a push does", async () => {
    const t = scope({ rows: [row] })
    await onPush(t.s, { json: () => ({ id: 7 }) })
    expect(t.fetched.map((f) => f.url)).toEqual(['/api/notifications'])
    expect(t.shown[0].title).toBe('Order filled · QNC')
    expect(t.shown[0].options.tag).toBe('bh-7')
  })

  it('never lets a push go unshown, even one it cannot read', async () => {
    for (const data of [null, { json: () => { throw new Error('not JSON') } }, { json: () => ({ id: 9 }) }]) {
      const t = scope()
      await onPush(t.s, data)
      expect(t.shown).toHaveLength(1)
      expect(t.shown[0].title).toBe('Bagholder')
    }
  })

  it('takes a click to an open tab, which opens the row as its history would', async () => {
    const t = scope({ tabs: [{ url: 'https://elsewhere.example/' }, { url: 'http://127.0.0.1:8765/#trades' }] })
    let closed = 0
    await onClick(t.s, { close: () => closed++, data: row })
    expect(closed).toBe(1)
    expect(t.posted).toEqual([{ tab: 'http://127.0.0.1:8765/#trades', message: { type: 'bh-note', row } }])
    expect(t.focused).toEqual(['http://127.0.0.1:8765/#trades'])
    expect(t.opened).toEqual([])
  })

  it('opens a tab on the row where none is open, for every kind', async () => {
    for (const kind of ['fills', 'problems', 'connection', 'updates', 'releases', 'disclosures']) {
      const t = scope()
      await onClick(t.s, { close: () => {}, data: { ...row, kind } })
      expect(t.opened).toEqual(['/?note=7'])
    }
    const t = scope()
    await onClick(t.s, { close: () => {}, data: null })
    expect(t.opened).toEqual(['/'])
  })

  it('subscribes again when the browser renews a subscription, and says so', async () => {
    const t = scope({ key: 'AQID' })
    await onSubscriptionChange(t.s, { endpoint: 'https://push.example/old' })
    expect(t.subscribed).toEqual([keyBytes('AQID')])
    expect(Array.from(keyBytes('AQID'))).toEqual([1, 2, 3])
    // the page's copy reads a key as the worker's does
    for (const k of ['AQID', 'BPz-_w', 'BPz-_w8']) expect(pageKeyBytes(k)).toEqual(keyBytes(k))
    const writes = t.fetched.filter((f) => f.init?.method === 'POST').map((f) => [f.url, JSON.parse(f.init?.body ?? '{}')])
    expect(writes).toEqual([
      ['/api/push/subscribe', { endpoint: 'https://push.example/new', keys: { p256dh: 'k', auth: 'a' } }],
      ['/api/push/unsubscribe', { endpoint: 'https://push.example/old' }],
    ])
  })
})
