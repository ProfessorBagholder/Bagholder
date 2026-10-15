import { expect, test, type Page } from '@playwright/test'
import { openWithStatus, ready, saidShown } from './helpers'

// SPEC.md order-ticket-2, -3 and -4; brief 34, section 1: every place a ticket opens
// from opens it on its own listing, named by Wealthsimple's id, never on another
// listing with the same ticker. The made-up book holds TD on the TSX. The US listing
// of TD comes in two ways:
// - a directory row (the bug as found: Buy on it opened the TSX holding's ticket);
// - for the Sell entry points, a second holding stood in beside the book's, on the
//   NYSE, under its own instrument and Wealthsimple id.

const US = { securityId: 'sec-td-nyse', symbol: 'TD', exchange: 'NYSE', currency: 'USD' }

// the book's TD, and a holding of TD on the NYSE beside it in the margin account
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function twin(m: any): { tsx: { id: string; security: string }; us: { id: string } } {
  const tsx = m.positions.find((p: { symbol: string }) => p.symbol === 'TD')
  const margin = m.accounts.find((a: { margin: boolean; tradable: boolean }) => a.margin && a.tradable)
  const us = { ...tsx, id: tsx.id + '-us', instrument: 'td-us', security: US.securityId, exchange: US.exchange, currency: US.currency, accountId: margin.id }
  m.positions.push(us)
  const known = m.options.instruments.find((i: { id: string }) => i.id === tsx.instrument)
  m.options.instruments.push({ ...known, id: 'td-us', security: US.securityId, exchange: US.exchange, currency: US.currency })
  return { tsx, us }
}

// the listing the open ticket's quote is asked for: the page names it in the document it watches
async function quotedListing(page: Page, said: Record<string, unknown>[]): Promise<string> {
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  let key = ''
  await expect.poll(() => {
    key = said.flatMap((d) => Object.keys(d)).filter((k) => k.startsWith('quote:')).pop() ?? ''
    return key
  }).not.toBe('')
  const q = new URLSearchParams(key.slice('quote:'.length))
  expect([...q.keys()].sort(), key).toEqual(['account', 'security'])
  return q.get('security')!
}

async function closeTicket(page: Page) {
  await page.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeHidden()
}

test("each ⌘K row of one ticker's two listings opens the ticket on its own listing", async ({ page, request }) => {
  let ids = { tsx: { id: '', security: '' }, us: { id: '' } }
  const said = saidShown(page)
  await openWithStatus(page, request, {}, '', (m) => { ids = twin(m) })
  await ready(page)
  for (const [exchange, side, want] of [['TSX', 'Buy', ids.tsx.security], ['NYSE', 'Buy', US.securityId], ['TSX', 'Sell', ids.tsx.security], ['NYSE', 'Sell', US.securityId]] as const) {
    said.length = 0
    await page.keyboard.press('Control+k')
    await page.getByRole('textbox', { name: 'Search' }).fill('TD')
    const row = page.locator('.pop-row').filter({ hasText: exchange }).filter({ hasText: 'TD' }).first()
    await row.getByRole('button', { name: `${side} TD`, exact: true }).click()
    expect(await quotedListing(page, said), `${side} on the ${exchange} row`).toBe(want)
    await closeTicket(page)
  }
})

test("each holding's page of one ticker opens the ticket on its own listing, Buy and Sell", async ({ page, request }) => {
  let ids = { tsx: { id: '', security: '' }, us: { id: '' } }
  const said = saidShown(page)
  await openWithStatus(page, request, {}, '', (m) => { ids = twin(m) })
  await ready(page)
  for (const [holding, want] of [[ids.tsx.id, ids.tsx.security], [ids.us.id, US.securityId]] as const) {
    for (const side of ['Buy', 'Sell']) {
      said.length = 0
      await page.evaluate((h) => { location.hash = 'portfolio/' + encodeURIComponent(h) }, holding)
      await page.locator('.tk-rowbtns').getByRole('button', { name: `${side} TD`, exact: true }).click()
      expect(await quotedListing(page, said), `${side} on holding ${holding}`).toBe(want)
      await closeTicket(page)
    }
  }
})

test('a directory row and its page open the ticket on the US listing, asked by its venue, never the TSX holding', async ({ page, request }) => {
  const said = saidShown(page)
  const asked: unknown[] = []
  await page.route('**/api/symbols/search?*', (route) => route.fulfill({ json: { ok: true, matches: [{ symbol: 'TD', name: 'Toronto-Dominion Bank', exchange: 'NYSE', currency: 'USD' }] } }))
  await page.route('**/api/order/listing', (route) => {
    asked.push(route.request().postDataJSON())
    return route.fulfill({ json: { ok: true, listing: US } })
  })
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('TD')
  const row = page.locator('.pop-row').filter({ hasText: 'NYSE' }).first()
  await row.getByRole('button', { name: 'Buy TD', exact: true }).click()
  expect(await quotedListing(page, said)).toBe(US.securityId)
  expect(asked).toEqual([{ of: { kind: 'venue', symbol: 'TD', venue: 'NYSE' } }])
  await closeTicket(page)
  // the listing's own page
  said.length = 0
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('TD')
  await page.locator('.pop-row').filter({ hasText: 'NYSE' }).first().click()
  await page.locator('.tk-rowbtns').getByRole('button', { name: 'Buy TD', exact: true }).click()
  expect(await quotedListing(page, said)).toBe(US.securityId)
  expect(asked[1]).toEqual({ of: { kind: 'venue', symbol: 'TD', venue: 'NYSE' } })
})

test('a listing that names no venue offers no Buy, on its row or its page', async ({ page, request }) => {
  await page.route('**/api/symbols/search?*', (route) => route.fulfill({ json: { ok: true, matches: [{ symbol: 'ZZQ', name: 'Somewhere Inc', exchange: '', currency: 'USD' }] } }))
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('ZZQ')
  const row = page.locator('.pop-row').filter({ hasText: 'Somewhere Inc' }).first()
  await expect(row).toBeVisible()
  await expect(row.getByRole('button', { name: 'Buy ZZQ', exact: true })).toHaveCount(0)
  await row.click()
  await expect(page.locator('.tk-rowbtns').first()).toBeVisible()
  await expect(page.getByRole('button', { name: 'Buy ZZQ', exact: true })).toHaveCount(0)
})

test("a draft closed on one listing is resumed on it, and never fills the other listing's ticket", async ({ page, request }) => {
  let ids = { tsx: { id: '', security: '' }, us: { id: '' } }
  const said = saidShown(page)
  const noOrders = { ok: true, live: false, refreshedAt: null, error: null, orders: [], brackets: [] }
  await openWithStatus(page, request, { openOrders: 0 }, '', (m) => { ids = twin(m) }, { orders: noOrders })
  await ready(page)
  const buyOn = async (exchange: string) => {
    await page.keyboard.press('Control+k')
    await page.getByRole('textbox', { name: 'Search' }).fill('TD')
    await page.locator('.pop-row').filter({ hasText: exchange }).filter({ hasText: 'TD' }).first().getByRole('button', { name: 'Buy TD', exact: true }).click()
    await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  }
  await buyOn('NYSE')
  await page.locator('#tk-qty').fill('7')
  await page.keyboard.press('Escape') // kept as a draft
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeHidden()
  // the draft card's Resume reopens the listing it was closed on, as it was
  said.length = 0
  await page.getByRole('button', { name: 'Orders' }).click()
  await page.locator('.od-card.draft').getByRole('button', { name: 'Resume' }).click()
  expect(await quotedListing(page, said)).toBe(US.securityId)
  await expect(page.locator('#tk-qty')).toHaveValue('7')
  await page.keyboard.press('Escape')
  // the other listing of the ticker starts afresh
  said.length = 0
  await buyOn('TSX')
  expect(await quotedListing(page, said)).toBe(ids.tsx.security)
  await expect(page.locator('#tk-qty')).toHaveValue('1')
})
