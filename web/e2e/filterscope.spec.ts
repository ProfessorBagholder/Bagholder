import { expect, test, type Page } from '@playwright/test'
import { ready, figures, view, stampDay, money0 } from './helpers'

// SPEC §5, Filters: a filter narrows every figure it can describe, each page
// reading it on the value that page shows. The accounts' value, its drawdown and
// its returns follow the date range; a holding answers the journal filters by the
// journal it shares with its trade; a payment answers Kind by its instrument.
// (The Orders panel following the account filter alone is in orderspanel.spec.ts.)

/** Open the funnel on one field. */
async function openField(page: Page, label: string): Promise<void> {
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: new RegExp('^' + label) }).click()
}

const ddText = (dd: { pct: number | null }) => (dd.pct == null ? '—' : '−' + Math.abs(dd.pct * 100).toFixed(1) + '%')

test('a date range narrows the Value curve to its days and moves Max drawdown', async ({ page, request }) => {
  const all = await view(request, 'dashboard')
  const series: { d: string }[] = all.equity.series
  expect(series.length).toBeGreaterThan(2) // the demo book carries a NAV history
  const at: string = all.equity.drawdown.at
  expect(at).toBeTruthy() // and a fall in it
  // a range ending the day before the deepest trough: that fall is outside it
  const before = series.filter((p) => p.d < at)
  const from = before[0].d
  const to = before[before.length - 1].d
  const ranged = await view(request, 'dashboard', { filters: { from, to } })
  expect(ranged.equity.series[0].d >= from && ranged.equity.series.at(-1).d <= to).toBe(true)
  expect(ranged.equity.drawdown.pct).not.toBe(all.equity.drawdown.pct)

  await page.goto('/')
  await ready(page)
  const tile = page.locator('.kpi', { has: page.locator('.lbl', { hasText: 'Max drawdown' }) })
  await expect(tile.locator('.v')).toHaveText(ddText(all.equity.drawdown))

  await openField(page, 'Date')
  await page.getByLabel('From').fill(from)
  await page.getByLabel('To').fill(to)
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(tile.locator('.v')).toHaveText(ddText(ranged.equity.drawdown))

  // the curve's line runs over the range's days only: its last point is the range's
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Equity curve' }) })
  await card.getByRole('button', { name: 'Value' }).click()
  const plot = card.locator('[role="presentation"]')
  const box = (await plot.boundingBox())!
  // daily points are closer than a pixel: the edge reads one of the range's last
  // (or first) few days, never one outside it
  const days: string[] = ranged.equity.series.map((p: { d: string }) => stampDay(p.d))
  await page.mouse.move(box.x + box.width - 1, box.y + box.height / 2)
  await expect(card.locator('.tip .tl')).toHaveText(new RegExp('^(' + days.slice(-3).join('|') + ')$'))
  await page.mouse.move(box.x + 1, box.y + box.height / 2)
  await expect(card.locator('.tip .tl')).toHaveText(new RegExp('^(' + days.slice(0, 3).join('|') + ')$'))
})

test('a tag narrows the Holdings to those whose trade carries it', async ({ page, request }) => {
  const options = (await figures(request)).options
  const all = await view(request, 'positions')
  // a tag that keeps some holdings and not others
  let tag = ''
  let kept: { id: string }[] = []
  for (const t of options.tags as string[]) {
    const p = (await view(request, 'positions', { filters: { lists: { tag: [t] } } })).positions
    if (p.length > 0 && p.length < all.positions.length) {
      tag = t
      kept = p
      break
    }
  }
  expect(tag).not.toBe('') // the demo book holds a tagged position beside untagged ones

  await page.goto('/#portfolio')
  await ready(page)
  const rows = page.locator('.card', { has: page.locator('h5', { hasText: 'Holdings' }) }).locator('tbody tr')
  await expect(rows).toHaveCount(all.positions.length)
  await openField(page, 'Tag')
  await page.locator('.pop-row', { hasText: tag }).first().click()
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(rows).toHaveCount(kept.length)
})

test('a kind narrows the Cashflow payments to those on an instrument of that kind', async ({ page, request }) => {
  const options = (await figures(request)).options
  const all = (await view(request, 'cashflow', { limit: 1_000_000 })).cashflow
  // a kind whose payments are fewer than all of them
  let kind = ''
  let kept: { rows: unknown[]; tiles: { label: string; total: { total: string } }[] } = { rows: [], tiles: [] }
  for (const k of options.kinds as string[]) {
    const c = (await view(request, 'cashflow', { limit: 1_000_000, filters: { lists: { kind: [k] } } })).cashflow
    if (c.rows.length < all.rows.length) {
      kind = k
      kept = c
      break
    }
  }
  expect(kind).not.toBe('') // the demo book trades a kind that pays nothing beside one that pays

  await page.goto('/#cashflow')
  await ready(page)
  const tile = page.locator('.kpi', { has: page.locator('.lbl', { hasText: 'All time' }) })
  const allTime = (c: typeof kept) => money0(c.tiles.find((t) => t.label === 'All time')!.total.total)
  await expect(tile.locator('.v')).toHaveText(allTime(all))
  await openField(page, 'Kind')
  await page.locator('.pop-row', { hasText: kind }).first().click()
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(tile.locator('.v')).toHaveText(allTime(kept))
  expect(allTime(kept)).not.toBe(allTime(all))
})
