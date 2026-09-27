import { expect, test } from '@playwright/test'
import { modelDoc, openWithStatus, ready, streamBody, standIn } from './helpers'

// SPEC §3, the header: what the status line says, in the order it says it, and the
// update on offer beside the version.

test('a release that can be installed is a button, and pressing it asks for the update', async ({ page, request }) => {
  await openWithStatus(page, request, { updateAvailable: true, canUpdate: true, latestVersion: 'v9.9.9' })
  let asked = 0
  await page.route('**/api/update', (route) => { asked++; return route.fulfill({ json: { ok: true } }) })
  await page.getByRole('button', { name: 'Update to v9.9.9' }).click()
  expect(asked).toBe(1)
})

test('the Update button is gone while the update installs, and back if it fails', async ({ page, request }) => {
  const offer = { updateAvailable: true, canUpdate: true, latestVersion: 'v9.9.9' }
  await openWithStatus(page, request, { ...offer, updating: 'Downloading…' })
  await expect(page.locator('#syncline')).toHaveText('Downloading…')
  await expect(page.getByRole('button', { name: 'Update to v9.9.9' })).toHaveCount(0)
  await openWithStatus(page, request, { ...offer, updating: '', updateError: 'The update could not be verified.' })
  await expect(page.getByRole('button', { name: 'Update to v9.9.9' })).toBeVisible()
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

// SPEC §2, Versions: the page reloads itself when it sees a new version answering.
// The stream here ends after each view and the page connects again every 200 ms, so
// each change of `server` is heard at the next connection.
for (const [what, next] of [
  ['a new version', (s: { version: string }) => ({ version: s.version + '-next' })],
  ['a server of another protocol', () => ({ protocol: '1999-01-01.1' })],
] as const) {
  test(`${what} answering after a restart loads the page again, once; a restart of the same build does not`, async ({ page, request }) => {
    const model = await modelDoc(request)
    let server: Record<string, unknown> = { startedAt: 'A' }
    await standIn(page, () => streamBody({ ...model, status: { ...model.status, ...server } }))
    let loads = 0
    page.on('request', (r) => { if (r.resourceType() === 'document') loads++ })
    await page.goto('/')
    await ready(page)
    expect(loads).toBe(1)
    server = { startedAt: 'B' } // started again, the same build
    await page.waitForTimeout(1000)
    expect(loads).toBe(1)
    server = { startedAt: 'C', ...next(model.status) }
    await expect.poll(() => loads).toBe(2)
    await ready(page)
    // the page now loaded hears the same server at every connection: it does not load again
    await page.waitForTimeout(1500)
    expect(loads).toBe(2)
  })
}

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

// SPEC §4, the header: a sync error takes the sync status on one line whatever its
// length, cut with an ellipsis past its room and read whole on hover, so the header keeps
// its height and holds it at every width the page is laid out for.
test('a sync error keeps the header to one line, cut past its room and read whole on hover', async ({ page, request }) => {
  const sentence = 'The account named in this answer could not be read, and the figure stored before it stays until a read answers.'
  for (const error of [sentence.slice(0, 61), sentence, (sentence + ' ').repeat(12).trim()]) {
    for (const width of [1200, 1340, 1440, 1680]) {
      await page.setViewportSize({ width, height: 800 })
      await openWithStatus(page, request, { connected: true, error })
      const line = page.locator('#syncline')
      await expect(line.locator('.status-err')).toHaveText(error)
      const fit = await page.evaluate(() => {
        const hdr = document.getElementById('hdr')!
        const el = document.getElementById('syncline')!
        const line = el.getBoundingClientRect()
        const brand = hdr.firstElementChild!.getBoundingClientRect()
        const buttons = [...hdr.querySelectorAll('button[aria-label]:not([aria-label="Copy error"])')].map((b) => b.getBoundingClientRect())
        const copy = hdr.querySelector('button[aria-label="Copy error"]')!.getBoundingClientRect()
        return {
          copyBeside: copy.left >= line.right && copy.right <= buttons[0].left,
          hdrFits: hdr.scrollWidth <= hdr.clientWidth,
          pageFits: document.documentElement.scrollWidth <= window.innerWidth,
          // one line: no taller than the line height of its text
          oneLine: line.height <= parseFloat(getComputedStyle(el).lineHeight || '16') + 1 || line.height <= 20,
          headerHeld: hdr.getBoundingClientRect().height <= buttons[0].height + 40,
          clearOfBrand: line.left >= brand.right,
          clearOfButtons: buttons.every((b) => line.right <= b.left || line.left >= b.right),
          // every header button at its own size, none squeezed by the text
          buttonsShown: buttons.every((b) => b.width === buttons[0].width && b.height === buttons[0].height && b.right <= window.innerWidth),
        }
      })
      expect(fit, `${error.length} characters at ${width} px`).toEqual({ copyBeside: true, hdrFits: true, pageFits: true, oneLine: true, headerHeld: true, clearOfBrand: true, clearOfButtons: true, buttonsShown: true })
    }
  }
})

test('a sync error cut in the header is read whole in the page\'s own tip on hover', async ({ page, request }) => {
  const error = ('The account named in this answer could not be read, and the figure stored before it stays until a read answers. ').repeat(6).trim()
  await page.setViewportSize({ width: 1340, height: 800 })
  await openWithStatus(page, request, { connected: true, error })
  await page.locator('#syncline').hover()
  await expect(page.locator('#cutTip .tv')).toHaveText(error)
  // and copied whole with the icon beside it
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.getByRole('button', { name: 'Copy error' }).click()
  await expect(page.locator('#syncline')).toHaveText('Error copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(error)
})
