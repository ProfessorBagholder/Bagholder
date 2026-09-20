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
  await expect.poll(() => sent).toEqual({ path: '/nowhere/at/all' })
})

test('Enter in the Add trade boxes submits the trade', async ({ page }) => {
  await page.goto('/')
  let sent: Record<string, unknown> | null = null
  await page.route('**/api/book/append', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: false, error: 'not in this test' } }) })
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Add trade').click()
  await page.getByPlaceholder('e.g. LUNR or LUNR 15JAN27 12.00 CALL').fill('ZZZQ')
  const boxes = page.locator('.input[inputmode="decimal"]')
  await boxes.nth(0).fill('10')
  await boxes.nth(1).fill('2.5')
  await page.keyboard.press('Enter')
  await expect.poll(() => sent?.symbol ?? (sent as { activities?: { symbol: string }[] } | null)?.activities?.[0]?.symbol ?? null).not.toBeNull()
})
