import { expect, test } from '@playwright/test'
import { figures, waits } from './helpers'

// The figures each screen shows agree with one another, as SPEC.md defines them:
// a trade's Hold and its P&L in CAD (§3 Trades), the Dashboard's Realized P&L and
// win counts (§6 Dashboard) summed from the trades, and each holding's Market,
// Book and unrealized P&L (§3 Positions). Held as rules over whatever the book
// holds, never over one trade of it; exact on the decimal text the server sends.

/** An exact decimal: `units × 10^-scale`. */
type Exact = { units: bigint; scale: number }

function exact(text: string): Exact {
  expect(text, 'a decimal written as text').toMatch(/^-?\d+(\.\d+)?$/)
  const [whole, frac = ''] = text.replace('-', '').split('.')
  const units = BigInt(whole + frac)
  return { units: text.startsWith('-') ? -units : units, scale: frac.length }
}
const at = (a: Exact, scale: number) => a.units * 10n ** BigInt(scale - a.scale)
const add = (a: Exact, b: Exact): Exact => {
  const s = Math.max(a.scale, b.scale)
  return { units: at(a, s) + at(b, s), scale: s }
}
const sub = (a: Exact, b: Exact): Exact => add(a, { units: -b.units, scale: b.scale })
const mul = (a: Exact, b: Exact): Exact => ({ units: a.units * b.units, scale: a.scale + b.scale })
const same = (a: Exact, b: Exact) => sub(a, b).units === 0n
const abs = (a: Exact): Exact => ({ units: a.units < 0n ? -a.units : a.units, scale: a.scale })
const atMost = (a: Exact, b: Exact) => sub(b, a).units >= 0n
const text = (a: Exact) => {
  const neg = a.units < 0n
  const d = (neg ? -a.units : a.units).toString().padStart(a.scale + 1, '0')
  return (neg ? '-' : '') + (a.scale ? d.slice(0, -a.scale) + '.' + d.slice(-a.scale) : d)
}
const ZERO: Exact = { units: 0n, scale: 0 }
const days = (from: string, to: string) => Math.round((Date.parse(to + 'T00:00:00Z') - Date.parse(from + 'T00:00:00Z')) / 86_400_000)

/* eslint-disable @typescript-eslint/no-explicit-any */

test('each trade: a positive quantity, Hold the calendar days it spans, and its CAD P&L its own where it trades in CAD', async ({ request }) => {
  const m = await figures(request)
  expect(m.trades.length).toBeGreaterThan(0)
  for (const t of m.trades as any[]) {
    const who = `${t.symbol} opened ${t.entryDate}`
    expect(exact(t.qty).units > 0n, who).toBe(true)
    // SPEC.md §3: calendar days from Open to Close, or to today while open
    expect(t.holdDays, who).toBe(days(t.entryDate, t.status === 'closed' ? t.exitDate : m.today))
    if (t.currency === 'CAD' && !waits(t.pnl)) expect(t.pnlCad, who).toBe(t.pnl)
    if (t.status === 'closed' && !waits(t.pnl)) {
      // a closed trade's P&L % is stated, and on the same side of zero as its P&L
      expect(typeof t.pnlPct, who).toBe('number')
      expect(Math.sign(t.pnlPct), who).toBe(Math.sign(Number(exact(t.pnl).units)))
    }
  }
})

test("the Dashboard's Realized P&L is the trades' realized P&L in CAD, and its counts are the closed trades'", async ({ request }) => {
  const m = await figures(request)
  const k = m.kpi
  const trades = m.trades as any[]
  const stated = trades.filter((t) => !waits(t.pnlCad))
  // every sale counts, open trades' partial sales included; a part that waits is left out and counted
  expect(k.realizedLeftOut).toBe(trades.filter((t) => waits(t.pnlCad)).length)
  expect(text(exact(k.realized))).toBe(text(stated.reduce((s, t) => add(s, exact(t.pnlCad)), ZERO)))
  const closed = trades.filter((t) => t.status === 'closed' && !waits(t.pnlCad)).map((t) => exact(t.pnlCad).units)
  expect([k.count, k.wins, k.losses, k.breakeven]).toEqual([closed.length, closed.filter((u) => u > 0n).length, closed.filter((u) => u < 0n).length, closed.filter((u) => u === 0n).length])
  expect(k.winRate).toBeCloseTo(k.wins / k.count, 12)
})

test("each holding: its unrealized P&L is Market less Book (reversed on a short), and Market and Book price the same units", async ({ request }) => {
  const m = await figures(request)
  const held = (m.positions as any[]).filter((p) => !['qty', 'avg', 'cost', 'last', 'mv', 'unreal'].some((f) => waits(p[f])))
  expect(held.length).toBeGreaterThan(0)
  for (const p of held) {
    const who = `${p.symbol} in ${p.account}`
    const [qty, avg, cost, last, mv, unreal] = ['qty', 'avg', 'cost', 'last', 'mv', 'unreal'].map((f) => exact(p[f]))
    expect(text(unreal), who).toBe(text(p.short ? sub(cost, mv) : sub(mv, cost)))
    // Market is units × last × the contract's size: the size is a whole number
    const priced = mul(qty, last)
    const size = Math.round(Number(text(mv)) / Number(text(priced)))
    expect(size, who).toBeGreaterThanOrEqual(1)
    const n: Exact = { units: BigInt(size), scale: 0 }
    expect(same(mv, mul(priced, n)), who).toBe(true)
    // Book is units × the average cost × the same size, to the average's own rounding
    const off = sub(cost, mul(mul(qty, avg), n))
    const unit: Exact = { units: 1n, scale: avg.scale }
    const bound = mul(mul(qty, n), unit)
    expect(atMost(abs(off), bound), who).toBe(true)
  }
})
