import { expect, test, type Locator, type Page } from '@playwright/test'
import { figures, ready } from './helpers'

// #528: a chip is never squeezed. What it narrows to stays whole wherever it sits,
// and what sits beside it gives way: the header's row of filter chips scrolls, never
// wider than the page. Every place the page renders a `.chip`: the header's filters,
// the Disclosures source and, in marketcards.spec.ts, the News card's symbol.

const WIDTHS = [1200, 1340, 1440, 1680]

/** Every chip holds what it says: its content no wider than its box. */
async function whole(chips: Locator, where: string): Promise<void> {
  const sizes = await chips.evaluateAll((els) => els.map((el) => [el.scrollWidth, el.clientWidth, el.textContent ?? '']))
  expect(sizes.length, `${where}: chips shown`).toBeGreaterThan(0)
  for (const [content, box, text] of sizes) expect(content as number, `${where}: ${text} holds ${content} px in ${box}`).toBeLessThanOrEqual(box as number)
}

/** The page itself never scrolls sideways. */
async function noSidewaysPage(page: Page, where: string): Promise<void> {
  const [doc, view] = await page.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth])
  expect(doc, `${where}: the page is ${doc} px in ${view}`).toBeLessThanOrEqual(view)
}

test("the header's filter chips are each whole at every width, the row never wider than the page, and every one reachable by scrolling it", async ({ page }) => {
  await page.goto('/#trades')
  await ready(page)
  // a value of every list field the book has, and a date window: more chips than the row holds
  for (const field of ['Account', 'Grade', 'Tag', 'Side', 'Kind', 'Exchange', 'Result']) {
    await page.getByRole('button', { name: 'Filters' }).click()
    await page.locator('.pop-row', { hasText: new RegExp('^' + field) }).click()
    const values = page.locator('.pop .pop-row')
    if (await values.count()) await values.first().click()
    const done = page.getByRole('button', { name: 'Done' })
    if (await done.isVisible()) await done.click()
    else await page.keyboard.press('Escape')
  }
  await page.getByRole('button', { name: 'Filters' }).click()
  await page.locator('.pop-row', { hasText: /^Date/ }).click()
  await page.locator('.pill', { hasText: '1Y' }).click()
  await page.getByRole('button', { name: 'Done' }).click()
  const chips = page.locator('.chip')
  const row = chips.first().locator('xpath=..')
  for (const width of WIDTHS) {
    await page.setViewportSize({ width, height: 800 })
    const where = `${width} px, ${await chips.count()} filters`
    await whole(chips, where)
    await noSidewaysPage(page, where)
    // the last chip is reached by scrolling the row to its end
    await row.evaluate((el) => (el.scrollLeft = el.scrollWidth))
    const [last, box] = await Promise.all([chips.last().boundingBox(), row.boundingBox()])
    expect(last!.x + last!.width, `${where}: the last chip is past the row's end`).toBeLessThanOrEqual(box!.x + box!.width + 1)
  }
  // the premise: at the narrowest width the row holds more than it shows
  await page.setViewportSize({ width: WIDTHS[0], height: 800 })
  const [content, shown] = await row.evaluate((el) => [el.scrollWidth, el.clientWidth])
  expect(content, 'the filters chosen fill more than the row shows').toBeGreaterThan(shown)
})

test('the Disclosures source chip is whole at every width', async ({ page, request }) => {
  const model = await figures(request)
  // TD's holdings list their filer's filings from both regulators: a source to pick
  const td = (model.positions as { id: string; symbol: string; kind: string }[]).find((p) => p.symbol === 'TD' && p.kind === 'Shares')!
  await page.goto('/#portfolio/' + encodeURIComponent(td.id))
  const card = page.locator('.card', { has: page.locator('h5', { hasText: 'Disclosures' }) })
  await card.locator('.dc-src').first().click()
  await expect(card.locator('.chip')).toContainText('Source')
  for (const width of WIDTHS) {
    await page.setViewportSize({ width, height: 800 })
    await whole(card.locator('.chip'), `${width} px`)
    await noSidewaysPage(page, `${width} px`)
  }
})
