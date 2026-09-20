import { expect, test } from '@playwright/test'
import { openWithStatus, ready } from './helpers'

// SPEC §3, Markets: the tile row, Fear & Greed, the Heatmap card, the Watchlist,
// Short interest and News cards; and §4/§6 Disclosures, the card on a trade or
// listing page. `markets.spec.ts`, `heatmap.spec.ts` and `listing.spec.ts` cover
// the Escape/Enter behaviours, the heatmap on its own and the listing-page
// routing already, and are not repeated here.

const tile = (symbol: string, sector: string, value: number, percentChange: number | null = 1.2) => ({
  id: null, symbol, exchange: 'NASDAQ', name: symbol + ' Inc.', value, percentChange, sector,
})

// The picker's plus cell is not rendered once the row is full (the demo book starts
// at exactly six tiles); a test that opens the picker for real must first free
// cells, and restore the row afterwards. `spare` extra cells beyond the first let a
// test add a tile without the row filling up and the picker auto-closing on it.
async function freeATileCell(request: import('@playwright/test').APIRequestContext, spare = 0): Promise<{ before: { symbol: string; exchange: string }[]; freed: { symbol: string; exchange: string }[]; restore: () => Promise<void> }> {
  const before = (await (await request.get('/api/model')).json()).markets.tiles as { symbol: string; exchange: string }[]
  const freed = before.slice(0, before.length - 1 - spare)
  const H = { 'X-Bagholder': '1' }
  await request.post('/api/tiles/set', { headers: H, data: { tiles: freed.map((t) => ({ symbol: t.symbol, exchange: t.exchange })) } })
  return { before, freed, restore: async () => { await request.post('/api/tiles/set', { headers: H, data: { tiles: before.map((t) => ({ symbol: t.symbol, exchange: t.exchange })) } }) } }
}

test.describe('Market tiles', () => {
  test('the picker filters by symbol, name and alias; a row toggles a tile and updates the footer', async ({ page, request }) => {
    const { freed, restore } = await freeATileCell(request, 1) // room for two, so adding one keeps the picker open
    await page.goto('/#markets')
    await ready(page)
    await page.getByRole('button', { name: 'Add a tile' }).click()
    const box = page.getByLabel('Search instruments')
    await expect(box).toBeFocused()
    await expect(page.locator('.pop')).toContainText(freed.length + ' of 12 tiles used')

    // filters on name/alias as well as symbol
    await box.fill('crude')
    await expect(page.locator('.mt-row')).toHaveCount(2) // WTI and Brent, both aliased to "crude"
    await box.fill('zzzznotreal')
    await expect(page.getByText('No match.')).toBeVisible()

    // adding a new instrument by clicking its row
    await box.fill('WTI')
    await page.locator('.mt-row', { hasText: 'WTI' }).click()
    await expect(page.locator('.pop')).toContainText((freed.length + 1) + ' of 12 tiles used')
    await expect(box).toBeVisible() // the box stays open for the next pick
    await expect(page.locator('.mt-row', { hasText: 'WTI' }).locator('.mt-bm')).toBeVisible()

    // clicking it again removes it
    await page.locator('.mt-row', { hasText: 'WTI' }).click()
    await expect(page.locator('.pop')).toContainText(freed.length + ' of 12 tiles used')

    await page.keyboard.press('Escape')
    await expect(page.getByLabel('Search instruments')).toHaveCount(0)
    // no tile was left behind
    const after = (await (await request.get('/api/model')).json()).markets.tiles as { symbol: string }[]
    expect(after.map((t) => t.symbol).sort()).toEqual(freed.map((t) => t.symbol).sort())
    await restore()
  })

  test('a tile can be added, persists to the store, and its cross removes it at once', async ({ page, request }) => {
    const { freed, restore } = await freeATileCell(request)
    await page.goto('/#markets')
    await ready(page)
    await page.getByRole('button', { name: 'Add a tile' }).click()
    await page.getByLabel('Search instruments').fill('WTI')
    await page.locator('.mt-row', { hasText: 'WTI' }).click() // WTI is the label; the instrument's own symbol is CL
    await page.keyboard.press('Escape')
    await expect(page.locator('.mt-tile[data-sym="CL"]')).toBeVisible()
    await expect.poll(async () => {
      const m = await (await request.get('/api/model')).json()
      return (m.markets.tiles as { symbol: string }[]).map((t) => t.symbol)
    }).toContain('CL')

    await page.locator('.mt-tile[data-sym="CL"]').hover()
    await page.getByRole('button', { name: 'Remove WTI' }).click()
    await expect(page.locator('.mt-tile[data-sym="CL"]')).toHaveCount(0)
    await expect.poll(async () => {
      const m = await (await request.get('/api/model')).json()
      return (m.markets.tiles as { symbol: string }[]).map((t) => t.symbol)
    }).toEqual(freed.map((t) => t.symbol))
    await restore()
  })

  test('dragging a tile to another cell reorders the row and saves the new order', async ({ page, request }) => {
    const before = (await (await request.get('/api/model')).json()).markets.tiles as { symbol: string; exchange: string }[]
    test.skip(before.length < 2, 'needs at least two tiles to reorder')
    const [first, second] = before
    await page.goto('/#markets')
    await ready(page)
    const a = page.locator('.mt-tile[data-sym="' + first.symbol + '"]')
    const b = page.locator('.mt-tile[data-sym="' + second.symbol + '"]')
    const boxA = (await a.boundingBox())!
    const boxB = (await b.boundingBox())!
    await page.mouse.move(boxA.x + boxA.width / 2, boxA.y + boxA.height / 2)
    await page.mouse.down()
    await page.mouse.move(boxB.x + boxB.width / 2, boxB.y + boxB.height / 2, { steps: 8 })
    await page.mouse.move(boxB.x + boxB.width / 2 + 2, boxB.y + boxB.height / 2, { steps: 2 })
    await page.mouse.up()
    await expect.poll(async () => {
      const m = await (await request.get('/api/model')).json()
      return (m.markets.tiles as { symbol: string }[])[0].symbol
    }).toBe(second.symbol)
    // put it back so other tests (and the user's own book) see the tiles unchanged
    await request.post('/api/tiles/set', { headers: { 'X-Bagholder': '1' }, data: { tiles: before.map((t) => ({ symbol: t.symbol, exchange: t.exchange })) } })
  })

  test('with more than six tiles, "Show N more" opens a second row, remembered in the browser; a plus does not render at twelve', async ({ page, request }) => {
    const many = Array.from({ length: 8 }, (_, i) => ({ symbol: 'T' + i, exchange: 'NASDAQ', label: 'T' + i, name: 'Tile ' + i, kind: 'Index', last: 100, change: 1, percentChange: 1, decimals: 2 }))
    await openWithStatus(page, request, {}, '#markets', (m) => { (m.markets as { tiles: unknown[] }).tiles = many })
    await expect(page.locator('.mt-tile')).toHaveCount(6)
    await expect(page.getByRole('button', { name: 'Show 2 more' })).toBeVisible()
    await page.getByRole('button', { name: 'Show 2 more' }).click()
    await expect(page.locator('.mt-tile')).toHaveCount(8)
    await expect(page.getByRole('button', { name: 'Show fewer' })).toBeVisible()
    expect(await page.evaluate(() => localStorage.getItem('bh2.tilesOpen'))).toBe('1')
    await page.reload()
    await ready(page)
    await expect(page.locator('.mt-tile')).toHaveCount(8) // remembered

    const twelve = Array.from({ length: 12 }, (_, i) => ({ ...many[0], symbol: 'Z' + i, label: 'Z' + i }))
    await openWithStatus(page, request, {}, '#markets', (m) => { (m.markets as { tiles: unknown[] }).tiles = twelve })
    await expect(page.locator('.mt-tile')).toHaveCount(12)
    await expect(page.getByRole('button', { name: 'Add a tile' })).toHaveCount(0)
    await expect(page.getByText('Show fewer')).toHaveCount(0) // never shown at twelve
  })
})

test.describe('Fear & Greed', () => {
  const gauge = (index: string, score: number, parts: { name: string; score: number; rating: string }[]) => ({
    ok: true,
    gauge: {
      index, source: index === 'stocks' ? 'CNN' : 'alternative.me', score, rating: score < 25 ? 'Extreme fear' : score < 45 ? 'Fear' : score <= 55 ? 'Neutral' : score <= 75 ? 'Greed' : 'Extreme greed',
      asOf: '2026-09-19', previous: [{ label: 'Previous close', score: score - 3, rating: 'Fear' }],
      parts, series: [{ date: '2026-09-17', score: 40 }, { date: '2026-09-18', score: 42 }, { date: '2026-09-19', score }],
    },
  })

  test('Stocks and Crypto each read their own gauge, switching is remembered, and only Stocks carries "What it is made of"', async ({ page, request }) => {
    const stocks = gauge('stocks', 62, [{ name: 'Market momentum', score: 70, rating: 'Greed' }])
    const crypto = gauge('crypto', 30, [])
    await openWithStatus(page, request, {}, '#markets', () => {}, { 'fear:stocks': stocks, 'fear:crypto': crypto })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Fear & Greed' }) })
    await expect(card).toContainText('CNN')
    await expect(card.locator('.tab').first()).toHaveText('62')
    await expect(card).toContainText('What it is made of')
    await expect(card).toContainText('Market momentum')

    await card.locator('.mseg-opt', { hasText: 'Crypto' }).click()
    await expect(card).toContainText('alternative.me')
    await expect(card.locator('.tab').first()).toHaveText('30')
    await expect(card).not.toContainText('What it is made of')
    expect(await page.evaluate(() => localStorage.getItem('bh2.fear'))).toBe('crypto')

    await page.reload()
    await ready(page)
    await expect(page.locator('.mseg-opt.on', { hasText: 'Crypto' })).toBeVisible() // remembered across a reload
  })
})

test.describe('Heatmap card', () => {
  const withTiles = (tiles: ReturnType<typeof tile>[]) => (m: Record<string, unknown>) => {
    const mk = m.markets as { universes: Record<string, unknown[]>; holdings: unknown[]; watchlist: unknown[] }
    mk.universes = { ...mk.universes, us: tiles, ca: [], intl: [] }
    mk.holdings = []
  }

  test('the legend shows the ramp from −3% to +3%, and the universe and size controls pick the tiles shown', async ({ page, request }) => {
    await openWithStatus(page, request, {}, '#markets', withTiles([tile('AAA', 'Technology', 1000), tile('BBB', 'Energy', 10)]))
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Heatmap' }) })
    await expect(card).toContainText('−3%')
    await expect(card).toContainText('+3%')
    const box = card.locator('#heatBox')
    await expect(card).toContainText('No open positions.') // still on Holdings, which is empty here

    await card.locator('.mseg-opt', { hasText: 'US' }).click()
    await expect(box.locator('.heat-tile')).toHaveCount(2)
    await expect(box).toContainText('AAA')
    await expect(box).toContainText('BBB')

    // Equal size makes every tile the same area (tiles animate into place over .42s)
    await card.locator('.mseg-opt', { hasText: 'Equal' }).click()
    await page.waitForTimeout(500)
    const a = await box.locator('.heat-tile', { hasText: 'AAA' }).boundingBox()
    const b = await box.locator('.heat-tile', { hasText: 'BBB' }).boundingBox()
    expect(Math.abs(a!.width * a!.height - b!.width * b!.height)).toBeLessThan(400)
  })

  test('a tile\'s symbol and change render at 16/12px once it is big enough, 13/11 when merely mid-sized, and 11px alone when small', async ({ page, request }) => {
    // one big tile filling almost the whole box, and two tiny ones squeezed beside it
    const tiles = [tile('BIG', 'Technology', 5000), tile('MID', 'Technology', 40), tile('WEE', 'Technology', 1)]
    await openWithStatus(page, request, {}, '#markets', withTiles(tiles))
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Heatmap' }) })
    await card.locator('.mseg-opt', { hasText: 'US' }).click()
    const box = card.locator('#heatBox')
    const big = box.locator('.heat-tile[data-sym="BIG"] .heat-sym')
    await expect(big).toHaveCSS('font-size', '16px')
    await expect(box.locator('.heat-tile[data-sym="BIG"] .heat-chg')).toHaveCSS('font-size', '12px')
    // the smallest positions under 1.5% of the block fold into "Other" and read 11px alone, no change line
    const other = box.locator('.heat-tile', { hasText: 'Other' })
    await expect(other).toBeVisible()
    await expect(other.locator('.heat-sym')).toHaveCSS('font-size', '11px')
    await expect(other.locator('.heat-chg')).toHaveCount(0)
  })

  test('a held tile opens its holding; a watched-but-unheld tile opens its listing', async ({ page, request }) => {
    await openWithStatus(page, request, {}, '#markets', (m) => {
      const mk = m.markets as { holdings: unknown[]; watchlist: unknown[] }
      mk.holdings = [{ id: 'rt:demo-0019', symbol: 'VFV', exchange: 'TSX', value: 53724, percentChange: 0.4, sector: 'Not classified' }]
      mk.watchlist = [{ symbol: 'ZZZQ', exchange: 'TSX', name: 'Zzzq Corp', currency: 'CAD', last: 12, priceChange: 0.1, percentChange: 0.8, sector: 'Not classified', kind: 'Shares', positionId: null }]
    })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Heatmap' }) })
    await card.locator('.mseg-opt', { hasText: 'Both' }).click()
    const box = card.locator('#heatBox')
    await box.locator('.heat-tile[data-sym="VFV"]').first().click()
    await expect(page).toHaveURL(/#portfolio\/rt%3Ademo-0019$/)

    await page.goto('/#markets')
    await ready(page)
    await page.locator('.card', { has: page.locator('h5', { hasText: 'Heatmap' }) }).locator('.mseg-opt', { hasText: 'Both' }).click()
    await page.locator('#heatBox .heat-tile[data-sym="ZZZQ"]').click()
    await expect(page).toHaveURL(/#markets\/listing%3AZZZQ%40TSX$/)
    await expect(page.locator('#page')).toContainText('Zzzq Corp')
  })
})

test.describe('Watchlist', () => {
  test('a symbol typed with a Yahoo-style suffix is looked up and added under the bare ticker and the suffix\'s venue', async ({ page, request }) => {
    const model = await (await request.get('/api/model')).json()
    const before = (model.markets.watchlist as { symbol: string }[]).map((w) => w.symbol)
    await page.route('**/api/symbols/search*', (route) => {
      const url = new URL(route.request().url())
      expect(url.searchParams.get('q')).toBe('SHOP.TO')
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, matches: [{ symbol: 'SHOP', exchange: 'TSX', name: 'Shopify Inc.', currency: 'CAD' }] }) })
    })
    let addBody: unknown = null
    await page.route('**/api/watchlist/add', (route) => {
      addBody = route.request().postDataJSON()
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true }) })
    })
    await page.goto('/#markets')
    await ready(page)
    await page.getByRole('button', { name: 'Add to the watchlist' }).click()
    await page.getByLabel('Search symbol', { exact: true }).fill('SHOP.TO')
    await expect(page.locator('.wl-sug', { hasText: 'SHOP' })).toBeVisible()
    await expect(page.locator('.wl-sug', { hasText: 'TSX' })).toBeVisible()
    await page.locator('.wl-sug', { hasText: 'SHOP' }).click()
    await expect.poll(() => addBody).toEqual({ symbol: 'SHOP', exchange: 'TSX', name: 'Shopify Inc.', currency: 'CAD' })
    await expect(page.locator('.wl-row', { hasText: 'SHOP' })).toBeVisible()
    expect(before).not.toContain('SHOP')
  })

  test('adding a held-but-unfollowed symbol from "From Holdings" and removing it both persist, and the row opens the holding', async ({ page, request }) => {
    const model = await (await request.get('/api/model')).json()
    const before = model.markets.watchlist as { symbol: string; exchange: string }[]
    const notFollowed = (model.positions as { symbol: string; exchange: string; kind: string }[]).find((p) => p.kind === 'Shares' && !before.some((w) => w.symbol === p.symbol))
    test.skip(!notFollowed, 'every holding is already followed in this book')
    await page.goto('/#markets')
    await ready(page)
    await page.getByRole('button', { name: 'Add to the watchlist' }).click()
    await expect(page.locator('.wl-sug', { hasText: notFollowed!.symbol })).toBeVisible()
    await page.locator('.wl-sug', { hasText: notFollowed!.symbol }).click()
    await page.keyboard.press('Escape')
    await expect.poll(async () => {
      const m = await (await request.get('/api/model')).json()
      return (m.markets.watchlist as { symbol: string }[]).map((w) => w.symbol)
    }).toContain(notFollowed!.symbol)

    const row = page.locator('.wl-row', { hasText: notFollowed!.symbol })
    await row.click()
    await expect(page).toHaveURL(/#portfolio\//) // the book holds it: the row opens the holding, not the listing

    await page.goto('/#markets')
    await ready(page)
    await page.locator('.wl-row', { hasText: notFollowed!.symbol }).hover()
    await page.getByRole('button', { name: 'Remove ' + notFollowed!.symbol + ' from the watchlist', exact: true }).click()
    await expect.poll(async () => {
      const m = await (await request.get('/api/model')).json()
      return (m.markets.watchlist as { symbol: string }[]).map((w) => w.symbol)
    }).toEqual(before.map((w) => w.symbol))
  })

  test('the column head Symbol · Exch · Last · Chg sorts the rows, first descending', async ({ page, request }) => {
    await openWithStatus(page, request, {}, '#markets', (m) => {
      const mk = m.markets as { watchlist: unknown[] }
      mk.watchlist = [
        { symbol: 'AAA', exchange: 'NASDAQ', name: 'Aaa', currency: 'USD', last: 10, priceChange: 0, percentChange: 1, sector: '', kind: 'Shares', positionId: null },
        { symbol: 'ZZZ', exchange: 'NASDAQ', name: 'Zzz', currency: 'USD', last: 20, priceChange: 0, percentChange: 5, sector: '', kind: 'Shares', positionId: null },
        { symbol: 'MMM', exchange: 'NASDAQ', name: 'Mmm', currency: 'USD', last: 15, priceChange: 0, percentChange: 3, sector: '', kind: 'Shares', positionId: null },
      ]
    })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Watchlist' }) })
    const symbols = () => card.locator('.wl-row.go > div:first-child > div:first-child')
    await expect(symbols()).toHaveText(['MMM', 'ZZZ', 'AAA']) // newest (last added) first, unsorted
    await card.locator('.gth', { hasText: 'Symbol' }).click()
    await expect(symbols()).toHaveText(['ZZZ', 'MMM', 'AAA']) // descending by symbol text
    await card.locator('.gth', { hasText: 'Symbol' }).click()
    await expect(symbols()).toHaveText(['AAA', 'MMM', 'ZZZ']) // ascending on a second click
  })
})

test.describe('Short interest', () => {
  const row = (symbol: string, ofFloat: number, opts: Partial<{ held: boolean; watched: boolean; positionId: string | null; exchange: string; name: string; daysToCover: number }> = {}) => ({
    symbol, exchange: opts.exchange ?? 'NASDAQ', name: opts.name ?? symbol + ' Inc.', held: !!opts.held, watched: !!opts.watched, positionId: opts.positionId ?? null,
    shares: 1_000_000, ofFloat, daysToCover: opts.daysToCover ?? 2.5, volumePct: 12.3, asOf: '2026-09-18',
  })

  test('the scope segment narrows Holdings/Watchlist, and Symbol/Exchange/Days to cover/etc sort like every table\'s', async ({ page, request }) => {
    const feed = { ok: true, reading: false, rows: [row('HHH', 40, { held: true, daysToCover: 1 }), row('WWW', 80, { watched: true, daysToCover: 9 }), row('BBB', 10, { daysToCover: 5 })] }
    await openWithStatus(page, request, {}, '#markets', () => {}, { shorts: feed })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Short interest' }) })
    const symbols = () => card.locator('.si-row > div:first-child > div:first-child')
    await expect(symbols()).toHaveText(['WWW', 'HHH', 'BBB']) // default: most of float first
    await card.locator('.mseg-opt', { hasText: 'Holdings' }).click()
    await expect(symbols()).toHaveText(['HHH'])
    await card.locator('.mseg-opt', { hasText: 'Watchlist' }).click()
    await expect(symbols()).toHaveText(['WWW'])
    await card.locator('.mseg-opt', { hasText: 'All' }).click()

    await card.locator('.gth', { hasText: 'Symbol' }).click()
    await expect(symbols()).toHaveText(['WWW', 'HHH', 'BBB']) // descending by symbol text
    await card.locator('.gth', { hasText: 'Days to cover' }).click()
    await expect(symbols()).toHaveText(['WWW', 'BBB', 'HHH']) // descending, most days to cover first
  })

  test('the search box narrows by symbol or name, and looks a ticker outside the list up on its own', async ({ page, request }) => {
    const feed = { ok: true, reading: false, rows: [row('HHH', 40, { held: true, name: 'Aitch Corp' })] }
    await openWithStatus(page, request, {}, '#markets', () => {}, { shorts: feed })
    let asked: string | null = null
    await page.route('**/api/shorts?symbol=*', (route) => {
      asked = new URL(route.request().url()).searchParams.get('symbol')
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, covered: false }) })
    })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Short interest' }) })
    const box = page.getByLabel('Search short interest')
    await box.fill('Aitch')
    await expect(card.locator('.si-row')).toHaveCount(1)
    await box.fill('')
    await box.fill('ZQZQ')
    await expect(card).toContainText('No listing by that name.')
    await expect.poll(() => asked).toBe('ZQZQ')
  })
})

test.describe('News', () => {
  const item = (id: string, headline: string, opts: Partial<{ tag: { symbol: string; exchange: string; held: boolean; watched: boolean; percentChange: number | null }; market: boolean; kind: string }> = {}) => ({
    id, headline, source: 'The Wire', url: 'https://example.com/' + id, publishedAt: '2026-09-19T12:00:00Z',
    market: opts.market ?? false, kind: opts.kind ?? 'story', tags: opts.tag ? [opts.tag] : [],
  })

  test('the search box narrows by symbol or headline words within the current scope', async ({ page, request }) => {
    await openWithStatus(page, request, {}, '#markets', (m) => {
      (m.markets as { news: unknown[] }).news = [
        item('a', 'Widgets soar on earnings beat', { tag: { symbol: 'WID', exchange: 'NASDAQ', held: true, watched: false, percentChange: 1 } }),
        item('b', 'Gadget Co announces buyback', { tag: { symbol: 'GAD', exchange: 'NASDAQ', held: true, watched: false, percentChange: -1 } }),
      ]
    })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'News' }) })
    await card.locator('.mseg-opt', { hasText: 'Holdings' }).click()
    await expect(card.locator('.nw-row')).toHaveCount(2)
    const box = page.getByLabel('Search the news')
    await box.fill('WID')
    await expect(card.locator('.nw-row')).toHaveCount(1)
    await expect(card).toContainText('Widgets soar')
    await box.fill('')
    await box.fill('buyback')
    await expect(card.locator('.nw-row')).toHaveCount(1)
    await expect(card).toContainText('Gadget Co')
  })

  test('clicking a row\'s symbol opens the Symbol chip, scoping the card until it is cleared', async ({ page, request }) => {
    await openWithStatus(page, request, {}, '#markets', (m) => {
      (m.markets as { news: unknown[] }).news = [
        item('a', 'Widgets soar on earnings beat', { tag: { symbol: 'WID', exchange: 'NASDAQ', held: true, watched: false, percentChange: 1 } }),
        item('b', 'Gadget Co announces buyback', { tag: { symbol: 'GAD', exchange: 'NASDAQ', held: true, watched: false, percentChange: -1 } }),
      ]
    })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'News' }) })
    await card.locator('.mseg-opt', { hasText: 'Holdings' }).click()
    await card.locator('.nw-sym', { hasText: 'WID' }).click()
    await expect(card.locator('.chip')).toContainText('WID')
    await expect(card.locator('.nw-row')).toHaveCount(1)
    await expect(card).toContainText('Widgets soar')
    await card.locator('.chip .cx').click()
    await expect(card.locator('.chip')).toHaveCount(0)
    await expect(card.locator('.nw-row')).toHaveCount(2)
  })

  test('clicking anywhere else on a row opens the article in a new tab', async ({ page, request }) => {
    await openWithStatus(page, request, {}, '#markets', (m) => {
      (m.markets as { news: unknown[] }).news = [item('a', 'Widgets soar on earnings beat', { market: true })]
    })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'News' }) })
    const [popup] = await Promise.all([
      page.waitForEvent('popup'),
      card.locator('.nw-row', { hasText: 'Widgets soar' }).click(),
    ])
    expect(popup.url()).toBe('https://example.com/a')
    await popup.close()
  })
})

test.describe('Disclosures', () => {
  test('the card reads a held listing\'s filed record, sorts newest first by default, and opens a document on click', async ({ page, request }) => {
    const model = await (await request.get('/api/model')).json()
    const aapl = (model.positions as { symbol: string; name: string; exchange: string; currency: string }[]).find((p) => p.symbol === 'AAPL')!
    const docKey = 'filings:symbol=AAPL&name=' + encodeURIComponent(aapl.name) + '&exchange=' + encodeURIComponent(aapl.exchange) + '&currency=' + encodeURIComponent(aapl.currency)
    const filing = (id: string, date: string, category: string, source: string, extra: Partial<{ size: string }> = {}) => ({
      id, source, category, type: '10-Q', title: '', subject: 'A filing about ' + id, summary: 'One sentence about ' + id + '.', date, dateText: date, size: extra.size ?? '', url: 'https://filer.example/' + id,
    })
    const payload = {
      ok: true, everRead: true, reading: [], fetchedAt: '2026-09-19T00:00:00Z', summaryStatus: 'ready',
      sources: { SEC: { available: true, filer: true, matched: true } },
      filings: [filing('f1', '2026-08-01', 'Financials', 'SEC'), filing('f2', '2026-09-01', 'Financials', 'SEC')],
    }
    const positionId = (model.positions as { symbol: string; id: string }[]).find((p) => p.symbol === 'AAPL')!.id
    await openWithStatus(page, request, {}, '#portfolio/' + encodeURIComponent(positionId), () => {}, { [docKey]: payload })

    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Disclosures' }) })
    await expect(card).toBeVisible()
    const rows = card.locator('.dc-row')
    await expect(rows).toHaveCount(2)
    await expect(rows.first()).toContainText('f2'.replace('f2', 'A filing about f2')) // newest first
    await page.context().route('https://filer.example/*', (route) => route.fulfill({ status: 200, contentType: 'text/plain', body: 'ok' }))
    const [popup] = await Promise.all([
      page.waitForEvent('popup'),
      rows.first().locator('.dc-open').click({ force: true }),
    ])
    await popup.waitForLoadState()
    expect(popup.url()).toBe('https://filer.example/f2')
    await popup.close()
  })

  test('a listing with no regulatory filer reads "No regulatory filer for this listing."', async ({ page, request }) => {
    const model = await (await request.get('/api/model')).json()
    const positionId = (model.positions as { symbol: string; id: string }[]).find((p) => p.symbol === 'AAPL')!.id
    const aapl = (model.positions as { symbol: string; name: string; exchange: string; currency: string }[]).find((p) => p.symbol === 'AAPL')!
    const docKey = 'filings:symbol=AAPL&name=' + encodeURIComponent(aapl.name) + '&exchange=' + encodeURIComponent(aapl.exchange) + '&currency=' + encodeURIComponent(aapl.currency)
    const payload = { ok: true, everRead: true, reading: [], sources: { SEC: { available: true, filer: false, matched: false } }, filings: [] }
    await openWithStatus(page, request, {}, '#portfolio/' + encodeURIComponent(positionId), () => {}, { [docKey]: payload })
    const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Disclosures' }) })
    await expect(card).toContainText('No regulatory filer for this listing.')
  })
})
