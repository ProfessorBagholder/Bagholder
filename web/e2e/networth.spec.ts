import { expect, test, type Locator, type Page } from '@playwright/test'
import { ready, view, openWithStatus, money0, stampDay } from './helpers'

// SPEC §4, Net worth: the tiles are today's, the chart is every day Bagholder
// works out to today, the lists are what each figure comes to, and the filters
// narrow all of it. Figures are checked against the positions document the page
// is sent, so the tests stay true if the made-up book's numbers change; what the
// made-up book cannot bring about (a day that cannot be stated, a long list) is
// stood in with made-up figures.

interface Item { id: string; kind: string; name: string; institution: string; amount: string; asOf: string | null }
interface Point { d: string; v: string; a: string; o: string }
interface NetWorth {
  netWorth: string; assetsTotal: string; liabilitiesTotal: string; assets: Item[]; liabilities: Item[]
  history: Point[]; unstated: { from: string; to: string; gaps: string[] }[]; pastDay: string | null; holdings: boolean
}

const netWorth = async (request: import('@playwright/test').APIRequestContext, filters?: Record<string, unknown>): Promise<NetWorth> =>
  (await view(request, 'positions', filters ? { filters } : {})).portfolio.netWorth

const tile = (page: Page, label: string) => page.locator('#page .kpi').filter({ has: page.locator('.lbl', { hasText: new RegExp('^' + label + '$') }) })
const card = (page: Page, heading: string) => page.locator('#page .card').filter({ has: page.locator('h5', { hasText: new RegExp('^' + heading + '$') }) })
const chart = (page: Page) => card(page, 'Net worth over time')

/** The chart read at one end: the pointer at the plot's very edge is that end's day. */
async function hoverEnd(plotCard: Locator, end: 'first' | 'last'): Promise<void> {
  const plot = plotCard.locator('[role="presentation"]')
  const box = (await plot.boundingBox())!
  // dispatched at the edge itself: a real pointer lands a fraction inside it, which,
  // with more days than pixels, can read the day before the last
  await plot.dispatchEvent('mousemove', { clientX: end === 'last' ? box.x + box.width : box.x, clientY: box.y + box.height / 2 })
}

/** Open the funnel on one field. */
async function openField(page: Page, label: string): Promise<void> {
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: new RegExp('^' + label) }).click()
}

async function tilesAre(page: Page, nw: NetWorth): Promise<void> {
  await expect(tile(page, 'Net worth').locator('.v')).toHaveText(money0(nw.netWorth))
  await expect(tile(page, 'Assets').locator('.v')).toHaveText(money0(nw.assetsTotal))
  await expect(tile(page, 'Liabilities').locator('.v')).toHaveText(money0(nw.liabilitiesTotal))
}

async function listsAre(page: Page, nw: NetWorth): Promise<void> {
  for (const [heading, rows] of [['Assets', nw.assets], ['Liabilities', nw.liabilities]] as const) {
    const shown = card(page, heading).locator('tbody tr')
    if (!rows.length) {
      await expect(card(page, heading)).toContainText('None.')
      continue
    }
    await expect(shown).toHaveCount(rows.length)
    for (let i = 0; i < rows.length; i++) {
      await expect(shown.nth(i).locator('td').first()).toContainText(rows[i].name)
      await expect(shown.nth(i).locator('td').last()).toHaveText(money0(rows[i].amount))
    }
  }
}

test("the tiles are today's net worth, and the chart, every day to today, ends on them", async ({ page, request }) => {
  const nw = await netWorth(request)
  const today: string = (await view(request, 'book')).today
  expect(nw.history.length).toBeGreaterThan(2) // the made-up book has a history to draw
  const last = nw.history[nw.history.length - 1]
  // today's point is the tiles' own figures
  expect(last.d).toBe(today)
  expect([last.v, last.a, last.o]).toEqual([nw.netWorth, nw.assetsTotal, nw.liabilitiesTotal])

  await page.goto('/#networth')
  await ready(page)
  await tilesAre(page, nw)
  // one line, a point a day
  const line = chart(page).locator('svg path[fill="none"]')
  await expect(line).toHaveCount(1)
  expect(((await line.getAttribute('d')) ?? '').split(/[ML]/).filter((s) => s.trim()).length).toBe(nw.history.length)

  await hoverEnd(chart(page), 'last')
  await expect(chart(page).locator('.tip .tv')).toHaveText(money0(nw.netWorth))
  await expect(chart(page).locator('.tip .tl')).toHaveText(stampDay(today))
  await hoverEnd(chart(page), 'first')
  await expect(chart(page).locator('.tip .tv')).toHaveText(money0(nw.history[0].v))
  await expect(chart(page).locator('.tip .tl')).toHaveText(stampDay(nw.history[0].d))

  // assets and debts: two lines on one scale, today's hover the three tiles
  await chart(page).getByRole('button', { name: 'Assets & debts' }).click()
  await expect(line).toHaveCount(2)
  await hoverEnd(chart(page), 'last')
  const tv = chart(page).locator('.tip .tv')
  await expect(tv).toHaveText(['Assets' + money0(nw.assetsTotal), 'Debts' + money0(nw.liabilitiesTotal), 'Net worth' + money0(nw.netWorth)])
  // every day is stated in the made-up book: nothing is said under the chart
  await expect(chart(page).locator('.unstated')).toHaveCount(0)
})

test("the lists are what the tiles come to: Wealthsimple's part as Holdings, Uninvested cash and Chequing, margin a debt", async ({ page, request }) => {
  const nw = await netWorth(request)
  const ids = nw.assets.map((a) => a.id)
  // the made-up book holds shares and cash and borrows on margin
  expect(ids).toEqual(expect.arrayContaining(['holdings', 'cash']))
  expect(ids.filter((id) => ['holdings', 'cash', 'chequing'].includes(id))).toEqual(['holdings', 'cash', 'chequing'].filter((id) => ids.includes(id)))
  expect(nw.liabilities[0].id).toBe('margin')
  await page.goto('/#networth')
  await ready(page)
  await listsAre(page, nw)
  const assets = card(page, 'Assets').locator('tbody tr')
  await expect(assets.filter({ hasText: 'Uninvested cash' })).toContainText('Wealthsimple')
  await expect(card(page, 'Liabilities').locator('tbody tr').first()).toContainText('Wealthsimple')
})

test("an account filter narrows the tiles, the lists and the chart to that account's value and debt", async ({ page, request }) => {
  const all = await netWorth(request)
  const accounts = (await view(request, 'book')).options.accounts as { id: string; name: string }[]
  // an account with no margin: its net worth is its value alone
  let picked: { id: string; name: string } | null = null
  let nw: NetWorth | null = null
  for (const a of accounts) {
    const n = await netWorth(request, { lists: { account: [a.id] } })
    if (n && !n.liabilities.length && n.history.length > 2 && n.netWorth !== all.netWorth) {
      picked = a
      nw = n
      break
    }
  }
  expect(picked).not.toBeNull() // the made-up book has an account that borrows nothing
  await page.goto('/#networth')
  await ready(page)
  await tilesAre(page, all)
  await openField(page, 'Account')
  await page.locator('.pop .pop-row', { hasText: picked!.name }).first().click()
  await page.getByRole('button', { name: 'Done' }).click()
  await tilesAre(page, nw!)
  await listsAre(page, nw!)
  await hoverEnd(chart(page), 'last')
  await expect(chart(page).locator('.tip .tv')).toHaveText(money0(nw!.netWorth))
})

test("a holding's filter is the holdings' value alone, with no debt and no chart", async ({ page, request }) => {
  const tags = (await view(request, 'book')).options.tags as string[]
  let tag = ''
  let nw: NetWorth | null = null
  for (const t of tags) {
    const n = await netWorth(request, { lists: { tag: [t] } })
    if (n) {
      tag = t
      nw = n
      break
    }
  }
  expect(tag).not.toBe('') // the made-up book holds a tagged position
  expect(nw!.holdings).toBe(true)
  await page.goto('/#networth')
  await ready(page)
  await openField(page, 'Tag')
  await page.locator('.pop-row', { hasText: tag }).first().click()
  await page.getByRole('button', { name: 'Done' }).click()
  await tilesAre(page, nw!)
  await expect(tile(page, 'Liabilities').locator('.v')).toHaveText(money0('0'))
  await listsAre(page, nw!)
  await expect(chart(page)).toContainText('A holding’s value by day is not kept')
  await expect(chart(page).locator('svg')).toHaveCount(0)
})

test("dates that end before today are that day's totals, the chart the range's days, and no item listed", async ({ page, request }) => {
  const all = await netWorth(request)
  const mid = Math.floor(all.history.length / 2)
  const from = all.history[mid - 60].d
  const to = all.history[mid].d
  const nw = await netWorth(request, { from, to })
  expect(nw.pastDay).toBe(to)
  expect([nw.history[0].d, nw.history[nw.history.length - 1].d]).toEqual([from, to])
  await page.goto('/#networth')
  await ready(page)
  await openField(page, 'Date')
  await page.getByLabel('From').fill(from)
  await page.getByLabel('To').fill(to)
  await page.getByRole('button', { name: 'Done' }).click()
  await tilesAre(page, nw)
  await expect(page.locator('#page')).toContainText('Totals as of ' + stampDay(to) + ", the range's last day.")
  await expect(card(page, 'Assets')).toContainText('None.')
  await expect(card(page, 'Liabilities')).toContainText('None.')
  await hoverEnd(chart(page), 'last')
  await expect(chart(page).locator('.tip .tl')).toHaveText(stampDay(to))
  await expect(chart(page).locator('.tip .tv')).toHaveText(money0(nw.netWorth))
  await hoverEnd(chart(page), 'first')
  await expect(chart(page).locator('.tip .tl')).toHaveText(stampDay(from))
})

test('days that cannot be stated are not drawn, and are said under the chart with what they wait on', async ({ page, request }) => {
  // made-up runs: three days waiting on a rate, one on a price
  const runs = [
    { from: '2026-03-02', to: '2026-03-04', gaps: ['rate-missing'] },
    { from: '2026-05-11', to: '2026-05-11', gaps: ['price-unknown'] },
  ]
  const out = (d: string) => runs.some((r) => d >= r.from && d <= r.to)
  await openWithStatus(page, request, {}, '#networth', (m) => {
    const nw = m.portfolio.netWorth as NetWorth
    nw.history = nw.history.filter((p) => !out(p.d))
    nw.unstated = runs
  })
  await ready(page)
  await expect(chart(page).locator('.unstated')).toHaveText(
    'Not drawn: ' + stampDay('2026-03-02') + ' to ' + stampDay('2026-03-04') + ', waiting on the rate; ' + stampDay('2026-05-11') + ', waiting on the price.',
  )
  await expect(chart(page).locator('svg path[fill="none"]')).toHaveCount(1)
})

test('the Assets and Liabilities cards are one height at every width, a long list scrolling inside its card', async ({ page, request }) => {
  // made-up outside assets, more than the card holds
  const many: Item[] = Array.from({ length: 40 }, (_, i) => ({ id: 'a' + i, kind: 'Savings', name: 'Savings ' + (i + 1), institution: 'Omicron Trust', amount: String(1000 + i), asOf: null }))
  await openWithStatus(page, request, {}, '#networth', (m) => {
    const nw = m.portfolio.netWorth as NetWorth
    nw.assets = [...nw.assets, ...many]
  })
  await ready(page)
  const assets = card(page, 'Assets')
  const liabilities = card(page, 'Liabilities')
  await expect(assets.locator('tbody tr')).toHaveCount(40 + (await netWorth(request)).assets.length)
  for (const width of [1200, 1340, 1440, 1680]) {
    await page.setViewportSize({ width, height: 900 })
    const [a, l] = [(await assets.boundingBox())!, (await liabilities.boundingBox())!]
    expect(Math.abs(a.height - l.height), `${width} px: Assets ${a.height} px, Liabilities ${l.height} px`).toBeLessThan(1)
    expect(Math.abs(a.y - l.y), `${width} px: the cards side by side`).toBeLessThan(1)
    // the list scrolls inside its card, and the card does not grow with it
    const list = assets.locator('.scroll')
    const [content, room] = await list.evaluate((el) => [el.scrollHeight, el.clientHeight])
    expect(content, `${width} px: the list holds ${content} px in ${room}`).toBeGreaterThan(room)
    const lb = (await list.boundingBox())!
    expect(lb.y + lb.height, `${width} px: the list ends inside its card`).toBeLessThanOrEqual(a.y + a.height + 0.5)
  }
  const list = assets.locator('.scroll')
  await list.evaluate((el) => (el.scrollTop = el.scrollHeight))
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBeGreaterThan(0)
  // the header row stays at the list's top as it scrolls
  const head = (await assets.locator('thead th').first().boundingBox())!
  const lb = (await list.boundingBox())!
  expect(Math.abs(head.y - lb.y)).toBeLessThan(1)
})
