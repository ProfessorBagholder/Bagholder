// The global filter state. Changing any of it recomputes the model server-side
// (loadModel sends it as ?filters=), so every tab reflects the same scope. A
// filter change genuinely changes which trades/positions are in play, so a full
// model recompute is the correct "update when needed" here.

export type ListKey = 'account' | 'symbol' | 'grade' | 'tag' | 'kind' | 'exchange' | 'side' | 'result'
export type RangeKey = 'price' | 'hold' | 'pnl' | 'qty'

export interface Filters {
  lists: Record<ListKey, string[]>
  ranges: Record<RangeKey, { op: string; v: number | null }>
  preset: string
  years: string[]
  from: string
  to: string
  search: string
  benchmark: string
}

export function emptyFilters(): Filters {
  return {
    lists: { account: [], symbol: [], grade: [], tag: [], kind: [], exchange: [], side: [], result: [] },
    ranges: { price: { op: '>', v: null }, hold: { op: '>', v: null }, pnl: { op: '>', v: null }, qty: { op: '>', v: null } },
    preset: 'all',
    years: [],
    from: '',
    to: '',
    search: '',
    benchmark: 'SP500',
  }
}

export const filters = $state<Filters>(emptyFilters())

export function activeCount(): number {
  let n = 0
  for (const k in filters.lists) n += filters.lists[k as ListKey].length
  for (const k in filters.ranges) if (filters.ranges[k as RangeKey].v != null) n++
  n += filters.years.length
  if (filters.from || filters.to) n++
  if (filters.search) n++
  return n
}

export function resetFilters(): void {
  const e = emptyFilters()
  e.benchmark = filters.benchmark
  Object.assign(filters, e)
}
