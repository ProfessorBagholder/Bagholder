import { cmp as decCmp, waits, type Dec, type Fig } from './dec'
// Sort state per table and the shared row sorter, ported from ledger.html
// (state.sort + sortRows + the 'sort' action). Reactive $state so a header click
// re-sorts only the table that reads it.

export type Dir = 'asc' | 'desc'
export interface SortState {
  key: string
  dir: Dir
}

const DEFAULTS: Record<string, SortState> = {
  trades: { key: 'exitDate', dir: 'desc' },
  positions: { key: 'unreal', dir: 'desc' },
  bySymbol: { key: 'pnl', dir: 'desc' },
  execs: { key: 'when', dir: 'desc' },
  disc: { key: 'date', dir: 'desc' },
  ndisc: { key: 'when', dir: 'desc' },
  yoc: { key: 'ytd', dir: 'desc' },
  cash: { key: 'date', dir: 'desc' },
  orders: { key: 'placed', dir: 'desc' },
  watchlist: { key: 'added', dir: 'desc' },
  news: { key: 'when', dir: 'desc' },
  shorts: { key: 'ofFloat', dir: 'desc' },
}

function load(): Record<string, SortState> {
  try {
    const raw = localStorage.getItem('bh2.sort')
    if (raw) {
      const kept = { ...DEFAULTS, ...JSON.parse(raw) } as Record<string, SortState>
      // a column the table no longer has (the trailing twelve months the earlier page sorted by)
      if (kept.yoc?.key === 'ttm') kept.yoc = { ...DEFAULTS.yoc }
      return kept
    }
  } catch {
    /* ignore */
  }
  return { ...DEFAULTS }
}

export const sort = $state<Record<string, SortState>>(load())

// A header click: flip direction on the active column, else make this the active
// column, descending. Persisted so a reload keeps the choice.
export function toggleSort(table: string, key: string): void {
  const s = sort[table]
  if (s.key === key) s.dir = s.dir === 'desc' ? 'asc' : 'desc'
  else {
    s.key = key
    s.dir = 'desc'
  }
  try {
    localStorage.setItem('bh2.sort', JSON.stringify(sort))
  } catch {
    /* ignore */
  }
}

const DECIMAL = /^-?\d+(\.\d+)?$/

// A figure that waits on something has no value to order by: it sinks like an empty cell.
const value = (v: unknown): unknown => (waits(v as Fig<unknown>) ? null : v)

function cmp(a: unknown, b: unknown): number {
  if (a == null && b == null) return 0
  if (a == null) return 1
  if (b == null) return -1
  if (typeof a === 'number' && typeof b === 'number') return a - b
  // exact decimals, ordered digit by digit, never through a float
  if (typeof a === 'string' && typeof b === 'string' && DECIMAL.test(a) && DECIMAL.test(b)) return decCmp(a as Dec, b as Dec)
  return String(a).localeCompare(String(b))
}

// Sort a copy; a row with nothing in the column always sinks to the bottom,
// whichever direction the rest is going. Rows equal in the column are ordered by
// the `then` columns, ascending, so a tie reads the same on every load rather
// than in whatever order the rows arrived.
export function sortRows<T>(rows: T[], key: string, dir: Dir, get: (row: T, key: string) => unknown, then: string[] = []): T[] {
  const out = rows.slice()
  out.sort((a, b) => {
    const va = value(get(a, key))
    const vb = value(get(b, key))
    if (va == null || vb == null) {
      const c = cmp(va, vb)
      if (c) return c
    } else {
      const c = cmp(va, vb)
      if (c) return dir === 'desc' ? -c : c
    }
    for (const k of then) {
      const c = cmp(value(get(a, k)), value(get(b, k)))
      if (c) return c
    }
    return 0
  })
  return out
}
