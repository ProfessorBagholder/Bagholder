import { expect, test } from '@playwright/test'
import { ready, figures } from './helpers'

// SPEC §3, navigation: addresses, where a page opens and where Back returns to.

test('an old address for the holdings still opens them', async ({ page }) => {
  await page.goto('/#positions')
  await expect(page.locator('.tabbtn.on')).toHaveText('Portfolio')
})

test('a trade opens at its top, and Back returns to the list where it was scrolled to', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 500 })
  await page.goto('/#trades')
  await ready(page)
  await page.evaluate(() => window.scrollTo(0, 300))
  const y = await page.evaluate(() => window.scrollY)
  expect(y).toBeGreaterThan(100)
  await page.locator('#page').getByText(/^\d{4}-\d{2}-\d{2}$/).nth(12).click()
  await expect(page).toHaveURL(/#trades\/.+/)
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0)
  await page.goBack()
  await expect(page).toHaveURL(/#trades$/)
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(y)
})

// A list that scrolls inside its own card is drawn again when Back returns to it; it
// comes back where it stood, each list its own, and the address carries none of it.
for (const [tab, card] of [['trades', 'Trades'], ['portfolio', 'Holdings']] as const) {
  test(`Back returns the ${card} list to where it was scrolled inside its card`, async ({ page }) => {
    await page.goto('/#' + tab)
    await ready(page)
    const list = page.locator('#page .card').filter({ has: page.locator('h5', { hasText: new RegExp('^' + card + '$') }) }).locator('.scroll-xy')
    const room = await list.evaluate((el) => el.scrollHeight - el.clientHeight)
    expect(room, 'the list must scroll inside its card for this to be seen').toBeGreaterThan(0)
    const top = Math.max(1, Math.round(room * 0.6))
    await list.evaluate((el, y) => (el.scrollTop = y), top)
    // a row the list shows where it stands now
    const row = await list.evaluate((el) => {
      const box = el.getBoundingClientRect()
      const rows = [...el.querySelectorAll('tbody tr')]
      return rows.findIndex((r) => r.getBoundingClientRect().top > box.top + 40 && r.getBoundingClientRect().bottom < box.bottom)
    })
    expect(row).toBeGreaterThanOrEqual(0)
    const url = page.url()
    await list.locator('tbody tr').nth(row).click()
    await expect(page).not.toHaveURL(url)
    await page.goBack()
    await expect(page).toHaveURL(url)
    await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(top)
  })
}

test('a filter set while a trade is open returns to the list, and Back does not reopen the trade', async ({ page, request }) => {
  // the chip names the account; the filter holds its id
  const account = ((await figures(request)).options.accounts[0] as { id: string; name: string }).name
  await page.goto('/#dashboard')
  await page.goto('/#trades')
  // a closed trade: an open one's row opens its holding instead
  await page.locator('#page table tbody tr').filter({ hasNot: page.locator('td:nth-child(2)', { hasText: /^Open$/ }) }).first().locator('td').first().click()
  await expect(page).toHaveURL(/#trades\/.+/)
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type(account)
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/#trades$/)
  await expect(page.locator('.chip')).toContainText(account)
  await page.keyboard.press('Escape')
  // the open trade's entry became the list's: Back is the list before it, never the trade again
  await page.goBack()
  await expect(page).toHaveURL(/#trades$/)
})

test('Left and Right change tabs only with nothing open', async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  const tab = page.locator('.tabbtn.on')
  await page.keyboard.press('ArrowRight')
  await expect(tab).toHaveText('Portfolio')
  await page.keyboard.press('ArrowLeft')
  await expect(tab).toHaveText('Trades')
  const blur = () => page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  // the Notifications panel
  await page.getByRole('button', { name: 'Notifications' }).click()
  await blur()
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('ArrowLeft')
  await expect(tab).toHaveText('Trades')
  await expect(page).toHaveURL(/#trades$/)
  await page.keyboard.press('Escape')
  // the order ticket
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await blur()
  await page.keyboard.press('ArrowRight')
  await expect(tab).toHaveText('Trades')
  await expect(page).toHaveURL(/#trades$/)
})
