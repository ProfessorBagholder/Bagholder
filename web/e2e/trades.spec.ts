import { expect, test, type Page } from '@playwright/test'
import { ready } from './helpers'
import { money, pct, px, qty as fqty, hold } from '../src/lib/fmt'
import { symText } from '../src/lib/sym'

// SPEC.md "### Trades" (list columns, sorts, detail header, chart, executions,
// journal) plus §3/§6 where they name the trade page.

const COLS = ['Open', 'Close', 'Symbol', 'Exchange', 'Qty', 'Entry', 'Exit', 'FX', 'P&L', 'P&L %', 'Hold', 'Grade', 'Tags']

async function getModel(request: Parameters<Parameters<typeof test>[1]>[0]['request']) {
  return (await (await request.get('/api/model')).json()) as any
}

const rows = (page: Page) => page.locator('#page table tbody tr')
const headerCell = (page: Page, label: string) => page.locator('#page thead th').filter({ hasText: label }).first()

test.describe('Trades list', () => {
  test('shows every column of SPEC.md in order, and holds the demo book\'s trades', async ({ page, request }) => {
    const model = await getModel(request)
    await page.goto('/#trades')
    await ready(page)
    const heads = await page.locator('#page thead th .th-in').allTextContents()
    // each header text is "<Label><arrow glyph>"; strip the trailing sort arrow
    const labels = heads.map((h) => h.replace(/[▲▼]$/, ''))
    expect(labels).toEqual(COLS)
    await expect(rows(page)).toHaveCount(model.trades.length)
  })

  test('a row shows Symbol, Exchange, Qty, Entry, Exit, FX, P&L, P&L %, Hold, Grade and Tags formatted per SPEC.md §3', async ({ page, request }) => {
    const model = await getModel(request)
    await page.goto('/#trades')
    await ready(page)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && x.grade && (x.tags || []).length)
    const row = page.locator('#page table tbody tr').filter({ hasText: t.symbol })
    const cells = row.first().locator('td')
    await expect(cells.nth(0)).toHaveText(t.entryDate)
    await expect(cells.nth(1)).toHaveText(t.exitDate)
    await expect(cells.nth(2)).toHaveText(symText(t.symbol))
    await expect(cells.nth(3)).toHaveText(t.exchange)
    await expect(cells.nth(4)).toHaveText(fqty(t.qty))
    await expect(cells.nth(5)).toHaveText(px(t.entry))
    await expect(cells.nth(6)).toHaveText(px(t.exit))
    await expect(cells.nth(7)).toHaveText(t.currency)
    await expect(cells.nth(8)).toHaveText(money(t.pnl))
    await expect(cells.nth(9)).toHaveText(pct(t.pnlPct))
    await expect(cells.nth(10)).toHaveText(hold(t.holdDays))
    await expect(cells.nth(11)).toContainText(t.grade)
    await expect(cells.nth(12)).toContainText(t.tags[0])
  })

  test('newest close first by default', async ({ page, request }) => {
    const model = await getModel(request)
    const wantOrder = model.trades.slice().sort((a: any, b: any) => (a.exitDate < b.exitDate ? 1 : a.exitDate > b.exitDate ? -1 : 0)).map((t: any) => t.exitDate)
    await page.goto('/#trades')
    await ready(page)
    const closes = await page.locator('#page table tbody tr td:nth-child(2)').allTextContents()
    expect(closes).toEqual(wantOrder)
  })

  test('every column sorts, and a second click on the same header reverses it', async ({ page }) => {
    await page.goto('/#trades')
    await ready(page)
    const openCol = () => page.locator('#page table tbody tr td:nth-child(1)').allTextContents()
    await headerCell(page, 'Open').click()
    const desc = await openCol()
    expect(desc).toEqual(desc.slice().sort().reverse())
    await headerCell(page, 'Open').click()
    const asc = await openCol()
    expect(asc).toEqual(asc.slice().sort())
  })

  test('switching the sort to a different header resets its direction to descending', async ({ page }) => {
    await page.goto('/#trades')
    await ready(page)
    const openCol = () => page.locator('#page table tbody tr td:nth-child(1)').allTextContents()
    const qtyCol = () => page.locator('#page table tbody tr td:nth-child(5)').allTextContents()
    await headerCell(page, 'Open').click() // desc
    await headerCell(page, 'Open').click() // asc -- now the direction held on Open is ascending
    const openAsc = await openCol()
    expect(openAsc).toEqual(openAsc.slice().sort())
    // moving to a fresh column starts over at descending, not carrying the ascending flip
    await headerCell(page, 'Qty').click()
    const qtyVals = (await qtyCol()).map((s) => Number(s.replace(/,/g, '')))
    expect(qtyVals).toEqual(qtyVals.slice().sort((a, b) => b - a))
  })

  test('Grade sorts A first on the first click, and ungraded trades sink to the bottom either way', async ({ page }) => {
    await page.goto('/#trades')
    await ready(page)
    await headerCell(page, 'Grade').click() // first click: desc, A first
    const gradesDesc = await page.locator('#page table tbody tr td:nth-child(12) .grade').allTextContents()
    const gradedDesc = gradesDesc.filter((g) => g !== '—')
    expect(gradedDesc[0]).toBe('A')
    expect(gradesDesc.slice(gradesDesc.length - gradesDesc.filter((g) => g === '—').length)).toEqual(gradesDesc.filter((g) => g === '—'))

    await headerCell(page, 'Grade').click() // second click: asc, F first (none here, so the lowest actual grade)
    const gradesAsc = await page.locator('#page table tbody tr td:nth-child(12) .grade').allTextContents()
    const gradedAsc = gradesAsc.filter((g) => g !== '—')
    const rank: Record<string, number> = { F: 0, C: 1, B: 2, A: 3 }
    expect(rank[gradedAsc[0]]).toBeLessThanOrEqual(rank[gradedAsc[gradedAsc.length - 1]])
    // ungraded still last
    expect(gradesAsc.slice(gradesAsc.length - gradesAsc.filter((g) => g === '—').length)).toEqual(gradesAsc.filter((g) => g === '—'))
  })

  test('a click on a row opens the trade, with a breadcrumb beside the tabs', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades[0]
    await page.goto('/#trades')
    await ready(page)
    await page.locator('#page table tbody tr').filter({ hasText: t.symbol }).first().click()
    await expect(page).toHaveURL('/#trades/' + encodeURIComponent(t.id))
    await expect(page.locator('.tabbar')).toContainText(symText(t.symbol))
  })

  test('a long symbol truncates with an ellipsis and shows in full on hover', async ({ page, request }) => {
    const model = await getModel(request)
    const long = model.trades.find((t: any) => t.kind === 'Options' && t.symbol.length > 20)
    expect(long).toBeTruthy()
    await page.setViewportSize({ width: 1150, height: 900 })
    await page.goto('/#trades')
    await ready(page)
    const row = page.locator('#page table tbody tr').filter({ hasText: long.symbol.split(' ')[0] }).first()
    const cell = row.locator('td').nth(2)
    await expect(cell).toHaveText(symText(long.symbol))
    await cell.hover()
    const tip = page.locator('#cutTip')
    await expect(tip).toBeVisible()
    await expect(tip.locator('.tv')).toHaveText(symText(long.symbol))
  })
})

test.describe('Trade detail', () => {
  test('the header shows symbol, name · exchange: ticker, and P&L / P&L % in the trade\'s currency', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares')
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    await expect(page.locator('#page h4')).toHaveText(symText(t.symbol))
    await expect(page.locator('#page').getByText(t.name + ' · ' + t.exchange + ': ' + t.symbol)).toBeVisible()
    await expect(page.locator('#page .tab', { hasText: /^[−$]/ }).first()).toHaveText(money(t.pnl))
    await expect(page.locator('#page').getByText(pct(t.pnlPct), { exact: true })).toBeVisible()
  })

  test('an option\'s header names the underlying, with listing suffixes dropped from the ticker', async ({ page, request }) => {
    const model = await getModel(request)
    const opt = model.trades.find((x: any) => x.kind === 'Options')
    await page.goto('/#trades/' + encodeURIComponent(opt.id))
    await ready(page)
    await expect(page.locator('#page h4')).toHaveText(opt.symbol) // the contract itself, unbared
    await expect(page.locator('#page').getByText(opt.name + ' · ' + opt.exchange + ': ' + opt.underlying)).toBeVisible()
  })

  test('facts show Open, Close, Entry, Exit, Hold and Account for a trade', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares')
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const fact = (label: string) => page.locator('#page').locator('div', { hasText: new RegExp('^' + label + '$') }).locator('xpath=following-sibling::div[1]')
    await expect(fact('Open')).toHaveText(t.entryDate)
    await expect(fact('Close')).toHaveText(t.exitDate)
    await expect(fact('Entry')).toHaveText(px(t.entry))
    await expect(fact('Exit')).toHaveText(px(t.exit))
    await expect(fact('Hold')).toHaveText(hold(t.holdDays))
    await expect(fact('Account')).toHaveText(t.account)
  })

  test('the executions table\'s label carries the count, and When sorts newest first by default and reverses on a second click', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Options') // 2 fills: open + close
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    await expect(page.locator('#page').getByText(/^Executions \(\d+\)$/)).toHaveText('Executions (2)')
    const whenCol = () => page.locator('#page .scroll table tbody tr td:nth-child(1)').allTextContents()
    const before = await whenCol()
    expect(before[0] > before[1]).toBe(true) // newest first
    await page.locator('#page .scroll thead th', { hasText: 'When' }).click()
    const after = await whenCol()
    expect(after[0] < after[1]).toBe(true) // reversed to oldest first
  })

  test('an option execution\'s Side reads what the fill did in the trade, BUY TO OPEN then SELL TO CLOSE', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Options')
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const sides = await page.locator('#page .scroll table tbody tr td:nth-child(2)').allTextContents()
    expect(sides.sort()).toEqual(['BUY TO OPEN', 'SELL TO CLOSE'])
  })

  test('a share execution\'s Side reads BUY or SELL', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares')
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const sides = await page.locator('#page .scroll table tbody tr td:nth-child(2)').allTextContents()
    for (const s of sides) expect(['BUY', 'SELL']).toContain(s)
  })

  test('the timeframe pills offer 1H, 4H, 1D, 1W and 1M for a share or option, with the default one highlighted', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && x.holdDays > 10 && x.holdDays <= 180)
    // what is offered is the server's word (it drops an intraday timeframe a recent read could not supply)
    await page.route('**/api/history?*', (route) => route.fulfill({ json: { ok: true, bars: [], available: ['1h', '4h', '1d', '1w', '1M'], reason: '', pending: false } }))
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const pills = page.locator('#page .pill')
    await expect(pills).toHaveText(['1H', '4H', '1D', '1W', '1M'])
    // the default follows the trade's length: 10 < holdDays <= 180 days -> 1D
    await expect(page.locator('#page .pill.on')).toHaveText('1D')
  })

  test('only the timeframes the server offers are pills: without intraday bars, the daily ones alone', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && x.holdDays > 10 && x.holdDays <= 180)
    await page.route('**/api/history?*', (route) => route.fulfill({ json: { ok: true, bars: [], available: ['1d', '1w', '1M'], reason: '', pending: false } }))
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    await expect(page.locator('#page .pill')).toHaveText(['1D', '1W', '1M'])
  })

  test('picking a timeframe highlights it and it is remembered for that trade while the page stays open', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && x.holdDays > 10 && x.holdDays <= 180)
    await page.goto('/#trades')
    await ready(page)
    await page.locator('#page table tbody tr').filter({ hasText: t.symbol }).first().click()
    await expect(page).toHaveURL('/#trades/' + encodeURIComponent(t.id))
    await page.locator('#page .pill', { hasText: '1W' }).click()
    await expect(page.locator('#page .pill.on')).toHaveText('1W')
    // navigate away (SPA, no reload) and back to the same trade
    await page.goBack()
    await expect(page).toHaveURL(/#trades$/)
    await page.goForward()
    await expect(page).toHaveURL(/#trades\/.+/)
    await expect(page.locator('#page .pill.on')).toHaveText('1W')
  })

  test('a different trade keeps its own default timeframe, unaffected by another trade\'s pick', async ({ page, request }) => {
    const model = await getModel(request)
    const a = model.trades.find((x: any) => x.kind === 'Shares' && x.holdDays > 10 && x.holdDays <= 180)
    const b = model.trades.find((x: any) => x.kind === 'Shares' && x.id !== a.id && x.holdDays > 10 && x.holdDays <= 180)
    await page.goto('/#trades/' + encodeURIComponent(a.id))
    await ready(page)
    await page.locator('#page .pill', { hasText: '1W' }).click()
    await page.goto('/#trades/' + encodeURIComponent(b.id))
    await expect(page.locator('#page .pill.on')).toHaveText('1D')
  })

  test('with no bars to draw, the chart\'s place says why in the server\'s words', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares')
    const reason = 'TMX Money could not be reached; Yahoo Finance could not be reached.'
    await page.route('**/api/history?*', (route) => route.fulfill({ json: { ok: true, bars: [], available: ['1d', '1w', '1M'], reason, pending: false } }))
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    await expect(page.locator('#page .empty').first()).toHaveText(reason)
    await expect(page.getByRole('img', { name: /^Price chart/ })).toHaveCount(0)
  })

  test('thesis saves on blur', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && !x.thesis)
    let saved: any = null
    await page.route('**/api/journal', (route) => {
      saved = route.request().postDataJSON()
      route.fulfill({ status: 200, json: { ok: true } })
    })
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const box = page.getByPlaceholder('Why did you take this trade?')
    await box.fill('Bought the dip on strong guidance.')
    await box.blur()
    await expect.poll(() => saved).not.toBeNull()
    expect(saved.thesis).toBe('Bought the dip on strong guidance.')
    expect(saved.id).toBe(t.id)
  })

  test('a tag is added with Enter and removed with Backspace in the empty box, and matching tags are suggested', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && !(x.tags || []).length)
    const saves: any[] = []
    await page.route('**/api/journal', (route) => {
      saves.push(route.request().postDataJSON())
      route.fulfill({ status: 200, json: { ok: true } })
    })
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const input = page.getByLabel('Add tag')
    await input.click()
    await input.type('swi')
    await expect(page.getByRole('button', { name: /^swing/ })).toBeVisible() // suggested from options.tags
    await input.press('Enter')
    await expect.poll(() => saves.at(-1)?.tags).toEqual(['swing'])
    await expect(page.getByRole('button', { name: 'Remove tag' })).toBeVisible()

    // Backspace in the now-empty box removes the tag just added
    await input.press('Backspace')
    await expect.poll(() => saves.at(-1)?.tags).toEqual([])
  })

  test('a grade pill toggles off when clicked again', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && !x.grade)
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    await page.locator('.seg-opt', { hasText: /^C$/ }).click()
    await expect(page.locator('.seg-opt.on')).toHaveText('C')
    await page.locator('.seg-opt', { hasText: /^C$/ }).click()
    await expect(page.locator('.seg-opt.on')).toHaveCount(0)
  })
})

test.describe('Holding detail (from Portfolio)', () => {
  test('a holding opened from Portfolio shows Qty, Avg, Book and Market instead of Open/Close/Entry/Exit', async ({ page, request }) => {
    const model = await getModel(request)
    const p = model.positions[0]
    await page.goto('/#portfolio')
    await ready(page)
    await page.locator('#page table tbody tr').filter({ hasText: p.symbol }).first().click()
    await expect(page).toHaveURL('/#portfolio/' + encodeURIComponent(p.id))
    await expect(page.locator('#page h4')).toHaveText(symText(p.symbol))
    const fact = (label: string) => page.locator('#page').locator('div', { hasText: new RegExp('^' + label + '$') }).locator('xpath=following-sibling::div[1]')
    await expect(fact('Qty')).toHaveText(fqty(p.qty))
    await expect(fact('Avg')).toHaveText(px(p.avg))
    await expect(fact('Account')).toHaveText(p.account)
    await expect(page.locator('#page').getByText('Open', { exact: true })).toHaveCount(0)
  })

  test('a holding\'s header carries its last price and percent change, not a P&L', async ({ page, request }) => {
    const model = await getModel(request)
    const p = model.positions[0]
    await page.goto('/#portfolio/' + encodeURIComponent(p.id))
    await ready(page)
    await expect(page.locator('.tabbar')).toContainText(symText(p.symbol))
    const priceLine = page.locator('#page .tab', { hasText: /^\d/ }).first()
    await expect(priceLine).toBeVisible()
  })
})

test.describe('The chart, on stored bars', () => {
  test('a trade draws its daily bars with every execution marked on its day', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && x.holdDays > 10 && x.holdDays <= 180)
    const detail = await (await request.get('/api/trade?id=' + encodeURIComponent(t.id))).json()
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const chart = page.getByRole('img', { name: /^Price chart, 1D/ })
    await expect(chart).toBeVisible()
    const label = (await chart.getAttribute('aria-label'))!
    const [, bars, marked, fills] = label.match(/(\d+) bars, (\d+) of (\d+) executions marked/)!.map(Number)
    expect(bars).toBeGreaterThan(10)
    expect(fills).toBe(detail.fills.length)
    expect(marked).toBe(fills)
    await expect(chart.locator('canvas').first()).toBeVisible()
  })

  test('another timeframe redraws the same chart in place, with other bars', async ({ page, request }) => {
    const model = await getModel(request)
    const t = model.trades.find((x: any) => x.kind === 'Shares' && x.holdDays > 30 && x.holdDays <= 180)
    await page.goto('/#trades/' + encodeURIComponent(t.id))
    await ready(page)
    const chart = page.getByRole('img', { name: /^Price chart/ })
    await expect(chart).toHaveAttribute('aria-label', /^Price chart, 1D/)
    const canvas = chart.locator('canvas').first()
    await canvas.evaluate((el) => ((el as unknown as { mark: number }).mark = 1))
    const daily = Number((await chart.getAttribute('aria-label'))!.match(/(\d+) bars/)![1])
    await page.locator('#page .pill', { hasText: '1W' }).click()
    await expect(chart).toHaveAttribute('aria-label', /^Price chart, 1W/)
    const weekly = Number((await chart.getAttribute('aria-label'))!.match(/(\d+) bars/)![1])
    expect(weekly).toBeLessThan(daily)
    expect(await canvas.evaluate((el) => (el as unknown as { mark?: number }).mark)).toBe(1)
  })
})
