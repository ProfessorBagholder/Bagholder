import { expect, test } from '@playwright/test'
import { ready } from './helpers'

// SPEC §3, navigation: addresses, where a page opens and where Back returns to.

test('an old address for the holdings still opens them', async ({ page }) => {
  await page.goto('/#positions')
  await expect(page.locator('.tabbtn.on')).toHaveText('Portfolio')
})

test('a trade opens at its top, and Back returns to the list where it was scrolled to', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 500 })
  await page.goto('/#trades')
  await ready(page)
  await page.evaluate(() => window.scrollTo(0, 300))
  const y = await page.evaluate(() => window.scrollY)
  expect(y).toBeGreaterThan(100)
  await page.locator('#page').getByText(/^\d{4}-\d{2}-\d{2}$/).nth(12).click()
  await expect(page).toHaveURL(/#trades\/.+/)
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0)
  await page.goBack()
  await expect(page).toHaveURL(/#trades$/)
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(y)
})

test('a filter set while a trade is open returns to the list, and Back does not reopen the trade', async ({ page, request }) => {
  const account = (await (await request.get('/api/model')).json()).options.accounts[0] as string
  await page.goto('/#dashboard')
  await page.goto('/#trades')
  await page.locator('#page').getByText(/^\d{4}-\d{2}-\d{2}$/).first().click()
  await expect(page).toHaveURL(/#trades\/.+/)
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type(account)
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/#trades$/)
  await expect(page.locator('.chip')).toContainText(account)
  await page.keyboard.press('Escape')
  // the open trade's entry became the list's: Back is the list before it, never the trade again
  await page.goBack()
  await expect(page).toHaveURL(/#trades$/)
})
