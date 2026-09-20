import { expect, test } from '@playwright/test'
import { ready, openWithStatus } from './helpers'

// SPEC §6, Portfolio: tiles, Allocation/Sectors/Regions donuts, the Holdings
// table and a holding's own page. Figures are checked against /api/model,
// formatted the way fmt.ts formats them (money0/signedMoney/pct/pctPlain),
// so the tests stay true if the demo book's numbers ever change.

function n2(v: number, dp: number): string {
  return Number(v).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}
function money(v: number, dp = 2): string {
  return (v < 0 ? '−' : '') + '$' + n2(Math.abs(v), dp)
}
function money0(v: number): string {
  return money(v, 0)
}
function signedMoney(v: number, dp = 2): string {
  return (v >= 0 ? '+' : '') + money(v, dp)
}
function pct(v: number, dp = 1): string {
  return (v < 0 ? '−' : '+') + Math.abs(v * 100).toFixed(dp) + '%'
}
function pctPlain(v: number, dp = 1): string {
  return (v * 100).toFixed(dp) + '%'
}
function qtyFmt(v: number): string {
  const a = Math.abs(v)
  const dp = a % 1 ? (a < 1 ? 6 : 2) : 0
  return (v < 0 ? '−' : '') + n2(a, dp)
}
function pxFmt(v: number): string {
  const a = Math.abs(v)
  const dp = a === 0 ? 2 : a < 0.01 ? 5 : a < 1 ? (Math.round(a * 1000) % 10 === 0 ? 2 : 3) : 2
  return n2(v, dp)
}
function holdFmt(d: number): string {
  return Math.round(d).toLocaleString('en-US') + 'd'
}

test('the tiles are the six CAD figures the spec gives, with a margin account in scope', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const pf = m.portfolio
  expect(pf.hasMargin).toBe(true) // the demo book's Trading account is a margin account
  await page.goto('/#portfolio')
  await ready(page)
  const tiles = page.locator('#page .kpi')
  await expect(tiles).toHaveCount(6)
  await expect(tiles.nth(0)).toContainText('Net asset value')
  await expect(tiles.nth(0)).toContainText(money0(pf.nav))
  await expect(tiles.nth(0)).toContainText(`${pf.navAccounts} accounts, ${pf.positionCount} positions`)
  await expect(tiles.nth(1)).toContainText('Cost basis')
  await expect(tiles.nth(1)).toContainText(money0(pf.costBasis))
  await expect(tiles.nth(1)).toContainText('Total book value')
  await expect(tiles.nth(2)).toContainText('Margin used')
  await expect(tiles.nth(2)).toContainText(money0(pf.marginUsed))
  await expect(tiles.nth(2)).toContainText(pctPlain(pf.marginUsedPct) + ' of market value')
  await expect(tiles.nth(3)).toContainText('Available margin')
  await expect(tiles.nth(3)).toContainText(money0(pf.availableMargin))
  await expect(tiles.nth(3)).toContainText('Buying power')
  await expect(tiles.nth(4)).toContainText('1d change')
  await expect(tiles.nth(4)).toContainText('—') // no quotes offline: a dash
  await expect(tiles.nth(5)).toContainText('Unrealized P&L')
  await expect(tiles.nth(5)).toContainText(signedMoney(pf.unrealized))
  await expect(tiles.nth(5)).toContainText(pct(pf.unrealizedPct) + ' gain')
})

test('Cash stands in for the margin tiles, in a five-tile row, when no margin account is in scope', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { hasMargin: false, cash: 12345.67, cashPct: 0.05, marginUsed: 0, availableMargin: null })
  })
  await ready(page)
  const tiles = page.locator('#page .kpi')
  await expect(tiles).toHaveCount(5)
  await expect(page.locator('#page')).not.toContainText('Margin used')
  await expect(page.locator('#page')).not.toContainText('Available margin')
  await expect(tiles.nth(2)).toContainText('Cash')
  await expect(tiles.nth(2)).toContainText(money0(12345.67))
  await expect(tiles.nth(2)).toContainText(pctPlain(0.05) + ' of net asset value')
})

test('Available margin names the account Wealthsimple could not price', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { availableMarginUnavailable: ['Trading'] })
  })
  await ready(page)
  const tile = page.locator('#page .kpi', { hasText: 'Available margin' })
  await expect(tile).toContainText('Unavailable for Trading')
})

test("the day's change is signed and coloured on the tile and on a position's own Change columns", async ({ page, request }) => {
  const m0 = await (await request.get('/api/model')).json()
  const vfv = m0.positions.find((p: { symbol: string }) => p.symbol === 'VFV')
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { dayChange: 1234.567, dayChangePct: 0.00842 })
    const p = m.positions.find((x: { id: string }) => x.id === vfv.id)
    Object.assign(p, { dayChange: 150.25, priceChange: 0.62, percentChange: 1.23 })
  })
  await ready(page)
  const tile = page.locator('#page .kpi', { hasText: '1d change' })
  await expect(tile).toContainText(signedMoney(1234.567, 2))
  await expect(tile).toContainText(pct(0.00842) + ' today')
  await expect(tile.locator('.v')).toHaveClass(/pos/)
  const row = page.locator('#page tbody tr', { hasText: 'VFV' })
  await expect(row.locator('td').nth(5)).toHaveText(signedMoney(150.25, 2))
  await expect(row.locator('td').nth(6)).toHaveText(pct(1.23 / 100, 2))
})

test("Holdings figures are in the position's own currency, unlike Allocation's CAD; a short position is marked SHORT", async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const aapl = m.positions.find((p: { symbol: string }) => p.symbol === 'AAPL' && p.account === 'Trading')
  const aaplAlloc = m.portfolio.allocation.find((a: { symbol: string }) => a.symbol === 'AAPL')
  await page.goto('/#portfolio')
  await ready(page)
  const row = page.locator('#page tbody tr', { hasText: 'AAPL' }).first()
  // the position's own currency (USD): never Allocation's CAD figure for the same holding
  await expect(row.locator('td').nth(2)).toHaveText(pxFmt(aapl.last))
  await expect(row.locator('td').nth(3)).toHaveText(money0(aapl.cost))
  await expect(row.locator('td').nth(4)).toHaveText(money0(aapl.mv))
  expect(money0(aapl.mv)).not.toBe(money0(aaplAlloc.value))
  const shortRow = page.locator('#page tbody tr', { hasText: 'SHORT' })
  await expect(shortRow).toHaveCount(1)
  await expect(shortRow).toContainText('AAPL 16OCT26 260.00 CALL')
})

test('Holdings sorts by Unrealized P&L first; a header click re-sorts it, and a second click reverses it', async ({ page }) => {
  await page.goto('/#portfolio')
  await ready(page)
  const rows = page.locator('#page tbody tr')
  // BTC (unreal ~$11,195) then AAPL (~$7,450) lead the default, most-unrealized-first sort
  await expect(rows.nth(0)).toContainText('BTC')
  await expect(rows.nth(1)).toContainText('AAPL')

  await page.locator('#page th', { hasText: 'Symbol' }).click()
  await expect(rows.nth(0).locator('td').first()).toContainText('XEQT') // Z→A on first click of a new column
  await page.locator('#page th', { hasText: 'Symbol' }).click()
  await expect(rows.nth(0).locator('td').first()).toHaveText('AAPL') // reversed: A→Z, the bare share before its own option chain
})

test('clicking a holding opens its page, with Qty/Avg/Book/Market/Hold/Account and both Buy and Sell for an open share', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const vfv = m.positions.find((p: { symbol: string }) => p.symbol === 'VFV')
  await page.goto('/#portfolio')
  await ready(page)
  await page.locator('#page tbody tr', { hasText: 'VFV' }).click()
  await expect(page).toHaveURL('/#portfolio/' + encodeURIComponent(vfv.id))
  await expect(page.getByRole('button', { name: 'Buy VFV' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Sell VFV' })).toBeVisible()

  const facts = page.locator('div[style*="grid-template-columns:repeat(6,minmax(0,1fr))"] > div')
  await expect(facts).toHaveCount(6)
  const labels = ['Qty', 'Avg', 'Book', 'Market', 'Hold', 'Account']
  const values = [qtyFmt(vfv.qty), pxFmt(vfv.avg), money0(vfv.cost), money0(vfv.mv), holdFmt(vfv.held), vfv.account]
  for (let i = 0; i < labels.length; i++) {
    await expect(facts.nth(i).locator('.lbl')).toHaveText(labels[i])
    await expect(facts.nth(i).locator('.tab')).toHaveText(values[i])
  }

  await expect(page.locator('#page')).toContainText(money(vfv.unreal, 2))
})

test("an option contract's holding page carries neither Buy nor Sell", async ({ page }) => {
  await page.goto('/#portfolio')
  await ready(page)
  await page.locator('#page tbody tr', { hasText: 'SHORT' }).click()
  await expect(page).toHaveURL(/#portfolio\/.+/)
  await expect(page.locator('.tk-rowbtns')).toHaveCount(0)
})

test('Allocation folds the smallest positions into one Other slice, and hovering a slice reads its own value and share', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const alloc = m.portfolio.allocation as { symbol: string; value: number; share: number }[]
  expect(alloc.length).toBeGreaterThan(10) // the demo book carries 11: 10 named plus one Other
  const rest = alloc.slice(10)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Allocation', exact: true }) })
  await page.goto('/#portfolio')
  await ready(page)
  await expect(card.getByText(`Other (${rest.length})`)).toBeVisible()
  const first = alloc[0]
  // the ring's stroke sits mostly under the centre overlay's hit area: dispatch the
  // mouseenter the app listens for directly, rather than fighting real-cursor occlusion
  await card.locator('svg path, svg circle').first().dispatchEvent('mouseenter')
  await expect(card.locator('.lbl')).toHaveText(first.symbol)
  await expect(card.locator('.tab').first()).toHaveText(money0(first.value))
  await expect(card.locator('.muted').first()).toHaveText(pctPlain(first.share))
})

test('Sectors and Regions read the span the positions cover, excluding Not classified from the count', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const pf = m.portfolio
  const secCount = (pf.sectors as { name: string; value: number }[]).filter((x) => x.name !== 'Not classified' && x.value > 0).length
  const regCount = (pf.regions as { name: string; value: number }[]).filter((x) => x.name !== 'Not classified' && x.value > 0).length
  await page.goto('/#portfolio')
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Sectors' }) })
  await expect(card.locator('h5', { hasText: 'Sectors' })).toBeVisible()
  await expect(card.locator('h5', { hasText: 'Regions' })).toBeVisible()
  // each donut's own centre overlay (inset:21%): Sectors' donut precedes Regions' in the markup
  const centres = card.locator('div[style*="inset:21%"]')
  await expect(centres).toHaveCount(2)
  await expect(centres.nth(0).locator('.tab')).toHaveText(String(secCount))
  await expect(centres.nth(1).locator('.tab')).toHaveText(String(regCount))
})
