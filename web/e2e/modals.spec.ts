import { expect, test } from '@playwright/test'

// SPEC §3, the menu's dialogs: Enter submits.

test('Enter in the folder box asks for that folder to be watched', async ({ page }) => {
  await page.goto('/')
  let sent: unknown = null
  await page.route('**/api/watch', (route) => (route.request().method() === 'POST' ? ((sent = route.request().postDataJSON()), route.fulfill({ status: 400, json: { ok: false, error: 'No such folder.' } })) : route.continue()))
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Load folder').click()
  await page.getByPlaceholder('/Users/you/Downloads/wealthsimple').fill('/nowhere/at/all')
  await page.keyboard.press('Enter')
  await expect.poll(() => sent).toEqual({ path: '/nowhere/at/all', account: '' })
})

test('Enter in the Add trade boxes submits the trade', async ({ page }) => {
  await page.goto('/')
  let sent: Record<string, unknown> | null = null
  await page.route('**/api/entries', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ status: 400, json: { ok: false, error: 'not in this test' } }) })
  await page.route('**/api/listings?*', (route) => route.fulfill({ json: { ok: true, unread: [], listings: [{ listing: 'ZZZQ@XTSE', symbol: 'ZZZQ', name: 'Zzzq Corp', exchange: 'TSX', currency: 'CAD', held: false }] } }))
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Add trade').click()
  await page.getByPlaceholder('e.g. LUNR or LUNR 15JAN27 12.00 CALL').fill('ZZZQ')
  const boxes = page.locator('.input[inputmode="decimal"]')
  await boxes.nth(0).fill('10')
  await boxes.nth(1).fill('2.5')
  await page.keyboard.press('Enter')
  await expect.poll(() => sent?.symbol ?? null).toBe('ZZZQ')
  // the server's refusal is said under the form, in its words
  await expect(page.locator('#modalDlg .status-err')).toHaveText('not in this test')
})

test('a dialog rises 8 px and scales from 98% over .24 s, and its scrim fades in over .18 s', async ({ page }) => {
  await page.goto('/')
  for (const item of ['Add trade', 'Import CSV', 'Load folder']) {
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByText(item, { exact: true }).click()
    const motion = (sel: string) =>
      page.locator(sel).evaluate((el) =>
        el.getAnimations().map((a) => {
          const k = (a.effect as KeyframeEffect).getKeyframes()
          return { ms: Number((a.effect as KeyframeEffect).getTiming().duration), from: { opacity: k[0].opacity, transform: k[0].transform }, to: { opacity: k[k.length - 1].opacity } }
        }),
      )
    const scrim = await motion('#modalDlg')
    expect(scrim, item).toEqual([{ ms: 180, from: { opacity: '0', transform: undefined }, to: { opacity: '1' } }])
    const dialog = await motion('#modalDlg > .card')
    expect(dialog, item).toHaveLength(1)
    expect(dialog[0].ms).toBe(240)
    expect(dialog[0].from.opacity).toBe('0')
    expect(String(dialog[0].from.transform).replace(/\s+/g, '')).toBe('translateY(8px)scale(0.98)')
    await page.keyboard.press('Escape')
    await expect(page.locator('#modalDlg')).toHaveCount(0)
  }
})

// Add trade's Exchange (brief 42, 4 and 10): a listing is taken alone only where every
// directory answered; "Not listed" only where every one answered with none; the prompt
// to choose goes once one is chosen or the symbol changes; the chosen listing is whole
// in its box at every width.
test.describe('Add trade, the exchange', () => {
  type L = { listing: string; symbol: string; name: string; exchange: string; currency: string; held: false }
  const on = (symbol: string, exchange: string, venue: string, name = symbol + ' Corp'): L => ({ listing: symbol + '@' + venue, symbol, name, exchange, currency: 'CAD', held: false })
  async function open(page: import('@playwright/test').Page, answers: Record<string, { listings: L[]; unread: string[] }>) {
    await page.goto('/')
    await page.route('**/api/listings?*', (route) => {
      const s = new URL(route.request().url()).searchParams.get('symbol')!.toUpperCase()
      route.fulfill({ json: { ok: true, ...(answers[s] ?? { listings: [], unread: [] }) } })
    })
    await page.route('**/api/entries', (route) => route.fulfill({ status: 400, json: { ok: false, error: 'not in this test' } }))
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByText('Add trade').click()
  }
  const symbolBox = (page: import('@playwright/test').Page) => page.getByPlaceholder('e.g. LUNR or LUNR 15JAN27 12.00 CALL')
  const exchange = (page: import('@playwright/test').Page) => page.getByLabel('Exchange')
  const typeSymbol = async (page: import('@playwright/test').Page, s: string) => {
    const asked = page.waitForRequest((r) => r.url().includes('/api/listings?'))
    await symbolBox(page).fill(s)
    await symbolBox(page).press('Tab')
    await asked
  }

  test('one listing beside a directory unread is offered, not taken, with the failure said', async ({ page }) => {
    await open(page, { ZQEA: { listings: [on('ZQEA', 'TSX', 'XTSE')], unread: ["The TSX Venture's directory could not be read: refused"] } })
    await typeSymbol(page, 'ZQEA')
    await expect(exchange(page)).toBeEnabled()
    await expect(exchange(page)).toHaveValue('')
    await expect(page.locator('#modalDlg .status-err')).toHaveText("The TSX Venture's directory could not be read: refused")
    await exchange(page).selectOption('ZQEA@XTSE')
    await expect(exchange(page)).toHaveValue('ZQEA@XTSE')
  })

  test('"Not listed" only where every directory answered with none', async ({ page }) => {
    await open(page, { ZQEB: { listings: [], unread: ["Nasdaq's directory could not be read: refused"] }, ZQEC: { listings: [], unread: [] } })
    await typeSymbol(page, 'ZQEB')
    await expect(exchange(page).locator('option')).toHaveText([''])
    await expect(page.locator('#modalDlg .status-err')).toHaveText("Nasdaq's directory could not be read: refused")
    await typeSymbol(page, 'ZQEC')
    await expect(exchange(page).locator('option')).toHaveText(['Not listed'])
  })

  test('a contract written by its terms leaves the box empty', async ({ page }) => {
    await open(page, {})
    await symbolBox(page).fill('ZQEC 15JAN27 12.00 CALL')
    await symbolBox(page).press('Tab')
    await expect(exchange(page)).toBeDisabled()
    await expect(exchange(page).locator('option')).toHaveText([''])
  })

  test('the prompt to choose goes once one is chosen, and once the symbol is one with a single listing', async ({ page }) => {
    await open(page, { ZQED: { listings: [on('ZQED', 'TSX', 'XTSE'), on('ZQED', 'NYSE', 'US')], unread: [] }, ZQEE: { listings: [on('ZQEE', 'TSX', 'XTSE')], unread: [] } })
    await typeSymbol(page, 'ZQED')
    const boxes = page.locator('.input[inputmode="decimal"]')
    await boxes.nth(0).fill('1')
    await boxes.nth(1).fill('1')
    await page.getByRole('button', { name: 'Add trade' }).click()
    const prompt = page.locator('#modalDlg .status-err', { hasText: 'Choose the exchange it trades on.' })
    await expect(prompt).toHaveCount(1)
    await exchange(page).selectOption('ZQED@US')
    await expect(prompt).toHaveCount(0)
    await exchange(page).selectOption('')
    await page.getByRole('button', { name: 'Add trade' }).click()
    await expect(prompt).toHaveCount(1)
    await typeSymbol(page, 'ZQEE')
    await expect(exchange(page)).toBeDisabled()
    await expect(prompt).toHaveCount(0)
  })

  test('the chosen listing is whole in its box at every width', async ({ page }) => {
    await open(page, { ZQEF: { listings: [on('ZQEF', 'NYSE', 'US', 'Toronto-Dominion Bank'), on('ZQEF', 'TSX', 'XTSE', 'Toronto-Dominion Bank')], unread: [] } })
    await typeSymbol(page, 'ZQEF')
    await exchange(page).selectOption('ZQEF@US')
    for (const width of [1200, 1340, 1440, 1680]) {
      await page.setViewportSize({ width, height: 900 })
      // the chosen option's words, set in the box's own font, fit inside its padding and arrow
      const fits = await exchange(page).evaluate((el) => {
        const s = el as HTMLSelectElement
        const cs = getComputedStyle(s)
        const probe = document.createElement('span')
        probe.style.font = cs.font
        probe.style.whiteSpace = 'pre'
        probe.textContent = s.selectedOptions[0].textContent
        document.body.appendChild(probe)
        const text = probe.getBoundingClientRect().width
        probe.remove()
        // the native arrow sits inside the padding box: 24 px kept for it
        const room = s.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight) - 24
        return { text, room }
      })
      expect(fits.text, `${width}px`).toBeLessThanOrEqual(fits.room)
    }
  })
})
