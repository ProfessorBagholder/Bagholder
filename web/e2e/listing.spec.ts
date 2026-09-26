import { expect, test } from '@playwright/test'
import { figures, subUrl } from './helpers'

// SPEC §4/§6: every listing on Markets opens its page -- the holding's where the book
// holds it, its own otherwise.

test('a ticker the book does not hold opens its own page under Markets', async ({ page }) => {
  await page.goto('/#markets/' + encodeURIComponent('listing:ZZZQ@TSX'))
  await expect(page.locator('#page')).toContainText('ZZZQ')
  await expect(page.locator('#page')).toContainText('TSX: ZZZQ')
  // a listing has no size, cost or P&L: none of the facts a trade or a holding shows
  await expect(page.locator('#page')).not.toContainText('Executions')
  await expect(page.getByText('Disclosures').first()).toBeVisible()
  // Back leaves it for the tab it sits under
  await page.goto('/#markets')
  await expect(page.getByText('Fear & Greed')).toBeVisible()
})

test('a listing the book turns out to hold opens the holding, and Back does not return to the listing', async ({ page, request }) => {
  const model = await figures(request)
  const held = model.positions.find((p: { kind: string }) => p.kind === 'Shares')
  await page.goto('/#markets')
  await page.goto('/#markets/' + encodeURIComponent('listing:' + held.symbol.toUpperCase() + '@' + String(held.exchange).toUpperCase()))
  await expect(page).toHaveURL(subUrl('portfolio', held.id))
  await expect(page.locator('#page')).toContainText('Executions')
  await page.goBack()
  await expect(page).toHaveURL(/#markets$/)
})
