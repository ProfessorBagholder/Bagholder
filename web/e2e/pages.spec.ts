import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// Behaviours of single cards that a rendering of the page alone does not show.

test('value-axis labels never touch, a small loss beside a large gain included', async ({ page, request }) => {
  // one large gain and one small loss: the axis runs in whole round steps, the
  // loss side one step down, so every label has its step's room
  await openWithStatus(page, request, {}, '', (m) => {
    m.monthly = [
      { key: '2026-01', label: "Jan '26", value: '9000', count: 3, tradeIds: [] },
      { key: '2026-02', label: "Feb '26", value: '-40', count: 1, tradeIds: [] },
    ]
  })
  const axis = page.locator('[data-axis]')
  await expect(axis.locator('span')).toHaveCount(7)
  await expect(axis.locator('span[hidden]')).toHaveCount(0)
  await expect(axis.locator('span').last()).toHaveText('−$2,000')
  const overlapping = await axis.evaluate((el) => {
    const rs = Array.from(el.children).filter((c) => !(c as HTMLElement).hidden).map((c) => c.getBoundingClientRect()).sort((a, b) => a.top - b.top)
    return rs.some((r, i) => i > 0 && r.top < rs[i - 1].bottom)
  })
  expect(overlapping).toBe(false)
})

test('Cashflow says nothing of filters when it ignored none', async ({ page }) => {
  await page.goto('/#cashflow')
  await expect(page.locator('#page .kpi').first()).toBeVisible()
  await expect(page.locator('#page')).not.toContainText('apply to distributions')
})
