import { expect, test } from '@playwright/test'
import { openWithStatus, ready } from './helpers'

// SPEC §3 Markets, "The heatmap on its own" and "The slideshow".

const tile = (symbol: string, sector: string) => ({ id: null, symbol, exchange: 'NYSE', name: symbol, value: 100, percentChange: 1.2, sector })
const withUs = (m: Record<string, unknown>) => {
  const mk = m.markets as { universes: Record<string, unknown[]> }
  mk.universes = { ...mk.universes, us: [tile('AAA', 'Technology'), tile('BBB', 'Energy')], ca: [], intl: [] }
}

test('the card opens it at an address that says what it shows; × and Esc return to Markets', async ({ page }) => {
  await page.goto('/#markets')
  await ready(page)
  await page.getByRole('button', { name: 'Heatmap on its own' }).click()
  await expect(page).toHaveURL(/#heatmap\/holdings\/value$/)
  // the window is the heatmap's: no header, no tabs, no frame
  await expect(page.locator('#heatFull')).toBeVisible()
  await expect(page.locator('#hdr')).toHaveCount(0)
  await expect(page.locator('.tabbtn')).toHaveCount(0)
  expect(await page.evaluate(() => getComputedStyle(document.getElementById('app')!).borderRadius)).toBe('0px')
  const box = await page.locator('#heatFull').boundingBox()
  expect(box!.height).toBe(page.viewportSize()!.height)
  // the arrows move no tab beneath it
  await page.keyboard.press('ArrowLeft')
  await expect(page).toHaveURL(/#heatmap\//)
  await page.keyboard.press('Escape')
  await expect(page).toHaveURL(/#markets$/)
  await expect(page.locator('#hdr')).toBeVisible()
  expect(await page.evaluate(() => getComputedStyle(document.getElementById('app')!).borderRadius)).not.toBe('0px')
  await page.goBack()
  await page.getByRole('button', { name: 'Back to Markets' }).click()
  await expect(page).toHaveURL(/#markets$/)
})

test('an address is read into the view and remembered; a control rewrites it without a history entry', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#heatmap/us/equal', withUs)
  await expect(page.locator('#heatFull .mseg-opt.on', { hasText: 'US' })).toBeVisible()
  await expect(page.locator('#heatFull .mseg-opt.on', { hasText: 'Equal' })).toBeVisible()
  await expect(page.locator('#heatFull')).toContainText('AAA')
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('bh2.heatmap')!))).toEqual({ universe: 'us', size: 'equal' })
  const entries = await page.evaluate(() => history.length)
  await page.locator('#heatFull .mseg-opt', { hasText: 'Market value' }).click()
  await expect(page).toHaveURL(/#heatmap\/us\/value$/)
  await page.locator('#heatFull .mseg-opt', { hasText: 'Holdings' }).click()
  await expect(page).toHaveURL(/#heatmap\/holdings\/value$/)
  expect(await page.evaluate(() => history.length)).toBe(entries)
  // the card on Markets is the same choice
  await page.keyboard.press('Escape')
  await expect(page.locator('#page .mseg-opt.on', { hasText: 'Holdings' })).toBeVisible()
})

test('a list of scopes with a dwell cycles through those with something to show; a scope picked by hand stops it', async ({ page, request }) => {
  await page.clock.install()
  await openWithStatus(page, request, {}, '#heatmap/holdings,ca,us/value/20', withUs)
  const lit = page.locator('#heatFull .mseg-opt.on').first()
  await expect(lit).toHaveText('Holdings')
  await expect(page.getByRole('button', { name: 'Stop cycling' })).toBeVisible()
  await page.clock.fastForward(19_000)
  await expect(lit).toHaveText('Holdings')
  await page.clock.fastForward(1_500)
  await expect(lit).toHaveText('US') // Canada has nothing to show and is passed over
  await page.clock.fastForward(20_000)
  await expect(lit).toHaveText('Holdings')
  await page.locator('#heatFull .mseg-opt', { hasText: 'Watchlist' }).click()
  await expect(page.getByRole('button', { name: 'Cycle through the scopes' })).toBeVisible()
  await expect(page).toHaveURL(/#heatmap\/watchlist\/value$/)
  await page.clock.fastForward(60_000)
  await expect(lit).toHaveText('Watchlist')
})

test('the play button starts a cycle over every scope at twenty seconds and writes it in the address; pressed again, it stops', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#heatmap/holdings/value', withUs)
  await page.getByRole('button', { name: 'Cycle through the scopes' }).click()
  await expect(page).toHaveURL(/#heatmap\/holdings,watchlist,both,ca,us,intl\/value\/20$/)
  await page.getByRole('button', { name: 'Stop cycling' }).click()
  await expect(page).toHaveURL(/#heatmap\/holdings\/value$/)
})

test('two sectors that each fold their small symbols to `Other (2)` both draw, and nothing breaks', async ({ page, request }) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  const sector = (name: string, p: string) => [
    { ...tile(p + 'BIG', name), value: 1000 },
    { ...tile(p + 'S1', name), value: 1 },
    { ...tile(p + 'S2', name), value: 1 },
  ]
  await openWithStatus(page, request, {}, '#heatmap/us/value', (m) => {
    const mk = m.markets as { universes: Record<string, unknown[]> }
    mk.universes = { ...mk.universes, us: [...sector('Technology', 'T'), ...sector('Energy', 'E')] }
  })
  await expect(page.locator('#heatBox .heat-tile', { hasText: 'Other (2)' })).toHaveCount(2)
  await expect(page.locator('#heatBox .heat-tile')).toHaveCount(4)
  // and re-tiling with them on screen still works
  await page.locator('#heatFull .mseg-opt', { hasText: 'Equal' }).click()
  await expect(page.locator('#heatBox .heat-tile')).toHaveCount(6)
  await page.locator('#heatFull .mseg-opt', { hasText: 'Market value' }).click()
  await expect(page.locator('#heatBox .heat-tile', { hasText: 'Other (2)' })).toHaveCount(2)
  expect(errors).toEqual([])
})
