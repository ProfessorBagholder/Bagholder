import { expect, test } from '@playwright/test'
import { ready, openWithStatus, figures, money, money0, pct, pctPlain, hold, stamp, symText, sign, waits, waiting, subUrl, type Waits } from './helpers'

// SPEC §4, Dashboard: the six KPI tiles, the equity curve, the annualized-returns
// card and its benchmark switch, Monthly P&L, Grade vs P&L, By symbol and the
// review queue. Figures are checked against the figures document (GET /api/figures),
// formatted from its exact decimal text the way SPEC.md §3 defines.

type Fig<T> = T | Waits
interface Trade { id: string; symbol: string; status: 'open' | 'closed' }
interface DashModel {
  kpi: {
    realized: Fig<string>; realizedLeftOut: number; count: number; wins: number; losses: number; breakeven: number
    winRate: number | null; grossWin: Fig<string>; grossLoss: Fig<string>
    profitFactor: Fig<number | null>; profitFactorInfinite: boolean
    expectancy: Fig<string | null>; avgWin: Fig<string | null>; avgLoss: Fig<string | null>
  }
  equity: {
    series: { d: string; v: string }[]
    drawdown: { pct: number | null; abs: string | null; at: string | null }
    annualized: { rate: number | null; count: number }
    skippedFilters: string[]
    pnl: { series: { d: string; v: string }[]; leftOut: number; gaps: string[] }
  }
  trades: Trade[]
}

async function getModel(request: import('@playwright/test').APIRequestContext): Promise<DashModel> {
  return (await figures(request)) as DashModel
}

// a trade the list holds, closed, whose page a drill-down opens
const closedTrade = (m: DashModel) => m.trades.find((t) => t.status === 'closed')!.id
const tradeUrl = (id: string) => subUrl('trades', id)

test('the six KPI tiles show label, value and subtitle against the model kpi block', async ({ page, request }) => {
  const m = await getModel(request)
  const k = m.kpi
  const dd = m.equity.drawdown
  const ann = m.equity.annualized
  await page.goto('/')
  await ready(page)
  const tiles = page.locator('#page .kpi')
  await expect(tiles).toHaveCount(6)

  await expect(tiles.nth(0).locator('.lbl')).toHaveText('Realized P&L')
  await expect(tiles.nth(0).locator('.v')).toHaveText(money(k.realized))
  await expect(tiles.nth(0).locator('.s')).toHaveText(k.count + (k.count === 1 ? ' trade' : ' trades'))

  await expect(tiles.nth(1).locator('.lbl')).toHaveText('Win rate')
  await expect(tiles.nth(1).locator('.v')).toHaveText(k.winRate == null ? '—' : pctPlain(k.winRate))
  const beCount = k.breakeven ? ' · ' + k.breakeven + ' BE' : ''
  await expect(tiles.nth(1).locator('.s')).toHaveText(k.wins + ' W · ' + k.losses + ' L' + beCount)

  await expect(tiles.nth(2).locator('.lbl')).toHaveText('Profit factor')
  const pf = k.profitFactorInfinite ? '∞' : waits(k.profitFactor) ? waiting(k.profitFactor) : k.profitFactor == null ? '—' : k.profitFactor.toFixed(2)
  await expect(tiles.nth(2).locator('.v')).toHaveText(pf)
  await expect(tiles.nth(2).locator('.s')).toHaveText('W ' + money0(k.grossWin) + ' · L ' + money0(k.grossLoss))

  await expect(tiles.nth(3).locator('.lbl')).toHaveText('Expectancy')
  await expect(tiles.nth(3).locator('.v')).toHaveText(k.expectancy == null ? '—' : money(k.expectancy))
  await expect(tiles.nth(3).locator('.s')).toHaveText('Avg W ' + money0(k.avgWin) + ' · L ' + money0(k.avgLoss))

  await expect(tiles.nth(4).locator('.lbl')).toHaveText('Max drawdown')
  await expect(tiles.nth(4).locator('.v')).toHaveText(dd.pct == null ? '—' : '−' + Math.abs(dd.pct * 100).toFixed(1) + '%')
  // the CAD fall, shown as a fall whatever sign the document gives it, and the trough month
  const fall = dd.abs == null ? '—' : money0(dd.abs.replace(/^-/, ''))
  const ddSub = dd.pct == null ? 'No NAV history' : '−' + fall + (dd.at ? ' · ' + stamp(dd.at) : '')
  await expect(tiles.nth(4).locator('.s')).toHaveText(ddSub)

  await expect(tiles.nth(5).locator('.lbl')).toHaveText('Avg annualized')
  await expect(tiles.nth(5).locator('.v')).toHaveText(ann.rate == null ? '—' : pct(ann.rate))
  const annSub = ann.rate == null ? 'No NAV history' : 'Over ' + ann.count + (ann.count === 1 ? ' year' : ' years')
  await expect(tiles.nth(5).locator('.s')).toHaveText(annSub)
})

test('positive KPI figures read in the positive colour class, negative in the negative one', async ({ page, request }) => {
  const m = await getModel(request)
  await page.goto('/')
  await ready(page)
  const realizedTile = page.locator('#page .kpi').nth(0).locator('.v')
  const k = m.kpi.realized
  expect(waits(k)).toBe(false) // the made-up book's realized P&L is stated
  if (!waits(k) && sign(k) > 0) await expect(realizedTile).toHaveClass(/\bpos\b/)
  else if (!waits(k) && sign(k) < 0) await expect(realizedTile).toHaveClass(/\bneg\b/)
})

test('the equity curve\'s Value draws a line for the series in scope and dims the chart past the hovered day', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.equity.series = [
      { d: '2026-01-05', v: '10000' },
      { d: '2026-02-10', v: '12000' },
      { d: '2026-03-15', v: '9000' },
      { d: '2026-04-20', v: '15000' },
      { d: '2026-05-25', v: '18000' },
    ]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Equity curve' }) })
  await card.getByRole('button', { name: 'Value' }).click()
  await expect(card.locator('svg path')).toHaveCount(2) // the area fill and the line
  const plot = card.locator('[role="presentation"]')
  const box = (await plot.boundingBox())!

  // hover near the left edge: the first point, 5 Jan '26 at $10,000
  await page.mouse.move(box.x + 2, box.y + box.height / 2)
  await expect(card.locator('.tip .tv')).toHaveText('$10,000')
  await expect(card.locator('.tip .tl')).toHaveText("5 Jan '26")
  await expect(card.locator('.xline')).toBeVisible()
  const maskAtStart = await plot.locator('svg').evaluate((el) => (el as SVGElement).style.maskImage)
  expect(maskAtStart).not.toBe('')

  // hover near the right edge: the last point, 25 May '26 at $18,000
  await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2)
  await expect(card.locator('.tip .tv')).toHaveText('$18,000')
  await expect(card.locator('.tip .tl')).toHaveText("25 May '26")

  // leaving the chart clears the crosshair, the tip and the dim mask
  await page.mouse.move(box.x + box.width + 40, box.y - 40)
  await expect(card.locator('.xline')).toBeHidden()
  const maskAfterLeave = await plot.locator('svg').evaluate((el) => (el as SVGElement).style.maskImage)
  expect(maskAfterLeave).toBe('')
})

test('the equity curve says what it has nothing of: no NAV history for Value, no realized P&L for P&L', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.equity.series = []
    m.equity.pnl = { series: [], leftOut: 0, gaps: [] }
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Equity curve' }) })
  await expect(card).toContainText('No realized P&L in scope.')
  await card.getByRole('button', { name: 'Value' }).click()
  await expect(card).toContainText('No NAV history yet. Sync to load it.')
})

test('the equity curve opens on P&L, a running total that reaches below zero, and remembers Value across a reload', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.equity.pnl = { series: [{ d: '2026-01-05', v: '-400' }, { d: '2026-02-10', v: '250.5' }, { d: '2026-03-15', v: '1200' }], leftOut: 2, gaps: [] }
    m.equity.series = [{ d: '2026-01-05', v: '10000' }, { d: '2026-03-15', v: '12000' }]
    m.equity.skippedFilters = ['date', 'symbol']
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Equity curve' }) })
  await expect(card.getByRole('button', { name: 'P&L' })).toHaveClass(/on/)
  // what the running total left out is not counted on the card (docs/decisions.md)
  await expect(card).not.toContainText('waiting')
  // the axis runs from below zero to above the peak
  await expect(card.locator('.tab span').first()).toHaveText('$1,200')
  await expect(card.locator('.tab span').last()).toHaveText('−$420')
  const plot = card.locator('[role="presentation"]')
  const box = (await plot.boundingBox())!
  await page.mouse.move(box.x + 2, box.y + box.height / 2)
  await expect(card.locator('.tip .tv')).toHaveText(money0('-400'))
  await expect(card.locator('.tip .tv')).toHaveClass(/neg/)
  await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2)
  await expect(card.locator('.tip .tv')).toHaveText('$1,200')
  // nothing is written under the chart, whichever line it shows
  await card.getByRole('button', { name: 'Value' }).click()
  await expect(card).not.toContainText('apply')
  await expect(card).not.toContainText('waiting')
  await page.reload()
  await ready(page)
  await expect(card.getByRole('button', { name: 'Value' })).toHaveClass(/on/)
})

test('the P&L curve ends at the Realized P&L in scope, whatever the filters', async ({ request }) => {
  const m0 = await getModel(request)
  const trade = m0.trades.find((t) => t.status === 'closed')!
  const account = (m0 as unknown as { accounts: { id: string; name: string }[] }).accounts.find((a) => a.name)!
  for (const filters of [undefined, { search: trade.symbol }, { lists: { account: [account.id] } }, { lists: { grade: ['Ungraded'] } }, { years: ['2026'] }]) {
    const m = (await figures(request, filters)) as DashModel
    const last = m.equity.pnl.series.at(-1)
    if (m.equity.pnl.leftOut || waits(m.kpi.realized)) continue
    expect(last ? last.v : '0', JSON.stringify(filters)).toBe(m.kpi.realized)
    expect(m.equity.pnl.leftOut).toBe(m.kpi.realizedLeftOut)
  }
})

test('annualized returns lists the years newest first, each with two bars, and a footer counting years beaten', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.years = [
      { year: '2024', r: 0.12, spR: 0.08, vs: 0.04 },
      { year: '2025', r: -0.05, spR: 0.1, vs: -0.15 },
      { year: '2026', r: 0.2, spR: 0.15, vs: 0.05 },
    ]
    m.benchmark = { key: 'SP500', label: 'S&P 500' }
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Annualized returns' }) })
  await expect(card).toContainText('Vs S&P 500')
  const rows = card.locator('.tab').filter({ hasText: /^20\d\d$/ })
  await expect(rows).toHaveCount(3)
  await expect(rows.nth(0)).toHaveText('2026') // newest first
  await expect(rows.nth(1)).toHaveText('2025')
  await expect(rows.nth(2)).toHaveText('2024')
  // 2026 beats S&P 500 (20% vs 15%), 2025 does not (-5% vs 10%), 2024 does (12% vs 8%): 2 of 3
  await expect(card.locator('.rule-t')).toHaveText('Outperformed S&P 500 in 2 of 3 years.')
})

test('hovering a year of annualized returns says how far it was over or under the index, in the server\'s figure', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.years = [
      { year: '2025', r: -0.05, spR: 0.1, vs: -0.15 },
      { year: '2026', r: 0.2, spR: 0.15, vs: 0.05 },
      { year: '2024', r: 0.12, spR: null, vs: null },
    ]
    m.benchmark = { key: 'TSX', label: 'S&P/TSX' }
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Annualized returns' }) })
  const row = (year: string) => card.locator('[role="presentation"]', { has: page.locator('.tab', { hasText: new RegExp('^' + year + '$') }) })
  await expect(card.locator('.tip')).toHaveCount(0)
  await row('2026').hover()
  await expect(card.locator('.tip .tv')).toHaveText('+5.0 pts')
  await expect(card.locator('.tip .tv')).toHaveAttribute('style', /var\(--pos\)/)
  await expect(card.locator('.tip .tl')).toHaveText('vs S&P/TSX')
  await row('2025').hover()
  await expect(card.locator('.tip .tv')).toHaveText('−15.0 pts')
  await expect(card.locator('.tip .tv')).toHaveAttribute('style', /var\(--neg\)/)
  // a year with no index figure has nothing to compare, and no tip
  await row('2024').hover()
  await expect(card.locator('.tip')).toHaveCount(0)
  await page.mouse.move(0, 0)
  await expect(card.locator('.tip')).toHaveCount(0)
})

test('annualized returns says there are no complete years yet when there are none', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.years = []
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Annualized returns' }) })
  await expect(card).toContainText('No complete years yet.')
})

test('switching the annualized-returns benchmark highlights the new choice, persists it, and asks the server again', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {})
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Annualized returns' }) })
  const sp500 = card.locator('.pill', { hasText: 'S&P 500' })
  const tsx = card.locator('.pill', { hasText: 'S&P/TSX' })
  await expect(sp500).toHaveClass(/\bon\b/)
  await expect(tsx).not.toHaveClass(/\bon\b/)

  // the mocked stream reconnects on its own every 200ms with the filters already in
  // force, so only a request naming the new benchmark proves the switch asked again
  const [req] = await Promise.all([
    page.waitForRequest((r) => r.url().includes('/api/events?filters=') && decodeURIComponent(r.url()).includes('"benchmark":"TSX"')),
    tsx.click(),
  ])
  const filters = JSON.parse(new URL(req.url()).searchParams.get('filters') ?? '')
  expect(filters.benchmark).toBe('TSX')
  await expect(tsx).toHaveClass(/\bon\b/)
  await expect(sp500).not.toHaveClass(/\bon\b/)
  expect(await page.evaluate(() => localStorage.getItem('bh2.benchmark'))).toBe('TSX')

  // it is a preference, not a filter: no chip appears for it
  await expect(page.locator('.chip')).toHaveCount(0)
})

test('the remembered benchmark choice survives a reload', async ({ page, request }) => {
  // SPEC.md "### Dashboard", Annualized returns: "the choice is remembered on this machine"
  await openWithStatus(page, request, {}, '', () => {})
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Annualized returns' }) })
  await card.locator('.pill', { hasText: 'S&P/TSX' }).click()
  await page.reload()
  await ready(page)
  await expect(card.locator('.pill', { hasText: 'S&P/TSX' })).toHaveClass(/\bon\b/)
})

test('Monthly P&L bar hover names the month and trade count, and clicking a month with one trade opens it', async ({ page, request }) => {
  const id = closedTrade(await getModel(request))
  await openWithStatus(page, request, {}, '', (m) => {
    m.monthly = [
      { key: '2026-01', label: "Jan '26", value: '500', count: 1, tradeIds: [id] },
      { key: '2026-02', label: "Feb '26", value: '-300', count: 2, tradeIds: ['a', 'b'] },
    ]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Monthly P&L' }) })
  const bars = card.locator('.bar-col')
  await expect(bars).toHaveCount(2)
  await bars.nth(0).hover()
  await expect(card.locator('.tip .tv')).toHaveText('$500')
  await expect(card.locator('.tip .tl')).toHaveText("Jan '26 · 1 trade")
  await bars.nth(1).hover()
  await expect(card.locator('.tip .tl')).toHaveText("Feb '26 · 2 trades")

  await bars.nth(0).click()
  await expect(page).toHaveURL(tradeUrl(id))
})

test('clicking a month with several trades filters Trades to that month, not to one trade', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.monthly = [{ key: '2026-03', label: "Mar '26", value: '1200', count: 3, tradeIds: ['a', 'b', 'c'] }]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Monthly P&L' }) })
  await card.locator('.bar-col').first().click()
  await expect(page).toHaveURL(/#trades$/)
  const chip = page.locator('.chip').first()
  await expect(chip.locator('.cf')).toHaveText('Date')
  await expect(chip.locator('.cv')).toHaveText('2026-03-01 → 2026-03-31')
})

test('Grade vs P&L shows four bars with a CAD sum and a count per grade', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.grades = {
      buckets: [
        { grade: 'A', n: 2, pnl: '900', tradeIds: ['a', 'b'] },
        { grade: 'B', n: 1, pnl: '100', tradeIds: ['c'] },
        { grade: 'C', n: 0, pnl: '0', tradeIds: [] },
        { grade: 'F', n: 1, pnl: '-400', tradeIds: ['d'] },
      ],
      graded: 4,
    }
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Grade vs P&L' }) })
  const bars = card.locator('.bar-col')
  await expect(bars).toHaveCount(4)
  await expect(bars.nth(0)).toContainText('$900')
  await expect(bars.nth(1)).toContainText('$100')
  await expect(bars.nth(2)).toContainText('—') // no graded C trades: em dash, not $0
  await expect(bars.nth(3)).toContainText('−$400')
  const counts = card.locator('span', { hasText: /^[ABCF] ·/ })
  await expect(counts.nth(0)).toHaveText('A · 2')
  await expect(counts.nth(3)).toHaveText('F · 1')
})

test('clicking a grade with one trade opens it; clicking one with several filters Trades to that grade', async ({ page, request }) => {
  const id = closedTrade(await getModel(request))
  await openWithStatus(page, request, {}, '', (m) => {
    m.grades = {
      buckets: [
        { grade: 'A', n: 1, pnl: '900', tradeIds: [id] },
        { grade: 'B', n: 2, pnl: '100', tradeIds: ['c', 'd'] },
        { grade: 'C', n: 0, pnl: '0', tradeIds: [] },
        { grade: 'F', n: 0, pnl: '0', tradeIds: [] },
      ],
      graded: 3,
    }
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Grade vs P&L' }) })
  await card.locator('.bar-col').nth(0).click()
  await expect(page).toHaveURL(tradeUrl(id))
  await page.goBack()
  await ready(page)
  await card.locator('.bar-col').nth(1).click()
  await expect(page).toHaveURL(/#trades$/)
  const chip = page.locator('.chip').first()
  await expect(chip.locator('.cf')).toHaveText('Grade is')
  await expect(chip.locator('.cv')).toHaveText('B')
})

test('a grade with no graded trades yet does nothing when clicked', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.grades = { buckets: [
      { grade: 'A', n: 0, pnl: '0', tradeIds: [] },
      { grade: 'B', n: 0, pnl: '0', tradeIds: [] },
      { grade: 'C', n: 0, pnl: '0', tradeIds: [] },
      { grade: 'F', n: 1, pnl: '-10', tradeIds: ['x'] },
    ], graded: 1 }
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Grade vs P&L' }) })
  await card.locator('.bar-col').nth(0).click()
  await expect(page).toHaveURL('/')
})

test('By symbol shows two underlyings that share a symbol as two rows', async ({ page, request }) => {
  // a symbol two listings share (a company on the TSX and a fund in New York) is two underlyings
  await openWithStatus(page, request, {}, '', (m) => {
    m.bySymbol = [
      { id: 'u-1', symbol: 'CH', pnl: '120', n: 2, winRate: 1, avgHold: 4, tradeIds: [] },
      { id: 'u-2', symbol: 'CH', pnl: '-30', n: 1, winRate: 0, avgHold: 9, tradeIds: [] },
    ]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'By symbol' }) })
  await expect(card.locator('tbody tr')).toHaveCount(2)
})

test('By symbol is grouped by underlying, sorted by P&L, and every column sorts on click', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.bySymbol = [
      { id: 'i-aaa', symbol: 'AAA', pnl: '500', n: 3, winRate: 0.667, avgHold: 4, tradeIds: ['a', 'b', 'c'] },
      { id: 'i-bbb', symbol: 'BBB.TO', pnl: '-200', n: 2, winRate: 0.0, avgHold: 12, tradeIds: ['d', 'e'] },
      { id: 'i-ccc', symbol: 'CCC', pnl: '1200', n: 1, winRate: 1, avgHold: 30, tradeIds: ['f'] },
    ]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'By symbol' }) })
  const rows = card.locator('tbody tr')
  await expect(rows).toHaveCount(3)
  // default sort: pnl desc — CCC 1200, AAA 500, BBB -200
  await expect(rows.nth(0).locator('td').first()).toHaveText(symText('CCC'))
  await expect(rows.nth(1).locator('td').first()).toHaveText(symText('AAA'))
  await expect(rows.nth(2).locator('td').first()).toHaveText(symText('BBB.TO')) // .TO dropped: bare ticker
  await expect(rows.nth(2).locator('td').nth(1)).toHaveText(money('-200'))
  await expect(rows.nth(2).locator('td').nth(1)).toHaveClass(/\bneg\b/)
  await expect(rows.nth(0).locator('td').nth(1)).toHaveClass(/\bpos\b/)
  await expect(rows.nth(0).locator('td').nth(3)).toHaveText(pctPlain(1))
  await expect(rows.nth(0).locator('td').nth(4)).toHaveText(hold(30))

  // click Symbol header: ascending alphabetical (first click on a new column is descending per the sort rule,
  // so the first click makes AAA/BBB/CCC descend... but symbols are strings: desc means Z→A)
  const symbolHeader = card.locator('th', { hasText: 'Symbol' })
  await symbolHeader.click()
  await expect(rows.nth(0).locator('td').first()).toHaveText('CCC')
  await expect(rows.nth(1).locator('td').first()).toHaveText('BBB')
  await expect(rows.nth(2).locator('td').first()).toHaveText('AAA')
  await symbolHeader.click() // second click flips to ascending
  await expect(rows.nth(0).locator('td').first()).toHaveText('AAA')
  await expect(rows.nth(2).locator('td').first()).toHaveText('CCC')

  const tradesHeader = card.locator('th', { hasText: 'Trades' })
  await tradesHeader.click() // first click: descending by count — CCC(1) BBB(2) AAA(3) -> desc is AAA,BBB,CCC
  await expect(rows.nth(0).locator('td').nth(2)).toHaveText('3')
  await expect(rows.nth(2).locator('td').nth(2)).toHaveText('1')
})

test('clicking a By symbol row with one trade opens it; with several it filters Trades to that symbol', async ({ page, request }) => {
  const id = closedTrade(await getModel(request))
  let asked: Record<string, unknown> | null = null
  page.on('request', (r) => {
    const f = new URL(r.url()).searchParams.get('filters')
    if (r.url().includes('/api/events') && f) asked = JSON.parse(f)
  })
  await openWithStatus(page, request, {}, '', (m) => {
    // a row names its underlying instrument by id; the chip shows the instrument's symbol
    m.options.instruments.push(
      { id: 'i-one', symbol: 'ONE', name: 'One Corp', exchange: 'NASDAQ', kind: 'Shares', currency: 'USD' },
      { id: 'i-two', symbol: 'TWO', name: 'Two Corp', exchange: 'NASDAQ', kind: 'Shares', currency: 'USD' },
    )
    m.bySymbol = [
      { id: 'i-one', symbol: 'ONE', pnl: '100', n: 1, winRate: 1, avgHold: 2, tradeIds: [id] },
      { id: 'i-two', symbol: 'TWO', pnl: '-50', n: 2, winRate: 0.5, avgHold: 5, tradeIds: ['x', 'y'] },
    ]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'By symbol' }) })
  // sorted by pnl desc: ONE (100) first, TWO (-50) second
  await card.locator('tbody tr').nth(0).click()
  await expect(page).toHaveURL(tradeUrl(id))
  await page.goBack()
  await ready(page)
  await card.locator('tbody tr').nth(1).click()
  await expect(page).toHaveURL(/#trades$/)
  const chip = page.locator('.chip').first()
  await expect(chip.locator('.cf')).toHaveText('Symbol is')
  await expect(chip.locator('.cv')).toHaveText('TWO')
  // the filter holds the instrument's id, never its symbol
  await expect.poll(() => (asked as { lists?: { symbol?: string[] } } | null)?.lists?.symbol).toEqual(['i-two'])
})

test('the review queue lists closed trades missing a grade or a thesis, newest first, and opens a trade on click', async ({ page, request }) => {
  const doc = await figures(request)
  const model = doc as DashModel
  // the book's own queue: its closed trades missing a grade or a thesis, newest first
  const queue = doc.queue as { id: string; date: string }[]
  const all = doc.trades as { id: string; status: string; grade: string; thesis: string }[]
  const want = all.filter((t) => t.status === 'closed' && (!t.grade || !t.thesis)).map((t) => t.id)
  expect(queue.map((q) => q.id).sort()).toEqual(want.sort())
  expect(queue.map((q) => q.date)).toEqual(queue.map((q) => q.date).sort().reverse())
  const id = closedTrade(model)
  await openWithStatus(page, request, {}, '', (m) => {
    m.queue = [
      { id, symbol: 'AAA', date: '2026-05-01', pnl: '300', missing: 'grade' },
      { id: 'x2', symbol: 'BBB', date: '2026-04-01', pnl: '-100', missing: 'grade, thesis' },
    ]
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Review queue' }) })
  const rows = card.locator('.queue-row')
  await expect(rows).toHaveCount(2)
  await expect(rows.nth(0)).toContainText('AAA')
  await expect(rows.nth(0)).toContainText('2026-05-01 · grade')
  await expect(rows.nth(1)).toContainText('2026-04-01 · grade, thesis')
  await rows.nth(0).click()
  await expect(page).toHaveURL(tradeUrl(id))
})

test('the review queue says nothing is left to review when every closed trade has a grade and a thesis', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => {
    m.queue = []
  })
  await ready(page)
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Review queue' }) })
  await expect(card).toContainText('Nothing left to review')
})
