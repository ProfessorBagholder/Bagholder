// SPEC §4 Markets, Short interest, on the card itself: a ticker that is none of the
// listings in scope is shown as a row of its own, whatever else the words match.

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync, tick } from 'svelte'
import type { ShortsFeedRow } from '../model'

type FeedDoc = { ok: boolean; rows: ShortsFeedRow[]; reading: boolean }
let feedHolder: { data: FeedDoc | null } | null = null
vi.mock(import('../live.svelte'), async (original) => ({
  ...(await original()),
  watchDoc: ((_key: string, _params: unknown, holder: { data: FeedDoc | null }) => {
    feedHolder = holder
    return () => {}
  }) as never,
}))
// a ticker typed resolves to the listings `listedOn` names (NYSE where none is named),
// and each listing read on its own answers with a row of its own
const asked: string[] = []
let listedOn: Record<string, string[]> = {}
// the directories that did not answer, and the venues whose read fails
let unread: string[] = []
let failsOn: string[] = []
vi.mock(import('../api'), async (original) => ({
  ...(await original()),
  call: ((key: string, input: { query: { symbol: string; exchange?: string } }) => {
    const s = input.query.symbol
    if (key === 'GET /api/listings') {
      const on = listedOn[s] ?? ['NYSE']
      const key = (ex: string) => s + '@' + (ex === 'TSX' ? 'XTSE' : 'US')
      return Promise.resolve({ ok: true, unread, listings: on.map((ex) => ({ listing: key(ex), symbol: s, name: s + ' Inc.', exchange: ex, currency: 'USD', held: false })) })
    }
    asked.push(s + '@' + input.query.exchange)
    if (failsOn.includes(input.query.exchange ?? '')) return Promise.resolve({ ok: false, error: 'TMX could not be reached.' })
    return Promise.resolve({ ok: true, covered: true, shorts: row(s, 5, { exchange: input.query.exchange }) })
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
  listedOn = {}
  unread = []
  failsOn = []
})
afterEach(() => vi.useRealTimers())

describe('a ticker searched for', () => {
  it('outside the scope, is shown from the book\'s own row, in every scope that leaves it out', async () => {
    const book = [row('HH', 40, { held: true }), row('WW', 80, { watched: true })]
    listedOn = { HH: ['NASDAQ'], WW: ['NASDAQ'] }
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
    expect(asked).toEqual(['T@NYSE'])
    expect(symbols(c)?.sort()).toEqual(['T', 'TD', 'TRP'])
  })

  it('that is in scope, is not read again nor shown twice', async () => {
    const c = await show([row('TD', 40, { held: true }), row('TDB', 20, { held: true })], 'holdings')
    listedOn = { TD: ['NASDAQ'] }
    await type(c, 'TD')
    expect(asked).toEqual([])
    expect(symbols(c)?.sort()).toEqual(['TD', 'TDB'])
  })

  it('on two venues, is two rows, each read on its own venue and the one held not read again', async () => {
    const c = await show([row('RY', 40, { held: true, exchange: 'NYSE' })], 'all')
    listedOn = { RY: ['NYSE', 'TSX'] }
    await type(c, 'RY')
    expect(asked).toEqual(['RY@TSX'])
    const exchanges = [...c.querySelectorAll('.si-row > div:nth-child(2)')].map((e) => e.textContent)
    expect(symbols(c)).toEqual(['RY', 'RY'])
    expect(exchanges.sort()).toEqual(['NYSE', 'TSX'])
  })

  it('on a venue whose read failed, says so beside the row the other venue gave', async () => {
    const c = await show([], 'all')
    listedOn = { RZ: ['NYSE', 'TSX'] }
    failsOn = ['TSX']
    await type(c, 'RZ')
    expect(symbols(c)).toEqual(['RZ'])
    expect(c.querySelector('.status-err')?.textContent).toBe('Could not read short interest for RZ: TMX could not be reached.')
  })

  it('with a directory that did not answer, shows what was found beside the failure', async () => {
    const c = await show([], 'all')
    listedOn = { RQ: ['NYSE'] }
    unread = ["The TSX's directory could not be read: refused"]
    await type(c, 'RQ')
    expect(symbols(c)).toEqual(['RQ'])
    expect(c.querySelector('.status-err')?.textContent).toContain("The TSX's directory could not be read")
  })

  it('found on no venue while a directory did not answer, says the failure, never that no listing is by that name', async () => {
    const c = await show([], 'all')
    listedOn = { RX: [] }
    unread = ["The TSX's directory could not be read: refused"]
    await type(c, 'RX')
    expect(c.textContent).toContain("Could not read short interest for RX: The TSX's directory could not be read")
    expect(c.textContent).not.toContain('No listing by that name')
  })
})
