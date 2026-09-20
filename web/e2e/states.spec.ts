import { expect, test } from '@playwright/test'
import { openWithStatus } from './helpers'

// What the page shows while it has nothing yet, when it cannot get it, and the two
// things it does for a reader everywhere: cut text shown whole, time said as it passes.

test('the app is there before the model is: header, tabs and the tab\'s own silhouette', async ({ page }) => {
  let release: () => void = () => {}
  const held = new Promise<void>((r) => (release = r))
  await page.route('**/api/events?*', async (route) => {
    await held
    await route.continue()
  })
  await page.goto('/#trades')
  await expect(page.locator('#hdr')).toBeVisible()
  await expect(page.locator('.tabbtn')).toHaveCount(5)
  await expect(page.locator('#pageSkel')).toBeVisible()
  // the trades silhouette is a table, not tiles
  expect(await page.locator('#pageSkel .bhsk').count()).toBeGreaterThan(100)
  await page.locator('.tabbtn', { hasText: 'Portfolio' }).click()
  await expect(page.locator('#pageSkel .bhsk[style*="border-radius:50%"]')).toBeVisible()
  // ⌘K has nothing to search yet
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.locator('.pop')).toHaveCount(0)
  release()
  // the page arrives under the skeleton, which fades and goes
  await expect(page.locator('#page .card').first()).toBeVisible()
  await expect(page.locator('#pageSkel')).toHaveCount(0)
  await expect(page.locator('.bh-skin')).toHaveCount(0)
})

test('a server that cannot be reached is said, with Retry, and Retry loads the page', async ({ page }) => {
  let refuse = true
  await page.route('**/api/events?*', (route) => (refuse ? route.abort() : route.continue()))
  await page.goto('/')
  await expect(page.locator('#page .status-err')).toHaveText('Could not reach Bagholder.')
  await expect(page.locator('#hdr')).toBeVisible()
  refuse = false
  await page.getByRole('button', { name: 'Retry' }).click()
  await expect(page.locator('#page .card').first()).toBeVisible()
  await expect(page.locator('#page .status-err')).toHaveCount(0)
})

test('text cut by an ellipsis is shown whole while the pointer is on it, and only then', async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 800 })
  await page.goto('/#trades')
  await expect(page.locator('#page table tbody tr').first()).toBeVisible()
  // cut one cell on purpose: which cells are cut otherwise depends on the book and the width
  const cell = page.locator('#page table tbody tr').first().locator('td').first()
  const whole = await cell.evaluate((td) => {
    const el = (td.querySelector('.cut, [style*="ellipsis"]') as HTMLElement) ?? td
    el.style.cssText += ';display:block;max-width:24px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap'
    el.setAttribute('data-e2e-cut', '')
    return (el.textContent ?? '').trim()
  })
  await page.locator('[data-e2e-cut]').hover()
  await expect(page.locator('#cutTip')).toBeVisible()
  await expect(page.locator('#cutTip')).toContainText(whole)
  await page.locator('#hdr').hover()
  await expect(page.locator('#cutTip')).toBeHidden()
  // nothing on the page carries a title tooltip
  expect(await page.locator('[title]').count()).toBe(0)
})

test('"Synced … ago" is said again as the minutes pass, without anything arriving', async ({ page, request }) => {
  await page.clock.install({ time: new Date('2026-09-18T15:00:20Z') })
  await openWithStatus(page, request, { connected: true, syncing: false, error: '', lastSync: '2026-09-18T14:58:00Z' })
  await expect(page.locator('#syncline')).toHaveText('Synced 2 min ago')
  await page.clock.fastForward(60_000)
  await expect(page.locator('#syncline')).toHaveText('Synced 3 min ago')
  await page.clock.fastForward(5 * 60_000)
  await expect(page.locator('#syncline')).toHaveText('Synced 8 min ago')
})

test('a tile\'s figure rolls to its new value and comes to rest as plain text', async ({ page, request }) => {
  const model = await (await request.get('/api/model')).json()
  const body = (m: unknown) => `retry: 200\nevent: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: m })}\n\n`
  let next = model
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body: body(next) }))
  await page.goto('/')
  const figure = page.locator('.kpi .v').first()
  const before = (await figure.textContent())!
  // the same figure, moved in its last digit; it arrives with the stream's next connection
  next = structuredClone(model)
  next.kpi.realized = model.kpi.realized + (Math.round(model.kpi.realized * 100) % 10 === 9 ? -0.01 : 0.01)
  await expect(figure.locator('.rl-w').first()).toBeVisible()
  // what a reader (or a copy) gets is the new figure throughout
  const after = (await figure.locator('.rl-plain').textContent())!
  expect(after).not.toBe(before)
  expect(after.length).toBe(before.length)
  // at rest: text, no wheels
  await expect(figure.locator('.rl-w')).toHaveCount(0)
  await expect(figure).toHaveText(after)
})

test('a list that scrolls inside its card shows its scrollbar only while it moves', async ({ page }) => {
  await page.goto('/#trades')
  const list = page.locator('.scroll, .scroll-xy').first()
  await expect(list).toBeVisible()
  await expect(list).not.toHaveClass(/scrolling/)
  await list.evaluate((el) => {
    el.scrollTop = 40
    el.dispatchEvent(new Event('scroll'))
  })
  await expect(list).toHaveClass(/scrolling/)
  await expect(list).not.toHaveClass(/scrolling/, { timeout: 3000 })
})
