import { expect, test } from '@playwright/test'

// SPEC §3, Filters: the popover is driven from its search box.

const open = async (page: import('@playwright/test').Page) => {
  await page.goto('/#trades')
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
  const model = await (await request.get('/api/model')).json()
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
  const account = (await (await request.get('/api/model')).json()).options.accounts[0] as string
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

test('⌘K opens the filters over the Orders panel', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: 'Orders' }).click()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.getByLabel('Search', { exact: true })).toBeVisible()
})
