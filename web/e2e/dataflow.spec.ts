import { expect, test, type Page } from '@playwright/test'
import { ready, following } from './helpers'

// docs/architecture.md §13, held in the browser: the page loads what is on screen when
// it is first needed, keeps it between opens, asks for nothing it does not show, and a
// change of screen or of filters touches only what it changes.

/** Every message the page's stream brings, by event and document, and its size: recorded before the page's own code runs. */
async function recordStream(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const w = window as unknown as { __sse: { name: string; doc: string; bytes: number; drawn: boolean }[] }
    w.__sse = []
    const Real = window.EventSource
    class Recorded extends Real {
      constructor(url: string | URL, init?: EventSourceInit) {
        super(url, init)
        for (const name of ['hello', 'snapshot', 'patch', 'same', 'refused']) {
          // registered before the page's own listeners: it sees each message first
          this.addEventListener(name, (e) => {
            const data = (e as MessageEvent).data as string
            let doc = ''
            try {
              doc = (JSON.parse(data) as { doc?: string }).doc ?? ''
            } catch {
              /* a message that is not JSON is recorded without its document */
            }
            // whether the screen was already drawn when this message came
            w.__sse.push({ name, doc, bytes: data.length, drawn: !!document.querySelector('#page .kpi .v') })
          })
        }
      }
    }
    window.EventSource = Recorded as unknown as typeof EventSource
  })
}

const sse = (page: Page) => page.evaluate(() => (window as unknown as { __sse: { name: string; doc: string; bytes: number; drawn: boolean }[] }).__sse)

test('a tab never visited loads nothing: each subscribes to its own screen, and to the holdings only where they are shown', async ({ page }) => {
  const shown = following(page)
  await page.goto('/#dashboard')
  await ready(page)
  await expect.poll(shown).toEqual(['book', 'dashboard', 'notifications', 'status'])
  await page.locator('.tabbtn', { hasText: 'Trades' }).click()
  await expect.poll(shown).toEqual(['book', 'notifications', 'status', 'trades'])
  await page.locator('.tabbtn', { hasText: 'Cashflow' }).click()
  await expect.poll(shown).toEqual(['book', 'cashflow', 'notifications', 'status'])
  await page.locator('.tabbtn', { hasText: 'Portfolio' }).click()
  await expect.poll(shown).toEqual(['book', 'exposure', 'notifications', 'positions', 'status'])
})

/** The documents the page has kept in the browser, by key. */
const keptDocs = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<string[]>((resolve) => {
        const q = indexedDB.open('bagholder')
        q.onerror = () => resolve([])
        q.onsuccess = () => {
          const d = q.result
          if (!d.objectStoreNames.contains('docs')) return resolve([])
          const r = d.transaction('docs', 'readonly').objectStore('docs').getAllKeys()
          r.onsuccess = () => resolve(r.result.map((k) => String(k).split('|')[2]).sort())
          r.onerror = () => resolve([])
        }
      }),
  )

test('opened a second time with nothing changed, the page is drawn from what it kept before any reply, and nothing is sent but acknowledgements', async ({ context }) => {
  const first = await context.newPage()
  await first.goto('/#dashboard')
  await ready(first)
  // kept as each arrives, so closing the page as a person does loses nothing: a write
  // begun as the page unloads is dropped by the browser
  await expect.poll(() => keptDocs(first)).toEqual(['book', 'dashboard', 'notifications', 'status'])
  await first.close()

  const again = await context.newPage()
  await recordStream(again)
  await again.goto('/#dashboard')
  await ready(again)
  await again.waitForTimeout(1000)
  const got = await sse(again)
  const hello = got.find((m) => m.name === 'hello')!
  expect(hello.drawn, 'the Dashboard was on screen before the server said a word').toBe(true)
  const sent = got.filter((m) => m.name === 'snapshot' || m.name === 'patch')
  expect(sent.map((m) => m.doc), 'nothing sent again').toEqual([])
  expect(got.filter((m) => m.name === 'same').map((m) => m.doc).sort()).toEqual(['book', 'dashboard', 'notifications', 'status'])
  // a few hundred bytes of acknowledgements, where the Dashboard alone is tens of kilobytes
  expect(got.reduce((n, m) => n + m.bytes, 0)).toBeLessThan(1000)
})

test('refreshed, the page is drawn from what it kept before the server says a word', async ({ page }) => {
  await recordStream(page)
  await page.goto('/#dashboard')
  await ready(page)
  await expect.poll(() => keptDocs(page)).toEqual(['book', 'dashboard', 'notifications', 'status'])
  await page.reload()
  await ready(page)
  const hello = (await sse(page)).find((m) => m.name === 'hello')!
  expect(hello.drawn, 'the Dashboard was on screen before the server said a word').toBe(true)
})

// What each interaction may ask of the server, at most: the requests made and the
// messages the stream brings. A growth fails here and is argued for, or undone.
const BUDGET = {
  'opening a tab': { requests: 1, messages: 1 },
  'changing a filter': { requests: 1, messages: 6 },
  'a minute idle': { requests: 0, messages: 0 },
}

test('each interaction keeps within its request budget', async ({ page }) => {
  test.setTimeout(120_000) // it sits a minute idle
  await recordStream(page)
  const asked: string[] = []
  page.on('request', (r) => {
    const u = new URL(r.url())
    if (u.pathname.startsWith('/api/')) asked.push(r.method() + ' ' + u.pathname)
  })
  await page.goto('/#dashboard')
  await ready(page)
  await page.waitForTimeout(500)
  const measure = async (what: keyof typeof BUDGET, act: () => Promise<void>) => {
    const [r0, m0] = [asked.length, (await sse(page)).length]
    await act()
    await page.waitForTimeout(1000)
    const requests = asked.slice(r0)
    const messages = (await sse(page)).slice(m0).filter((m) => m.name !== 'same')
    expect(requests.length, `${what}: ${requests.join(', ')}`).toBeLessThanOrEqual(BUDGET[what].requests)
    expect(messages.length, `${what}: ${messages.map((m) => m.name + ' ' + m.doc).join(', ')}`).toBeLessThanOrEqual(BUDGET[what].messages)
  }
  await measure('opening a tab', async () => {
    await page.locator('.tabbtn', { hasText: 'Cashflow' }).click()
    await expect(page.locator('h5', { hasText: 'Distribution history' })).toBeVisible()
  })
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: /^Date/ }).click()
  await measure('changing a filter', async () => {
    await page.locator('.pill', { hasText: '1Y' }).click()
    await expect(page.locator('.chip', { hasText: 'Date' })).toContainText('1Y')
  })
  await page.keyboard.press('Escape')
  await measure('a minute idle', async () => {
    await page.waitForTimeout(60_000)
  })
})

test('switching tabs touches nothing in the header', async ({ page }) => {
  await page.goto('/#dashboard')
  await ready(page)
  await page.evaluate(() => {
    const w = window as unknown as { __hdr: MutationRecord[] }
    w.__hdr = []
    new MutationObserver((r) => w.__hdr.push(...r)).observe(document.querySelector('#hdr')!, { subtree: true, childList: true, characterData: true, attributes: true })
  })
  for (const tab of ['Trades', 'Portfolio', 'Cashflow', 'Dashboard']) {
    await page.locator('.tabbtn', { hasText: tab }).click()
    await page.waitForTimeout(300)
  }
  // the sync line says the minutes as they pass; nothing else in the header moves
  const touched = await page.evaluate(() => (window as unknown as { __hdr: MutationRecord[] }).__hdr.filter((m) => !(m.target as Element).closest?.('#syncline') && !(m.target.parentElement?.closest('#syncline'))).length)
  expect(touched).toBe(0)
})
