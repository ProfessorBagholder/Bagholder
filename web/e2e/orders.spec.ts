import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC §5, the Orders panel.

const resting = {
  ok: true, live: false, brackets: [],
  orders: [{ id: 'o-1', createdAt: '2026-09-18T14:31:00Z', account: 'TFSA', symbol: 'QNC', currency: 'CAD', side: 'BUY', type: 'LIMIT', quantity: 100, limitPrice: 1.75, stopPrice: null, tif: 'GTC', status: 'pending', filledQty: 0, avgFill: null, role: '', exchange: 'TSX-V' }],
}

test('the page behind an open panel does not scroll, and does again when it closes', async ({ page }) => {
  await page.goto('/#trades')
  await page.getByRole('button', { name: 'Orders' }).click()
  await expect(page.locator('html')).toHaveClass(/panel-open/)
  expect(await page.evaluate(() => getComputedStyle(document.documentElement).overflow)).toBe('hidden')
  await page.keyboard.press('Escape')
  await expect(page.locator('html')).not.toHaveClass(/panel-open/)
})

test('an order\'s editor takes the keyboard when it opens, and Enter saves it', async ({ page, request }) => {
  await openWithStatus(page, request, { openOrders: 1 }, '', () => {}, { orders: resting })
  let sent: unknown = null
  await page.route('**/api/order/modify', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true } }) })
  await page.getByRole('button', { name: 'Orders' }).click()
  await page.getByRole('button', { name: 'Edit' }).first().click()
  await expect(page.locator('#od-qty')).toBeFocused()
  await page.locator('#od-qty').fill('150')
  await page.keyboard.press('Enter')
  await expect.poll(() => sent).toEqual({ id: 'o-1', quantity: 150, limitPrice: 1.75 })
  await expect(page.locator('#od-qty')).toHaveCount(0)
})
