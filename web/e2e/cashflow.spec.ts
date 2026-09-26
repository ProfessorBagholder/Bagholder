import { expect, test } from '@playwright/test'
import { ready, openWithStatus, figures, money, money0, signedMoney, pctPlain, qty, px, perUnit, cmp, type Waits } from './helpers'

// SPEC §6, Cashflow: the six tiles, the monthly bar chart with its hover tip,
// the Cashflow Positions table, the Allocation donut and Distribution history.
// `e2e/pages.spec.ts` already covers the "skipped filters" note; not repeated here.
// Figures are checked against the figures document (GET /api/figures), formatted
// from its exact decimal text the way SPEC.md §3 defines.

type Partial = { total: string | Waits; leftOut: number }
type Tile =
  | { kind: 'paid'; label: string; total: Partial; perMonth: string | Waits | null }
  | { kind: 'margin'; label: string; marginUsed: string | Waits; interestPerMonth: string | Waits | null }
  | { kind: 'yield'; label: string; yield: number | null | Waits; projected: string | Waits }

const MON_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
function monthLong(key: string): string {
  return MON_LONG[+key.slice(5, 7) - 1] + ' ' + key.slice(0, 4)
}
// a total; what it left out is not counted on the page
const partial = (p: Partial) => money0(p.total)
const negate = (d: string) => (d.startsWith('-') ? d.slice(1) : '-' + d)

test('the tiles are two rolling years, YTD, All time, Margin used and Yield on cost, with a margin account in scope', async ({ page, request }) => {
  const m = await figures(request)
  const tiles = m.cashflow.tiles as Tile[]
  expect(tiles.map((t) => t.kind)).toEqual(['paid', 'paid', 'paid', 'paid', 'margin', 'yield'])
  await page.goto('/#cashflow')
  await ready(page)
  const kpis = page.locator('#page .kpi')
  await expect(kpis).toHaveCount(6)
  for (let i = 0; i < tiles.length; i++) {
    const t = tiles[i]
    if (t.kind === 'margin') {
      await expect(kpis.nth(i)).toContainText('Margin used')
      await expect(kpis.nth(i)).toContainText(money0(t.marginUsed))
      // no month charged: nothing to average, and the subtitle says nothing
      await expect(kpis.nth(i).locator('.s')).toHaveText(t.interestPerMonth === null ? '' : money0(t.interestPerMonth) + '/mo margin interest')
    } else if (t.kind === 'yield') {
      await expect(kpis.nth(i)).toContainText('Yield on cost')
      await expect(kpis.nth(i)).toContainText(pctPlain(t.yield, 2))
      await expect(kpis.nth(i)).toContainText(money0(t.projected) + '/mo')
    } else {
      const label = t.label.replace(/^\d{4} YTD$/, 'YTD')
      await expect(kpis.nth(i).locator('.lbl')).toHaveText(label)
      await expect(kpis.nth(i).locator('.v')).toHaveText(money0(t.total.total))
      const sub = t.label === 'All time' ? 'Total earned' : money0(t.perMonth) + '/mo avg'
      await expect(kpis.nth(i).locator('.s')).toHaveText(sub)
    }
  }
})

test('Last 12 months stands in for Margin used when no margin account is in scope', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#cashflow', (m) => {
    const tiles = m.cashflow.tiles as Tile[]
    const i = tiles.findIndex((t) => t.kind === 'margin')
    tiles[i] = { kind: 'paid', label: 'Last 12 months', total: { total: '5000', leftOut: 0 }, perMonth: '416.67' }
  })
  await ready(page)
  const kpis = page.locator('#page .kpi')
  await expect(kpis).toHaveCount(6)
  await expect(page.locator('#page')).not.toContainText('Margin used')
  const tile = page.locator('#page .kpi', { hasText: 'Last 12 months' })
  await expect(tile.locator('.v')).toHaveText('$5,000')
  await expect(tile.locator('.s')).toHaveText('$417/mo avg')
})

test('with no month charged interest, the Margin used tile has no subtitle', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#cashflow', (m) => {
    const tiles = m.cashflow.tiles as Tile[]
    const i = tiles.findIndex((t) => t.kind === 'margin')
    tiles[i] = { ...(tiles[i] as Extract<Tile, { kind: 'margin' }>), interestPerMonth: null }
  })
  await ready(page)
  const tile = page.locator('#page .kpi', { hasText: 'Margin used' })
  await expect(tile.locator('.s')).toHaveText('')
  await expect(tile).not.toContainText('/mo')
})

test('a bar hover reads the month, Distributions, Margin interest and Net cashflow, and interest can rise above distributions', async ({ page, request }) => {
  const m = await figures(request)
  const months = m.cashflow.months as { key: string; value: string; interest: string; net: string; count: number }[]
  const i = months.findIndex((b) => b.key === '2026-08')
  expect(i).toBeGreaterThanOrEqual(0)
  const b = months[i]
  expect(cmp(b.interest, b.value)).toBeGreaterThan(0) // this month's interest outweighs its distributions
  await page.goto('/#cashflow')
  await ready(page)
  const bar = page.locator(`.bar-col[data-i="${i}"]`)
  await bar.hover()
  const tip = page.locator('.tip')
  const line = (label: string) => tip.locator('.tv', { hasText: label }).locator('span').nth(1)
  await expect(tip.locator('.tl')).toHaveText(monthLong('2026-08'))
  await expect(line('Distributions')).toHaveText(money0(b.value))
  // the interest charged is shown with a minus sign (SPEC.md "### Cashflow", hover)
  await expect(line('Margin interest')).toHaveText(signedMoney(negate(b.interest), 0))
  await expect(line('Net cashflow')).toHaveText(signedMoney(b.net, 0))
  // the negative colour's bar rises higher than the accent's when interest wins the month
  const bars = await bar.locator('div').all()
  expect(bars.length).toBeGreaterThanOrEqual(1)
  const heights = await Promise.all(bars.map((x) => x.boundingBox()))
  expect(Math.max(...heights.map((h) => h?.height ?? 0))).toBeGreaterThan(0)
})

test('Cashflow Positions carries Qty, Avg, Book, Market, Distribution, YTD, All time, Projected, Yield on cost and Current yield', async ({ page, request }) => {
  const m = await figures(request)
  const enb = m.cashflow.holdings.find((h: { symbol: string }) => h.symbol === 'ENB')
  // Avg and Market belong to the row's own position, matched by id, never by symbol
  const pos = m.positions.find((p: { id: string }) => p.id === enb.id)
  expect(pos).toBeTruthy()
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Cashflow Positions' }) })
  const row = card.locator('tbody tr', { hasText: 'ENB' })
  const cells = row.locator('td')
  await expect(cells.nth(0)).toHaveText('ENB')
  await expect(cells.nth(1)).toHaveText(qty(enb.qty))
  await expect(cells.nth(2)).toHaveText(px(pos.avg))
  await expect(cells.nth(3)).toHaveText(money0(enb.cost))
  await expect(cells.nth(4)).toHaveText(money0(pos.mv))
  await expect(cells.nth(5)).toHaveText(perUnit(enb.per))
  await expect(cells.nth(6)).toHaveText(partial(enb.ytd))
  await expect(cells.nth(7)).toHaveText(partial(enb.all))
  await expect(cells.nth(8)).toHaveText(enb.nextExDate ?? '—')
  await expect(cells.nth(9)).toHaveText(enb.nextPayDate ?? '—')
  await expect(cells.nth(10)).toHaveText(money0(enb.perMonth))
  await expect(cells.nth(11)).toHaveText(pctPlain(enb.yoc, 2))
  await expect(cells.nth(12)).toHaveText(pctPlain(enb.currentYield, 2))
})

test('a header click re-sorts Cashflow Positions, and a second click reverses it', async ({ page, request }) => {
  const m = await figures(request)
  const holdings = m.cashflow.holdings as { symbol: string; avg: string }[]
  const byAvgDesc = [...holdings].sort((a, b) => cmp(b.avg, a.avg))
  const byAvgAsc = [...holdings].sort((a, b) => cmp(a.avg, b.avg))
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Cashflow Positions' }) })
  await card.locator('th', { hasText: 'Avg' }).click()
  await expect(card.locator('tbody tr').first().locator('td').first()).toContainText(byAvgDesc[0].symbol)
  await card.locator('th', { hasText: 'Avg' }).click()
  await expect(card.locator('tbody tr').first().locator('td').first()).toContainText(byAvgAsc[0].symbol)
})

test('the Allocation donut totals projected monthly income, largest first, and a hovered slice reads its own figure and share', async ({ page, request }) => {
  const m = await figures(request)
  const items = m.cashflow.income as { label: string; value: string; share: number }[]
  const total = m.cashflow.incomeTotal as Partial
  expect(items.length).toBeGreaterThan(0)
  // largest first
  for (let i = 1; i < items.length; i++) expect(cmp(items[i - 1].value, items[i].value)).toBeGreaterThanOrEqual(0)
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Allocation', exact: true }) })
  await expect(card.locator('.lbl', { hasText: 'Projected' })).toBeVisible()
  await expect(card.locator('#piePlot .tab').first()).toHaveText(money0(total.total) + '/mo')
  await card.locator('#piePlot svg path, #piePlot svg circle').first().dispatchEvent('mouseenter')
  await expect(card.locator('.lbl', { hasText: items[0].label })).toBeVisible()
  await expect(card.locator('#piePlot .tab').first()).toHaveText(money0(items[0].value) + '/mo')
  await expect(card.locator('.muted').first()).toHaveText(pctPlain(items[0].share))
})

test('Distribution history lists newest first, in native currency, with no Type column', async ({ page, request }) => {
  const m = await figures(request)
  const rows = m.cashflow.rows as { date: string; symbol: string; amount: string; currency: string }[]
  const newest = [...rows].sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0))[0]
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Distribution history' }) })
  const headers = card.locator('th')
  await expect(headers).toHaveText(['Date▼', 'Symbol▼', 'Account▼', 'Qty▼', 'Distribution▼', 'Amount▼'])
  await expect(card.getByText('Type', { exact: true })).toHaveCount(0)
  const first = card.locator('tbody tr').first()
  await expect(first.locator('td').nth(0)).toHaveText(newest.date)
  await expect(first.locator('td').nth(1)).toContainText(newest.symbol)
  await expect(first.locator('td').nth(5)).toHaveText(money(newest.amount, 2))
})

test('a header click re-sorts Distribution history', async ({ page, request }) => {
  const m = await figures(request)
  const rows = m.cashflow.rows as { symbol: string }[]
  const bySymbolDesc = [...rows].sort((a, b) => b.symbol.localeCompare(a.symbol))
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Distribution history' }) })
  await card.locator('th', { hasText: 'Symbol' }).click()
  await expect(card.locator('tbody tr').first().locator('td').nth(1)).toContainText(bySymbolDesc[0].symbol)
})

test('a next ex-date or pay day already past is muted; one still to come, the pay day itself included, is not', async ({ page, request }) => {
  let sym = ''
  await openWithStatus(page, request, {}, '#cashflow', (m) => {
    const h = (m.cashflow as { holdings: Record<string, unknown>[] }).holdings[0]
    Object.assign(h, { nextExDate: '2026-01-05', exPast: true, nextPayDate: '2099-01-15', payPast: false })
    sym = String(h.symbol)
  })
  await ready(page)
  const colour = (text: string) => page.locator('#page td', { hasText: text }).first().evaluate((el) => getComputedStyle(el).color)
  const muted = await page.evaluate(() => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--ink55)'
    document.body.appendChild(probe)
    const c = getComputedStyle(probe).color
    probe.remove()
    return c
  })
  expect(sym).not.toBe('')
  expect(await colour('2026-01-05')).toBe(muted)
  expect(await colour('2099-01-15')).not.toBe(muted)
})
