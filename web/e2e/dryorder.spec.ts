import { expect, test } from '@playwright/test'
import { ready } from './helpers'

// One order through the page's own path, nothing stood in: the real stream, the
// real ticket, the real POST /api/order, answered by the server under dry orders
// (it records the order and sends nothing). The same test drives the image in CI
// (tests.yml, container), started on the made-up book (E2E_URL), so the container's
// server, page and order path are proved together.
test('an order placed through the ticket is recorded and not sent under dry orders', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  const answered = page.waitForResponse((r) => r.url().endsWith('/api/order') && r.request().method() === 'POST')
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.getByRole('button', { name: 'Review' }).click()
  await page.getByRole('button', { name: 'Submit' }).click()
  const answer = await (await answered).json()
  expect(answer).toMatchObject({ ok: true, status: 'dry' })
  await expect(page.locator('#syncline')).toContainText('Not sent (orders are off) · Buy 1 NVDA')
})
