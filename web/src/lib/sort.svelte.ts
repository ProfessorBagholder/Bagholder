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
  yoc: { key: 'ttm', dir: 'desc' },
  cash: { key: 'date', dir: 'desc' },
  orders: { key: 'placed', dir: 'desc' },
  watchlist: { key: 'added', dir: 'desc' },
  news: { key: 'when', dir: 'desc' },
  shorts: { key: 'ofFloat', dir: 'desc' },
}

function load(): Record<string, SortState> {
  try {
    const raw = localStorage.getItem('bh2.sort')
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) }
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

function cmp(a: unknown, b: unknown): number {
  if (a == null && b == null) return 0
  if (a == null) return 1
  if (b == null) return -1
  if (typeof a === 'number' && typeof b === 'number') return a - b
  return String(a).localeCompare(String(b))
}

// Sort a copy; a row with nothing in the column always sinks to the bottom,
// whichever direction the rest is going.
export function sortRows<T>(rows: T[], key: string, dir: Dir, get: (row: T, key: string) => unknown): T[] {
  const out = rows.slice()
  out.sort((a, b) => {
    const va = get(a, key)
    const vb = get(b, key)
    if (va == null || vb == null) return cmp(va, vb)
    const c = cmp(va, vb)
    return dir === 'desc' ? -c : c
  })
  return out
}
