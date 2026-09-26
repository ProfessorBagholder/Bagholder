import { describe, expect, it } from 'vitest'
import { abs, absBelow, cmp, dec, digits, sign, waits } from './dec'

describe('exact decimals', () => {
  it('refuses text that is not a decimal', () => {
    for (const bad of ['', '1e5', '1,000', ' 1', '.5', '1.', 'NaN', '--1']) expect(() => dec(bad), bad).toThrow()
    expect(dec('-0.05')).toBe('-0.05')
  })

  it('orders decimals exactly, beyond what a float can hold', () => {
    const d = (s: string) => dec(s)
    expect(cmp(d('0.1'), d('0.10'))).toBe(0)
    expect(cmp(d('-0'), d('0'))).toBe(0)
    expect(cmp(d('9007199254740993'), d('9007199254740992'))).toBe(1) // equal as floats
    expect(cmp(d('0.30000000000000001'), d('0.3'))).toBe(1)
    expect(cmp(d('-2'), d('-10'))).toBe(1)
    expect(cmp(d('-0.5'), d('0.25'))).toBe(-1)
    const sorted = ['10', '-3', '2.5', '-3.25', '0', '2.50', '100'].map(d).sort(cmp)
    expect(sorted).toEqual(['-3.25', '-3', '0', '2.5', '2.50', '10', '100'])
  })

  it('knows a sign and a magnitude from the text', () => {
    expect(sign(dec('0.000'))).toBe(0)
    expect(sign(dec('-0.01'))).toBe(-1)
    expect(abs(dec('-12.5'))).toBe('12.5')
    expect(absBelow(dec('-0.009'), '0.01')).toBe(true)
    expect(absBelow(dec('0.01'), '0.01')).toBe(false)
  })

  it('writes the exact value it is given, rounded only for display', () => {
    expect(digits(dec('123456789012345678.125'), 2)).toBe('123,456,789,012,345,678.13')
    expect(digits(dec('-1247.4'), 2)).toBe('1,247.40')
    expect(digits(dec('0.1234567'), 2, 6)).toBe('0.123457')
  })

  it('tells a figure that waits from one that is stated', () => {
    expect(waits({ gaps: ['rate-pending'] })).toBe(true)
    expect(waits(dec('1'))).toBe(false)
  })
})
