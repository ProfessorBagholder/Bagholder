import { expect, test } from '@playwright/test'
import { ready, view, money0, px, type Waits } from './helpers'

// SPEC §4 Portfolio, "A past range", and §5: under dates that end before today the
// Portfolio is the holdings at the close of their last day. The day is picked from the
// made-up book itself (the day before a holding now held was opened), so the test holds
// whatever the book's dates.

interface Position { id: string; symbol: string; kind: string; opened: string; last: string | Waits; short: boolean }

const dayBefore = (d: string) => {
  const t = new Date(d + 'T12:00:00Z')
  t.setUTCDate(t.getUTCDate() - 1)
  return t.toISOString().slice(0, 10)
}

test('a range ending before today shows the holdings of its last day, margin as —, and a holding then offers no order', async ({ page, request }) => {
  const now = await view(request, 'positions', {})
  expect(now.portfolio.asOf).toBeNull()
  // the share opened last: the day before, it was not held
  const shares = (now.positions as Position[]).filter((p) => p.kind === 'Shares' && !p.short).sort((a, b) => b.opened.localeCompare(a.opened))
  const newest = shares[0]
  const to = dayBefore(newest.opened)
  const filters = { lists: {}, ranges: {}, preset: 'all', years: [], from: '', to, search: '', benchmark: '' }
  const past = await view(request, 'positions', { filters })
  expect(past.portfolio.asOf).toBe(to)
  expect((past.positions as Position[]).map((p) => p.id)).not.toContain(newest.id)
  expect(past.portfolio.marginUsed).toBeNull()
  expect(past.portfolio.availableMargin).toBeNull()
  // the Markets tab's holdings are valued today: those held at some time in the dates (§5)
  const markets = await view(request, 'positions', { filters, now: true })
  expect(markets.portfolio.asOf).toBeNull()
  const today = new Map((now.positions as Position[]).map((p) => [p.id, p.last]))
  for (const p of markets.positions as Position[]) expect(p.last).toEqual(today.get(p.id))

  await page.goto('/#portfolio')
  await ready(page)
  await page.keyboard.press('ControlOrMeta+k')
  await page.locator('.fp-field, button', { hasText: /^Date/ }).first().click()
  await page.getByLabel('To').fill(to)
  await page.getByLabel('To').dispatchEvent('change')
  await page.keyboard.press('Escape')

  const tiles = page.locator('#page .kpi')
  await expect(tiles.filter({ hasText: 'Margin used' }).locator('.v')).toHaveText('—')
  await expect(tiles.filter({ hasText: 'Available margin' }).locator('.v')).toHaveText('—')
  await expect(tiles.filter({ hasText: 'Cost basis' }).locator('.v')).toHaveText(money0(past.portfolio.costBasis.total))
  const rows = page.locator('#page tbody tr')
  await expect(rows).toHaveCount(past.positions.length)
  // Last is that day's close, not today's quote
  const first = (past.positions as Position[]).find((p) => p.kind === 'Shares' && !p.short)!
  const row = rows.filter({ has: page.locator('td:first-child', { hasText: new RegExp('^' + first.symbol + '$') }) }).first()
  await expect(row.locator('td').nth(2)).toHaveText(px(first.last))
  // that day's holding offers no order
  await row.click()
  await expect(page).toHaveURL(/#portfolio\/.+/)
  await expect(page.locator('.tk-rowbtn')).toHaveCount(0)
})
