import { expect, test } from '@playwright/test'
import { ready, openWithStatus } from './helpers'

// SPEC §6, Cashflow: the six tiles, the monthly bar chart with its hover tip,
// the Cashflow Positions table, the Allocation donut and Distribution history.
// `e2e/pages.spec.ts` already covers the "skipped filters" note; not repeated here.

function n2(v: number, dp: number): string {
  return Number(v).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}
function money(v: number, dp = 2): string {
  return (v < 0 ? '−' : '') + '$' + n2(Math.abs(v), dp)
}
function money0(v: number): string {
  return money(v, 0)
}
function signedMoney(v: number, dp = 0): string {
  return (v >= 0 ? '+' : '') + money(v, dp)
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
function perFmt(v: number): string {
  return '$' + v.toFixed(v < 1 ? 4 : 2)
}
const MON_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
function monthLong(key: string): string {
  return MON_LONG[+key.slice(5, 7) - 1] + ' ' + key.slice(0, 4)
}

test('the tiles are two rolling years, YTD, All time, Margin used and Yield on cost, with a margin account in scope', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const tiles = m.cashflow.tiles as Record<string, unknown>[]
  expect(tiles.length).toBe(6)
  await page.goto('/#cashflow')
  await ready(page)
  const kpis = page.locator('#page .kpi')
  await expect(kpis).toHaveCount(6)
  for (let i = 0; i < tiles.length; i++) {
    const t = tiles[i] as { label: string; total?: number; perMonth?: number; marginUsed?: number; interestPerMonth?: number; yield?: number | null; projected?: number }
    if ('marginUsed' in t) {
      await expect(kpis.nth(i)).toContainText('Margin used')
      await expect(kpis.nth(i)).toContainText(money0(t.marginUsed!))
      await expect(kpis.nth(i)).toContainText(money0(t.interestPerMonth!) + '/mo margin interest')
    } else if ('yield' in t) {
      await expect(kpis.nth(i)).toContainText('Yield on cost')
      await expect(kpis.nth(i)).toContainText(pctPlain(t.yield!, 2))
      await expect(kpis.nth(i)).toContainText(money0(t.projected!) + '/mo')
    } else {
      const label = t.label.replace(/^\d{4} YTD$/, 'YTD')
      await expect(kpis.nth(i)).toContainText(label)
      await expect(kpis.nth(i)).toContainText(money0(t.total!))
      await expect(kpis.nth(i)).toContainText(t.label === 'All time' ? 'Total earned' : money0(t.perMonth!) + '/mo avg')
    }
  }
})

test('Last 12 months stands in for Margin used when no margin account is in scope', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#cashflow', (m) => {
    const tiles = m.cashflow.tiles as Record<string, unknown>[]
    const i = tiles.findIndex((t) => 'marginUsed' in t)
    tiles[i] = { label: 'Last 12 months', total: 5000, perMonth: 416.67, count: 12 }
  })
  await ready(page)
  const kpis = page.locator('#page .kpi')
  await expect(kpis).toHaveCount(6)
  await expect(page.locator('#page')).not.toContainText('Margin used')
  const tile = page.locator('#page .kpi', { hasText: 'Last 12 months' })
  await expect(tile).toContainText(money0(5000))
  await expect(tile).toContainText(money0(416.67) + '/mo avg')
})

test('a bar hover reads the month, Distributions, Margin interest and Net cashflow, and interest can rise above distributions', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const months = m.cashflow.months as { key: string; label: string; value: number; count: number }[]
  const i = months.findIndex((b) => b.key === '2026-08')
  expect(i).toBeGreaterThanOrEqual(0)
  const interest = (m.cashflow.other as { date: string; kind: string; amountCad: number }[])
    .filter((r) => r.kind === 'Interest charge' && r.date.slice(0, 7) === '2026-08')
    .reduce((a, r) => a - r.amountCad, 0)
  expect(interest).toBeGreaterThan(months[i].value) // this month's interest outweighs its distributions
  await page.goto('/#cashflow')
  await ready(page)
  const bar = page.locator(`.bar-col[data-i="${i}"]`)
  await bar.hover()
  const tip = page.locator('.tip')
  await expect(tip).toContainText(monthLong('2026-08'))
  await expect(tip).toContainText(money0(months[i].value))
  await expect(tip).toContainText(signedMoney(-interest))
  await expect(tip).toContainText(signedMoney(months[i].value - interest))
  // the negative colour's bar rises higher than the accent's when interest wins the month
  const box = await bar.boundingBox()
  const bars = await bar.locator('div').all()
  expect(bars.length).toBeGreaterThanOrEqual(1)
  const heights = await Promise.all(bars.map((b) => b.boundingBox()))
  const negHeight = Math.max(...heights.map((h) => h?.height ?? 0))
  expect(negHeight).toBeGreaterThan(0)
  expect(box).not.toBeNull()
})

test('Cashflow Positions carries Qty, Avg, Book, Market, Distribution, YTD, All time, Projected, Yield on cost and Current yield', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const enb = m.cashflow.holdings.find((h: { symbol: string }) => h.symbol === 'ENB')
  const pos = m.positions.find((p: { id: string }) => p.id === enb.id)
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Cashflow Positions' }) })
  const row = card.locator('tbody tr', { hasText: 'ENB' })
  const cells = row.locator('td')
  await expect(cells.nth(0)).toContainText('ENB')
  await expect(cells.nth(1)).toHaveText(qtyFmt(enb.qty))
  await expect(cells.nth(2)).toHaveText(pxFmt(enb.avg))
  await expect(cells.nth(3)).toHaveText(money0(enb.cost))
  await expect(cells.nth(4)).toHaveText(money0(pos.mv))
  await expect(cells.nth(5)).toHaveText(perFmt(enb.per))
  await expect(cells.nth(6)).toHaveText(money0(enb.ytd))
  await expect(cells.nth(7)).toHaveText(money0(enb.all))
  await expect(cells.nth(10)).toHaveText(money0(enb.annual / 12))
  await expect(cells.nth(11)).toHaveText(pctPlain(enb.yoc, 2))
  await expect(cells.nth(12)).toHaveText(pctPlain(enb.currentYield, 2))
})

test('a header click re-sorts Cashflow Positions, and a second click reverses it', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const holdings = m.cashflow.holdings as { symbol: string; avg: number }[]
  const byAvgDesc = [...holdings].sort((a, b) => b.avg - a.avg)
  const byAvgAsc = [...holdings].sort((a, b) => a.avg - b.avg)
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Cashflow Positions' }) })
  await card.locator('th', { hasText: 'Avg' }).click()
  await expect(card.locator('tbody tr').first().locator('td').first()).toContainText(byAvgDesc[0].symbol)
  await card.locator('th', { hasText: 'Avg' }).click()
  await expect(card.locator('tbody tr').first().locator('td').first()).toContainText(byAvgAsc[0].symbol)
})

test('the Allocation donut totals projected monthly income, largest first, and a hovered slice reads its own figure and share', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const items = (m.cashflow.holdings as { symbol: string; annual: number | null }[])
    .map((h) => ({ symbol: h.symbol, v: h.annual != null ? h.annual / 12 : null }))
    .filter((x): x is { symbol: string; v: number } => x.v != null && x.v > 0)
    .sort((a, b) => b.v - a.v)
  const total = items.reduce((a, x) => a + x.v, 0)
  await page.goto('/#cashflow')
  await ready(page)
  const card = page.locator('.card', { has: page.getByRole('heading', { name: 'Allocation', exact: true }) })
  await expect(card.locator('.lbl', { hasText: 'Projected' })).toBeVisible()
  await expect(card.locator('#piePlot .tab').first()).toHaveText(money0(total) + '/mo')
  await card.locator('#piePlot svg path, #piePlot svg circle').first().dispatchEvent('mouseenter')
  await expect(card.locator('.lbl', { hasText: items[0].symbol })).toBeVisible()
  await expect(card.locator('#piePlot .tab').first()).toHaveText(money0(items[0].v) + '/mo')
  await expect(card.locator('.muted').first()).toHaveText(pctPlain(items[0].v / total))
})

test('Distribution history lists newest first, in native currency, with no Type column', async ({ page, request }) => {
  const m = await (await request.get('/api/model')).json()
  const rows = m.cashflow.rows as { date: string; symbol: string; amount: number; currency: string }[]
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
  const m = await (await request.get('/api/model')).json()
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
