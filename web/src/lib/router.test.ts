import { describe, it, expect, afterEach, beforeEach, vi } from 'vitest'
import { tabFromHash, route, startRouter, go, goSub, keepScroll } from './router.svelte'

describe('hash router', () => {
  // jsdom has no window scrolling to do
  beforeEach(() => void vi.spyOn(window, 'scrollTo').mockImplementation(() => {}))
  let stop: (() => void) | undefined
  afterEach(() => {
    stop?.()
    stop = undefined
    location.hash = ''
    vi.restoreAllMocks()
  })

  it('parses known tabs and falls back to dashboard', () => {
    expect(tabFromHash('#markets')).toBe('markets')
    expect(tabFromHash('#/cashflow')).toBe('cashflow')
    expect(tabFromHash('#trades/rt:123')).toBe('trades')
    expect(tabFromHash('')).toBe('dashboard')
    expect(tabFromHash('#nonsense')).toBe('dashboard')
  })

  it('go() sets the hash and the router reflects it in the reactive store', async () => {
    stop = startRouter()
    go('portfolio')
    // hashchange is async in jsdom; wait a tick
    await new Promise((r) => setTimeout(r, 0))
    expect(location.hash).toBe('#portfolio')
    expect(route.tab).toBe('portfolio')

    go('markets')
    await new Promise((r) => setTimeout(r, 0))
    expect(route.tab).toBe('markets')
  })

  // a list scrolled inside its card, as the page draws one: jsdom lays nothing out, so its offsets are plain fields
  function list(name: string, top = 0, left = 0) {
    const el = document.createElement('div')
    Object.defineProperty(el, 'scrollTop', { value: top, writable: true })
    Object.defineProperty(el, 'scrollLeft', { value: left, writable: true })
    return { el, action: keepScroll(el, name) }
  }
  const frame = () => new Promise((r) => requestAnimationFrame(() => r(null)))

  it('Back to a list returns each of its inner lists to where it stood when a row was opened', async () => {
    stop = startRouter()
    go('trades')
    const before = list('trades', 480, 12)
    goSub('trades', 'rt:1')
    before.action.destroy() // the list is not drawn while the row is open
    await frame()
    go('trades') // back to the list
    const after = list('trades') // drawn again, at its top
    await frame()
    expect(after.el.scrollTop).toBe(480)
    expect(after.el.scrollLeft).toBe(12)
    after.action.destroy()
  })

  it('a list keeps its own place, and one on another tab is not moved', async () => {
    stop = startRouter()
    go('portfolio')
    const holdings = list('positions', 200)
    goSub('portfolio', 'pos:1')
    holdings.action.destroy()
    await frame()
    go('trades')
    const trades = list('trades', 30)
    await frame()
    expect(trades.el.scrollTop).toBe(30) // not Back to the list the row was opened from
    trades.action.destroy()
    go('portfolio')
    const again = list('positions')
    await frame()
    expect(again.el.scrollTop).toBe(0) // a tab chosen, not Back from its row
    again.action.destroy()
  })

  it('a trade row that opens its holding returns to the Trades list on Back', async () => {
    stop = startRouter()
    go('trades')
    const before = list('trades', 300)
    goSub('portfolio', 'pos:1')
    before.action.destroy()
    await frame()
    go('trades')
    const after = list('trades')
    await frame()
    expect(after.el.scrollTop).toBe(300)
    after.action.destroy()
  })
})
