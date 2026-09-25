import { expect, test, type Page } from '@playwright/test'
import { ready } from './helpers'

// Screenshot baselines of every screen at the four widths SPEC.md §7 checks (stage 3c
// plan, §1). Taken on the made-up book before the switch; from then on a baseline changes
// only where a SPEC.md change in the same commit names it.
//
// They are made and compared in one place only, the CI job `baselines` (the Playwright
// image, with the server's clock set by faketime), because a screenshot depends on the
// machine's rendering: compared anywhere else they would fail on antialiasing, not on the
// page. Elsewhere this file compares nothing and says so.
const IN_THE_IMAGE = !!process.env.BAGHOLDER_BASELINES
test.skip(!IN_THE_IMAGE, 'screenshot baselines are compared only in the CI job `baselines` (BAGHOLDER_BASELINES=1)')

// The instant the server's clock is set to in that job (BAGHOLDER_E2E_FAKETIME), so every
// figure measured to today is the same on every run.
const NOW = new Date(process.env.BAGHOLDER_E2E_FAKETIME ?? '2026-09-10T16:00:00Z')
const WIDTHS = [1200, 1340, 1440, 1680]

// What changes from run to run without the page changing: the release number, and how
// long ago a source was read.
const masks = (page: Page) => [page.getByText(/^v\d+\.\d+\.\d+$/), page.getByText(/^read (.* ago|just now)$/)]

async function open(page: Page, width: number, hash: string) {
  await page.clock.setFixedTime(NOW)
  await page.setViewportSize({ width, height: 1000 })
  await page.goto('/#' + hash)
  await ready(page)
  await page.mouse.move(0, 0)
}

// the pointer is taken off the page first: a click leaves it where the row was, and on a
// chart that is a crosshair the screen does not otherwise show
async function shot(page: Page, name: string) {
  await page.mouse.move(0, 0)
  await expect(page).toHaveScreenshot(name + '.png', { fullPage: true, mask: masks(page) })
}

for (const width of WIDTHS) {
  test.describe(`at ${width} px`, () => {
    for (const tab of ['dashboard', 'trades', 'portfolio', 'markets', 'cashflow']) {
      test(`the ${tab} tab`, async ({ page }) => {
        await open(page, width, tab)
        await shot(page, `${tab}-${width}`)
      })
    }

    // a trade and a holding are opened from their lists, by what the row shows, so the
    // screen is found the same way whatever id the server gives it
    test('a closed trade', async ({ page }) => {
      await open(page, width, 'trades')
      await page.locator('tbody tr', { hasText: 'UBER' }).first().click()
      await expect(page.getByText('Executions (2)')).toBeVisible()
      await expect(page.getByRole('img', { name: /^Price chart/ })).toBeVisible()
      await shot(page, `trade-${width}`)
    })

    test('a holding', async ({ page }) => {
      await open(page, width, 'portfolio')
      await page.getByRole('row').filter({ has: page.getByRole('cell', { name: 'TD', exact: true }) }).first().click()
      await expect(page.getByText(/^Executions/)).toBeVisible()
      await expect(page.getByRole('img', { name: /^Price chart/ })).toBeVisible()
      await shot(page, `holding-${width}`)
    })

    test('the order ticket', async ({ page }) => {
      await open(page, width, 'dashboard')
      await page.keyboard.press('Control+k')
      await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
      await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
      await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
      await shot(page, `ticket-${width}`)
    })

    for (const [name, button] of [['orders', 'Orders'], ['notifications', 'Notifications'], ['menu', 'Menu'], ['filter', 'Filters']]) {
      test(`the ${name} overlay`, async ({ page }) => {
        await open(page, width, 'dashboard')
        await page.getByRole('button', { name: button, exact: true }).click()
        await shot(page, `${name}-${width}`)
      })
    }
  })
}
