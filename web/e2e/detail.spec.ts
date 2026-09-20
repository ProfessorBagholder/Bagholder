import { expect, test } from '@playwright/test'

// SPEC §4, the trade page: its journal and its keys.

const openFirstTrade = async (page: import('@playwright/test').Page) => {
  await page.goto('/#trades')
  await page.locator('#page').getByText(/^\d{4}-\d{2}-\d{2}$/).first().click()
  await expect(page).toHaveURL(/#trades\/.+/)
}

test('a journal save that fails says so in the header, and the row goes back to what the server has', async ({ page }) => {
  await openFirstTrade(page)
  await page.route('**/api/journal', (route) => route.fulfill({ status: 500, json: { ok: false, error: 'store failed' } }))
  await page.locator('.seg-opt', { hasText: /^A$/ }).click()
  await expect(page.locator('#syncline .status-err')).toHaveText('Could not save journal entry.')
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
  const model = await (await request.get('/api/model')).json()
  const trade = model.trades[0]
  let startedAt = 'A'
  await page.route('**/api/events?*', (route) => {
    const m = { ...model, status: { ...model.status, startedAt } }
    route.fulfill({ status: 200, contentType: 'text/event-stream', body: `retry: 200\nevent: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: m })}\n\n` })
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
  const model = await (await request.get('/api/model')).json()
  const trade = model.trades[0]
  let sent = 0
  await page.route('**/api/events?*', (route) => {
    sent++
    route.fulfill({ status: 200, contentType: 'text/event-stream', body: `retry: 200\nevent: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: model })}\n\n` })
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
