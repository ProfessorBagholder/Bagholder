import { expect, test } from '@playwright/test'
import { ready, openWithStatus, figures, money, money0, signedMoney, pct, pctPlain, qty, px, hold, cmp, waits, subUrl, symText, type Waits } from './helpers'

// SPEC §6, Portfolio: tiles, Allocation/Sectors/Regions donuts, the Holdings
// table and a holding's own page. Figures are checked against the figures document
// (GET /api/figures), formatted from its exact decimal text the way SPEC.md §3
// defines, so the tests stay true if the demo book's numbers ever change.

type Partial = { total: string | Waits; leftOut: number }
interface Position {
  id: string; symbol: string; account: string; kind: string; short: boolean; currency: string
  qty: string | Waits; avg: string | Waits; cost: string | Waits; last: string | Waits; mv: string | Waits; unreal: string | Waits
  unrealPct: number | null; held: number | Waits
}

// a total's sub line; what it left out is not counted on the page
const also = (sub: string, _n: number) => sub

test('the tiles are the six CAD figures the spec gives, with a margin account in scope', async ({ page, request }) => {
  const m = await figures(request)
  const pf = m.portfolio
  expect(pf.hasMargin).toBe(true) // the demo book's Trading account is a margin account
  await page.goto('/#portfolio')
  await ready(page)
  const tiles = page.locator('#page .kpi')
  await expect(tiles).toHaveCount(6)
  const n = pf.positionCount
  await expect(tiles.nth(0).locator('.lbl')).toHaveText('Net asset value')
  await expect(tiles.nth(0).locator('.v')).toHaveText(money0(pf.nav))
  await expect(tiles.nth(0).locator('.s')).toHaveText(`${pf.navAccounts} accounts, ${n} ${n === 1 ? 'position' : 'positions'}`)
  await expect(tiles.nth(1).locator('.lbl')).toHaveText('Cost basis')
  await expect(tiles.nth(1).locator('.v')).toHaveText(money0(pf.costBasis.total))
  await expect(tiles.nth(1).locator('.s')).toHaveText(also('Total book value', pf.costBasis.leftOut))
  await expect(tiles.nth(2).locator('.lbl')).toHaveText('Margin used')
  await expect(tiles.nth(2).locator('.v')).toHaveText(money0(pf.marginUsed))
  await expect(tiles.nth(2).locator('.s')).toHaveText(pctPlain(pf.marginUsedPct) + ' of market value')
  // Available margin: Wealthsimple's buying power over the open margin accounts in scope,
  // `Buying power` under it; `—` only with no margin account in scope (SPEC.md "### Portfolio")
  await expect(tiles.nth(3).locator('.lbl')).toHaveText('Available margin')
  expect(pf.availableMargin, 'a margin account is in scope, so Available margin is a figure').not.toBeNull()
  await expect(tiles.nth(3).locator('.v')).toHaveText(money0(pf.availableMargin))
  await expect(tiles.nth(3).locator('.s')).toHaveText('Buying power')
  const day = pf.dayChange as Partial | null
  await expect(tiles.nth(4).locator('.lbl')).toHaveText('1d change')
  await expect(tiles.nth(4).locator('.v')).toHaveText(day == null ? '—' : signedMoney(day.total, 2))
  await expect(tiles.nth(4).locator('.s')).toHaveText(also(pf.dayChangePct == null ? '—' : pct(pf.dayChangePct) + ' today', day?.leftOut ?? 0))
  await expect(tiles.nth(5).locator('.lbl')).toHaveText('Unrealized P&L')
  await expect(tiles.nth(5).locator('.v')).toHaveText(signedMoney(pf.unrealized.total, 2))
  const u = pf.unrealizedPct as number | null
  await expect(tiles.nth(5).locator('.s')).toHaveText(also(u == null ? '—' : pct(u) + (u >= 0 ? ' gain' : ' loss'), pf.unrealized.leftOut))
})

test('Cash stands in for the margin tiles, in a five-tile row, when no margin account is in scope', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { hasMargin: false, cash: '12345.67', cashPct: 0.05, marginUsed: '0', availableMargin: null })
  })
  await ready(page)
  const tiles = page.locator('#page .kpi')
  await expect(tiles).toHaveCount(5)
  await expect(page.locator('#page')).not.toContainText('Margin used')
  await expect(page.locator('#page')).not.toContainText('Available margin')
  await expect(tiles.nth(2).locator('.lbl')).toHaveText('Cash')
  await expect(tiles.nth(2).locator('.v')).toHaveText('$12,346')
  await expect(tiles.nth(2).locator('.s')).toHaveText(pctPlain(0.05) + ' of net asset value')
})

test('Available margin names the account Wealthsimple could not price', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { availableMarginUnavailable: ['Trading'] })
  })
  await ready(page)
  const tile = page.locator('#page .kpi', { hasText: 'Available margin' })
  await expect(tile.locator('.s')).toHaveText('Unavailable for Trading')
})

test('Available margin reads `Buying power` under a value only: a figure that waits has no subtitle', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { availableMargin: { gaps: ['buying-power-unread'] }, availableMarginUnavailable: [] })
  })
  await ready(page)
  const tile = page.locator('#page .kpi', { hasText: 'Available margin' })
  await expect(tile.locator('.v')).toHaveText('— unread')
  await expect(tile.locator('.s')).not.toContainText('Buying power')
})

test("the day's change is signed and coloured on the tile and on a position's own Change columns", async ({ page, request }) => {
  const m0 = await figures(request)
  const vfv = (m0.positions as Position[]).find((p) => p.symbol === 'VFV')!
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    Object.assign(m.portfolio, { dayChange: { total: '1234.567', leftOut: 0 }, dayChangePct: 0.00842 })
    const p = m.positions.find((x: { id: string }) => x.id === vfv.id)
    // the day's move of the price is a fraction
    Object.assign(p, { dayChange: '150.25', percentChange: 0.0123 })
  })
  await ready(page)
  const tile = page.locator('#page .kpi', { hasText: '1d change' })
  await expect(tile.locator('.v')).toHaveText('+$1,234.57')
  await expect(tile.locator('.s')).toHaveText(pct(0.00842) + ' today')
  await expect(tile.locator('.v')).toHaveClass(/pos/)
  const row = page.locator('#page tbody tr', { hasText: 'VFV' })
  await expect(row.locator('td').nth(5)).toHaveText('+$150.25')
  await expect(row.locator('td').nth(6)).toHaveText('+1.23%')
})

test("Holdings figures are in the position's own currency, unlike Allocation's CAD; a short position is marked SHORT", async ({ page, request }) => {
  const m = await figures(request)
  const aapl = (m.positions as Position[]).find((p) => p.symbol === 'AAPL' && p.kind === 'Shares' && p.account === 'Trading')!
  expect(aapl.currency).toBe('USD')
  const aaplAlloc = (m.portfolio.allocation as { id: string | null; value: string | Waits }[]).find((a) => a.id === aapl.id)!
  await page.goto('/#portfolio')
  await ready(page)
  const row = page.locator('#page tbody tr').filter({ has: page.locator('td:first-child', { hasText: /^AAPL$/ }) })
  await expect(row).toHaveCount(1)
  // the position's own currency (USD): never Allocation's CAD figure for the same holding
  await expect(row.locator('td').nth(1)).toHaveText(px(aapl.avg))
  await expect(row.locator('td').nth(2)).toHaveText(px(aapl.last))
  await expect(row.locator('td').nth(3)).toHaveText(money0(aapl.cost))
  await expect(row.locator('td').nth(4)).toHaveText(money0(aapl.mv))
  await expect(row.locator('td').nth(7)).toHaveText(signedMoney(aapl.unreal) + ' (' + pct(aapl.unrealPct) + ')')
  expect(money0(aapl.mv)).not.toBe(money0(aaplAlloc.value))
  const short = (m.positions as Position[]).filter((p) => p.short)
  const shortRow = page.locator('#page tbody tr', { hasText: 'SHORT' })
  await expect(shortRow).toHaveCount(short.length)
  await expect(shortRow.first()).toContainText(symText(short[0].symbol))
})

test('Holdings sorts by Unrealized P&L first; a header click re-sorts it, and a second click reverses it', async ({ page, request }) => {
  const positions = (await figures(request)).positions as Position[]
  // most unrealized first; a holding whose figure waits sinks, whichever way the rest goes
  const stated = positions.filter((p) => !waits(p.unreal))
  const byUnreal = [...stated].sort((a, b) => cmp(b.unreal as string, a.unreal as string))
  const bySymbolDesc = [...positions].sort((a, b) => b.symbol.localeCompare(a.symbol))
  const bySymbolAsc = [...positions].sort((a, b) => a.symbol.localeCompare(b.symbol))
  await page.goto('/#portfolio')
  await ready(page)
  const rows = page.locator('#page tbody tr')
  const firstCells = () => rows.locator('td:first-child')
  await expect(rows.nth(0).locator('td').first()).toContainText(symText(byUnreal[0].symbol))
  await expect(rows.nth(1).locator('td').first()).toContainText(symText(byUnreal[1].symbol))
  const last = await firstCells().last().textContent()
  expect(positions.filter((p) => waits(p.unreal)).map((p) => symText(p.symbol)).some((s) => last!.startsWith(s))).toBe(true)

  await page.locator('#page th', { hasText: 'Symbol' }).click()
  await expect(rows.nth(0).locator('td').first()).toContainText(symText(bySymbolDesc[0].symbol)) // Z→A on first click of a new column
  await page.locator('#page th', { hasText: 'Symbol' }).click()
  await expect(rows.nth(0).locator('td').first()).toHaveText(symText(bySymbolAsc[0].symbol)) // reversed: A→Z, the bare share before its own option chain
})

test('clicking a holding opens its page, with Qty/Avg/Book/Market/Hold/Account and both Buy and Sell for an open share', async ({ page, request }) => {
  const m = await figures(request)
  const vfv = (m.positions as Position[]).find((p) => p.symbol === 'VFV')!
  await page.goto('/#portfolio')
  await ready(page)
  await page.locator('#page tbody tr', { hasText: 'VFV' }).click()
  await expect(page).toHaveURL(subUrl('portfolio', vfv.id))
  await expect(page.getByRole('button', { name: 'Buy VFV' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Sell VFV' })).toBeVisible()

  const facts = page.locator('div[style*="grid-template-columns:repeat(6,minmax(0,1fr))"] > div')
  await expect(facts).toHaveCount(6)
  const labels = ['Qty', 'Avg', 'Book', 'Market', 'Hold', 'Account']
  const values = [qty(vfv.qty), px(vfv.avg), money0(vfv.cost), money0(vfv.mv), waits(vfv.held) ? '—' : hold(vfv.held), vfv.account]
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

test('Allocation is one slice a holding up to ten, largest first, and hovering a slice reads its own value and share', async ({ page, request }) => {
  const m = await figures(request)
  const alloc = m.portfolio.allocation as { label: string; value: string; share: number; id: string | null }[]
  const valued = (m.positions as Position[]).filter((p) => !waits(p.mv))
  // the book's own: one slice a valued holding up to ten, otherwise the ten largest and `Other (N)`
  if (valued.length <= 10) {
    expect(alloc.length).toBe(valued.length)
    expect(alloc.every((a) => a.id != null)).toBe(true)
  } else {
    expect(alloc.length).toBe(11)
    expect(alloc[10].label).toBe(`Other (${valued.length - 10})`)
  }
  for (let i = 1; i < Math.min(alloc.length, 10); i++) expect(cmp(alloc[i - 1].value, alloc[i].value)).toBeGreaterThanOrEqual(0)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Allocation', exact: true }) })
  await page.goto('/#portfolio')
  await ready(page)
  const first = alloc[0]
  // the ring's stroke sits mostly under the centre overlay's hit area: dispatch the
  // mouseenter the app listens for directly, rather than fighting real-cursor occlusion
  await card.locator('svg path, svg circle').first().dispatchEvent('mouseenter')
  await expect(card.locator('.lbl')).toHaveText(first.label)
  await expect(card.locator('.tab').first()).toHaveText(money0(first.value))
  await expect(card.locator('.muted').first()).toHaveText(pctPlain(first.share))
})

test('Allocation shows the folded Other slice the document sends past ten holdings', async ({ page, request }) => {
  // the made-up book values fewer than eleven holdings, so the fold is stood in for
  const slice = (label: string, value: string, share: number) => ({ label, value, share, id: null })
  const eleven = [...Array.from({ length: 10 }, (_, i) => slice('S' + i, String(1000 - i * 10), 0.09)), slice('Other (3)', '50', 0.1)]
  await openWithStatus(page, request, {}, '#portfolio', (m) => {
    m.portfolio.allocation = eleven
  })
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Allocation', exact: true }) })
  await expect(card.getByText('Other (3)')).toBeVisible()
  await card.locator('svg path, svg circle').nth(10).dispatchEvent('mouseenter')
  await expect(card.locator('.lbl')).toHaveText('Other (3)')
  await expect(card.locator('.tab').first()).toHaveText('$50')
})

test('Sectors and Regions read the span the positions cover, excluding Not classified from the count', async ({ page, request }) => {
  const m = await figures(request)
  // every sector or country with value, `Not classified` not counted; `Other (N)` is N of them
  const span = (rows: { name: string; value: number }[]) =>
    rows.filter((x) => x.name !== 'Not classified' && x.value > 0).reduce((n, x) => n + (Number(/^Other \((\d+)\)$/.exec(x.name)?.[1]) || 1), 0)
  const secCount = span(m.sectors)
  const regCount = span(m.regions)
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
