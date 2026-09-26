import { describe, expect, it } from 'vitest'
import { sortRows } from './sort.svelte'

type Row = { date: string; symbol: string; account: string; amount: string | null }
const get = (r: Row, k: string) => r[k as keyof Row]

// every order the rows could arrive in
function orders<T>(xs: T[]): T[][] {
  if (xs.length <= 1) return [xs]
  return xs.flatMap((x, i) => orders([...xs.slice(0, i), ...xs.slice(i + 1)]).map((rest) => [x, ...rest]))
}

describe('sortRows', () => {
  const rows: Row[] = [
    { date: '2026-03-28', symbol: 'XEQT', account: 'TFSA', amount: '127.50' },
    { date: '2026-03-28', symbol: 'VFV', account: 'RRSP', amount: '125.80' },
    { date: '2026-03-28', symbol: 'VFV', account: 'Margin', amount: '12.00' },
    { date: '2026-04-30', symbol: 'TD', account: 'TFSA', amount: '157.50' },
  ]

  it('orders rows equal in the sorted column by the tie columns, whatever order they arrived in', () => {
    const want = sortRows(rows, 'date', 'desc', get, ['symbol', 'account']).map((r) => r.symbol + ' ' + r.account)
    expect(want).toEqual(['TD TFSA', 'VFV Margin', 'VFV RRSP', 'XEQT TFSA'])
    for (const o of orders(rows)) {
      expect(sortRows(o, 'date', 'desc', get, ['symbol', 'account']).map((r) => r.symbol + ' ' + r.account)).toEqual(want)
    }
  })

  it('ties follow the tie columns ascending in either direction of the sorted column', () => {
    const asc = sortRows(rows, 'date', 'asc', get, ['symbol', 'account']).map((r) => r.symbol + ' ' + r.account)
    expect(asc).toEqual(['VFV Margin', 'VFV RRSP', 'XEQT TFSA', 'TD TFSA'])
  })

  it('a row with nothing in the sorted column still sinks, and empties tie by the tie columns', () => {
    const withEmpty: Row[] = [...rows, { date: '2026-05-01', symbol: 'B', account: 'x', amount: null }, { date: '2026-05-01', symbol: 'A', account: 'x', amount: null }]
    for (const dir of ['asc', 'desc'] as const) {
      const out = sortRows(withEmpty, 'amount', dir, get, ['symbol', 'account'])
      expect(out.slice(-2).map((r) => r.symbol)).toEqual(['A', 'B'])
    }
  })
})
