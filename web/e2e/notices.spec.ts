import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC.md §4, the header (docs/decisions.md 2026-10-09): nothing the header says
// removes itself on a timer. A notice of the person's own action stays until they
// close it, with its × or Esc; the newest shows, and one it covers shows again
// once it is closed, so none is gone unread; each is read and copied whole. The clock is run on far past any timer the page
// once had (four to ten seconds) to show nothing goes by itself.

async function refreshSession(page: import('@playwright/test').Page, answer: { ok: boolean; error?: string }): Promise<void> {
  await page.unroute('**/api/refresh')
  await page.route('**/api/refresh', (route) => route.fulfill({ status: answer.ok ? 200 : 500, json: answer }))
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Refresh session').click()
}

test('a notice stays until the person closes it, however long it is left', async ({ page, request }) => {
  await page.clock.install()
  await openWithStatus(page, request, { connected: true, email: 'someone@example.test' })
  await refreshSession(page, { ok: true })
  const line = page.locator('#syncline')
  await expect(line).toHaveText('Session refreshed')
  await page.clock.runFor(10 * 60 * 1000)
  await expect(line).toHaveText('Session refreshed')
  await page.getByRole('button', { name: 'Close notice' }).click()
  await expect(line).not.toHaveText('Session refreshed')
  await expect(page.getByRole('button', { name: 'Close notice' })).toHaveCount(0)
})

test('the newest notice shows, the one it covers shows again once it is closed, and Esc closes', async ({ page, request }) => {
  await page.clock.install()
  await openWithStatus(page, request, { connected: true, email: 'someone@example.test' })
  await refreshSession(page, { ok: true })
  await refreshSession(page, { ok: false, error: 'Wealthsimple refused the refresh.' })
  const line = page.locator('#syncline')
  await expect(line.locator('.status-err')).toHaveText('Wealthsimple refused the refresh.')
  await page.clock.runFor(10 * 60 * 1000)
  await expect(line.locator('.status-err')).toHaveText('Wealthsimple refused the refresh.')
  // copied whole, confirmed by the button's own state, and still there to read
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.getByRole('button', { name: 'Copy notice' }).click()
  await expect(page.getByRole('button', { name: 'Copy notice' })).toHaveAttribute('data-copied', 'true')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('Wealthsimple refused the refresh.')
  await page.keyboard.press('Escape')
  await expect(line).toHaveText('Session refreshed')
  await page.keyboard.press('Escape')
  await expect(page.getByRole('button', { name: 'Close notice' })).toHaveCount(0)
})
