import { createECDH, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'
import { expect, test, type Page } from '@playwright/test'
import { modelDoc, openWithStatus, streamBody, standIn } from './helpers'

// SPEC §3, Notifications: the server tells, the browser shows, through the page's
// service worker and under the browser's own permission.

/**
 * Stand in for the browser's permission, in the state given, and for its push
 * service: a subscription made here names `endpoint`. Every banner the service
 * worker is asked to show is recorded, with its options.
 */
const browserSays = (page: Page, permission: 'granted' | 'denied' | 'default', answer: 'granted' | 'denied' = 'granted', endpoint = 'http://127.0.0.1:9/push/e2e') => {
  // a browser's own key pair and secret, as its push manager makes them
  const ecdh = createECDH('prime256v1')
  const keys = { p256dh: ecdh.generateKeys().toString('base64url'), auth: randomBytes(16).toString('base64url') }
  return page.addInitScript(
    ([permission, answer, endpoint, keys]) => {
      const made: { title: string; body: string; tag: string; data: unknown }[] = []
      class Banner {
        static permission = permission
        static requestPermission() { Banner.permission = answer; return Promise.resolve(answer) }
      }
      Object.assign(window, { Notification: Banner, __banners: made })
      ServiceWorkerRegistration.prototype.showNotification = function (title: string, o?: NotificationOptions) {
        made.push({ title, body: o?.body ?? '', tag: o?.tag ?? '', data: o?.data })
        return Promise.resolve()
      }
      const sub = { endpoint, toJSON: () => ({ endpoint, keys }) }
      let held: typeof sub | null = null
      PushManager.prototype.subscribe = function () { held = sub; return Promise.resolve(sub as unknown as PushSubscription) }
      PushManager.prototype.getSubscription = function () { return Promise.resolve(held as unknown as PushSubscription) }
    },
    [permission, answer, endpoint, keys] as const,
  )
}

/** A click on the banner for `row`, as the service worker passes it to an open tab. */
const clickBanner = (page: Page, row: unknown) =>
  page.evaluate((row) => navigator.serviceWorker.dispatchEvent(new MessageEvent('message', { data: { type: 'bh-note', row } })), row)

const openNotifyRow = async (page: Page) => {
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.locator('.menu button', { hasText: 'Notifications' }).first().click() // a tap opens the row, as hovering does
}

const W = { 'X-Bagholder': '1' }

// a subscription a test made is the shared server's until dropped: none outlives its test
test.afterEach(async ({ request }) => {
  await request.post('/api/push/unsubscribe', { headers: W, data: { endpoint: 'http://127.0.0.1:9/push/e2e' } })
})

test('Turn on notifications subscribes this browser; with the page open the page shows a row and nothing goes to the push service; with it closed the row is pushed', async ({ browser, request }) => {
  // the stream's end is noticed at its next write, the keep-alive at the latest (15 s)
  test.setTimeout(60_000)
  // the push service, on this machine: what reaches it is kept
  const got: { path: string; headers: Record<string, unknown>; bytes: number }[] = []
  const service = createServer((req, res) => {
    let bytes = 0
    req.on('data', (c: Buffer) => (bytes += c.length))
    req.on('end', () => {
      got.push({ path: req.url ?? '', headers: req.headers, bytes })
      res.writeHead(201).end()
    })
  })
  await new Promise<void>((r) => service.listen(0, '127.0.0.1', r))
  const endpoint = `http://127.0.0.1:${(service.address() as { port: number }).port}/push/one`
  const page = await (await browser.newContext()).newPage()
  try {
    await browserSays(page, 'default', 'granted', endpoint)
    await page.goto('/')
    await openNotifyRow(page)
    await expect(page.locator('.menu.sub button', { hasText: 'Turn on notifications' })).toBeEnabled()
    const subscribed = page.waitForResponse((r) => r.url().endsWith('/api/push/subscribe'))
    await page.locator('.menu.sub button', { hasText: 'Turn on notifications' }).click()
    expect(await (await subscribed).json()).toMatchObject({ ok: true, subscriptions: 1 })
    await expect(page.locator('.menu.sub button', { hasText: 'Turn on notifications' })).toHaveCount(0)
    // open: the page shows it, once, and the push service hears nothing
    await page.locator('.menu.sub button', { hasText: 'Send a test notification' }).click()
    await expect.poll(() => page.evaluate(() => (window as unknown as { __banners: unknown[] }).__banners.length)).toBe(1)
    expect(got).toEqual([])
    // closed: the server pushes it
    await page.context().close()
    await expect.poll(async () => {
      await request.post('/api/notifications/test', { headers: W })
      return got.length
    }, { timeout: 40_000, intervals: [2000] }).toBeGreaterThan(0)
    expect(got[0].path).toBe('/push/one')
    expect(got[0].headers['content-encoding']).toBe('aes128gcm')
    expect(String(got[0].headers.authorization)).toMatch(/^vapid t=[\w-]+\.[\w-]+\.[\w-]+, k=[\w-]{87}$/)
    expect(got[0].bytes).toBeLessThanOrEqual(4096)
  } finally {
    await request.post('/api/push/unsubscribe', { headers: W, data: { endpoint } })
    service.close()
  }
})

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
  // dimmed until the server's key is on the page, as a person sees them
  await expect(page.locator('.menu.sub button', { hasText: 'Updates' })).toBeEnabled()
  const subscribed = page.waitForRequest((r) => r.url().endsWith('/api/push/subscribe'))
  const saved = page.waitForRequest((r) => r.url().endsWith('/api/notifications/settings'))
  await page.locator('.menu.sub button', { hasText: 'Updates' }).click()
  expect(((await subscribed).postDataJSON() as { endpoint: string }).endpoint).toBe('http://127.0.0.1:9/push/e2e')
  expect((await saved).postDataJSON()).toEqual({ updates: true })
  await expect(page.locator('.menu button', { hasText: 'Notifications' }).first()).toContainText('On')
  // put back
  await page.locator('.menu.sub button', { hasText: 'Updates' }).click()
  await expect(page.locator('.menu button', { hasText: 'Notifications' }).first()).toContainText('Off')
})

test('a test notification arrives over the page\'s one stream, is shown by the service worker once under its row\'s tag, and is marked seen', async ({ page }) => {
  await browserSays(page, 'granted')
  await page.goto('/')
  await expect(page.locator('#syncline')).toHaveText('Not connected')
  const seen = page.waitForRequest((r) => r.url().endsWith('/api/notifications/seen'))
  await openNotifyRow(page)
  await page.locator('.menu.sub button', { hasText: 'Send a test notification' }).click()
  const ids = ((await seen).postDataJSON() as { ids: number[] }).ids
  await expect.poll(() => page.evaluate(() => (window as unknown as { __banners: unknown[] }).__banners.length)).toBe(1)
  const banners = await page.evaluate(() => (window as unknown as { __banners: { title: string; tag: string }[] }).__banners)
  expect(banners[0].title).toContain('Bagholder')
  expect(banners[0].tag).toBe('bh-' + ids[0])
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
    { kind: 'disclosures', symbol: 'ZZQX', exchange: 'TSX', lands: (u: string) => u.endsWith('#markets/listing:ZZQX@XTSE') },
    { kind: 'releases', symbol: held.symbol, exchange: held.exchange, lands: (u: string) => u.endsWith('#portfolio/' + held.id) },
  ]
  await browserSays(page, 'granted')
  let row: unknown = null
  await standIn(page, () => streamBody(model, { notifications: { rows: row ? [row] : [], unread: row ? 1 : 0 } }))
  await page.route('**/api/notifications/seen', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":true}' }))
  await page.goto('/#dashboard')
  await expect(page.locator('#page > [data-arrived]')).toBeVisible()
  let id = 900
  for (const c of cases) {
    const before = await page.evaluate(() => (window as unknown as { __banners: unknown[] }).__banners.length)
    row = { id: ++id, kind: c.kind, key: c.kind + ':' + id, title: c.symbol + ' filed', body: '', at: '2026-09-18T14:31:00Z', readAt: null, seenAt: null, extra: { symbol: c.symbol, exchange: c.exchange } }
    // the stream reconnects every 200 ms here, and the row arrives with it
    await expect.poll(() => page.evaluate(() => (window as unknown as { __banners: unknown[] }).__banners.length)).toBe(before + 1)
    await clickBanner(page, row)
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
