import { expect, test } from '@playwright/test'

// SPEC §4, the trade page: its journal and its keys.

const openFirstTrade = async (page: import('@playwright/test').Page) => {
  await page.goto('/#trades')
  await page.locator('#page').getByText(/^\d{4}-\d{2}-\d{2}$/).first().click()
  await expect(page).toHaveURL(/#trades\/.+/)
}

test('a journal save that fails says so in the header, and the row goes back to what the server has', async ({ page }) => {
  await openFirstTrade(page)
  await page.route('**/api/journal', (route) => route.fulfill({ status: 500, json: { ok: false, error: 'store failed' } }))
  await page.locator('.seg-opt', { hasText: /^A$/ }).click()
  await expect(page.locator('#syncline .status-err')).toHaveText('Could not save journal entry.')
  await expect(page.locator('.seg-opt.on', { hasText: /^A$/ })).toHaveCount(0)
})

test('a grade saved reaches the server, shows at once, and survives a reload', async ({ page }) => {
  await openFirstTrade(page)
  await page.locator('.seg-opt', { hasText: /^B$/ }).click()
  await expect(page.locator('.seg-opt.on')).toHaveText('B')
  await page.reload()
  await expect(page.locator('.seg-opt.on')).toHaveText('B')
  await page.locator('.seg-opt', { hasText: /^B$/ }).click() // put it back
  await expect(page.locator('.seg-opt.on')).toHaveCount(0)
})

test('Escape in the tag box stays on the trade; outside it, Escape goes back to the list', async ({ page }) => {
  await openFirstTrade(page)
  await page.getByLabel('Add tag').click()
  await page.keyboard.type('swi')
  await page.keyboard.press('Escape')
  await expect(page).toHaveURL(/#trades\/.+/)
  await page.locator('h5', { hasText: 'Disclosures' }).click()
  await page.keyboard.press('Escape')
  await expect(page).toHaveURL(/#trades$/)
})
