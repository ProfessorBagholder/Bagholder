import { describe, expect, it } from 'vitest'
import { cubicBezier } from './motion'

describe('a CSS curve in script', () => {
  it('is solved as a browser solves a cubic-bezier', () => {
    const linear = cubicBezier(0, 0, 1, 1)
    for (const t of [0, 0.1, 0.37, 0.5, 0.9, 1]) expect(linear(t)).toBeCloseTo(t, 6)
    // CSS's `ease`, whose midpoint is published as 0.8024
    expect(cubicBezier(0.25, 0.1, 0.25, 1)(0.5)).toBeCloseTo(0.8024, 4)
    // its ends are its ends
    expect(cubicBezier(0.2, 0, 0, 1)(0)).toBe(0)
    expect(cubicBezier(0.2, 0, 0, 1)(1)).toBe(1)
  })
})
