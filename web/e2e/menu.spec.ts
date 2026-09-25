import { expect, test, type Page } from '@playwright/test'
import { ready, openWithStatus } from './helpers'

// SPEC §3, the header's menu, and §3 "The menu."/"Connecting." for what each item
// does. Connect, CSV import, the watch folder, Clear data and Disconnect never
// touch anything real: their writes are intercepted with page.route.

const openMenu = async (page: Page) => {
  await page.getByRole('button', { name: 'Menu' }).click()
}

test('the menu offers Connect Wealthsimple when there is no session, and Sync now / Refresh session when there is one', async ({ page, request }) => {
  await openWithStatus(page, request, { connected: false, email: '' })
  await openMenu(page)
  await expect(page.getByRole('button', { name: 'Connect Wealthsimple' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Sync now' })).toHaveCount(0)

  await openWithStatus(page, request, { connected: true, email: 'someone@example.test' })
  await openMenu(page)
  await expect(page.getByRole('button', { name: 'Sync now' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Refresh session' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Connect Wealthsimple' })).toHaveCount(0)
})

test('Disconnect is live whenever a login is saved, connected or not, and greyed when there is none', async ({ page, request }) => {
  await openWithStatus(page, request, { connected: false, email: '' })
  await openMenu(page)
  await expect(page.getByRole('button', { name: 'Disconnect' })).toBeDisabled()

  await openWithStatus(page, request, { connected: false, email: 'someone@example.test' })
  await openMenu(page)
  await expect(page.getByRole('button', { name: 'Disconnect' })).toBeEnabled()

  await openWithStatus(page, request, { connected: true, email: 'someone@example.test' })
  await openMenu(page)
  await expect(page.getByRole('button', { name: 'Disconnect' })).toBeEnabled()
})

test('the menu closes on a press outside it, and stays open on a press inside it', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await expect(page.getByRole('button', { name: 'Add trade' })).toBeVisible()
  // a press inside the menu (its own separator, not a button) leaves it open
  await page.locator('.menu .sep').first().click()
  await expect(page.getByRole('button', { name: 'Add trade' })).toBeVisible()
  await page.getByText('Bagholder').click()
  await expect(page.getByRole('button', { name: 'Add trade' })).toHaveCount(0)
})

test('Connect opens the streamed sign-in view, forwards keystrokes to it, and Cancel closes it and tells the server to stop', async ({ page, request }) => {
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', 'base64')
  let started = 0
  let cancelled = 0
  const inputs: unknown[] = []
  await page.route('**/api/login/start', (route) => { started++; return route.fulfill({ json: { ok: true } }) })
  await page.route('**/api/login/stream*', (route) => route.fulfill({ status: 200, contentType: 'image/png', body: png }))
  await page.route('**/api/login/input', (route) => { inputs.push(route.request().postDataJSON()); return route.fulfill({ json: { ok: true } }) })
  await page.route('**/api/login/cancel', (route) => { cancelled++; return route.fulfill({ json: { ok: true } }) })

  await openWithStatus(page, request, { connected: false, loginView: true })
  await openMenu(page)
  await page.getByRole('button', { name: 'Connect Wealthsimple' }).click()
  expect(started).toBe(1)
  await expect(page.getByText('Waiting for Wealthsimple login…')).toBeVisible()
  await expect(page.getByText('Sign in to Wealthsimple')).toBeVisible()
  await expect(page.locator('#loginDlg img')).toBeVisible()

  await page.keyboard.press('a')
  await expect.poll(() => inputs).toContainEqual({ kind: 'text', text: 'a' })
  await page.keyboard.press('Escape') // forwarded to the streamed browser, not treated as this page's Escape
  await expect.poll(() => inputs).toContainEqual({ kind: 'key', key: 'Escape' })
  await expect(page.getByText('Sign in to Wealthsimple')).toBeVisible()

  await page.locator('#loginDlg').getByRole('button', { name: 'Cancel' }).click()
  expect(cancelled).toBe(1)
  await expect(page.getByText('Sign in to Wealthsimple')).toHaveCount(0)
})

test('Sync now marks the header as syncing while the request is in flight', async ({ page, request }) => {
  await openWithStatus(page, request, { connected: true, syncing: false, error: '' })
  let release: () => void = () => {}
  let asked = 0
  await page.route('**/api/sync', async (route) => { asked++; await new Promise<void>((r) => (release = r)); await route.fulfill({ json: { ok: true } }) })
  await openMenu(page)
  await page.getByRole('button', { name: 'Sync now' }).click()
  await expect(page.locator('#syncline')).toHaveText('Syncing…')
  release()
  await expect.poll(() => asked).toBe(1)
})

test('the theme choice persists across a reload', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await page.locator('.menu button', { hasText: 'Theme' }).first().click()
  await page.locator('.menu.sub button', { hasText: 'Midnight' }).click()
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'midnight')
  expect(await page.evaluate(() => localStorage.getItem('bh2.theme'))).toBe('midnight')

  await page.reload()
  await ready(page)
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'midnight')
  await openMenu(page)
  await expect(page.locator('.menu button', { hasText: 'Theme' }).first()).toContainText('Midnight')

  // put it back so later runs of this file start from the default
  await page.locator('.menu button', { hasText: 'Theme' }).first().click()
  await page.locator('.menu.sub button', { hasText: 'Nocturne' }).click()
})

test('Add trade requires a date, symbol, positive quantity and a price, and submits once they are given', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await page.getByText('Add trade').click()
  await expect(page.getByRole('heading', { name: 'Add trade' })).toBeVisible()
  await page.getByRole('button', { name: 'Add trade' }).click()
  await expect(page.locator('.status-err')).toHaveText('Date, symbol, a positive quantity and a price are required.')

  let sent: Record<string, unknown> | null = null
  await page.route('**/api/book/append', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true, added: true } }) })
  await page.getByPlaceholder('e.g. LUNR or LUNR 15JAN27 12.00 CALL').fill('zzzq')
  const boxes = page.locator('.input[inputmode="decimal"]')
  await boxes.nth(0).fill('10')
  await boxes.nth(1).fill('2.5')
  await page.getByRole('button', { name: 'Add trade' }).click()
  await expect.poll(() => sent?.symbol).toBe('ZZZQ')
  await expect(page.getByText('Trade added')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Add trade' })).toHaveCount(0)
})

test('Export trades CSV downloads the book as a CSV file', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  const downloadPromise = page.waitForEvent('download')
  await page.getByText('Export trades CSV').click()
  const download = await downloadPromise
  expect(download.suggestedFilename()).toBe('bagholder-trades.csv')
})

test('Import CSV reads the file, sends its name and text, and reports what was added and skipped', async ({ page }) => {
  const csv = 'Date,Action,Symbol,Quantity,Price,Amount\n2026-01-05,Buy,ZZZQ,10,2.50,-25.00\nbad-date,Buy,ZZZQ,5,1.00,-5.00\n'
  let sent: Record<string, unknown> | null = null
  await page.route('**/api/import', (route) => {
    sent = route.request().postDataJSON()
    return route.fulfill({
      json: {
        ok: true, file: 'trades.csv', format: 'legacy', rows: 2, added: 1, duplicates: 0,
        skipped: [{ row: 3, message: 'Unparsed row (missing or invalid date)' }], skippedCount: 1,
        footerStripped: false, countsByType: { Trade: 1 },
      },
    })
  })
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  const chooserPromise = page.waitForEvent('filechooser')
  await page.getByText('Import CSV').click()
  const chooser = await chooserPromise
  await chooser.setFiles([{ name: 'trades.csv', mimeType: 'text/csv', buffer: Buffer.from(csv) }])

  await expect(page.getByRole('heading', { name: 'Import CSV' })).toBeVisible()
  await expect.poll(() => (sent as { name?: string } | null)?.name).toBe('trades.csv')
  expect((sent as unknown as { text?: string }).text).toBe(csv)
  await expect(page.locator('#modalDlg')).toContainText('1 file · 1 new activity · 0 already stored')
  await expect(page.locator('#modalDlg')).toContainText('legacy · 2 rows · 1 new · 0 duplicates')
  await expect(page.locator('#modalDlg')).toContainText('1 skipped')
  await expect(page.locator('#modalDlg')).toContainText('row 3: Unparsed row (missing or invalid date)')
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(page.getByRole('heading', { name: 'Import CSV' })).toHaveCount(0)
})

test('the watch-folder box shows an error for a bad path, then the watch, Scan now and Stop watching once one is set', async ({ page }) => {
  await page.route('**/api/watch', (route) => {
    if (route.request().method() !== 'GET') return route.continue()
    return route.fulfill({ json: { ok: true, watching: false } })
  })
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await page.getByText('Load folder').click()
  await expect(page.getByRole('heading', { name: 'Load folder' })).toBeVisible()

  let lastPath = ''
  await page.route('**/api/watch', (route) => {
    if (route.request().method() !== 'POST') return route.continue()
    lastPath = (route.request().postDataJSON() as { path: string }).path
    return route.fulfill({ status: 400, json: { ok: false, error: 'Not a folder: ' + lastPath } })
  })
  await page.getByPlaceholder('/Users/you/Downloads/wealthsimple').fill('/nowhere/at/all')
  await page.getByRole('button', { name: 'Watch folder' }).click()
  await expect(page.locator('.status-err')).toHaveText('Not a folder: /nowhere/at/all')

  await page.route('**/api/watch', (route) => {
    if (route.request().method() !== 'POST') return route.continue()
    return route.fulfill({ json: { ok: true, path: '/some/watched/folder', status: { ok: true, path: '/some/watched/folder', watching: true, lastScan: '', files: [] } } })
  })
  await page.getByPlaceholder('/Users/you/Downloads/wealthsimple').fill('/some/watched/folder')
  await page.getByRole('button', { name: 'Watch folder' }).click()
  await expect(page.locator('#modalDlg')).toContainText('Watching /some/watched/folder')
  await expect(page.getByRole('button', { name: 'Scan now' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Stop watching' })).toBeVisible()

  await page.route('**/api/watch/scan', (route) => route.fulfill({ json: { ok: true, path: '/some/watched/folder', files: [{ file: 'a.csv', format: 'legacy', added: 2, duplicates: 1 }], status: { ok: true, path: '/some/watched/folder', watching: true, lastScan: '2026-09-20T12:00:00Z', files: [{ file: 'a.csv', format: 'legacy', added: 2, duplicates: 1, scannedAt: '2026-09-20T12:00:00Z' }] } } }))
  await page.getByRole('button', { name: 'Scan now' }).click()
  await expect(page.locator('#modalDlg')).toContainText('a.csv')
  await expect(page.locator('#modalDlg')).toContainText('legacy · 2 new · 1 dup')

  await page.route('**/api/watch/clear', (route) => route.fulfill({ json: { ok: true, watching: false } }))
  await page.getByRole('button', { name: 'Stop watching' }).click()
  await expect(page.getByRole('button', { name: 'Stop watching' })).toHaveCount(0)
  await expect(page.getByPlaceholder('/Users/you/Downloads/wealthsimple')).toHaveValue('')
})

test('Clear data asks once: Esc and an unfocused Enter leave it standing, only the button acts', async ({ page }) => {
  let asked = 0
  await page.route('**/api/data/clear', (route) => { asked++; return route.fulfill({ json: { ok: true } }) })
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await page.getByText('Clear data').click()
  await expect(page.getByRole('heading', { name: 'Clear data' })).toBeVisible()
  await expect(page.locator('#confirmDlg')).toContainText('Deletes everything synced from Wealthsimple, your journal and the downloaded market data from this machine. Your Wealthsimple login stays.')

  await page.keyboard.press('Escape')
  await expect(page.getByRole('heading', { name: 'Clear data' })).toHaveCount(0)
  expect(asked).toBe(0)

  await openMenu(page)
  await page.getByText('Clear data').click()
  await page.keyboard.press('Enter') // nothing is focused inside the dialog; Enter is not a shortcut for it
  await expect(page.getByRole('heading', { name: 'Clear data' })).toBeVisible()
  expect(asked).toBe(0)

  let sent: Record<string, unknown> | null = null
  await page.route('**/api/data/clear', (route) => { sent = route.request().postDataJSON(); asked++; return route.fulfill({ json: { ok: true } }) })
  await page.locator('#confirmDlg').getByRole('button', { name: 'Clear data' }).click()
  await expect.poll(() => asked).toBe(1)
  expect(sent).toEqual({ journal: true, market: true })
  await expect(page.getByRole('heading', { name: 'Clear data' })).toHaveCount(0)
})

test('Disconnect asks once, and Esc keeps the session while the button ends it', async ({ page, request }) => {
  let asked = 0
  await page.route('**/api/disconnect', (route) => { asked++; return route.fulfill({ json: { ok: true } }) })
  await openWithStatus(page, request, { connected: true, email: 'someone@example.test' })
  await openMenu(page)
  await page.getByRole('button', { name: 'Disconnect' }).click()
  await expect(page.getByRole('heading', { name: 'Disconnect' })).toBeVisible()
  await expect(page.locator('#confirmDlg')).toContainText('Signs out of Wealthsimple on this machine. Your synced history stays.')

  await page.keyboard.press('Escape')
  await expect(page.getByRole('heading', { name: 'Disconnect' })).toHaveCount(0)
  expect(asked).toBe(0)

  await openMenu(page)
  await page.getByRole('button', { name: 'Disconnect' }).click()
  await page.locator('#confirmDlg').getByRole('button', { name: 'Disconnect' }).click()
  await expect.poll(() => asked).toBe(1)
  await expect(page.getByRole('heading', { name: 'Disconnect' })).toHaveCount(0)
})
