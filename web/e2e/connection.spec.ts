import { expect, test, type Page } from '@playwright/test'
import { modelDoc, ready, streamBody } from './helpers'

// docs/plans/sign-in-renews-itself.md: a stream the browser gave up on is opened
// again by itself, said in the header meanwhile on every screen; a sign-in in front
// of Bagholder that expired is renewed by the page loading itself again, once.
// The stand-in stream ends after its documents and the browser connects again
// every 200 ms, so each change to what the stand-in answers is met at once.

type Answer = 'serve' | 'refuse' | 'sign-in' | 'hold'

/**
 * The page's stream stood in for: served, refused as a proxy refuses it, sent to a
 * sign-in elsewhere, or held unanswered: the stand-in's stream ends after its
 * documents and the browser connects again, where a real one stays open, so `hold`
 * is a stream that stays open, with no new hello to say it opened again.
 */
async function stream(page: Page, request: Parameters<typeof modelDoc>[0], answer: () => Answer): Promise<{ asked: () => number }> {
  const model = await modelDoc(request)
  let asked = 0
  await page.route('**/api/events?*', (route) => {
    asked++
    const a = answer()
    if (a === 'refuse') return route.fulfill({ status: 502, contentType: 'text/plain', body: 'Bad gateway' })
    if (a === 'sign-in') return route.fulfill({ status: 302, headers: { location: 'https://team.example.com/cdn-cgi/access/login/page' } })
    if (a === 'hold') return new Promise<void>(() => {})
    return route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody(model) })
  })
  await page.route('**/api/events/watch', (route) => route.fallback())
  // the sign-in's page: another origin, which grants the page nothing
  await page.route('https://team.example.com/**', (route) => route.fulfill({ status: 200, contentType: 'text/html', body: '<p>Sign in</p>' }))
  return { asked: () => asked }
}

/** The status asked as the page asks whether the sign-in in front has expired: sent to its login page while `expired`. */
async function signIn(page: Page, expired: () => boolean): Promise<void> {
  await page.route('**/api/status', (route) => {
    if (expired() && route.request().headers()['x-requested-with']) return route.fulfill({ status: 302, headers: { location: 'https://team.example.com/cdn-cgi/access/login/page' } })
    return route.fallback()
  })
}

const line = (page: Page) => page.locator('#syncline')

test('a stream refused with the book on screen is said in the header, and is opened again with no click', async ({ page, request }) => {
  let answer: Answer = 'serve'
  let refusals = 0
  const s = await stream(page, request, () => {
    if (answer === 'refuse' && ++refusals > 3) answer = 'serve'
    return answer
  })
  await page.goto('/')
  await ready(page)
  answer = 'refuse'
  // said where the page is, with the figures still shown
  await expect(line(page)).toContainText('Reconnecting to Bagholder…')
  await expect(page.locator('#syncline').getByRole('button', { name: 'Retry' })).toBeVisible()
  await expect(page.locator('#page > [data-arrived]')).toBeVisible()
  // three refusals, then the stream opens again on its own: the line says the sync again
  await expect(line(page)).not.toContainText('Reconnecting', { timeout: 20_000 })
  expect(refusals).toBeGreaterThan(3)
  expect(s.asked()).toBeGreaterThan(4)
})

test('a closed stream is opened again at once when the network comes back, and when the page is shown again', async ({ page, request }) => {
  let answer: Answer = 'serve'
  const s = await stream(page, request, () => answer)
  await page.goto('/')
  await ready(page)
  for (const event of ['online', 'pageshow']) {
    answer = 'refuse'
    await expect(line(page)).toContainText('Reconnecting to Bagholder…')
    // refused until the next try is several seconds away
    const from = s.asked()
    await expect.poll(() => s.asked() - from, { timeout: 15_000 }).toBeGreaterThanOrEqual(3)
    answer = 'serve'
    const before = s.asked()
    await page.evaluate((e) => window.dispatchEvent(e === 'online' ? new Event('online') : new PageTransitionEvent('pageshow', { persisted: true })), event)
    // asked at once, not at the next try
    await expect.poll(() => s.asked() - before, { timeout: 2_000 }).toBeGreaterThan(0)
    await expect(line(page)).not.toContainText('Reconnecting', { timeout: 3_000 })
  }
})

test('an expired sign-in in front of Bagholder loads the page again once, and not again within the minute', async ({ page, request }) => {
  let expired = false
  await stream(page, request, () => (expired ? 'sign-in' : 'serve'))
  await signIn(page, () => expired)
  let loads = 0
  page.on('request', (r) => { if (r.resourceType() === 'document' && new URL(r.url()).pathname === '/') loads++ })
  await page.goto('/')
  await ready(page)
  expect(loads).toBe(1)
  expired = true
  await expect.poll(() => loads, { timeout: 15_000 }).toBe(2)
  // loaded again and still sent to the sign-in: it needs the person, said (in the
  // header, or in the page's body when nothing is drawn), never a loop
  await expect(page.getByText('The sign-in in front of Bagholder did not renew', { exact: false }).first()).toBeVisible({ timeout: 15_000 })
  await page.waitForTimeout(3000)
  expect(loads).toBe(2)
})

// The owner, 2026-11-06: "TypeError: Failed to fetch" under the filter's search, on
// a listing's chart and for its short interest. The stream, opened before the sign-in
// in front expired, still ran, and every new request was sent to the sign-in's page,
// another origin, which the browser reports as no answer at all.
test('a request an expired sign-in sends away says so in words, and the sign-in renews once the field is left', async ({ page, request }) => {
  let expired = false
  // the stream, opened before the sign-in expired, stays open
  await stream(page, request, () => (expired ? 'hold' : 'serve'))
  await signIn(page, () => expired)
  await page.route('**/api/symbols/search?*', (route) => (expired ? route.fulfill({ status: 302, headers: { location: 'https://team.example.com/cdn-cgi/access/login/page' } }) : route.fallback()))
  let loads = 0
  page.on('request', (r) => { if (r.resourceType() === 'document' && new URL(r.url()).pathname === '/') loads++ })
  await page.goto('/#trades')
  await ready(page)
  expired = true
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.getByLabel('Search', { exact: true })).toBeFocused()
  await page.keyboard.type('ZQV')
  await expect(page.locator('.pop .status-err').first()).toHaveText('Signing in again…')
  await expect(line(page)).toContainText('Signing in again…')
  await expect(page.getByText('Failed to fetch', { exact: false })).toHaveCount(0)
  // not while the field is in use
  await page.waitForTimeout(1000)
  expect(loads).toBe(1)
  await page.keyboard.press('Escape')
  await expect.poll(() => loads, { timeout: 5_000 }).toBe(2)
})

test('the page waits to load itself again while a dialog is open, and loads once it closes', async ({ page, request }) => {
  let expired = false
  await stream(page, request, () => (expired ? 'sign-in' : 'serve'))
  await signIn(page, () => expired)
  let loads = 0
  page.on('request', (r) => { if (r.resourceType() === 'document' && new URL(r.url()).pathname === '/') loads++ })
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Add trade').click()
  await page.getByPlaceholder('e.g. LUNR or LUNR 15JAN27 12.00 CALL').fill('ZZZQ')
  expired = true
  await expect(line(page)).toContainText('Signing in again…', { timeout: 15_000 })
  await page.waitForTimeout(2000)
  expect(loads).toBe(1)
  await page.keyboard.press('Escape')
  await expect(page.locator('#modalDlg')).toHaveCount(0)
  await expect.poll(() => loads, { timeout: 5_000 }).toBe(2)
})
