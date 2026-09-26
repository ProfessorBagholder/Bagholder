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
import { applyOps, type Op } from './live.svelte'
import type { Markets, Trade } from './model'

// the old recorded model's parts, as the Markets tab's and the Trades list's documents carry them
type Model = { markets: Markets; trades: Trade[] }
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
    const model = $state({ total: book(journal).trades.length, trades: book(journal).trades })
    expect(model.trades.length).toBeGreaterThan(1)
    const { container } = render(Trades, { props: { doc: model } })
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

// The server's own documents for a month of one account's recorded replies (the trades
// and the Cashflow as a page subscribing to them is sent them).
import pulled from './fixtures/figures_pulled_month.json'
import Cashflow from './Cashflow.svelte'
import { reconcile } from './live.svelte'
import { ROW_KEYS } from './generated/keys'
import type { TradesDoc, CashflowDoc } from './model'

describe('a new trade arrives', () => {
  it('is one row put in; the rows already there are the same elements, untouched', () => {
    const doc = $state(structuredClone(pulled.trades) as unknown as TradesDoc)
    const { container } = render(Trades, { props: { doc } })
    flushSync()
    const before = [...container.querySelectorAll('tbody tr')]
    const fresh = { ...structuredClone($state.snapshot(doc.trades[0])), id: 'trade-new', symbol: 'NEWCO' }
    const stop = watch(container)
    applyOps(doc, [['set', ['total'], doc.total + 1], ['rows', ['trades'], 'id', ['trade-new', ...doc.trades.map((t) => t.id)], { 'trade-new': fresh }]] as Op[])
    flushSync()
    const seen = stop()
    expect(container.textContent).toContain('NEWCO')
    const moved = elementsMoved(seen)
    expect(moved.length).toBeGreaterThan(0)
    expect(moved.filter((el) => !el.textContent!.includes('NEWCO'))).toEqual([])
    for (const r of before) expect(r.isConnected).toBe(true)
    expect(seen.filter((m) => m.type !== 'childList' && before.some((r) => r.contains(m.target))).length).toBe(0)
  })
})

describe('a correction moves one trade\'s P&L', () => {
  it('writes in that trade\'s row and no other', () => {
    const doc = $state(structuredClone(pulled.trades) as unknown as TradesDoc)
    const { container } = render(Trades, { props: { doc } })
    flushSync()
    const rows = [...container.querySelectorAll('tbody tr')]
    const t = doc.trades.find((x) => typeof x.pnl === 'string')!
    const stop = watch(container)
    applyOps(doc, [
      ['set', ['trades', { k: 'id', v: t.id }, 'pnl'], '12345.67'],
      ['set', ['trades', { k: 'id', v: t.id }, 'pnlCad'], '12345.67'],
    ] as Op[])
    flushSync()
    const seen = stop()
    expect(seen.length).toBeGreaterThan(0)
    expect(new Set(seen.map((m) => within(m.target, 'tr'))).size).toBe(1)
    expect(elementsMoved(seen)).toEqual([])
    expect([...container.querySelectorAll('tbody tr')]).toEqual(rows)
  })
})

describe('a distribution is paid', () => {
  it('is one row put in the history; the rows already there are the same elements, untouched', () => {
    const doc = $state(structuredClone(pulled.cashflow) as unknown as CashflowDoc)
    const { container } = render(Cashflow, { props: { model: doc } })
    flushSync()
    const history = [...container.querySelectorAll('h5')].find((h) => h.textContent === 'Distribution history')!.closest('.card')!
    const before = [...history.querySelectorAll('tbody tr')]
    expect(before.length).toBeGreaterThan(0)
    const fresh = { ...structuredClone($state.snapshot(doc.cashflow.rows[0])), id: 'paid-new', symbol: 'PAYCO' }
    const stop = watch(history)
    applyOps(doc, [['set', ['rowsTotal'], doc.rowsTotal + 1], ['rows', ['cashflow', 'rows'], 'id', ['paid-new', ...doc.cashflow.rows.map((r) => r.id)], { 'paid-new': fresh }]] as Op[])
    flushSync()
    const seen = stop()
    expect(history.textContent).toContain('PAYCO')
    expect(elementsMoved(seen).filter((el) => !el.textContent!.includes('PAYCO'))).toEqual([])
    for (const r of before) expect(r.isConnected).toBe(true)
  })
})

describe('the filters change', () => {
  it('the rows that stay are the same elements, untouched; only those that go are taken away', () => {
    const doc = $state(structuredClone(pulled.trades) as unknown as TradesDoc)
    const { container } = render(Trades, { props: { doc } })
    flushSync()
    const rows = [...container.querySelectorAll('tbody tr')] as HTMLElement[]
    // the list under the new filters: every other trade, as the server sends it whole
    const kept = structuredClone(pulled.trades) as unknown as TradesDoc
    kept.trades = kept.trades.filter((_, i) => i % 2 === 0)
    kept.total = kept.trades.length
    const staying = rows.filter((_, i) => i % 2 === 0)
    const stop = watch(container)
    reconcile(doc as unknown as Record<string, unknown>, kept as unknown as Record<string, unknown>, ROW_KEYS.trades)
    flushSync()
    const seen = stop()
    expect([...container.querySelectorAll('tbody tr')]).toEqual(staying)
    expect(seen.filter((m) => m.type !== 'childList' && staying.some((r) => r.contains(m.target))).length).toBe(0)
  })
})
