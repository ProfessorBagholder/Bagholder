import { describe, expect, it } from 'vitest'
import { heatFromHash, tabFromHash, subFromHash } from '../router.svelte'
import { marketsOnShow, nextScope } from './heat.svelte'

describe("the heatmap's own address", () => {
  it('is Markets, with no row open', () => {
    expect(tabFromHash('#heatmap/us/equal')).toBe('markets')
    expect(subFromHash('#heatmap/us/equal')).toBeNull()
    expect(heatFromHash('#markets')).toBeNull()
  })
  it('carries the scope and the sizing, each read only when it is one', () => {
    expect(heatFromHash('#heatmap/us/equal')).toEqual({ list: ['us'], size: 'equal', seconds: null })
    expect(heatFromHash('#heatmap')).toEqual({ list: [], size: null, seconds: null })
    expect(heatFromHash('#heatmap/mars/huge')).toEqual({ list: [], size: null, seconds: null })
  })
  it('is a slideshow with several scopes and a dwell of five seconds to an hour', () => {
    expect(heatFromHash('#heatmap/holdings,us,intl/value/20')).toEqual({ list: ['holdings', 'us', 'intl'], size: 'value', seconds: 20 })
    expect(heatFromHash('#heatmap/holdings%2Cus/value/20')?.seconds).toBe(20)
    expect(heatFromHash('#heatmap/us/value/20')?.seconds).toBeNull()
    expect(heatFromHash('#heatmap/holdings,us/value/4')?.seconds).toBeNull()
    expect(heatFromHash('#heatmap/holdings,us/value/3601')?.seconds).toBeNull()
  })
})

describe("the slideshow's next scope", () => {
  const list = ['holdings', 'watchlist', 'ca', 'us']
  it('is the next with something to show, passing over an empty one and wrapping', () => {
    expect(nextScope(list, 'holdings', (u) => u !== 'watchlist')).toBe('ca')
    expect(nextScope(list, 'us', () => true)).toBe('holdings')
  })
  it('stays when nothing has anything', () => {
    expect(nextScope(list, 'holdings', () => false)).toBe('holdings')
  })
})

describe('the market universes on show', () => {
  const books = ['holdings', 'watchlist', 'both']
  const markets = ['ca', 'us', 'intl']
  it('are the one shown when it is a market, and none when it is the book', () => {
    for (const u of markets) expect(marketsOnShow(u, null)).toEqual([u])
    for (const u of books) expect(marketsOnShow(u, null)).toEqual([])
  })
  it('are, while a slideshow runs, every market it goes through, each once', () => {
    for (const u of [...books, ...markets]) {
      const shown = marketsOnShow(u, { list: [...books, ...markets, ...markets] })
      expect(shown.slice().sort()).toEqual(markets.slice().sort())
    }
    expect(marketsOnShow('holdings', { list: ['holdings', 'watchlist'] })).toEqual([])
  })
})
