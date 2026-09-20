import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC §6, Markets: what Enter and Escape do in its cards.

test('Escape closes the watchlist add row and the tile picker wherever the focus is', async ({ page }) => {
  await page.goto('/#markets')
  await page.getByRole('button', { name: 'Add to the watchlist' }).click()
  await expect(page.getByLabel('Search symbol', { exact: true })).toBeVisible()
  await page.locator('h5', { hasText: 'Fear & Greed' }).click() // the focus leaves the row
  await page.keyboard.press('Escape')
  await expect(page.getByLabel('Search symbol', { exact: true })).toHaveCount(0)
})

test('Escape in the News box clears the words typed; in the Short interest box too; and never the filters while it does', async ({ page, request }) => {
  const account = (await (await request.get('/api/model')).json()).options.accounts[0] as string
  await page.goto('/#markets')
  await expect(page.getByText('Fear & Greed')).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type(account)
  await page.keyboard.press('Enter')
  await page.keyboard.press('Escape')

  const news = page.getByLabel('Search the news')
  await news.fill('dividend')
  await page.keyboard.press('Escape')
  await expect(news).toHaveValue('')
  await expect(news).toBeFocused()
  const shorts = page.getByLabel('Search short interest')
  await shorts.fill('zzz')
  await page.keyboard.press('Escape')
  await expect(shorts).toHaveValue('')
  // the filter set before is still in force: those presses cleared words, not filters
  await page.getByRole('button', { name: 'Trades' }).click()
  await expect(page.locator('.chip').first()).toContainText(account)
})

test('the News card reads Reading… while the server\'s pass still has the market feed to read', async ({ page, request }) => {
  await openWithStatus(page, request, { newsReading: ['*'] }, '#markets', (m) => { (m.markets as { news: unknown[] }).news = [] })
  await expect(page.locator('#page')).toContainText('Reading…')
  await openWithStatus(page, request, { newsReading: [] }, '#markets', (m) => { (m.markets as { news: unknown[] }).news = [] })
  await expect(page.locator('#page')).toContainText('No news.')
})
