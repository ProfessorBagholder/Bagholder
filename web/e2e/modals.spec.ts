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
