import { expect, test, type Page } from '@playwright/test'
import { openWithStatus, ready, saidShown } from './helpers'

// SPEC §3 Markets, "The heatmap on its own" and "The slideshow".

// The heatmap document as the server sends it: its sector blocks, each tile with its key,
// and how many tiles each universe has.
type Tile = { key: string; id: string | null; symbol: string; exchange: string; currency: string; name: string; value: string; percentChange: number | null; other: boolean }
const tile = (symbol: string, value = '100', other = false, key = symbol): Tile => ({ key, id: null, symbol, exchange: other ? '' : 'NYSE', currency: other ? '' : 'USD', name: other ? '' : symbol, value, percentChange: 0.012, other })
const block = (label: string, tiles: Tile[], value = '200') => ({ label, value, percentChange: 0.012, tiles })
const counts = (c: Partial<Record<'holdings' | 'watchlist' | 'both' | 'ca' | 'us' | 'intl', number>> = {}) => ({ holdings: 0, watchlist: 0, both: 0, ca: 0, us: 0, intl: 0, ...c })
const heatmapDoc = (universe: string, size: string, blocks: ReturnType<typeof block>[], n = counts()) => ({ universe, size, blocks, counts: n })
// the US universe read, Canada's and the international one not; the book holds something
const usRead = { heatmap: heatmapDoc('us', 'value', [block('Technology', [tile('AAA')]), block('Energy', [tile('BBB')])], counts({ holdings: 3, both: 3, us: 2 })) }
const nothingRead = { heatmap: heatmapDoc('us', 'value', [], counts({ holdings: 3, both: 3 })) }

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
  await openWithStatus(page, request, {}, '#heatmap/us/equal', () => {}, usRead)
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
  await openWithStatus(page, request, {}, '#heatmap/holdings,ca,us/value/20', () => {}, usRead)
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
  await openWithStatus(page, request, {}, '#heatmap/holdings/value', () => {}, usRead)
  await page.getByRole('button', { name: 'Cycle through the scopes' }).click()
  await expect(page).toHaveURL(/#heatmap\/holdings,watchlist,both,ca,us,intl\/value\/20$/)
  await page.getByRole('button', { name: 'Stop cycling' }).click()
  await expect(page).toHaveURL(/#heatmap\/holdings\/value$/)
})

/** The keys of what the page last told the server it shows. */
function watched(page: Page): { last: () => string[] } {
  const said = saidShown(page)
  return { last: () => Object.keys(said.at(-1) ?? {}) }
}
const universesIn = (docs: string[] | undefined) => (docs ?? []).filter((k) => k.startsWith('universe:')).sort()

// SPEC §4 Markets, Heatmap: a market universe on show is read by the server when it has
// no rows or they are stale, whichever way it came on show.
for (const u of ['ca', 'us', 'intl']) {
  test(`a market universe addressed with no rows is asked for from the address alone (${u})`, async ({ page, request }) => {
    const said = watched(page)
    await openWithStatus(page, request, {}, `#heatmap/${u}/value`, () => {}, nothingRead)
    await expect(page.locator('#heatFull')).toContainText('Not read yet.')
    await expect.poll(() => universesIn(said.last())).toEqual([`universe:${u}`])
  })

  test(`a market universe remembered with no rows is asked for when Markets opens (${u})`, async ({ page, request }) => {
    const said = watched(page)
    await page.addInitScript((universe) => localStorage.setItem('bh2.heatmap', JSON.stringify({ universe, size: 'value' })), u)
    await openWithStatus(page, request, {}, '#markets', () => {}, nothingRead)
    const card = page.locator('#page .card', { has: page.locator('h5', { hasText: 'Heatmap' }) })
    await expect(card).toContainText('Not read yet.')
    await expect.poll(() => universesIn(said.last())).toEqual([`universe:${u}`])
    // the book's own scopes are nobody's to read: leaving the market stops asking
    await card.locator('.mseg-opt', { hasText: /^Holdings$/ }).click()
    await expect.poll(() => universesIn(said.last())).toEqual([])
  })
}

test('a slideshow asks for every market it goes through, not only the one on show', async ({ page, request }) => {
  const said = watched(page)
  await openWithStatus(page, request, {}, '#heatmap/holdings,ca,intl/value/20', () => {}, usRead)
  await expect.poll(() => universesIn(said.last())).toEqual(['universe:ca', 'universe:intl'])
})

test("a market universe whose read failed says what failed instead of 'Not read yet.'", async ({ page, request }) => {
  await openWithStatus(page, request, {}, '#heatmap/ca/value', () => {}, { ...nothingRead, 'universe:ca': { failed: 'TMX Money could not be reached.' } })
  await expect(page.locator('#heatFull')).toContainText('TMX Money could not be reached.')
  await expect(page.locator('#heatFull')).not.toContainText('Not read yet.')
})

test('two sectors that each fold their small symbols to `Other (2)` both draw, each under its own key, and nothing breaks', async ({ page, request }) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  const sector = (name: string, p: string) => block(name, [tile(p + 'BIG', '1000'), tile('Other (2)', '2', true, 'other|' + name)], '1002')
  await openWithStatus(page, request, {}, '#heatmap/us/value', () => {}, { heatmap: heatmapDoc('us', 'value', [sector('Technology', 'T'), sector('Energy', 'E')], counts({ us: 6 })) })
  await expect(page.locator('#heatBox .heat-tile', { hasText: 'Other (2)' })).toHaveCount(2)
  await expect(page.locator('#heatBox .heat-tile')).toHaveCount(4)
  await expect(page.locator('#heatBox .heat-blk')).toHaveCount(2)
  // a folded tile opens nothing
  await page.locator('#heatBox .heat-tile', { hasText: 'Other (2)' }).first().click()
  await expect(page).toHaveURL(/#heatmap\/us\/value$/)
  expect(errors).toEqual([])
})
