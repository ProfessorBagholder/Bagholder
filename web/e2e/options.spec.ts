import { expect, test, type Page } from '@playwright/test'
import { figures } from './helpers'

// SPEC.md markets-123 to markets-129, on the made-up book: the Options card, Cboe's
// chain and Yahoo's chart answered by the stand-in `serve.mjs` starts from the server's
// made-up captures (KO's chain, AAPL's the same chain under its symbol with each strike
// six times over, one KO put's chart), Cboe's 403 for every other symbol.

type Held = { id: string; symbol: string; exchange: string; kind: string; last: string | null }

const card = (page: Page) => page.getByTestId('options-card')
const rows = async (page: Page) => (await card(page).locator('.oc-row').evaluateAll((els) => els.map((e) => Array.from(e.querySelectorAll('span')).map((s) => (s.textContent ?? '').trim()))))

test("a US listing's card: Puts first, its expiries, a row a strike, and a row opening its contract", async ({ page }) => {
  await page.goto('/#markets/' + encodeURIComponent('listing:KO@US'))
  await expect(card(page)).toBeVisible()
  // directly under the price chart, before short interest and disclosures
  const order = await page.locator('#page .card').evaluateAll((els) => els.map((e) => (e.getAttribute('data-testid') === 'options-card' ? 'options' : (e.textContent ?? '').includes('Disclosures') ? 'disclosures' : 'other')))
  expect(order.indexOf('options')).toBe(1)
  expect(order.indexOf('options')).toBeLessThan(order.indexOf('disclosures'))
  await expect(card(page).getByRole('heading', { name: 'Options' })).toBeVisible()
  await expect(card(page).getByRole('button', { name: 'Puts' })).toHaveAttribute('aria-pressed', 'true')
  // an expiry in a later year than today's carries its year (formatting-10)
  const jan = new Date().getFullYear() < 2027 ? "Jan 15 '27" : 'Jan 15'
  await expect(card(page).getByRole('group', { name: 'Expiry' }).getByRole('button')).toHaveText(['Nov 13', 'Nov 20', 'Dec 11', jan])
  await expect(card(page).locator('.oc-head span')).toHaveText(['Strike', 'Cost', 'Premium', 'Yield'])
  await card(page).getByRole('button', { name: 'Dec 11' }).click()
  const puts = await rows(page)
  // highest strike first; the adjusted contract (KO1) is not listed
  expect(puts[0][0]).toBe('76')
  expect(puts).toContainEqual(['66', '$6,600', '$24', '0.36%'])
  expect(puts).toContainEqual(['62.50', '$6,250', '$11', '0.18%'])
  // twenty puts listed for Dec 11; the 60, no one bidding on it, is not (markets-125)
  expect(puts.length).toBe(19)
  expect(puts.map((r) => r[0])).not.toContain('60')
  // offline, the listing's price is not read: a call has no Cost and no Yield
  await card(page).getByRole('button', { name: 'Calls' }).click()
  expect(await rows(page)).toContainEqual(['66', '—', '$266', '—'])
  await card(page).getByRole('button', { name: 'Puts' }).click()
  // the list scrolls inside the card at 436 px
  expect(await card(page).locator('.scroll').evaluate((e) => getComputedStyle(e).maxHeight)).toBe('436px')
  // a row opens its contract; Back goes back to the table
  await card(page).locator('.oc-row', { hasText: '$6,600' }).click()
  await expect(card(page).getByRole('heading', { name: 'KO Dec 11 66 put' })).toBeVisible()
  await expect(card(page).locator('.oc-figs > div')).toHaveText(['Cost$6,600', 'Premium$24'])
  const chart = card(page).getByRole('img', { name: 'Premium each day from Sep 29 to today' })
  await expect(chart.locator('text')).toHaveText(['$64', '$44', '$24', 'Sep 29', 'Today'])
  await card(page).getByRole('button', { name: 'Back' }).click()
  await expect(card(page).getByRole('heading', { name: 'Options' })).toBeVisible()
  // a contract Yahoo has no bars for draws no chart
  await card(page).locator('.oc-row', { hasText: '$6,500' }).click()
  await expect(card(page).getByRole('heading', { name: 'KO Dec 11 65 put' })).toBeVisible()
  await expect(card(page).locator('.oc-figs > div')).toHaveText(['Cost$6,500', 'Premium$20'])
  await expect(card(page).getByRole('img')).toHaveCount(0)
})

test("a holding's card opens at the strike nearest its price, and figures a call on that price", async ({ page, request }) => {
  const model = await figures(request)
  const aapl = (model.positions as Held[]).find((p) => p.symbol === 'AAPL' && p.kind === 'Shares')!
  expect(aapl.last).not.toBeNull()
  await page.goto('/#portfolio/' + encodeURIComponent(aapl.id))
  await expect(card(page)).toBeVisible()
  // the stand-in's AAPL strikes run 360 to 456, highest first: the nearest the price is
  // the last row, scrolled into the list's view
  const strikes = (await rows(page)).map((r) => Number(r[0]))
  const nearest = strikes.reduce((a, b) => (Math.abs(b - Number(aapl.last)) < Math.abs(a - Number(aapl.last)) ? b : a))
  expect(strikes.indexOf(nearest)).toBeGreaterThan(9)
  const shown = await card(page).locator('.scroll').evaluate((e, k) => {
    const row = [...e.querySelectorAll('.oc-row')].find((r) => r.querySelector('span')?.textContent === k) as HTMLElement
    const top = row.offsetTop - (e as HTMLElement).offsetTop
    return e.scrollTop > 0 && top >= e.scrollTop && top + row.offsetHeight <= e.scrollTop + e.clientHeight
  }, String(nearest))
  expect(shown).toBe(true)
  await card(page).getByRole('button', { name: 'Calls' }).click()
  const cost = '$' + Math.round(Number(aapl.last) * 100).toLocaleString('en-US')
  const calls = await rows(page)
  expect(calls.length).toBeGreaterThan(0)
  for (const r of calls) expect(r[1]).toBe(cost)
  // the other side, and every other expiry, opens at its strike nearest the price too
  expect(await nearestShown(page, Number(aapl.last)), 'calls').toBe(true)
  for (const day of ['Nov 20', 'Dec 11', 'Nov 13']) {
    await card(page).getByRole('button', { name: day }).click()
    expect(await nearestShown(page, Number(aapl.last)), day).toBe(true)
  }
})

/** Whether the list shows its strike nearest `price`, scrolled into its view. */
async function nearestShown(page: Page, price: number): Promise<boolean> {
  const strikes = (await rows(page)).map((r) => Number(r[0]))
  const nearest = strikes.reduce((a, b) => (Math.abs(b - price) < Math.abs(a - price) ? b : a))
  return card(page).locator('.scroll').evaluate((e, k) => {
    const row = [...e.querySelectorAll('.oc-row')].find((r) => Number(r.querySelector('span')?.textContent) === k) as HTMLElement
    const top = row.offsetTop - (e as HTMLElement).offsetTop
    return top >= e.scrollTop && top + row.offsetHeight <= e.scrollTop + e.clientHeight
  }, nearest)
}

test('no card for a listing with no options, nor for a Canadian one', async ({ page, request }) => {
  const model = await figures(request)
  for (const exchange of ['NYSE', 'TSX']) {
    const td = (model.positions as Held[]).find((p) => p.symbol === 'TD' && p.exchange === exchange && p.kind === 'Shares')!
    await page.goto('/#portfolio/' + encodeURIComponent(td.id))
    await expect(page.locator('#page')).toContainText('Disclosures')
    await expect(card(page)).toHaveCount(0)
  }
})
