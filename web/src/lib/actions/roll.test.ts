import { describe, expect, it } from 'vitest'
import { comparable } from './roll'

describe('which figures roll into one another', () => {
  it('the same shape with other digits', () => {
    expect(comparable('$1,234.50', '$1,239.75')).toBe(true)
    expect(comparable('−4.2%', '−3.9%')).toBe(true)
  })
  it('not a figure that did not move, grew a digit, changed sign or has no digits', () => {
    expect(comparable('$12.00', '$12.00')).toBe(false)
    expect(comparable('$99.00', '$100.00')).toBe(false)
    expect(comparable('+4.2%', '−4.2%')).toBe(false)
    expect(comparable('—', '∞')).toBe(false)
    expect(comparable('', '$1.00')).toBe(false)
    expect(comparable('1,000', '10.00')).toBe(false)
  })
})
