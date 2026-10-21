import { describe, expect, it } from 'vitest'
import { sortRows, TIES } from './sort.svelte'

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


// #533: rows tied on the sort column come out in an order the person can see,
// whatever order the server sent them in (its ids are random beyond their time).
// Over generated rows of every table, each with many ties on every column: the
// same rows sent in the reverse order, or shuffled, sort the same, in each
// direction, wherever the table's tie columns tell the rows apart.
describe('every table breaks a tie by what the person sees', () => {
  // mulberry32: the same seed, the same draws
  let seed = 7
  const rand = (n: number) => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return (((t ^ (t >>> 14)) >>> 0) % n)
  }
  const shuffled = <T,>(rows: T[]) => {
    const out = rows.slice()
    for (let i = out.length - 1; i > 0; i--) {
      const j = rand(i + 1)
      ;[out[i], out[j]] = [out[j], out[i]]
    }
    return out
  }
  // what the holdings table sorts by, and its columns (Portfolio.svelte)
  const COLUMNS: Record<keyof typeof TIES, string[]> = {
    positions: ['symbol', 'avg', 'last', 'cost', 'mv', 'day', 'dayPct', 'unreal'],
    bySymbol: ['symbol', 'pnl', 'n', 'winRate', 'avgHold'],
    execs: ['when', 'side', 'qty', 'currency', 'price', 'amount'],
    yoc: ['symbol', 'qty', 'yoc', 'currentYield', 'annual'],
    watchlist: ['symbol', 'exchange', 'last', 'chg'],
    shorts: ['symbol', 'exchange', 'ofFloat', 'daysToCover', 'asOf'],
    news: ['when', 'news', 'symbol'],
    disclosures: ['when', 'document', 'category', 'source', 'size'],
    netWorth: ['name', 'amount'],
  }
  // what tells one row of each table from another, as the person sees it: rows are
  // generated distinct by these, never by the table's own tie columns, so a table
  // that breaks no tie fails here
  const IDENTITY: Record<keyof typeof TIES, string[]> = {
    positions: ['symbol', 'exchange', 'account'],
    bySymbol: ['symbol'],
    execs: ['when', 'side', 'qty', 'price'],
    yoc: ['symbol', 'account'],
    watchlist: ['symbol', 'exchange'],
    shorts: ['symbol', 'exchange'],
    news: ['news', 'symbol'],
    disclosures: ['document', 'source'],
    netWorth: ['name', 'institution', 'kind'],
  }
  for (const table of Object.keys(TIES) as (keyof typeof TIES)[]) {
    it(table, () => {
      for (let round = 0; round < 25; round++) {
        // a few values per field, so every column ties; the tie columns give each row its own key
        const keys = new Set<string>()
        const rows: Record<string, unknown>[] = []
        for (let tries = 0; rows.length < 12 && tries < 500; tries++) {
          const row: Record<string, unknown> = {}
          // the sort columns tie often; what identifies a row ranges wider
          for (const c of COLUMNS[table]) row[c] = rand(2) ? ['TD', 'RY', 'ENB'][rand(3)] : rand(3)
          for (const c of IDENTITY[table]) row[c] = rand(2) ? ['TD', 'RY', 'ENB', 'BNS', 'CM', 'SU'][rand(6)] : rand(6)
          const key = IDENTITY[table].map((k) => String(row[k])).join('|')
          if (keys.has(key)) continue
          keys.add(key)
          rows.push(row)
        }
        expect(rows.length, `${table}: rows generated`).toBe(12)
        const get = (r: Record<string, unknown>, k: string) => r[k]
        for (const col of COLUMNS[table]) {
          for (const dir of ['asc', 'desc'] as const) {
            const want = sortRows(rows, col, dir, get, TIES[table])
            expect(sortRows(rows.slice().reverse(), col, dir, get, TIES[table]), `${table}, ${col} ${dir}, reversed`).toEqual(want)
            expect(sortRows(shuffled(rows), col, dir, get, TIES[table]), `${table}, ${col} ${dir}, shuffled`).toEqual(want)
          }
        }
      }
    })
  }
})
