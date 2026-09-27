import { expect, test } from '@playwright/test'
import { figures, ready } from './helpers'

// A change the server refuses is said where the app says a failure: the header's
// status line, in the server's own words, and what the page showed at once is put
// back as the server has it (docs/plans/stage-5-interface-and-running.md, B).

const refuse = { status: 500, json: { ok: false, error: 'store failed' } }

test.describe('a change the server refuses', () => {
  test('a watchlist add refused is said in the header, and the row it showed goes', async ({ page, request }) => {
    const before = (await figures(request)).markets.watchlist as { symbol: string }[]
    await page.route('**/api/symbols/search*', (route) =>
      route.fulfill({ json: { ok: true, matches: [{ symbol: 'ZQWM', exchange: 'NYSE', name: 'Zqwm Inc', currency: 'USD' }] } }),
    )
    let asked = 0
    await page.route('**/api/watchlist/add', (route) => {
      asked++
      return route.fulfill(refuse)
    })
    await page.goto('/#markets')
    await ready(page)
    await page.getByRole('button', { name: 'Add to the watchlist' }).click()
    await page.getByLabel('Search symbol', { exact: true }).fill('ZQWM')
    await page.locator('.wl-sug', { hasText: 'ZQWM' }).click()
    await expect(page.locator('#syncline .status-err')).toHaveText('Could not add ZQWM to the watchlist: store failed')
    expect(asked).toBe(1)
    await page.keyboard.press('Escape')
    await expect(page.locator('.wl-row', { hasText: 'ZQWM' })).toHaveCount(0)
    expect(((await figures(request)).markets.watchlist as { symbol: string }[]).map((w) => w.symbol)).toEqual(before.map((w) => w.symbol))
  })

  test('a watchlist remove refused is said in the header, and the row comes back', async ({ page, request }) => {
    // a listing watched for this test, and no longer after it
    const added = await (await request.post('/api/watchlist/add', { headers: { 'X-Bagholder': '1' }, data: { symbol: 'NVDA', exchange: 'NASDAQ', name: 'NVIDIA Corp.', currency: 'USD' } })).json()
    expect(added.ok, added.error).toBe(true)
    try {
      await page.route('**/api/watchlist/remove', (route) => route.fulfill(refuse))
      await page.goto('/#markets')
      await ready(page)
      const remove = page.getByRole('button', { name: 'Remove NVDA from the watchlist', exact: true })
      await page.locator('.wl-row.go').filter({ has: remove }).hover()
      await remove.click()
      await expect(page.locator('#syncline .status-err')).toHaveText('Could not remove NVDA from the watchlist: store failed')
      await expect(remove).toHaveCount(1)
    } finally {
      await request.post('/api/watchlist/remove', { headers: { 'X-Bagholder': '1' }, data: { id: added.id } })
    }
  })

  test('a tiles change refused is said in the header, and the row goes back to the server\'s', async ({ page, request }) => {
    const tiles = (await figures(request)).markets.tiles as { symbol: string; label: string }[]
    test.skip(!tiles.length, 'the made-up book has no tiles')
    await page.route('**/api/tiles/set', (route) => route.fulfill(refuse))
    await page.goto('/#markets')
    await ready(page)
    const remove = page.getByRole('button', { name: 'Remove ' + tiles[0].label, exact: true })
    await remove.click({ force: true }) // shown on hover
    await expect(page.locator('#syncline .status-err')).toHaveText('Could not save the tiles: store failed')
    await expect(remove).toHaveCount(1)
  })
})
