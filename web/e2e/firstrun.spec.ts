import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC §3: the page with nothing to show yet, in its three states.
const empty = (model: Record<string, unknown>) => { model.activityCount = 0 }

test('not connected: what connecting does, and the button that does it', async ({ page, request }) => {
  await openWithStatus(page, request, { connected: false }, '', empty)
  await expect(page.locator('#page')).toContainText('No activity yet')
  await expect(page.locator('#page')).toContainText('Nothing leaves this machine.')
  await expect(page.getByRole('button', { name: 'Connect Wealthsimple' })).toBeVisible()
})

test('connected and syncing: the first pull is under way, and there is nothing to press', async ({ page, request }) => {
  await openWithStatus(page, request, { connected: true, syncing: true, syncStep: 'Reading activity' }, '#trades', empty)
  await expect(page.locator('#page')).toContainText('Pulling your history')
  await expect(page.locator('#page').getByRole('button')).toHaveCount(0)
})

test('connected with nothing back: sync again, and a sync error in full here and not in the header', async ({ page, request }) => {
  const error = 'Wealthsimple answered 503 to the activity query three times in a row; the sync stopped and will be tried again at the next window.'
  await openWithStatus(page, request, { connected: true, syncing: false, error }, '', empty)
  await expect(page.getByRole('button', { name: 'Sync now' })).toBeVisible()
  await expect(page.locator('#page .status-err')).toHaveText(error)
  await expect(page.locator('#syncline')).not.toContainText('Wealthsimple answered')
})
