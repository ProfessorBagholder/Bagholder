import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { figures, ready } from './helpers'

// SPEC.md order-ticket-2, -3 and -4; brief 34, section 1; brief 36, 7: every place a
// ticket opens from opens it on its own listing, named by Wealthsimple's id, with the
// account and quantity of that listing's holding, and the preview and the order the
// server is sent name that listing, never another with the same ticker. Run against
// the signed-in server (`serve.mjs --signed-in`): the made-up book holds TD on the TSX
// (the TFSA) and on the NYSE (the RRSP), and Wealthsimple's search, stood in, knows
// ENB on the NYSE beside the book's ENB on the TSX. Nothing else is stood in.

interface Held { id: string; security: string; account: string; qty: string; exchange: string }

// each holding of a ticker, by the venue it is listed on
async function holdings(request: APIRequestContext, symbol: string): Promise<Record<string, Held>> {
  const m = await figures(request)
  const brokerOf = (id: string) => (m.accounts as { id: string; brokerAccount: string }[]).find((a) => a.id === id)!.brokerAccount
  const out: Record<string, Held> = {}
  for (const p of m.positions as { id: string; symbol: string; security: string; accountId: string; qty: string; exchange: string }[]) {
    if (p.symbol === symbol) out[p.exchange] = { id: p.id, security: p.security, account: brokerOf(p.accountId), qty: p.qty, exchange: p.exchange }
  }
  return out
}

// what the page sends while the ticket is open: each preview's listing, and the order
async function sent(page: Page) {
  const previews: string[] = []
  let order: Record<string, unknown> | null = null
  page.on('request', (r) => {
    if (r.method() !== 'POST') return
    const path = new URL(r.url()).pathname
    if (path === '/api/order/preview') previews.push((r.postDataJSON() as { securityId: string }).securityId)
    if (path === '/api/order') order = r.postDataJSON() as Record<string, unknown>
  })
  return { previews, order: () => order }
}

// the open ticket, checked and placed as a market order (dry: the server records it, sends nothing)
async function placed(page: Page, want: { security: string; account: string; qty: string }, seen: Awaited<ReturnType<typeof sent>>) {
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('#tk-account')).toHaveValue(want.account)
  await expect(page.locator('#tk-qty')).toHaveValue(want.qty)
  await page.getByRole('combobox').filter({ has: page.getByRole('option', { name: 'Stop limit' }) }).selectOption('MARKET')
  for (const leg of ['Remove stop loss', 'Remove take profit']) {
    const b = page.getByRole('button', { name: leg })
    if (await b.count()) await b.click()
  }
  await page.getByRole('button', { name: 'Review' }).click()
  const answered = page.waitForResponse((r) => new URL(r.url()).pathname === '/api/order' && r.request().method() === 'POST')
  await page.getByRole('button', { name: 'Submit' }).click()
  expect(await (await answered).json()).toMatchObject({ ok: true, status: 'dry' })
  expect(seen.previews.length).toBeGreaterThan(0)
  expect(new Set(seen.previews), 'every preview names the listing').toEqual(new Set([want.security]))
  expect(seen.order()).toMatchObject({ listing: { securityId: want.security }, accountId: want.account, quantity: want.qty })
}

async function searchRow(page: Page, text: string, exchange: string) {
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill(text)
  return page.locator('.pop-row').filter({ hasText: exchange }).filter({ hasText: text }).first()
}

test("each ⌘K row of one ticker's two held listings opens and places on its own listing, account and quantity", async ({ page, request }) => {
  const td = await holdings(request, 'TD')
  expect(Object.keys(td).sort()).toEqual(['NYSE', 'TSX'])
  expect(td.NYSE.qty).not.toBe(td.TSX.qty)
  await page.goto('/')
  await ready(page)
  for (const exchange of ['TSX', 'NYSE']) {
    for (const side of ['Buy', 'Sell'] as const) {
      const seen = await sent(page)
      await (await searchRow(page, 'TD', exchange)).getByRole('button', { name: `${side} TD`, exact: true }).click()
      await placed(page, { security: td[exchange].security, account: td[exchange].account, qty: side === 'Buy' ? '1' : td[exchange].qty }, seen)
      page.removeAllListeners('request')
    }
  }
})

test("each holding's page of one ticker opens and places on its own listing, account and quantity", async ({ page, request }) => {
  const td = await holdings(request, 'TD')
  await page.goto('/')
  await ready(page)
  for (const exchange of ['TSX', 'NYSE']) {
    for (const side of ['Buy', 'Sell'] as const) {
      const seen = await sent(page)
      await page.evaluate((h) => { location.hash = 'portfolio/' + encodeURIComponent(h) }, td[exchange].id)
      await page.locator('.tk-rowbtns').getByRole('button', { name: `${side} TD`, exact: true }).click()
      await placed(page, { security: td[exchange].security, account: td[exchange].account, qty: side === 'Buy' ? '1' : td[exchange].qty }, seen)
      page.removeAllListeners('request')
    }
  }
})

test("a directory row and its page open on the listing Wealthsimple's search finds on that venue, never the book's TSX listing", async ({ page, request }) => {
  const enb = await holdings(request, 'ENB')
  const margin = ((await figures(request)).accounts as { brokerAccount: string; margin: boolean; tradable: boolean; status: string }[]).find((a) => a.margin && a.tradable && a.status !== 'closed')!.brokerAccount
  // the exchanges' directories are out of reach offline: their row is stood in
  await page.route('**/api/symbols/search?*', (route) => route.fulfill({ json: { ok: true, matches: [{ symbol: 'ENB', name: 'Enbridge Inc.', exchange: 'NYSE', currency: 'USD' }] } }))
  const asked: unknown[] = []
  page.on('request', (r) => { if (new URL(r.url()).pathname === '/api/order/listing') asked.push(r.postDataJSON()) })
  await page.goto('/')
  await ready(page)
  let seen = await sent(page)
  await (await searchRow(page, 'ENB', 'NYSE')).getByRole('button', { name: 'Buy ENB', exact: true }).click()
  await placed(page, { security: 'sec-enb-nyse', account: margin, qty: '1' }, seen)
  expect(asked).toEqual([{ of: { kind: 'venue', symbol: 'ENB', venue: 'NYSE' } }])
  expect('sec-enb-nyse').not.toBe(enb.TSX.security)
  // its own page
  page.removeAllListeners('request')
  seen = await sent(page)
  await (await searchRow(page, 'ENB', 'NYSE')).click()
  await page.locator('.tk-rowbtns').getByRole('button', { name: 'Buy ENB', exact: true }).click()
  await placed(page, { security: 'sec-enb-nyse', account: margin, qty: '1' }, seen)
})

test('a listing page whose address names no venue offers no Buy, for a ticker the book has traded', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  // a heatmap tile or a notification opens a listing by its ticker alone
  await page.evaluate(() => { location.hash = 'markets/listing:UBER@' })
  await expect(page.locator('.tk-rowbtns').first()).toBeVisible()
  await expect(page.getByRole('button', { name: 'Buy UBER', exact: true })).toHaveCount(0)
  // and the page stays the listing's: no holding is opened by the ticker
  await expect(page).toHaveURL(/#markets\/listing:UBER@$/)
})

test("a draft closed on one listing is resumed on it, and never fills the other listing's ticket", async ({ page, request }) => {
  const td = await holdings(request, 'TD')
  await page.goto('/')
  await ready(page)
  await (await searchRow(page, 'TD', 'NYSE')).getByRole('button', { name: 'Buy TD', exact: true }).click()
  await expect(page.locator('#tk-account')).toHaveValue(td.NYSE.account)
  await page.locator('#tk-qty').fill('7')
  await page.keyboard.press('Escape') // kept as a draft
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeHidden()
  // the other listing of the ticker starts afresh
  await (await searchRow(page, 'TD', 'TSX')).getByRole('button', { name: 'Buy TD', exact: true }).click()
  await expect(page.locator('#tk-account')).toHaveValue(td.TSX.account)
  await expect(page.locator('#tk-qty')).toHaveValue('1')
  await page.getByRole('button', { name: 'Cancel' }).click()
  // Cancel discarded that draft only after it replaced the first: open and set aside again
  await (await searchRow(page, 'TD', 'NYSE')).getByRole('button', { name: 'Buy TD', exact: true }).click()
  await page.locator('#tk-qty').fill('7')
  await page.keyboard.press('Escape')
  // the draft card's Resume reopens the listing it was closed on, as it was
  const seen = await sent(page)
  await page.getByRole('button', { name: 'Orders' }).click()
  await page.locator('.od-card.draft').getByRole('button', { name: 'Resume' }).click()
  await expect(page.locator('#tk-account')).toHaveValue(td.NYSE.account)
  await expect(page.locator('#tk-qty')).toHaveValue('7')
  await expect.poll(() => seen.previews.at(-1)).toBe(td.NYSE.security)
})
