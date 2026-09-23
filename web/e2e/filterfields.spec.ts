import { expect, test, type Page } from '@playwright/test'
import { ready } from './helpers'

// SPEC §5, Filters: every field re-asks the server (GET /api/events?filters=…),
// narrowing every page; the chips beside the tabs; Clear all; Esc with nothing
// open; and the ranked order the search box lists its matches in.

async function requestFilters(req: Awaited<ReturnType<Page['waitForRequest']>>): Promise<Record<string, unknown>> {
  const u = new URL(req.url())
  return JSON.parse(decodeURIComponent(u.searchParams.get('filters')!))
}

function eventsRequest(page: Page) {
  return page.waitForRequest((r) => r.url().includes('/api/events?filters='))
}

/** Open the funnel, pick one field from the fields list, pick one value from it, and close with Done. */
async function selectListValue(page: Page, fieldLabel: string, value: string): Promise<Record<string, unknown>> {
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: new RegExp('^' + fieldLabel) }).click()
  const reqPromise = eventsRequest(page)
  await page.locator('.pop-row', { hasText: value }).first().click()
  const filters = await requestFilters(await reqPromise)
  await page.getByRole('button', { name: 'Done' }).click()
  return filters
}

test('each date preset narrows the book to that window and sets the chip', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: /^Date/ }).click()
  for (const [code, label] of [['1d', '1D'], ['1w', '1W'], ['1m', '1M'], ['3m', '3M'], ['6m', '6M'], ['ytd', 'YTD'], ['1y', '1Y'], ['5y', '5Y']] as const) {
    const reqPromise = eventsRequest(page)
    await page.locator('.pill', { hasText: label }).click()
    const filters = await requestFilters(await reqPromise)
    expect(filters.preset).toBe(code)
    await expect(page.locator('.chip', { hasText: 'Date' })).toContainText(label)
  }
})

test('a custom date range sets the chip and clears the preset, and picking a year does the same', async ({ page, request }) => {
  const options = (await (await request.get('/api/model')).json()).options
  await page.goto('/#trades')
  await ready(page)
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: /^Date/ }).click()
  // the preset's own request is awaited, or on a slow run it lands after the wait
  // below has begun and is read as the date's
  let reqPromise = eventsRequest(page)
  await page.locator('.pill', { hasText: '1M' }).click()
  expect((await requestFilters(await reqPromise)).preset).toBe('1m')

  reqPromise = eventsRequest(page)
  await page.getByLabel('From').fill('2026-01-01')
  let filters = await requestFilters(await reqPromise)
  expect(filters.from).toBe('2026-01-01')
  expect(filters.preset).toBe('all')

  reqPromise = eventsRequest(page)
  await page.getByLabel('To').fill('2026-03-01')
  filters = await requestFilters(await reqPromise)
  expect(filters.to).toBe('2026-03-01')
  await expect(page.locator('.chip', { hasText: 'Date' })).toContainText('2026-01-01 → 2026-03-01')

  if (options.years?.length) {
    const year = options.years[0] as string
    reqPromise = eventsRequest(page)
    await page.locator('.pill', { hasText: year }).click()
    filters = await requestFilters(await reqPromise)
    expect((filters.years as string[])[0]).toBe(year)
    expect(filters.from).toBe('')
    expect(filters.to).toBe('')
    await expect(page.locator('.chip', { hasText: 'Date' })).toContainText(year)
  }
})

test('Grade, Side and Result each narrow the book and show it as a chip', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  for (const [key, label, value] of [['grade', 'Grade', 'A'], ['side', 'Side', 'SELL'], ['result', 'Result', 'Winners']] as const) {
    const filters = await selectListValue(page, label, value)
    expect((filters.lists as Record<string, string[]>)[key]).toEqual([value])
    await expect(page.locator('.chip', { hasText: label + ' is' })).toContainText(value)
  }
})

test('Account, Tag, Kind and Exchange each narrow the book and show it as a chip', async ({ page, request }) => {
  const options = (await (await request.get('/api/model')).json()).options
  await page.goto('/#trades')
  await ready(page)
  const cases: [string, string, string][] = []
  if (options.accounts?.[0]) cases.push(['account', 'Account', options.accounts[0]])
  if (options.tags?.[0]) cases.push(['tag', 'Tag', options.tags[0]])
  if (options.kinds?.[0]) cases.push(['kind', 'Kind', options.kinds[0]])
  if (options.exchanges?.[0]) cases.push(['exchange', 'Exchange', options.exchanges[0]])
  expect(cases.length).toBeGreaterThan(0) // the demo book carries at least one of each
  for (const [key, label, value] of cases) {
    const filters = await selectListValue(page, label, value)
    expect((filters.lists as Record<string, string[]>)[key]).toEqual([value])
    await expect(page.locator('.chip', { hasText: label + ' is' })).toContainText(value)
  }
})

test('Symbol, from its own field editor, narrows the book by the funnel icon', async ({ page, request }) => {
  const model = await (await request.get('/api/model')).json()
  const held = model.positions.find((p: { kind: string }) => p.kind === 'Shares')
  await page.goto('/#trades')
  await ready(page)
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: /^Symbol/ }).click()
  await page.getByLabel('Search values').fill(held.symbol)
  const reqPromise = eventsRequest(page)
  await page.locator('.tk-rowbtn.funnel').first().click()
  const filters = await requestFilters(await reqPromise)
  expect((filters.lists as Record<string, string[]>).symbol).toContain(held.symbol)
  await expect(page.locator('.chip', { hasText: 'Symbol is' })).toBeVisible()
})

test('the Price range narrows by a step and by More/Less than', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: /^Price/ }).click()

  let reqPromise = eventsRequest(page)
  await page.locator('.pill', { hasText: '$100' }).click()
  let filters = await requestFilters(await reqPromise)
  expect(filters.ranges).toMatchObject({ price: { op: '>', v: 100 } })
  await expect(page.locator('.chip', { hasText: 'Price >' })).toContainText('$100')

  reqPromise = eventsRequest(page)
  await page.locator('.pill', { hasText: 'Less than' }).click()
  filters = await requestFilters(await reqPromise)
  expect(filters.ranges).toMatchObject({ price: { op: '<', v: 100 } })
  await expect(page.locator('.chip', { hasText: 'Price <' })).toContainText('$100')

  reqPromise = eventsRequest(page)
  await page.getByLabel('Custom value').fill('12.34')
  filters = await requestFilters(await reqPromise)
  expect(filters.ranges).toMatchObject({ price: { op: '<', v: 12.34 } })
  await expect(page.locator('.chip', { hasText: 'Price <' })).toContainText('$12.34')

  // Clear, from the field's own editor
  reqPromise = eventsRequest(page)
  await page.getByRole('button', { name: 'Clear', exact: true }).click()
  filters = await requestFilters(await reqPromise)
  expect(filters.ranges).toMatchObject({ price: { v: null } })
  await expect(page.locator('.chip', { hasText: 'Price' })).toHaveCount(0)
})

for (const [label, custom, formatted] of [['Hold', '45', '45 days'], ['P&L', '250', '$250'], ['Qty', '1500', '1,500']] as const) {
  test(`the ${label} range narrows by a custom value and sets the chip`, async ({ page }) => {
    await page.goto('/#trades')
    await ready(page)
    await page.getByRole('button', { name: 'Filters' }).click()
    await page.locator('.pop-row', { hasText: new RegExp('^' + label) }).click()
    const reqPromise = eventsRequest(page)
    await page.getByLabel('Custom value').fill(custom)
    const filters = await requestFilters(await reqPromise)
    expect(filters.ranges).toBeTruthy()
    await expect(page.locator('.chip', { hasText: label + ' >' })).toContainText(formatted)
  })
}

test("a chip's value reopens the popover on that field, and its × removes the filter", async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  await selectListValue(page, 'Grade', 'A')
  await expect(page.locator('.chip', { hasText: 'Grade is' })).toContainText('A')

  await page.locator('.chip', { hasText: 'Grade is' }).locator('.cv').click()
  await expect(page.getByRole('button', { name: 'Back' })).toBeVisible()
  await expect(page.locator('.pop .lbl', { hasText: 'Grade' })).toBeVisible()
  await page.keyboard.press('Escape') // this Esc only closes the popover (SPEC §5)
  await expect(page.locator('.pop')).toHaveCount(0)
  await expect(page.locator('.chip', { hasText: 'Grade is' })).toContainText('A')

  const reqPromise = eventsRequest(page)
  await page.locator('.chip', { hasText: 'Grade is' }).locator('.cx').click()
  const filters = await requestFilters(await reqPromise)
  expect((filters.lists as Record<string, string[]>).grade).toEqual([])
  await expect(page.locator('.chip', { hasText: 'Grade is' })).toHaveCount(0)
})

test('Clear all clears every filter and closes the popover', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  await selectListValue(page, 'Grade', 'A')
  await page.getByRole('button', { name: 'Filters' }).click()
  const reqPromise = eventsRequest(page)
  await page.getByRole('button', { name: 'Clear all' }).click()
  const filters = await requestFilters(await reqPromise)
  expect((filters.lists as Record<string, string[]>).grade).toEqual([])
  await expect(page.locator('.chip')).toHaveCount(0)
  await expect(page.locator('.pop')).toHaveCount(0)
})

test('Escape with nothing open and nothing being typed clears every filter, the same as Clear all', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  await selectListValue(page, 'Grade', 'A') // closes with Done, so nothing is open afterwards
  await expect(page.locator('.chip')).not.toHaveCount(0)
  const reqPromise = eventsRequest(page)
  await page.keyboard.press('Escape')
  await requestFilters(await reqPromise)
  await expect(page.locator('.chip')).toHaveCount(0)
})

test('filters do not survive a reload', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  await selectListValue(page, 'Grade', 'A')
  await expect(page.locator('.chip')).not.toHaveCount(0)
  await page.reload()
  await ready(page)
  await expect(page.locator('.chip')).toHaveCount(0)
})

test('typing lists the book\'s own symbols first, then matches on other fields, then listings found outside the book', async ({ page, request }) => {
  await page.route('**/api/symbols/search*', (route) => route.fulfill({ json: { ok: true, matches: [{ symbol: 'ZZZQF', name: 'Zzz Quantum Fund', exchange: 'OTC' }] } }))

  const model = await (await request.get('/api/model')).json()
  model.options.symbols = [...model.options.symbols, 'ZZZQ']
  model.options.listings = { ...model.options.listings, ZZZQ: { name: 'Zzz Quantum Corp', exchange: 'NASDAQ', kind: 'Shares', currency: 'USD' } }
  model.options.accounts = [...model.options.accounts, 'ZZZQ Trust']
  const body = `retry: 200\nevent: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: model })}\n\n`
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body }))

  await page.goto('/')
  await ready(page)
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search', { exact: true }).fill('ZZZQ')
  await expect(page.locator('.pop .scroll .pop-row', { hasText: 'ZZZQF' })).toBeVisible() // wait for the external round trip

  const rows = await page.locator('.pop .scroll .pop-row').allTextContents()
  const iBook = rows.findIndex((t) => t.includes('Zzz Quantum Corp'))
  const iAccount = rows.findIndex((t) => t.includes('ZZZQ Trust'))
  const iExternal = rows.findIndex((t) => t.includes('ZZZQF'))
  expect(iBook).toBeGreaterThanOrEqual(0)
  expect(iAccount).toBeGreaterThan(iBook)
  expect(iExternal).toBeGreaterThan(iAccount)
})
