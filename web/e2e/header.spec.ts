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

test('while the language model downloads, the status line says how far it has come, behind a sync under way', async ({ page, request }) => {
  const modelDownload = { received: 663483039, size: 1951420702 }
  await openWithStatus(page, request, { modelDownload })
  await expect(page.locator('#syncline')).toHaveText('Downloading the language model… 34%')
  await openWithStatus(page, request, { modelDownload, syncing: true, syncStep: 'Reading activity' })
  await expect(page.locator('#syncline')).toHaveText('Reading activity')
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

// The page keeps what it showed in the browser and draws it first on the next open,
// the header's status among it. A page loaded after an update therefore draws the old
// server's status before the new server answers: it must never take that for a server
// it heard, or it meets the old version against the new at every load and reloads for
// ever (2.0.0 on the owner's page, the header reading the version before).
test('a page that opens drawing what it kept of the version before loads once, and shows the new version', async ({ page, request }) => {
  const model = await modelDoc(request)
  const version = (model.status as { version: string }).version
  let server: Record<string, unknown> = { startedAt: 'A' }
  let hold: Promise<void> = Promise.resolve()
  await page.route('**/api/events?*', async (route) => {
    await hold
    await route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody({ ...model, status: { ...model.status, ...server } }) })
  })
  await page.route('**/api/events/watch', (route) => route.fallback())
  // the page served names the book the stand-in stream names
  await page.route((u) => u.pathname === '/', async (route) => {
    const r = await route.fetch()
    await route.fulfill({ response: r, body: (await r.text()).replace(/<meta name="bagholder-book" content="[^"]*">/, '<meta name="bagholder-book" content="test">') })
  })
  await page.goto('/')
  await ready(page)
  // the page keeps the status of the version it heard
  await expect.poll(() => page.evaluate(keptVersion)).toBe(version)
  // updated: the new server answers only once the page has drawn what it kept
  server = { startedAt: 'C', version: version + '-next' }
  let answer = () => {}
  hold = new Promise((r) => (answer = r))
  let loads = 0
  page.on('request', (r) => { if (r.resourceType() === 'document') loads++ })
  await page.reload()
  await expect(page.getByText('v' + version, { exact: true })).toBeVisible()
  answer()
  hold = Promise.resolve()
  // the new version's status is taken, not answered by loading the page again
  await expect(page.getByText('v' + version + '-next', { exact: true })).toBeVisible()
  expect(loads).toBe(1)
})

/** The version in the header's status the page keeps in the browser, or '' when none is kept. */
function keptVersion(): Promise<string> {
  return new Promise((resolve) => {
    const req = indexedDB.open('bagholder')
    req.onerror = () => resolve('')
    req.onsuccess = () => {
      const d = req.result
      if (!d.objectStoreNames.contains('docs')) return resolve('')
      const all = d.transaction('docs', 'readonly').objectStore('docs').openCursor()
      let found = ''
      all.onsuccess = () => {
        const c = all.result
        if (!c) return resolve(found)
        if (String(c.key).includes('|status|')) found = String((c.value as { data?: { version?: string } }).data?.version ?? '')
        c.continue()
      }
      all.onerror = () => resolve('')
    }
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
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(error)
  // the copy confirmed by the button's own state, and the error still there to read
  await expect(page.getByRole('button', { name: 'Copy error' })).toHaveAttribute('data-copied', 'true')
  await expect(page.locator('#syncline .status-err')).toHaveText(error)
})
