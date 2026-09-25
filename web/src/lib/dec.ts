// Exact decimals as the server sends them (docs/plans/stage-3c-switch.md, §4 and §5).
//
// A money amount, a quantity or a price arrives as the exact decimal text the
// engine holds ("1247.41", "-0.05", "12"). The page never makes a float of one to
// show it or to order a list by it: it is formatted from the text
// (`Intl.NumberFormat.prototype.format` given a string formats the exact value
// the string states) and compared digit by digit here. The page does no money
// arithmetic; the one place a Dec becomes a number is a chart's plotted
// coordinate (`plot`), never shown and never summed.

/** An exact decimal, as text. */
export type Dec = string & { readonly __dec: unique symbol }

/** A figure the server could not state: the gaps it waits on, each a word SPEC.md lists. */
export type Waits = { gaps: string[] }

/** A figure: its value, or what it waits on. */
export type Fig<T> = T | Waits

export function waits<T>(f: Fig<T>): f is Waits {
  return typeof f === 'object' && f !== null && Array.isArray((f as Waits).gaps)
}

const SHAPE = /^-?\d+(\.\d+)?$/

/** Text the server sent as a decimal; anything else is refused, never coerced. */
export function dec(s: string): Dec {
  if (!SHAPE.test(s)) throw new Error(`not a decimal: ${JSON.stringify(s)}`)
  return s as Dec
}

type Parts = { neg: boolean; int: string; frac: string }

function parts(d: Dec): Parts {
  const neg = d.startsWith('-')
  const body = neg ? d.slice(1) : d
  const [int, frac = ''] = body.split('.')
  const i = int.replace(/^0+(?=\d)/, '')
  const f = frac.replace(/0+$/, '')
  const zero = i === '0' && f === ''
  return { neg: neg && !zero, int: i, frac: f }
}

/** -1, 0 or 1. */
export function sign(d: Dec): -1 | 0 | 1 {
  const p = parts(d)
  if (p.int === '0' && p.frac === '') return 0
  return p.neg ? -1 : 1
}

function cmpAbs(a: Parts, b: Parts): -1 | 0 | 1 {
  if (a.int.length !== b.int.length) return a.int.length < b.int.length ? -1 : 1
  if (a.int !== b.int) return a.int < b.int ? -1 : 1
  const n = Math.max(a.frac.length, b.frac.length)
  const fa = a.frac.padEnd(n, '0')
  const fb = b.frac.padEnd(n, '0')
  return fa === fb ? 0 : fa < fb ? -1 : 1
}

/** The order of two decimals, exactly. */
export function cmp(a: Dec, b: Dec): -1 | 0 | 1 {
  const pa = parts(a)
  const pb = parts(b)
  const sa = sign(a)
  const sb = sign(b)
  if (sa !== sb) return sa < sb ? -1 : 1
  const m = cmpAbs(pa, pb)
  return (sa < 0 ? -m : m) as -1 | 0 | 1
}

/** The decimal without its sign. */
export function abs(d: Dec): Dec {
  return (d.startsWith('-') ? d.slice(1) : d) as Dec
}

/** Whether `|d|` is less than `limit` (a decimal written as text). */
export function absBelow(d: Dec, limit: string): boolean {
  return cmpAbs(parts(d), parts(limit as Dec)) < 0
}

/** A chart's plotted coordinate: the one place a Dec becomes a number. Never shown, never summed. */
export function plot(d: Dec): number {
  return Number(d)
}

const formats = new Map<string, Intl.NumberFormat>()

/** `|d|` written with `min`..`max` decimals and en-US grouping, from the exact text. */
export function digits(d: Dec, min: number, max: number = min): string {
  const key = min + ':' + max
  let f = formats.get(key)
  if (!f) {
    f = new Intl.NumberFormat('en-US', { minimumFractionDigits: min, maximumFractionDigits: max })
    formats.set(key, f)
  }
  // a string is formatted as the exact value it states (ECMA-402, Intl.NumberFormat v3)
  return f.format(abs(d) as unknown as number)
}
