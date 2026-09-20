import { expect, test } from '@playwright/test'

test('the page opens on the book, over one connection, and asks nothing again', async ({ page }) => {
  const asked: string[] = []
  page.on('request', (r) => { if (r.url().includes('/api/')) asked.push(new URL(r.url()).pathname) })
  await page.goto('/')
  await expect(page.getByText('Bagholder').first()).toBeVisible()
  await expect(page.getByRole('link', { name: 'Trades' }).or(page.getByText('Trades', { exact: true })).first()).toBeVisible()
  await page.waitForTimeout(1500)
  expect(asked.filter((p) => p === '/api/events')).toHaveLength(1)
  expect(asked).not.toContain('/api/model')
  expect(asked).not.toContain('/api/status')
  expect(asked).not.toContain('/api/notifications/stream')
})
