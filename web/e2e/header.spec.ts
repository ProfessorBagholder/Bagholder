import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC §3, the header: what the status line says, in the order it says it, and the
// update on offer beside the version.

test('a release that can be installed is a button, and pressing it asks for the update', async ({ page, request }) => {
  await openWithStatus(page, request, { updateAvailable: true, canUpdate: true, latestVersion: 'v9.9.9' })
  let asked = 0
  await page.route('**/api/update', (route) => { asked++; return route.fulfill({ json: { ok: true } }) })
  await page.getByRole('button', { name: 'Update to v9.9.9' }).click()
  expect(asked).toBe(1)
})

test('a release this copy cannot install itself is a link to it', async ({ page, request }) => {
  await openWithStatus(page, request, { updateAvailable: true, canUpdate: false, updateBy: 'app', updateUrl: 'https://example.test/release' })
  await expect(page.getByRole('link', { name: 'Update available' })).toHaveAttribute('href', 'https://example.test/release')
  await openWithStatus(page, request, { updateAvailable: true, canUpdate: false, updateBy: 'image', latestVersion: 'v9.9.9' })
  await expect(page.getByRole('link', { name: 'v9.9.9 image available' })).toBeVisible()
})

test('the status line says what the update is doing, then that it failed', async ({ page, request }) => {
  await openWithStatus(page, request, { updating: 'Downloading…', syncing: true, syncStep: 'Reading activity' })
  await expect(page.locator('#syncline')).toHaveText('Downloading…')
  await openWithStatus(page, request, { updating: '', updateError: 'The update could not be verified.' })
  await expect(page.locator('#syncline .status-err')).toHaveText('The update could not be verified.')
})

test('a server of another protocol is told apart: restart to finish the update', async ({ page, request }) => {
  await openWithStatus(page, request, { protocol: '1999-01-01.1' })
  await expect(page.locator('#syncline')).toHaveText('Restart Bagholder to finish the update')
})

test('the status line on the book as it is: not connected, and nothing on offer', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('#syncline')).toHaveText('Not connected')
  await expect(page.getByText(/Update (to|available)/)).toHaveCount(0)
})

test('refreshing the session says so while it runs', async ({ page, request }) => {
  await openWithStatus(page, request, { connected: true, email: 'someone@example.test' })
  let release: () => void = () => {}
  await page.route('**/api/refresh', async (route) => { await new Promise<void>((r) => (release = r)); await route.fulfill({ json: { ok: true } }) })
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Refresh session').click()
  await expect(page.locator('#syncline')).toHaveText('Refreshing session…')
  release()
  await expect(page.locator('#syncline')).toHaveText('Session refreshed')
})
