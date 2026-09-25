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

test('Add trade requires a date, symbol, quantity and price, and sends the entry once they are given', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await page.getByText('Add trade').click()
  await expect(page.getByRole('heading', { name: 'Add trade' })).toBeVisible()
  await page.getByRole('button', { name: 'Add trade' }).click()
  await expect(page.locator('.status-err')).toHaveText('Date, symbol, a quantity and a price are required.')

  let sent: Record<string, unknown> | null = null
  await page.route('**/api/entries', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true } }) })
  await page.getByPlaceholder('e.g. LUNR or LUNR 15JAN27 12.00 CALL').fill('zzzq')
  const boxes = page.locator('.input[inputmode="decimal"]')
  await boxes.nth(0).fill('10')
  await boxes.nth(1).fill('2.5')
  await page.getByRole('button', { name: 'Add trade' }).click()
  // the person's entry, its quantity and price the text typed, for the server to read exactly
  await expect.poll(() => sent).toMatchObject({ entry: 'trade', account: '', symbol: 'ZZZQ', side: 'BUY', quantity: '10', price: '2.5', currency: 'CAD', fee: '' })
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

/** Import CSV: the account chosen, then the files through the browser's picker. */
async function importFiles(page: Page, files: { name: string; text: string }[], account?: string) {
  await openMenu(page)
  await page.getByText('Import CSV').click()
  await expect(page.getByRole('heading', { name: 'Import CSV' })).toBeVisible()
  await expect(page.getByLabel('Account')).toHaveValue('')
  if (account) await page.getByLabel('Account').selectOption({ label: account })
  const chooser = page.waitForEvent('filechooser')
  await page.getByRole('button', { name: 'Choose files' }).click()
  await (await chooser).setFiles(files.map((f) => ({ name: f.name, mimeType: 'text/csv', buffer: Buffer.from(f.text) })))
}

test('Import CSV sends each file with the account chosen and reports what its rows did', async ({ page }) => {
  const csv = 'Date,Action,Symbol,Quantity,Price,Amount,Currency\n2026-01-05,Buy,ZZZQ,10,2.50,25.00,USD\n'
  const sent: Record<string, unknown>[] = []
  await page.route('**/api/import', (route) => {
    sent.push(route.request().postDataJSON())
    return route.fulfill({
      json: {
        file: 'trades.csv', layout: 'simple', account: 'Trading', rows: 3, added: 2, unchanged: 1, linked: 1,
        ambiguous: [{ line: 3, message: 'the same fill as 2 of the broker\'s rows: not linked' }],
        problems: [{ line: 4, message: 'the date "01/05/2026" is not a day written YYYY-MM-DD' }],
      },
    })
  })
  await page.goto('/')
  await ready(page)
  const m = await (await page.request.get('/api/figures')).json()
  const account = m.accounts.find((a: { name: string; brokerAccount: string }) => a.name && a.brokerAccount !== 'manual')
  await importFiles(page, [{ name: 'trades.csv', text: csv }], account.name)
  await expect.poll(() => sent.length).toBe(1)
  expect(sent[0]).toEqual({ name: 'trades.csv', text: csv, account: account.id })
  const dlg = page.locator('#modalDlg')
  await expect(dlg).toContainText('1 file · 2 new · 1 linked · 1 already stored')
  await expect(dlg).toContainText('Trading · simple · 3 rows · 2 new · 1 linked · 1 already stored')
  await expect(dlg).toContainText('1 not linked')
  await expect(dlg).toContainText("line 3: the same fill as 2 of the broker's rows: not linked")
  await expect(dlg).toContainText('1 with a problem')
  await expect(dlg).toContainText('line 4: the date "01/05/2026" is not a day written YYYY-MM-DD')
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(page.getByRole('heading', { name: 'Import CSV' })).toHaveCount(0)
})

test('Import CSV keeps a file\'s rows in the Manual account, and says why a file it cannot read was refused', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  const good = 'Date,Action,Symbol,Quantity,Price,Amount,Currency\n2026-01-05,Buy,ZZZQ,10,2.50,25.00,USD\n'
  await importFiles(page, [{ name: 'mine.csv', text: good }, { name: 'other.csv', text: 'foo,bar\n1,2\n' }])
  const dlg = page.locator('#modalDlg')
  await expect(dlg).toContainText('2 files · 1 new · 0 linked · 0 already stored')
  await expect(dlg).toContainText('Manual · simple · 1 rows · 1 new · 0 linked · 0 already stored')
  await expect(dlg).toContainText('other.csv')
  await expect(dlg).toContainText('its headers (foo, bar) are none of the layouts read')
})

test('Load folder: a folder that is not one is refused, a watched one lists its files, Scan now and Stop watching', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await openMenu(page)
  await page.getByText('Load folder').click()
  await expect(page.getByRole('heading', { name: 'Load folder' })).toBeVisible()
  // the real server: no such folder
  await page.getByLabel('Folder').fill('/nowhere/at/all')
  await page.getByRole('button', { name: 'Watch folder' }).click()
  await expect(page.locator('#modalDlg .status-err')).toHaveText('/nowhere/at/all is not a folder')

  const report = { file: 'a.csv', layout: 'activities', account: 'Manual', rows: 3, added: 2, unchanged: 1, linked: 0, ambiguous: [], problems: [] }
  const watched = {
    path: '/some/watched/folder', watching: true, account: '', lastScan: '2026-09-20T12:00:00Z', scanError: '',
    files: [
      { file: 'a.csv', size: 10, modified: '2026-09-20T11:00:00Z', scannedAt: '2026-09-20T12:00:00Z', read: { outcome: 'imported', report } },
      { file: 'b.csv', size: 3, modified: '2026-09-20T11:00:00Z', scannedAt: '2026-09-20T12:00:00Z', read: { outcome: 'failed', error: 'the file is not UTF-8 text' } },
    ],
  }
  let sent: unknown = null
  await page.route('**/api/watch', (route) => {
    if (route.request().method() !== 'POST') return route.continue()
    sent = route.request().postDataJSON()
    return route.fulfill({ json: watched })
  })
  await page.getByLabel('Folder').fill('/some/watched/folder')
  await page.getByRole('button', { name: 'Watch folder' }).click()
  await expect.poll(() => sent).toEqual({ path: '/some/watched/folder', account: '' })
  const dlg = page.locator('#modalDlg')
  await expect(dlg).toContainText('Watching /some/watched/folder')
  await expect(dlg).toContainText('Manual · activities · 3 rows · 2 new · 0 linked · 1 already stored')
  await expect(dlg).toContainText('the file is not UTF-8 text')

  await page.route('**/api/watch/scan', (route) => route.fulfill({ json: { ...watched, scanError: 'the folder: No such file or directory' } }))
  await page.getByRole('button', { name: 'Scan now' }).click()
  await expect(dlg.locator('.status-err')).toHaveText('the folder: No such file or directory')

  await page.route('**/api/watch/clear', (route) => route.fulfill({ json: { path: '', watching: false, account: '', lastScan: '', scanError: '', files: [] } }))
  await page.getByRole('button', { name: 'Stop watching' }).click()
  await expect(page.getByRole('button', { name: 'Stop watching' })).toHaveCount(0)
  await expect(page.getByLabel('Folder')).toHaveValue('')
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
