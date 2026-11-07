/// <reference types="node" />
// the stylesheet read from disk as written: vitest reads any stylesheet it imports as empty
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { TRAVEL, cubicBezier, travel } from './motion'

// the tests run from the page's folder (`npm test`)
const css = readFileSync('src/app.css', 'utf8')

describe('the app curve', () => {
  it('is the one the stylesheet names: --ease-travel is cubic-bezier of TRAVEL', () => {
    const named = /--ease-travel:\s*cubic-bezier\(([^)]*)\)/.exec(css)
    expect(named, 'app.css names --ease-travel').not.toBeNull()
    expect(named![1].split(',').map((p) => Number(p.trim()))).toEqual([...TRAVEL])
  })

  it('is solved as a browser solves a cubic-bezier', () => {
    const linear = cubicBezier(0, 0, 1, 1)
    for (const t of [0, 0.1, 0.37, 0.5, 0.9, 1]) expect(linear(t)).toBeCloseTo(t, 6)
    // CSS's `ease`, whose midpoint is published as 0.8024
    expect(cubicBezier(0.25, 0.1, 0.25, 1)(0.5)).toBeCloseTo(0.8024, 4)
    // the app's: starts at once and settles, never past its end, always moving forward
    expect(travel(0)).toBe(0)
    expect(travel(1)).toBe(1)
    expect(travel(0.1)).toBeGreaterThan(0.25)
    let last = 0
    for (let i = 1; i <= 100; i++) {
      const v = travel(i / 100)
      expect(v).toBeGreaterThanOrEqual(last)
      expect(v).toBeLessThanOrEqual(1)
      last = v
    }
  })
})
