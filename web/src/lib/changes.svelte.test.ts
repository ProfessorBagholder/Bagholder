// Rule 7 held at the DOM, one kind of change at a time: a change of the shape the server
// sends is written into the model while the page is watched, and what was touched is
// the element that shows it -- never the card, the list or the rows beside it.
// (tick.svelte.test.ts does the same for a holding's price, and counts the code re-run.)

import { describe, it, expect } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import markets from '../../../tests/wire/scenario_markets.json'
import journal from '../../../tests/wire/case_dashboard_journal_monthly_by_symbol_queue.json'
import Watchlist from './markets/Watchlist.svelte'
import News from './markets/News.svelte'
import MarketTiles from './markets/MarketTiles.svelte'
import Trades from './Trades.svelte'
import { applyOps, type Op } from './live'
import type { Model } from './model'

const book = (from: { wire: unknown }): Model => structuredClone(from.wire) as unknown as Model

function watch(root: Node) {
  const seen: MutationRecord[] = []
  const mo = new MutationObserver((r) => seen.push(...r))
  mo.observe(root, { subtree: true, childList: true, characterData: true, attributes: true })
  return () => {
    seen.push(...mo.takeRecords())
    mo.disconnect()
    return seen
  }
}
const within = (n: Node, sel: string): Element | null => (n instanceof Element ? n : n.parentElement)?.closest(sel) ?? null
const elementsMoved = (seen: MutationRecord[]) =>
  seen.filter((m) => m.type === 'childList').flatMap((m) => [...m.addedNodes, ...m.removedNodes]).filter((n) => n.nodeType === Node.ELEMENT_NODE) as Element[]

describe('a watched listing\'s quote moves', () => {
  it('writes in that row and no other, and no row is made or taken away', () => {
    const model = $state(book(markets))
    const { container } = render(Watchlist, { props: { watchlist: model.markets.watchlist } })
    flushSync()
    const rows = [...container.querySelectorAll('.wl-row')]
    const enb = rows.find((r) => r.textContent!.includes('ENB'))!
    const stop = watch(container)
    applyOps(model, [
      ['set', ['markets', 'watchlist', { k: 'symbol', v: 'ENB' }, 'last'], 56.25],
      ['set', ['markets', 'watchlist', { k: 'symbol', v: 'ENB' }, 'percentChange'], 1.35],
    ] as Op[])
    flushSync()
    const seen = stop()
    expect(seen.length).toBeGreaterThan(0)
    expect(enb.textContent).toContain('56.25')
    expect(elementsMoved(seen)).toEqual([])
    expect(seen.filter((m) => within(m.target, '.wl-row') !== enb).map((m) => m.target.textContent)).toEqual([])
    expect([...container.querySelectorAll('.wl-row')]).toEqual(rows)
  })
})

describe('a news item arrives', () => {
  it('is one row put in; the rows already there are the same elements, untouched', () => {
    const model = $state(book(markets))
    const { container } = render(News, { props: { news: model.markets.news } })
    flushSync()
    const before = [...container.querySelectorAll('.nw-row')]
    expect(before.length).toBeGreaterThan(0)
    const fresh = { ...structuredClone($state.snapshot(model.markets.news[0])), id: 'news-new', headline: 'A headline nobody has seen yet', publishedAt: '2099-01-01T00:00:00Z' }
    const order = ['news-new', ...model.markets.news.map((n) => n.id)]
    const stop = watch(container)
    applyOps(model, [['rows', ['markets', 'news'], 'id', order, { 'news-new': fresh }]] as Op[])
    flushSync()
    const seen = stop()
    expect(container.textContent).toContain('A headline nobody has seen yet')
    // every element that came in belongs to the new row, and none went out
    const moved = elementsMoved(seen)
    expect(moved.length).toBeGreaterThan(0)
    expect(moved.filter((el) => !el.textContent!.includes('A headline nobody has seen yet'))).toEqual([])
    for (const r of before) expect(r.isConnected).toBe(true)
    // nothing written inside a row that was already there
    expect(seen.filter((m) => m.type !== 'childList' && before.some((r) => r.contains(m.target))).length).toBe(0)
  })
})

describe('a market tile\'s quote moves', () => {
  it('writes in that tile and no other', () => {
    const model = $state(book(markets))
    const { container } = render(MarketTiles, { props: { tiles: model.markets.tiles, instruments: model.markets.instruments } })
    flushSync()
    const tiles = [...container.querySelectorAll('.mt-tile')] as HTMLElement[]
    const spx = tiles.find((t) => t.dataset.sym === 'SPX')!
    const stop = watch(container)
    applyOps(model, [['set', ['markets', 'tiles', { k: 'symbol', v: 'SPX' }, 'last'], 7713.5]] as Op[])
    flushSync()
    const seen = stop()
    expect(seen.length).toBeGreaterThan(0)
    expect(seen.filter((m) => within(m.target, '.mt-tile') !== spx).map((m) => m.target.textContent)).toEqual([])
    expect([...container.querySelectorAll('.mt-tile')]).toEqual(tiles)
  })
})

describe('a trade is graded', () => {
  it('writes in that trade\'s row and no other', () => {
    const model = $state(book(journal))
    expect(model.trades.length).toBeGreaterThan(1)
    const { container } = render(Trades, { props: { trades: model.trades } })
    flushSync()
    const rows = [...container.querySelectorAll('tbody tr')] as HTMLElement[]
    const t = model.trades[0]
    const stop = watch(container)
    applyOps(model, [['set', ['trades', { k: 'id', v: t.id }, 'grade'], t.grade === 'A' ? 'B' : 'A']] as Op[])
    flushSync()
    const seen = stop()
    expect(seen.length).toBeGreaterThan(0)
    const touched = new Set(seen.map((m) => within(m.target, 'tr')))
    expect(touched.size).toBe(1)
    expect([...container.querySelectorAll('tbody tr')]).toEqual(rows)
  })
})
