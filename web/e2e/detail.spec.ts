import { expect, test, type Page } from '@playwright/test'
import { modelDoc, streamBody } from './helpers'

// SPEC §4, the trade page: its journal and its keys.

// the first closed trade in the list (an open one's row opens its holding instead)
const openFirstTrade = async (page: Page) => {
  await page.goto('/#trades')
  const row = page.locator('#page table tbody tr').filter({ hasNot: page.locator('td:nth-child(2)', { hasText: /^Open$/ }) }).first()
  await row.locator('td').first().click()
  await expect(page).toHaveURL(/#trades\/.+/)
}

test('a journal save that fails says so in the header, and the row goes back to what the server has', async ({ page }) => {
  await openFirstTrade(page)
  await page.route('**/api/journal', (route) => route.fulfill({ status: 500, json: { ok: false, error: 'store failed' } }))
  await page.locator('.seg-opt', { hasText: /^A$/ }).click()
  // said with the server's own reason
  await expect(page.locator('#syncline .status-err')).toHaveText('Could not save journal entry: store failed')
  await expect(page.locator('.seg-opt.on', { hasText: /^A$/ })).toHaveCount(0)
})

test('a grade saved reaches the server, shows at once, and survives a reload', async ({ page }) => {
  await openFirstTrade(page)
  await page.locator('.seg-opt', { hasText: /^B$/ }).click()
  await expect(page.locator('.seg-opt.on')).toHaveText('B')
  await page.reload()
  await expect(page.locator('.seg-opt.on')).toHaveText('B')
  await page.locator('.seg-opt', { hasText: /^B$/ }).click() // put it back
  await expect(page.locator('.seg-opt.on')).toHaveCount(0)
})

test('Escape in the tag box stays on the trade; outside it, Escape goes back to the list', async ({ page }) => {
  await openFirstTrade(page)
  await page.getByLabel('Add tag').click()
  await page.keyboard.type('swi')
  await page.keyboard.press('Escape')
  await expect(page).toHaveURL(/#trades\/.+/)
  await page.locator('h5', { hasText: 'Disclosures' }).click()
  await page.keyboard.press('Escape')
  await expect(page).toHaveURL(/#trades$/)
})

test('a server started again is asked for the chart again; the same server is not', async ({ page, request }) => {
  const model = await modelDoc(request)
  const trade = model.trades.find((t: { status: string }) => t.status === 'closed')
  let startedAt = 'A'
  await page.route('**/api/events?*', (route) => {
    const m = { ...model, status: { ...model.status, startedAt } }
    route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody(m) })
  })
  let asked = 0
  page.on('request', (r) => { if (r.url().includes('/api/history?')) asked++ })
  await page.goto('/#trades/' + encodeURIComponent(trade.id))
  await expect.poll(() => asked).toBeGreaterThan(0)
  // the stream reconnects every 200 ms here: the same server, so nothing is asked again
  await page.waitForTimeout(700)
  const settled = asked
  await page.waitForTimeout(700)
  expect(asked).toBe(settled)
  startedAt = 'B'
  await expect.poll(() => asked).toBeGreaterThan(settled)
  // and once: the new server is then the same server
  await page.waitForTimeout(700)
  const after = asked
  await page.waitForTimeout(700)
  expect(asked).toBe(after)
})

test('an open trade keeps its executions when the view arrives again, and a trade opened by its address has them', async ({ page, request }) => {
  const model = await modelDoc(request)
  const trade = model.trades.find((t: { status: string }) => t.status === 'closed')
  let sent = 0
  await page.route('**/api/events?*', (route) => {
    sent++
    route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody(model) })
  })
  await page.goto('/#trades/' + encodeURIComponent(trade.id))
  const rows = page.locator('#page table tbody tr')
  await expect(rows.first()).toBeVisible()
  const n = await rows.count()
  const first = rows.first()
  await first.evaluate((el) => ((el as unknown as { mark: number }).mark = 1))
  const before = sent
  await expect.poll(() => sent).toBeGreaterThan(before + 2)
  // the same rows, the very same elements: nothing was taken away and drawn again
  expect(await rows.count()).toBe(n)
  expect(await first.evaluate((el) => (el as unknown as { mark?: number }).mark)).toBe(1)
})

test('short interest is asked for again when the reader returns to a reading over thirty minutes old, and not before', async ({ page }) => {
  await page.clock.install()
  let asked = 0
  page.on('request', (r) => { if (r.url().includes('/api/shorts?') && !r.url().includes('trend=1')) asked++ })
  await openFirstTrade(page)
  await expect.poll(() => asked).toBe(1)
  const comeBack = () => page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')))
  await page.clock.fastForward(10 * 60_000)
  await comeBack()
  await page.clock.fastForward(1000)
  expect(asked).toBe(1)
  await page.clock.fastForward(25 * 60_000)
  await comeBack()
  await expect.poll(() => asked).toBe(2)
})

test('short interest is neither asked for nor drawn on any page but a share listing', async ({ page, request }) => {
  const model = await modelDoc(request)
  const pages: { hash: string; shares: boolean }[] = [
    ...model.trades.map((t: { id: string; kind: string }) => ({ hash: '#trades/' + encodeURIComponent(t.id), shares: t.kind === 'Shares' })),
    ...model.positions.map((p: { id: string; kind: string }) => ({ hash: '#portfolio/' + encodeURIComponent(p.id), shares: p.kind === 'Shares' })),
  ]
  const others = pages.filter((p) => !p.shares)
  expect(others.length, 'the book has pages that are not a share listing').toBeGreaterThan(0)
  let asked = 0
  let charted = 0
  page.on('request', (r) => {
    if (r.url().includes('/api/shorts?')) asked++
    if (r.url().includes('/api/history?')) charted++
  })
  for (const p of others) {
    const before = charted
    await page.goto('/' + p.hash)
    await expect(page.locator('#page > [data-arrived]')).toBeVisible()
    // the page has asked for its chart, which it asks for beside the card: the card's own ask would be out by now
    await expect.poll(() => charted).toBeGreaterThan(before)
    await page.waitForTimeout(300)
    expect(asked, p.hash).toBe(0)
    await expect(page.locator('#page').getByText('Short volume', { exact: true })).toHaveCount(0)
  }
  // a share listing's page does ask: the watch above would have seen it
  await page.goto('/' + pages.find((p) => p.shares)!.hash)
  await expect.poll(() => asked).toBeGreaterThan(0)
})

test("a chart request that fails says the failure in the chart's place, never that the span has no bars", async ({ page }) => {
  const error = 'history refused ' + Date.now()
  await page.route('**/api/history?*', (route) => route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ ok: false, error }) }))
  await openFirstTrade(page)
  await expect(page.locator('#page').getByText(error, { exact: true })).toBeVisible()
  await expect(page.locator('#page').getByText('No price history for this span.')).toHaveCount(0)
})

test('with no history source for the instrument, its priced executions stand on a time axis; a covered span with no bars says why', async ({ page }) => {
  const answer = (source: string) => JSON.stringify({ ok: true, symbol: 'X', chartSymbol: 'X', source, tf: '1d', available: source ? ['1d'] : [], bars: [], pending: false, reason: source ? 'No bars for this span from TMX Money.' : 'No price source covers this instrument.' })
  let source = ''
  await page.route('**/api/history?*', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: answer(source) }))
  await openFirstTrade(page)
  const executions = page.locator('#page [aria-label^="Executions chart"]')
  await expect(executions).toBeVisible()
  await expect(executions.locator('canvas').first()).toBeVisible()
  await expect(page.locator('#page').getByText('No price source covers this instrument.')).toHaveCount(0)
  // a new page asks again: bars once answered are kept for the page's life
  source = 'tmx'
  await page.goto('about:blank')
  await openFirstTrade(page)
  await expect(page.locator('#page').getByText('No bars for this span from TMX Money.')).toBeVisible()
  await expect(executions).toHaveCount(0)
})
