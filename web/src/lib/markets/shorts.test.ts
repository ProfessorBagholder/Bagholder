// SPEC §4 Markets, Short interest, on the card itself: a ticker that is none of the
// listings in scope is shown as a row of its own, whatever else the words match.

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync, tick } from 'svelte'
import type { ShortsFeedRow } from '../model'

type FeedDoc = { ok: boolean; rows: ShortsFeedRow[]; reading: boolean }
let feedHolder: { data: FeedDoc | null } | null = null
vi.mock(import('../live'), async (original) => ({
  ...(await original()),
  watchDoc: ((_key: string, _params: unknown, holder: { data: FeedDoc | null }) => {
    feedHolder = holder
    return () => {}
  }) as never,
}))
// a ticker read on its own: every one answers with a row of its own
const asked: string[] = []
vi.mock(import('../api'), async (original) => ({
  ...(await original()),
  call: ((_key: string, input: { query: { symbol: string } }) => {
    asked.push(input.query.symbol)
    return Promise.resolve({ ok: true, covered: true, shorts: row(input.query.symbol, 5, { exchange: 'NYSE' }) })
  }) as never,
}))

const { default: Shorts } = await import('./Shorts.svelte')

function row(symbol: string, ofFloat: number, o: Partial<{ held: boolean; watched: boolean; exchange: string; name: string }> = {}): ShortsFeedRow {
  return {
    symbol, exchange: o.exchange ?? 'NASDAQ', name: o.name ?? symbol + ' Inc.', held: !!o.held, watched: !!o.watched, positionId: null,
    shares: 1000, ofFloat, daysToCover: 1, volumePct: 1, asOf: '2026-09-18',
  } as unknown as ShortsFeedRow
}

const symbols = (c: HTMLElement) => [...c.querySelectorAll('.si-row > div:first-child > div:first-child')].map((e) => e.textContent)

async function show(rows: ShortsFeedRow[], scope: string) {
  localStorage.setItem('bh2.shorts', scope)
  const view = render(Shorts)
  flushSync()
  feedHolder!.data = { ok: true, reading: false, rows }
  flushSync()
  return view.container
}

async function type(c: HTMLElement, text: string) {
  const box = c.querySelector('[aria-label="Search short interest"]') as HTMLInputElement
  box.value = text
  box.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
  await vi.advanceTimersByTimeAsync(500)
  await tick()
  flushSync()
}

beforeEach(() => {
  vi.useFakeTimers()
  asked.length = 0
})
afterEach(() => vi.useRealTimers())

describe('a ticker searched for', () => {
  it('outside the scope, is shown from the book\'s own row, in every scope that leaves it out', async () => {
    const book = [row('HH', 40, { held: true }), row('WW', 80, { watched: true })]
    for (const [scope, outside] of [['holdings', 'WW'], ['watchlist', 'HH']]) {
      const c = await show(book, scope)
      await type(c, outside)
      expect(symbols(c)).toEqual([outside])
      expect(asked).toEqual([])
    }
  })

  it('among prefix matches, is added as a row of its own beside them', async () => {
    const c = await show([row('TD', 40, { held: true }), row('TRP', 20, { held: true })], 'all')
    await type(c, 'T')
    expect(asked).toEqual(['T'])
    expect(symbols(c)?.sort()).toEqual(['T', 'TD', 'TRP'])
  })

  it('that is in scope, is not read again nor shown twice', async () => {
    const c = await show([row('TD', 40, { held: true }), row('TDB', 20, { held: true })], 'holdings')
    await type(c, 'TD')
    expect(asked).toEqual([])
    expect(symbols(c)?.sort()).toEqual(['TD', 'TDB'])
  })
})
