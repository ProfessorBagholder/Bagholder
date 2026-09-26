import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC §5, the Orders panel.

// the orders document as the server builds it: one card, amounts as exact decimal text
const resting = {
  ok: true, live: false, refreshedAt: null, error: null, brackets: [],
  orders: [{
    id: 'o-1', account: 'acct-tfsa', exchange: 'TSX-V', symbol: 'QNC', side: 'buy', kind: 'limit', tif: 'until-cancel',
    quantity: '100', limitPrice: '1.75', stopPrice: null, state: 'pending', filled: '0', average: null, why: null,
    value: '175', approx: false, tab: 'pending', at: '2026-09-18T14:31:00Z', live: true, editable: true, legs: [],
  }],
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
  await expect.poll(() => sent).toEqual({ id: 'o-1', quantity: '150', limitPrice: '1.75' })
  await expect(page.locator('#od-qty')).toHaveCount(0)
})
