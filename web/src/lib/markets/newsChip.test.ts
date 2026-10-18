import { describe, expect, it } from 'vitest'
import { chipKey, tickerKey } from './newsChip'

describe('a word typed in the News box', () => {
  it('can be a ticker only as one short word', () => {
    expect(tickerKey(' abc ')).toBe('ABC')
    expect(tickerKey('two words')).toBeNull()
    expect(tickerKey('seventeen')).toBeNull()
    expect(tickerKey('')).toBeNull()
  })

  it('is a listing by its key: the same ticker on two venues is two chips, and one with no venue is none', () => {
    expect(chipKey({ symbol: 'ABC', exchange: 'TSX' })).toBe('ABC@XTSE')
    expect(chipKey({ symbol: 'ABC', exchange: 'NYSE' })).toBe('ABC@US')
    expect(chipKey({ symbol: 'ABC', exchange: '' })).toBe('')
  })
})
