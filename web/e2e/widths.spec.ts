import { expect, test } from '@playwright/test'
import { ready } from './helpers'

// A web app works at any width: on a phone and a tablet nothing on any tab
// runs past the screen, save inside a part that scrolls sideways (a wide
// table, the tab bar); at a desktop width the tiles keep their one row.

const TABS = ['dashboard', 'trades', 'portfolio', 'markets', 'cashflow', 'networth']

for (const width of [390, 768]) {
  test(`at ${width}px wide nothing runs past the screen`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    for (const tab of TABS) {
      await page.goto('/#' + tab)
      await ready(page)
      const past = await page.evaluate(() => {
        const scrolls = (el: Element) => ['auto', 'scroll'].includes(getComputedStyle(el).overflowX)
        const out: string[] = []
        if (document.documentElement.scrollWidth > innerWidth) out.push('the page itself')
        for (const el of document.querySelectorAll('#app *')) {
          const r = el.getBoundingClientRect()
          if (!r.width || (r.right <= innerWidth + 1 && r.left >= -1)) continue
          let p = el.parentElement, inside = false
          while (p && !inside) { inside = scrolls(p); p = p.parentElement }
          if (!inside) out.push(`${el.tagName.toLowerCase()} ${(el.textContent ?? '').trim().slice(0, 30)}`)
        }
        return out
      })
      expect(past, tab).toEqual([])
    }
  })
}

test('at a desktop width the tiles keep one row', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/#dashboard')
  await ready(page)
  const tops = await page.locator('.tiles').first().evaluate((g) => new Set(Array.from(g.children).map((c) => Math.round(c.getBoundingClientRect().top))).size)
  expect(tops).toBe(1)
})
