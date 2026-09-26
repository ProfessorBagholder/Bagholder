import { expect, test, type Page } from '@playwright/test'
import { modelDoc, openWithStatus, streamBody } from './helpers'

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

test("a disclosure's or a release's banner opens the instrument's page: the holding's where the book holds it, the listing's where it does not", async ({ page, request }) => {
  const model = await modelDoc(request)
  // a holding whose symbol no other holding shares, so the page it opens is its own
  type P = { id: string; symbol: string; exchange: string; kind: string }
  const positions = model.positions as P[]
  const held = positions.find((p) => p.kind === 'Shares' && positions.filter((q) => q.symbol === p.symbol).length === 1)!
  const cases = [
    { kind: 'disclosures', symbol: 'ZZQX', exchange: 'TSX', lands: (u: string) => u.endsWith('#markets/listing:ZZQX@TSX') },
    { kind: 'releases', symbol: held.symbol, exchange: held.exchange, lands: (u: string) => u.endsWith('#portfolio/' + held.id) },
  ]
  await browserSays(page, 'granted')
  let row: unknown = null
  await page.route('**/api/events?*', (route) =>
    route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody(model, { notifications: { rows: row ? [row] : [], unread: row ? 1 : 0 } }) }),
  )
  await page.route('**/api/notifications/seen', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":true}' }))
  await page.goto('/#dashboard')
  await expect(page.locator('#page > [data-arrived]')).toBeVisible()
  let id = 900
  for (const c of cases) {
    const before = await page.evaluate(() => (window as unknown as { __banners: unknown[] }).__banners.length)
    row = { id: ++id, kind: c.kind, key: c.kind + ':' + id, title: c.symbol + ' filed', body: '', at: '2026-09-18T14:31:00Z', readAt: null, seenAt: null, extra: { symbol: c.symbol, exchange: c.exchange } }
    // the stream reconnects every 200 ms here, and the row arrives with it
    await expect.poll(() => page.evaluate(() => (window as unknown as { __banners: unknown[] }).__banners.length)).toBe(before + 1)
    await page.evaluate(() => (window as unknown as { __last: { onclick: () => void } }).__last.onclick())
    await expect.poll(() => c.lands(decodeURIComponent(page.url())), c.kind).toBe(true)
    await page.goto('/#dashboard')
  }
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
