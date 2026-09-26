import { expect, test } from '@playwright/test'
import { ready, figures } from './helpers'

// SPEC §3, Filters: the popover is driven from its search box.

const open = async (page: import('@playwright/test').Page) => {
  await page.goto('/#trades')
  await ready(page)
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.getByLabel('Search', { exact: true })).toBeFocused()
}

test('Tab runs box, Done, Clear all, box; Shift+Tab runs it backwards', async ({ page }) => {
  await open(page)
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Done' })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Clear all' })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByLabel('Search', { exact: true })).toBeFocused()
  await page.keyboard.press('Shift+Tab')
  await expect(page.getByRole('button', { name: 'Clear all' })).toBeFocused()
})

test('Enter on a held symbol opens its holding; Shift+Enter narrows the book by it; Backspace in the empty box deselects it', async ({ page, request }) => {
  const model = await figures(request)
  const held = model.positions.find((p: { kind: string }) => p.kind === 'Shares')
  await open(page)
  await page.keyboard.type(held.symbol)
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(new RegExp('#portfolio/'))

  await open(page)
  await page.keyboard.type(held.symbol)
  await page.keyboard.press('Shift+Enter')
  await expect(page.locator('.chip')).toContainText(held.symbol)
  await expect(page.getByLabel('Search', { exact: true })).toBeFocused()

  // the box is cleared of what picked the value; Backspace in it deselects the highlighted value
  await page.getByLabel('Search', { exact: true }).fill(held.symbol)
  await page.getByLabel('Search', { exact: true }).fill('')
  await page.keyboard.type(held.symbol)
  for (let i = 0; i < held.symbol.length; i++) await page.keyboard.press('Backspace')
  await page.keyboard.type(held.symbol)
  await page.keyboard.press('Delete')
  await expect(page.locator('.chip')).toHaveCount(0)
})

test('a click on a value keeps the keyboard in the box, and ⌘K from one field returns to the search', async ({ page, request }) => {
  // an account is shown, and so typed and matched, by its name
  const account = ((await figures(request)).options.accounts[0] as { name: string }).name
  await open(page)
  await page.keyboard.type(account.slice(0, 3))
  await page.locator('.pop .pop-row', { hasText: account }).first().click()
  await expect(page.locator('.chip')).toContainText(account)
  await expect(page.getByLabel('Search', { exact: true })).toBeFocused()

  // from one field's own list, ⌘K is the search again
  // Esc closes the popover and leaves the filter standing
  await page.keyboard.press('Escape')
  await expect(page.locator('.pop')).toHaveCount(0)
  await expect(page.locator('.chip')).toContainText(account)
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop .pop-row').filter({ hasText: /^Symbol/ }).click()
  await expect(page.getByLabel('Search values')).toBeFocused()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.locator('.pop .lbl', { hasText: 'Filter by' })).toBeVisible()
})

test('a single list with no box of its own is driven by the same keys: arrows, Enter, Backspace, Tab', async ({ page, request }) => {
  const options = (await figures(request)).options
  // every single list but Symbol and Tag, which carry a box
  const lists: [string, string[]][] = [
    ['Account', options.accounts.map((a: { name: string }) => a.name)],
    ['Grade', options.grades], ['Side', options.sides], ['Kind', options.kinds], ['Exchange', options.exchanges], ['Result', options.results],
  ]
  for (const [label, values] of lists) {
    if (values.length < 2) continue
    await open(page)
    await page.locator('.pop .pop-row').filter({ hasText: new RegExp('^' + label) }).click()
    await expect(page.getByLabel('Search values')).toHaveCount(0)
    const rows = page.locator('.pop .scroll .pop-row')
    await expect(rows.first(), label).toBeFocused()
    await page.keyboard.press('ArrowDown')
    await expect(rows.nth(1), label).toBeFocused()
    await expect(rows.nth(1)).toHaveClass(/\bhi\b/)
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('ArrowUp') // round past the first to the last
    await expect(rows.last(), label).toBeFocused()
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Enter')
    await expect(page.locator('.chip'), label).toHaveCount(1)
    await expect(rows.first()).toHaveClass(/\bon\b/)
    await page.keyboard.press('Backspace')
    await expect(page.locator('.chip'), label).toHaveCount(0)
    await page.keyboard.press('Tab')
    await expect(page.getByRole('button', { name: 'Done' })).toBeFocused()
    await page.keyboard.press('Tab')
    await page.keyboard.press('Tab')
    await expect(rows.first(), label).toBeFocused() // the ring comes back to the list
    await page.keyboard.press('Escape')
    await expect(page.locator('.pop')).toHaveCount(0)
  }
})

test('a listing found outside the book carries the three icons in their places, and choosing it opens its page', async ({ page }) => {
  await page.route('**/api/symbols/search?*', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ok: true, matches: [
        { symbol: 'ZQVX', exchange: 'CBOE', name: 'Zqv Volatility Index', currency: 'USD', kind: 'Indices', rank: 0 },
        { symbol: 'ZQVS', exchange: 'NYSE', name: 'Zqv Shares Inc', currency: 'USD' },
      ] }),
    }),
  )
  const found: [string, string][] = [['ZQVX', 'CBOE'], ['ZQVS', 'NYSE']]
  await open(page)
  await page.keyboard.type('ZQV')
  // funnel, Buy, Sell: three in every row, in the same places
  const places: number[][] = []
  for (const [sym] of found) {
    const row = page.locator('.pop .pop-row', { hasText: sym })
    await expect(row.locator('.tk-rowbtn')).toHaveCount(3)
    places.push(await row.locator('.tk-rowbtn').evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().right))))
  }
  expect(places[0]).toEqual(places[1])
  for (const [sym, venue] of found) {
    await open(page)
    await page.keyboard.type('ZQV')
    await page.locator('.pop .pop-row', { hasText: sym }).click()
    await expect.poll(() => decodeURIComponent(page.url()).endsWith('#markets/listing:' + sym + '@' + venue)).toBe(true)
  }
})

test('⌘K opens the filters over the Orders panel', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: 'Orders' }).click()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.getByLabel('Search', { exact: true })).toBeVisible()
})
