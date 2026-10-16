import { expect, test } from '@playwright/test'
import { figures, subUrl } from './helpers'

// Plan market-data-keyed-by-listing, on the made-up book: what is kept of a listing is
// its own. The book holds TD on the TSX and on the NYSE, which the TSX's interlisted
// list pairs; the market cache holds one made-up ticker, ZQX, on the TSX Venture and on
// Nasdaq, two companies, each with its own filer (server/src/demo_facts.rs).

type Held = { id: string; symbol: string; exchange: string; currency: string; kind: string }

const disclosures = (page: import('@playwright/test').Page) => page.locator('.card', { has: page.locator('h5', { hasText: 'Disclosures' }) })

test("one ticker on two venues: each holding shows its own listing's short interest and chart", async ({ page, request }) => {
  const model = await figures(request)
  const tds = (model.positions as Held[]).filter((p) => p.symbol === 'TD' && p.kind === 'Shares')
  expect(tds.map((p) => p.exchange).sort()).toEqual(['NYSE', 'TSX'])
  const seen = new Map<string, { cover: number; last: number }>()
  for (const p of tds) {
    const read = await request.get('/api/shorts?' + new URLSearchParams({ symbol: 'TD', exchange: p.exchange, currency: p.currency, name: '', trend: 'false' }))
    const shorts = (await read.json()).shorts
    const history = page.waitForResponse((r) => r.url().includes('/api/history') && new URL(r.url()).searchParams.get('exchange') === p.exchange)
    await page.goto('/#portfolio/' + encodeURIComponent(p.id))
    const bars = (await (await history).json()).bars as { close: number }[]
    await expect(page.locator('#page')).toContainText(shorts.daysToCover.toFixed(1) + ' days to cover')
    seen.set(p.exchange, { cover: shorts.daysToCover, last: bars[bars.length - 1].close })
  }
  expect(seen.get('TSX')!.cover).not.toBe(seen.get('NYSE')!.cover)
  expect(seen.get('TSX')!.last).not.toBe(seen.get('NYSE')!.last)
})

test("each listing lists its own filer's filings and its interlisted partner's, never another company's of its ticker", async ({ page, request }) => {
  // the made-up filings are dated by filer: TD's on the TSX the 10th, on the NYSE the 11th,
  // ZQX's on the TSX Venture the 12th, on Nasdaq the 13th (of August and of July)
  for (const [address, own, never] of [
    ['ZQX@XTSX', '12 Aug 2026', '13 Aug 2026'],
    ['ZQX@US', '13 Aug 2026', '12 Aug 2026'],
  ]) {
    await page.goto('/#markets/' + encodeURIComponent('listing:' + address))
    const card = disclosures(page)
    await expect(card.locator('.dc-row')).toHaveCount(2)
    await expect(card).toContainText(own)
    await expect(card).not.toContainText(never)
  }
  const model = await figures(request)
  for (const p of (model.positions as Held[]).filter((p) => p.symbol === 'TD' && p.kind === 'Shares')) {
    await page.goto('/#portfolio/' + encodeURIComponent(p.id))
    const card = disclosures(page)
    // its own and its partner's: both regulators, both filers
    await expect(card.locator('.dc-row')).toHaveCount(4)
    await expect(card).toContainText('10 Aug 2026')
    await expect(card).toContainText('11 Aug 2026')
    await expect(card).not.toContainText('12 Aug 2026')
  }
})

test('an address naming only a symbol offers each listing of it, and picking one opens that listing', async ({ page, request }) => {
  const model = await figures(request)
  const nyse = (model.positions as Held[]).find((p) => p.symbol === 'TD' && p.exchange === 'NYSE')!
  await page.goto('/#markets/' + encodeURIComponent('listing:TD'))
  const rows = page.locator('#page .lc-row')
  await expect(rows).toHaveCount(2)
  await expect(rows.filter({ hasText: 'TSX' })).toHaveCount(1)
  await rows.filter({ hasText: 'NYSE' }).click()
  await expect(page).toHaveURL(subUrl('portfolio', nyse.id))
})

test("an older link that wrote the venue in words is the same listing's page", async ({ page }) => {
  await page.goto('/#markets/' + encodeURIComponent('listing:ZQX@TSX-V'))
  await expect(page).toHaveURL(/#markets\/listing:ZQX@XTSX$/)
  await expect(disclosures(page).locator('.dc-row')).toHaveCount(2)
})
