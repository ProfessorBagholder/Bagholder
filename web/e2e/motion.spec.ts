import { expect, test, type Page } from '@playwright/test'
import { ready } from './helpers'

// SPEC.md motion-23 to motion-25: the filters panel, the header menu and the menu's two
// lists slide down from their button when a pointer opens them, long enough and far
// enough to be seen; the panels slide back up as they fade and take no clicks while they
// go; the lists go at once; what the keyboard opens or closes shows or goes at once; with
// less motion asked for, they only fade.

const FILTERS = '#popAnchor > .pop'
const MENU = 'div.menu.elev-md'
const SUB = 'div.menu.sub'

type Seen = { there: boolean; n: number; end: number; moves: boolean; clicks: boolean; y20: number }

/** The element at `selector` in the next frame: whether it is there, how many
 * animations it runs, when the last of them ends, whether any keyframe scales or moves
 * it, and whether it takes clicks. */
const nextFrame = (page: Page, selector: string) =>
  page.evaluate(
    (sel) =>
      new Promise<Seen>((done) =>
        requestAnimationFrame(() => {
          const el = document.querySelector(sel)
          if (!el) return done({ there: false, n: 0, end: 0, moves: false, clicks: false, y20: 0 })
          const a = el.getAnimations()
          done({
            there: true,
            n: a.length,
            end: Math.max(0, ...a.map((x) => Number(x.effect?.getComputedTiming().endTime ?? 0))),
            moves: a.some((x) => (x.effect as KeyframeEffect | null)?.getKeyframes().some((k) => k.transform != null && k.transform !== 'none') ?? false),
            clicks: getComputedStyle(el).pointerEvents !== 'none',
            // how far from its place it still is a fifth of the way through
            y20: a.reduce((y, x) => {
              const k = (x.effect as KeyframeEffect | null)?.getKeyframes() ?? []
              const at = k[Math.round((k.length - 1) * 0.2)]?.transform
              return typeof at === 'string' ? Number(/translateY\((-?[\d.]+)px\)/.exec(at)?.[1] ?? y) : y
            }, 0),
          })
        }),
      ),
    selector,
  )

/** The most elements matching `selector` seen in any one frame over `ms`, from now. */
const mostAtOnce = (page: Page, selector: string, ms: number) =>
  page.evaluate(
    ([sel, span]) =>
      new Promise<number>((done) => {
        let n = 0
        const t0 = performance.now()
        const look = () => {
          n = Math.max(n, document.querySelectorAll(sel).length)
          if (performance.now() - t0 < span) requestAnimationFrame(look)
          else done(n)
        }
        look()
      }),
    [selector, ms] as const,
  )

const settled = (page: Page, selector: string) =>
  expect.poll(() => page.evaluate((sel) => document.querySelector(sel)?.getAnimations().length ?? -1, selector), { timeout: 2000 }).toBe(0)

/** Opened from a pointer: moving in its first frame, for long enough and far enough to
 * be seen, unless less motion is asked for, when it only fades. */
async function dropsIn(page: Page, selector: string, open: () => Promise<void>, slides = true) {
  await open()
  const s = await nextFrame(page, selector)
  expect(s.there, `${selector} opens`).toBe(true)
  expect(s.n, `${selector} is moving in its first frame`).toBeGreaterThan(0)
  expect(s.moves, `${selector} ${slides ? 'slides into place' : 'only fades'}`).toBe(slides)
  if (slides) {
    expect(s.end, `${selector} takes long enough to be seen`).toBeGreaterThanOrEqual(300)
    expect(s.y20, `${selector} is still well short of its place a fifth of the way through`).toBeLessThanOrEqual(-15)
  }
  expect(s.end, `${selector} has arrived within 360 ms`).toBeLessThanOrEqual(360)
  await settled(page, selector)
}

/** Closed from a pointer: it stays to go, sliding back up unless less motion is asked
 * for, taking no clicks, then is removed. */
async function goesOut(page: Page, selector: string, close: () => Promise<void>, slides = true) {
  await close()
  const s = await nextFrame(page, selector)
  expect(s.there, `${selector} stays to go`).toBe(true)
  expect(s.n, `${selector} is going`).toBeGreaterThan(0)
  expect(s.moves, `${selector} ${slides ? 'slides back up' : 'only fades'} as it goes`).toBe(slides)
  expect(s.end, `${selector} goes within 220 ms`).toBeLessThanOrEqual(220)
  expect(s.clicks, `${selector} takes no clicks while it goes`).toBe(false)
  await expect(page.locator(selector)).toHaveCount(0, { timeout: 1000 })
}

/** Shown or gone at once: in its next frame it is in its final state, with nothing running. */
async function atOnce(page: Page, selector: string, act: () => Promise<void>, there: boolean) {
  await act()
  const s = await nextFrame(page, selector)
  expect(s.there, `${selector} ${there ? 'shows' : 'goes'} at once`).toBe(there)
  expect(s.n, `${selector} does not move`).toBe(0)
}

const row = (page: Page, text: string) => page.locator(`${MENU} > div > button`, { hasText: text })
// a press on the page outside any panel: the outside click that closes them
const outside = (page: Page) => page.mouse.click(700, 500)

test('opened by a pointer, the panels and the menu lists slide down; the panels slide back up as they go, the lists go at once', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await dropsIn(page, FILTERS, () => page.getByRole('button', { name: 'Filters' }).click())
  await goesOut(page, FILTERS, () => outside(page))
  await dropsIn(page, MENU, () => page.getByRole('button', { name: 'Menu' }).click())
  await dropsIn(page, SUB, () => row(page, 'Theme').hover())
  // from one list's row to the other's: never two lists at once
  const most = mostAtOnce(page, SUB, 400)
  await row(page, 'Notifications').hover()
  expect(await most, 'one list at every frame').toBe(1)
  await expect(page.locator(SUB)).toContainText('Send a test notification')
  await settled(page, SUB)
  await atOnce(page, SUB, () => page.getByRole('button', { name: 'Add trade' }).hover(), false)
  await goesOut(page, MENU, () => outside(page))
})

test('what the keyboard opens or closes shows or goes at once', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await atOnce(page, FILTERS, () => page.keyboard.press('ControlOrMeta+k'), true)
  // ⌘K again, from a field's own editor: back to the search in place, never a second panel
  await page.locator(`${FILTERS} .pop-row`, { hasText: 'Symbol' }).first().click()
  await expect(page.getByRole('textbox', { name: 'Search values' })).toBeFocused()
  const most = mostAtOnce(page, FILTERS, 400)
  await page.keyboard.press('ControlOrMeta+k')
  expect(await most, 'one filters panel at every frame').toBe(1)
  await expect(page.getByRole('textbox', { name: 'Search', exact: true })).toBeFocused()
  await atOnce(page, FILTERS, () => page.keyboard.press('Escape'), false)
  // the menu opened by a click, and closed by Escape
  await page.getByRole('button', { name: 'Menu' }).click()
  await settled(page, MENU)
  await atOnce(page, MENU, () => page.keyboard.press('Escape'), false)
})

test('with less motion asked for, the panels and the lists only fade', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await page.goto('/')
  await ready(page)
  await dropsIn(page, FILTERS, () => page.getByRole('button', { name: 'Filters' }).click(), false)
  await goesOut(page, FILTERS, () => outside(page), false)
  await dropsIn(page, MENU, () => page.getByRole('button', { name: 'Menu' }).click(), false)
  await dropsIn(page, SUB, () => row(page, 'Theme').hover(), false)
})
