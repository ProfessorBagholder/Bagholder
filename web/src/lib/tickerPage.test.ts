import { describe, expect, it } from 'vitest'
import { tickerPage } from './tickerPage'

describe("a ticker chosen in the filter's search", () => {
  const holdings = [{ instrument: 'i-held', id: 'pos-1' }]

  it('opens its holding where the book holds it, and its listing otherwise', () => {
    expect(tickerPage('i-held', holdings)).toEqual({ holding: 'pos-1' })
    expect(tickerPage('i-sold', holdings)).toBe('markets')
    expect(tickerPage('i-held', [])).toBe('markets')
  })

  it('waits while the holdings are not read: not known is never not held', () => {
    expect(tickerPage('i-held', null)).toBe('wait')
    expect(tickerPage('i-held', undefined)).toBe('wait')
  })

  it('opens a listing a directory found under Markets at once, the holdings read or not', () => {
    expect(tickerPage('', null)).toBe('markets')
    expect(tickerPage('', holdings)).toBe('markets')
  })
})
