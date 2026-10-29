import { readdirSync, readFileSync } from 'node:fs'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { ready, view } from './helpers'

// Brief 13, the owner's requirement, over every screen the app can show (docs/decisions.md,
// 2026-09-28): anything the app has already loaded is shown at once, on a reload as on a
// screen's first opening; a value on screen is never replaced by a loading state; an
// answer that has not changed moves nothing.
//
// The screens are the router's tabs and the page's own panels and dialogs, read from
// their sources, so a screen added later is walked without anyone writing a test for it:
// one this walk cannot open fails it until it is taught how.

const src = (p: string) => readFileSync(new URL('../src/' + p, import.meta.url), 'utf8')
const TABS = [...(/export const TABS = \[([^\]]*)\]/.exec(src('lib/router.svelte.ts'))?.[1] ?? '').matchAll(/'([a-z]+)'/g)].map((m) => m[1])
const uiState = /export const ui = \$state<\{([\s\S]*?)\}>\(/.exec(src('lib/ui.svelte.ts'))?.[1] ?? ''
const MODALS = [...(/\n\s*modal: ([^\n]*)/.exec(uiState)?.[1] ?? '').matchAll(/'([a-z]+)'/g)].map((m) => m[1])
const PANELS = [...uiState.matchAll(/\n\s*([a-z]+Open): boolean/g)].map((m) => m[1])
const CONFIRMS = ['clear'] // ui.confirm's dialogs that show data of their own

interface Screen {
  name: string
  open: (page: Page) => Promise<void>
}

const click = (label: string) => async (page: Page) => {
  await page.getByRole('button', { name: label, exact: true }).click()
}
const fromMenu = (item: string | RegExp) => async (page: Page) => {
  await page.getByRole('button', { name: 'Menu', exact: true }).click()
  await page.getByRole('button', { name: item }).click()
}
/** How each of the page's panels and dialogs is opened, as the person opens it. */
const OPENERS: Record<string, (page: Page) => Promise<void>> = {
  menuOpen: click('Menu'),
  ordersOpen: click('Orders'),
  notesOpen: click('Notifications'),
  trade: fromMenu('Add trade'),
  import: fromMenu('Import CSV'),
  folder: fromMenu('Load folder'),
  clear: fromMenu('Clear data'),
  agents: fromMenu(/^Agents/),
  remote: fromMenu(/^Open from anywhere/),
  filter: click('Filters'),
}

async function screens(request: APIRequestContext): Promise<Screen[]> {
  const trades = await view(request, 'trades')
  const positions = await view(request, 'positions')
  const markets = await view(request, 'markets')
  const trade = trades.trades.find((t: { kind: string }) => t.kind === 'Shares') ?? trades.trades[0]
  const holding = positions.positions.find((p: { kind: string }) => p.kind === 'Shares') ?? positions.positions[0]
  const unheld = markets.watchlist.find((w: { symbol: string }) => !positions.positions.some((p: { symbol: string }) => p.symbol === w.symbol))
  const at = (hash: string) => async (page: Page) => {
    await page.evaluate((h) => (location.hash = h), hash)
  }
  const out: Screen[] = TABS.map((t) => ({ name: 'the ' + t + ' tab', open: at('#' + t) }))
  // a detail page is the one asked for: its symbol heads it
  const detail = (hash: string, symbol: string) => async (page: Page) => {
    await at(hash)(page)
    await expect(page.locator('#page')).toContainText(symbol)
  }
  out.push({ name: 'a trade page', open: detail('#trades/' + encodeURIComponent(trade.id), trade.symbol) })
  out.push({ name: 'a holding page', open: detail('#portfolio/' + encodeURIComponent(holding.id), holding.symbol) })
  // a listing the book does not hold: a watched one, else the listing of a trade since closed
  const closed = trades.trades.find((t: { kind: string; symbol: string }) => t.kind === 'Shares' && !positions.positions.some((p: { symbol: string }) => p.symbol === t.symbol))
  const listed = unheld ?? closed
  expect(listed, 'a watched listing to open').toBeTruthy()
  out.push({ name: 'a listing page', open: detail('#markets/' + encodeURIComponent('listing:' + listed.symbol.toUpperCase() + '@' + String(listed.exchange || '').toUpperCase()), listed.symbol) })
  for (const p of [...PANELS, ...MODALS, ...CONFIRMS, 'filter']) {
    const open = OPENERS[p]
    if (!open) throw new Error(`no way to open ${p}: teach the walk how, in OPENERS`)
    out.push({ name: p, open: async (page) => { await at('#dashboard')(page); await open(page) } })
  }
  return out
}

/**
 * Whether an answer says the server is still working on it: anywhere in it, a
 * `pending` that is true, or a `reading` that is true or names anything (#531).
 * Such an answer is not the one the server will give once it is done, so a screen
 * drawn from it is not the screen to compare against. Self-contained: it is also
 * the page's own, injected from this source.
 */
function stillWorking(v: unknown): boolean {
  if (Array.isArray(v)) return v.some(stillWorking)
  if (typeof v !== 'object' || v === null) return false
  return Object.entries(v).some(([k, x]) => ((k === 'pending' || k === 'reading') && (x === true || (Array.isArray(x) && x.length > 0))) || stillWorking(x))
}

/**
 * Every message the stream brings, every placeholder drawn, and each document the
 * page watches as the server has it (whole on a snapshot, then patched), from the
 * page's first script on. The documents watched are what is on screen: a chart
 * whose bars are still being read watches its own (`history:…`), as a disclosure
 * list watches the titles being read; a read made ahead for a screen not shown is
 * waited on by nothing, so it is not counted.
 */
async function watchFromTheStart(page: Page): Promise<void> {
  await page.addInitScript({ content: `window.__stillWorking = ${stillWorking.toString()}` })
  await page.addInitScript(() => {
    const w = window as unknown as {
      __messages: number; __placeholders: string[]; __changes: string[]; __counting: boolean
      __docs: Record<string, unknown>; __working: () => string[]; __stillWorking: (v: unknown) => boolean
    }
    w.__messages = 0
    w.__docs = {}
    w.__working = () => Object.entries(w.__docs).filter(([, v]) => w.__stillWorking(v)).map(([k]) => k)
    // a document the page stops watching (the whole set is said each time) is no longer on screen
    const realFetch = window.fetch
    window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
      if (new URL(url, location.href).pathname === '/api/events/watch' && typeof init?.body === 'string') {
        const watched = Object.keys(JSON.parse(init.body).docs ?? {})
        for (const k of Object.keys(w.__docs)) if (!watched.includes(k)) delete w.__docs[k]
      }
      return realFetch(input, init)
    }
    // a stream document, whole on a snapshot and patched as the page patches it
    type Step = string | { k: string; v: string }
    const at = (root: unknown, path: Step[]): unknown => {
      let cur = root
      for (const s of path) {
        if (cur == null) return undefined
        cur = typeof s === 'string' ? (cur as Record<string, unknown>)[s] : (cur as Record<string, unknown>[]).find((r) => String(r?.[s.k]) === s.v)
      }
      return cur
    }
    const patch = (root: unknown, ops: unknown[][]) => {
      for (const op of ops) {
        const path = op[1] as Step[]
        if (op[0] === 'rows') {
          const list = at(root, path)
          if (!Array.isArray(list)) continue
          const [, , key, order, added] = op as [string, Step[], string, string[], Record<string, unknown>]
          const have = new Map(list.map((r) => [String(r?.[key]), r]))
          list.splice(0, list.length, ...order.map((id) => have.get(id) ?? added[id]).filter((r) => r !== undefined))
          continue
        }
        const last = path[path.length - 1]
        const parent = at(root, path.slice(0, -1))
        if (parent == null) continue
        if (typeof last === 'string') {
          if (op[0] === 'set') (parent as Record<string, unknown>)[last] = op[2]
          else delete (parent as Record<string, unknown>)[last]
        } else if (op[0] === 'set' && Array.isArray(parent)) {
          const i = parent.findIndex((r) => String(r?.[last.k]) === last.v)
          if (i >= 0) parent[i] = op[2]
        }
      }
    }
    w.__placeholders = []
    w.__changes = []
    w.__counting = false
    const Real = window.EventSource
    class Counted extends Real {
      constructor(url: string | URL, init?: EventSourceInit) {
        super(url, init)
        for (const name of ['hello', 'snapshot', 'patch', 'same', 'refused']) this.addEventListener(name, () => w.__messages++)
        this.addEventListener('snapshot', (e) => { const { doc, data } = JSON.parse((e as MessageEvent).data); w.__docs[doc] = data })
        this.addEventListener('patch', (e) => { const { doc, ops } = JSON.parse((e as MessageEvent).data); if (w.__docs[doc] != null) patch(w.__docs[doc], ops) })
      }
    }
    window.EventSource = Counted as unknown as typeof EventSource
    const LOADING = /^(Loading…|Reading…|Reading [^\s]+…)$/
    new MutationObserver((records) => {
      // a relative time turning over with the clock (`read 2 min ago`) is the clock, not an answer
      if (w.__counting)
        for (const r of records) {
          const el = r.target.nodeType === 1 ? (r.target as Element) : r.target.parentElement
          if (/ ago\b|just now/.test(el?.textContent ?? '')) continue
          // what changed, said so a failure names it
          w.__changes.push(`${r.type} ${el?.nodeName ?? ''}.${String(el?.className ?? '').trim().replace(/\s+/g, '.')} ${r.attributeName ?? ''} +[${Array.from(r.addedNodes).map((n) => (n.textContent ?? '').slice(0, 30)).join('|')}] -[${Array.from(r.removedNodes).map((n) => (n.textContent ?? '').slice(0, 30)).join('|')}]`)
        }
      for (const r of records)
        for (const n of Array.from(r.addedNodes)) {
          const el = n.nodeType === 1 ? (n as Element) : n.parentElement
          if (!el) continue
          if (el.matches?.('.bhsk, .bh-skin') || el.querySelector?.('.bhsk, .bh-skin')) w.__placeholders.push('placeholder: ' + el.className)
          const text = (n.textContent ?? '').trim()
          if (LOADING.test(text)) w.__placeholders.push('loading: ' + text)
        }
    }).observe(document, { subtree: true, childList: true, characterData: true, attributes: true })
  })
}

/** What the page shows, a relative time (`3 min ago`, `just now`) read as the clock allows it to differ. */
const shownText = (page: Page) => page.evaluate(() => document.body.innerText.replace(/\d+ (s|min|h|d|mo|y) ago|just now/g, 'AGO'))

/** The page's reads, held until let through: what is drawn meanwhile can only be what was kept. */
async function holdServer(page: Page): Promise<() => void> {
  let open = false
  const waiting: (() => void)[] = []
  await page.route('**/api/**', async (route) => {
    if (!open) await new Promise<void>((r) => waiting.push(r))
    await route.continue()
  })
  return () => {
    open = true
    waiting.splice(0).forEach((r) => r())
  }
}

/**
 * Every read answered and the stream quiet: nothing more is on its way. `done`:
 * and no answer the page holds says the server is still working on it, so what is
 * drawn is what the server will answer again.
 */
async function settled(page: Page, done = false): Promise<void> {
  let last = -1
  await expect
    .poll(async () => {
      const [n, working] = await page.evaluate(() => {
        const w = window as unknown as { __messages: number; __working: () => string[] }
        return [w.__messages, w.__working()] as const
      })
      const quiet = n === last && inFlight.get(page) === 0 && (!done || working.length === 0)
      last = n
      // not settled: what the server is still working on, so a wait that never ends names it
      return quiet || (done && working.length ? 'still working: ' + working.join(', ') : false)
    }, { intervals: [300], timeout: 60_000 })
    .toBe(true)
}
const inFlight = new WeakMap<Page, number>()
function countReads(page: Page): void {
  inFlight.set(page, 0)
  const api = (u: string) => new URL(u).pathname.startsWith('/api/') && !new URL(u).pathname.startsWith('/api/events')
  page.on('request', (r) => { if (api(r.url())) inFlight.set(page, (inFlight.get(page) ?? 0) + 1) })
  const done = (r: { url(): string }) => { if (api(r.url())) inFlight.set(page, (inFlight.get(page) ?? 1) - 1) }
  page.on('requestfinished', done)
  page.on('requestfailed', done)
}

test('the walk finds the router\'s tabs and the page\'s panels and dialogs', () => {
  expect(TABS).toEqual(['dashboard', 'trades', 'portfolio', 'markets', 'cashflow', 'networth'])
  expect(PANELS.length).toBeGreaterThan(0)
  expect(MODALS.length).toBeGreaterThan(0)
})

test('an answer that says the server is still working is told from one it gives when done, for every such field on the wire', () => {
  // every field of the page's generated types whose name says work is under way
  const dir = new URL('../src/lib/generated/', import.meta.url)
  const fields = readdirSync(dir).flatMap((f) => [...readFileSync(new URL(f, dir), 'utf8').matchAll(/\b(\w*(?:pending|reading|loading|busy|inFlight)\w*)\??: (boolean|Array<[^>]+>)/gi)].map((m) => ({ file: f, name: m[1], type: m[2] })))
  expect(fields.length).toBeGreaterThan(0)
  for (const { file, name, type } of fields) {
    const on = type === 'boolean' ? true : ['x']
    const off = type === 'boolean' ? false : []
    expect(stillWorking({ ok: true, nested: [{ [name]: on }] }), `${file}: ${name} under way`).toBe(true)
    expect(stillWorking({ ok: true, nested: [{ [name]: off }] }), `${file}: ${name} done`).toBe(false)
  }
  // a status word is not a field saying work is under way
  expect(stillWorking({ status: 'pending' })).toBe(false)
})

test('every screen, opened again after a reload with the server held back, is drawn at once from what the app had, and the server\'s unchanged answers move nothing', async ({ page, request }) => {
  test.setTimeout(300_000)
  // a page that throws is stuck on what it last drew: every error fails the walk
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await watchFromTheStart(page)
  countReads(page)
  // the first open: everything each screen reads arrives once and is kept
  await page.goto('/#dashboard')
  await ready(page)
  const list = await screens(request)
  // walked until a walk captures what the one before it did: a walk sets off reads the
  // app makes ahead for pages not open, which can change what the server answers for a
  // screen walked before them (a listing's minute bars found missing stops it offering
  // 1H, #531), so a screen is recorded once nothing the walk sets off changes it any
  // more. Each screen is captured once the server has finished what it reads for it.
  const WALKS = 4 // read-ahead settles in one walk; a walk that keeps changing past this is a fault of its own
  let seen: Record<string, string> = {}
  for (let walk = 1; ; walk++) {
    const now: Record<string, string> = {}
    for (const s of list) {
      await s.open(page)
      await settled(page, true)
      now[s.name] = await shownText(page)
      await page.keyboard.press('Escape')
      await page.keyboard.press('Escape')
    }
    const moved = list.map((s) => s.name).filter((n) => walk > 1 && now[n] !== seen[n])
    const fixed = walk > 1 && moved.length === 0
    seen = now
    if (fixed) break
    expect(walk < WALKS, `still changing after ${WALKS} walks: ${moved.join(', ')}`).toBe(true)
  }
  for (const s of list) {
    await test.step(s.name, async () => {
      await page.goto('/#dashboard')
      await ready(page)
      await settled(page)
      await page.evaluate(() => { (window as unknown as { __placeholders: string[] }).__placeholders = [] })
      const release = await holdServer(page)
      await page.reload()
      await s.open(page)
      // drawn from what was kept, the server not yet heard from
      await expect.poll(() => shownText(page), { timeout: 3000 }).toBe(seen[s.name])
      expect(await page.evaluate(() => (window as unknown as { __placeholders: string[] }).__placeholders)).toEqual([])
      // the server's answers, unchanged, change nothing on screen
      await page.evaluate(() => { const w = window as unknown as { __changes: string[]; __counting: boolean }; w.__changes = []; w.__counting = true })
      release()
      await settled(page)
      expect(await page.evaluate(() => (window as unknown as { __changes: string[] }).__changes), 'elements changed when the server answered the same').toEqual([])
      expect(await page.evaluate(() => (window as unknown as { __placeholders: string[] }).__placeholders)).toEqual([])
      await page.unrouteAll({ behavior: 'ignoreErrors' })
    })
  }
  expect(errors).toEqual([])
})
