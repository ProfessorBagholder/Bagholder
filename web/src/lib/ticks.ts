// A chart's axis in round steps: the "nice numbers" of Heckbert ("Nice Numbers
// for Graph Labels", Graphics Gems, 1990), as D3 picks them (d3-array's
// tickIncrement): a step of 1, 2 or 5 times a power of ten, about `count` of
// them across the span, and the span widened to whole steps.

const E10 = Math.sqrt(50)
const E5 = Math.sqrt(10)
const E2 = Math.sqrt(2)

/** The round step for about `count` steps from `lo` to `hi`. */
export function niceStep(lo: number, hi: number, count: number): number {
  const raw = (hi - lo) / Math.max(1, count)
  if (!(raw > 0) || !Number.isFinite(raw)) return 1
  const power = Math.floor(Math.log10(raw))
  const error = raw / Math.pow(10, power)
  const factor = error >= E10 ? 10 : error >= E5 ? 5 : error >= E2 ? 2 : 1
  return factor * Math.pow(10, power)
}

/**
 * The axis for values from `lo` to `hi` (zero included): its ends widened to
 * whole round steps, and every step's value from the top down. A span of
 * nothing is one step of one.
 */
export function niceTicks(lo: number, hi: number, count = 4): { lo: number; hi: number; step: number; ticks: number[] } {
  let a = Math.min(lo, 0)
  let b = Math.max(hi, 0)
  // nothing to show: one whole step
  if (!(b > a)) return { lo: a, hi: a + 1, step: 1, ticks: [a + 1, a] }
  // the step from the data's own span, then the ends widened to whole steps
  // (d3-array's ticks); the step is not picked again from the widened span,
  // which would only add room the data does not use
  const step = niceStep(a, b, count)
  a = Math.floor(a / step) * step
  b = Math.ceil(b / step) * step
  const n = Math.round((b - a) / step)
  // each value rounded at the step's own places, so no float residue reads as a cent
  const places = Math.max(0, -Math.floor(Math.log10(step)))
  const ticks = Array.from({ length: n + 1 }, (_, i) => Number((b - i * step).toFixed(places)))
  return { lo: a, hi: b, step, ticks }
}
