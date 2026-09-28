import { readFileSync } from 'node:fs'
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
const fromMenu = (item: string) => async (page: Page) => {
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

/** Every message the stream brings, and every placeholder drawn, from the page's first script on. */
async function watchFromTheStart(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const w = window as unknown as { __messages: number; __placeholders: string[]; __changes: string[]; __counting: boolean }
    w.__messages = 0
    w.__placeholders = []
    w.__changes = []
    w.__counting = false
    const Real = window.EventSource
    class Counted extends Real {
      constructor(url: string | URL, init?: EventSourceInit) {
        super(url, init)
        for (const name of ['hello', 'snapshot', 'patch', 'same', 'refused']) this.addEventListener(name, () => w.__messages++)
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

/** Every read answered and the stream quiet: nothing more is on its way. */
async function settled(page: Page): Promise<void> {
  let last = -1
  await expect
    .poll(async () => {
      const n = await page.evaluate(() => (window as unknown as { __messages: number }).__messages)
      const quiet = n === last && inFlight.get(page) === 0
      last = n
      return quiet
    }, { intervals: [300], timeout: 20_000 })
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
  expect(TABS).toEqual(['dashboard', 'trades', 'portfolio', 'markets', 'cashflow'])
  expect(PANELS.length).toBeGreaterThan(0)
  expect(MODALS.length).toBeGreaterThan(0)
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
  const seen: Record<string, string> = {}
  for (const s of list) {
    await s.open(page)
    await settled(page)
    seen[s.name] = await shownText(page)
    await page.keyboard.press('Escape')
    await page.keyboard.press('Escape')
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
