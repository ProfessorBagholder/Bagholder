import { describe, expect, it } from 'vitest'
import { bookListing, directoryListing, tickerKey } from './newsChip'

describe('a word typed in the News box', () => {
  it('can be a ticker only as one short word', () => {
    expect(tickerKey(' abc ')).toBe('ABC')
    expect(tickerKey('two words')).toBeNull()
    expect(tickerKey('seventeen')).toBeNull()
    expect(tickerKey('')).toBeNull()
  })

  it("names the book's listing by its ticker, never a contract written on it", () => {
    const rows = [
      { symbol: 'ABC 20NOV26 3.00 CALL', exchange: 'OPRA' },
      { symbol: 'ABC.TO', exchange: 'TSX', name: 'Abc Corp', currency: 'CAD' },
    ]
    expect(bookListing('ABC', rows)).toEqual({ symbol: 'ABC', exchange: 'TSX', name: 'Abc Corp', currency: 'CAD' })
    expect(bookListing('ABD', rows)).toBeNull()
    expect(bookListing('ABC', rows.slice(0, 1))).toBeNull()
  })

  it("names a directory's listing only where the directory states that ticker: a name match is a text search", () => {
    const matches = [{ symbol: 'XYZW', exchange: 'NASDAQ', name: 'Word Holdings' }, { symbol: 'WORD', exchange: 'NYSE', name: 'Other' }]
    expect(directoryListing('WORD', matches)?.exchange).toBe('NYSE')
    expect(directoryListing('WOR', matches)).toBeNull()
    expect(directoryListing('ANY', [])).toBeNull()
  })
})
