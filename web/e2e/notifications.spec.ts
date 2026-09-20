import { expect, test, type Page } from '@playwright/test'
import { openWithStatus } from './helpers'

// SPEC §3, Notifications: the server tells, the browser shows -- under its own permission.

/** Stand in for the browser's Notification, in the state given, recording the banners made. */
const browserSays = (page: Page, permission: 'granted' | 'denied' | 'default', answer: 'granted' | 'denied' = 'granted') =>
  page.addInitScript(
    ([permission, answer]) => {
      const made: { title: string; body: string }[] = []
      class Banner {
        static permission = permission
        static requestPermission() { Banner.permission = answer; return Promise.resolve(answer) }
        onclick: (() => void) | null = null
        constructor(title: string, o: { body?: string }) { made.push({ title, body: o?.body ?? '' }); (window as unknown as { __last: Banner }).__last = this }
        close() {}
      }
      Object.assign(window, { Notification: Banner, __banners: made })
    },
    [permission, answer] as const,
  )

const openNotifyRow = async (page: Page) => {
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.locator('.menu button', { hasText: 'Notifications' }).first().click() // a tap opens the row, as hovering does
}

test('a browser that refuses says Blocked and its switches are dimmed', async ({ page }) => {
  await browserSays(page, 'denied')
  await page.goto('/')
  await openNotifyRow(page)
  await expect(page.locator('.menu button', { hasText: 'Notifications' }).first()).toContainText('Blocked')
  await expect(page.locator('.menu.sub button', { hasText: 'Fills' })).toBeDisabled()
  await expect(page.locator('.menu.sub button', { hasText: 'Send a test notification' })).toBeDisabled()
})

test('the first switch asks the browser, then turns the kind on at the server', async ({ page }) => {
  await browserSays(page, 'default', 'granted')
  await page.goto('/')
  await openNotifyRow(page)
  await expect(page.locator('.menu button', { hasText: 'Notifications' }).first()).toContainText('Off')
  const saved = page.waitForRequest((r) => r.url().endsWith('/api/notifications/settings'))
  await page.locator('.menu.sub button', { hasText: 'Updates' }).click()
  expect((await saved).postDataJSON()).toEqual({ updates: true })
  await expect(page.locator('.menu button', { hasText: 'Notifications' }).first()).toContainText('On')
  // put back
  await page.locator('.menu.sub button', { hasText: 'Updates' }).click()
  await expect(page.locator('.menu button', { hasText: 'Notifications' }).first()).toContainText('Off')
})

test('a test notification arrives over the page\'s one stream, is shown by the browser once, and is marked seen', async ({ page }) => {
  await browserSays(page, 'granted')
  await page.goto('/')
  await expect(page.locator('#syncline')).toHaveText('Not connected')
  const seen = page.waitForRequest((r) => r.url().endsWith('/api/notifications/seen'))
  await openNotifyRow(page)
  await page.locator('.menu.sub button', { hasText: 'Send a test notification' }).click()
  expect(((await seen).postDataJSON() as { ids: number[] }).ids).toHaveLength(1)
  const banners = await page.evaluate(() => (window as unknown as { __banners: { title: string }[] }).__banners)
  expect(banners).toHaveLength(1)
  expect(banners[0].title).toContain('Bagholder')
})

test('a fill\'s card opens the Orders panel at Filled', async ({ page, request }) => {
  const rows = [{ id: 7, kind: 'fills', key: 'order:1:filled', title: 'Order filled · QNC', body: 'Bought 5 at 1.75', at: '2026-09-18T14:31:00Z', readAt: null, seenAt: '2026-09-18T14:31:01Z', extra: {} }]
  await openWithStatus(page, request, {}, '', () => {}, { notifications: { rows, unread: 1 } })
  await page.getByRole('button', { name: 'Notifications' }).click()
  await page.getByText('Order filled · QNC').click()
  await expect(page.getByRole('dialog', { name: 'Orders' })).toBeVisible()
  await expect(page.locator('#odWrap .on, [aria-label="Orders"] .on').filter({ hasText: /Filled/ })).toBeVisible()
})

test('the theme row opens on a tap, and choosing a theme closes its list', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.locator('.menu button', { hasText: 'Theme' }).first().click()
  const choices = page.locator('.menu.sub button')
  await expect(choices.first()).toBeVisible()
  await choices.nth(1).click()
  await expect(page.locator('.menu.sub')).toHaveCount(0)
})
