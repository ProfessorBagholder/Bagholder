import { describe, expect, it } from 'vitest'
import { niceStep, niceTicks } from './ticks'

// a step is 1, 2 or 5 times a power of ten
const nice = (step: number) => {
  const p = Math.pow(10, Math.floor(Math.log10(step)))
  return [1, 2, 5, 10].some((f) => Math.abs(step / p - f) < 1e-9)
}

describe('an axis in round steps', () => {
  it('reads round for the spans a chart meets', () => {
    expect(niceTicks(0, 10865).ticks).toEqual([12000, 10000, 8000, 6000, 4000, 2000, 0])
    expect(niceTicks(0, 680).ticks).toEqual([800, 600, 400, 200, 0])
    expect(niceTicks(0, 13745).ticks).toEqual([15000, 10000, 5000, 0])
    expect(niceTicks(-1200, 3400).ticks).toEqual([4000, 3000, 2000, 1000, 0, -1000, -2000])
    expect(niceTicks(-5000, 41000).ticks).toEqual([50000, 40000, 30000, 20000, 10000, 0, -10000])
    expect(niceTicks(0, 0).ticks).toEqual([1, 0])
  })

  // generated spans across ten orders of magnitude, signs either side of zero
  it('for any span: whole round steps, zero among them, the data inside, a handful of lines', () => {
    let seed = 7
    const rand = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648)
    for (let i = 0; i < 5000; i++) {
      const scale = Math.pow(10, Math.floor(rand() * 10) - 2)
      const hi = rand() * 10 * scale
      const lo = rand() < 0.3 ? -rand() * 10 * scale : 0
      const t = niceTicks(lo, hi)
      expect(nice(t.step), `${lo} ${hi}: step ${t.step}`).toBe(true)
      expect(t.hi).toBeGreaterThanOrEqual(hi)
      expect(t.lo).toBeLessThanOrEqual(lo)
      expect(t.ticks).toContain(0)
      expect(t.ticks.length).toBeGreaterThanOrEqual(2)
      expect(t.ticks.length).toBeLessThanOrEqual(9)
      for (const v of t.ticks) expect(Math.abs(v / t.step - Math.round(v / t.step)), `${v} on ${t.step}`).toBeLessThan(1e-6)
    }
  })

  it('a step is round whatever it divides', () => {
    for (const [lo, hi] of [[0, 1], [0, 7], [0, 233], [0, 1e9], [-5, 5], [0, 0.03]] as const) expect(nice(niceStep(lo, hi, 4))).toBe(true)
  })
})
