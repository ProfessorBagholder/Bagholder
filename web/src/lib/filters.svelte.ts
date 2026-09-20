// The global filter state. Changing any of it recomputes the model server-side
// (loadModel sends it as ?filters=), so every tab reflects the same scope. A
// filter change genuinely changes which trades/positions are in play, so a full
// model recompute is the correct "update when needed" here.

import type { Options } from './model'
import { symText } from './sym'

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

// The benchmark is not a filter but travels with them; the choice is remembered on this machine.
function rememberedBenchmark(): string {
  try {
    return localStorage.getItem('bh2.benchmark') || 'SP500'
  } catch {
    return 'SP500'
  }
}

export const filters = $state<Filters>({ ...emptyFilters(), benchmark: rememberedBenchmark() })

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

// ---------------------------------------------------------------------------
// Field metadata, the date/summary words, the active-filter chips.
// Ported verbatim from ledger.html: FIELDS, PRESETS, dateLabel(), listSummary(),
// rangeSummary(), chips(). Kept here (not in FilterPopover) so the popover and the
// tab-bar chips share one definition and neither redefines the other.
// ---------------------------------------------------------------------------

export const PRESETS: [string, string][] = [
  ['1d', '1D'], ['1w', '1W'], ['1m', '1M'], ['3m', '3M'], ['6m', '6M'], ['ytd', 'YTD'], ['1y', '1Y'], ['5y', '5Y'],
]

export type FieldKind = 'date' | 'list' | 'range'
export interface Field {
  key: string
  label: string
  kind: FieldKind
  opt?: keyof Options
  search?: boolean
  unit?: string
  steps?: number[]
  ph?: string
}

export const FIELDS: Field[] = [
  { key: 'date', label: 'Date', kind: 'date' },
  { key: 'account', label: 'Account', kind: 'list', opt: 'accounts' },
  { key: 'symbol', label: 'Symbol', kind: 'list', opt: 'symbols', search: true },
  { key: 'grade', label: 'Grade', kind: 'list', opt: 'grades' },
  { key: 'tag', label: 'Tag', kind: 'list', opt: 'tags', search: true },
  { key: 'side', label: 'Side', kind: 'list', opt: 'sides' },
  { key: 'kind', label: 'Kind', kind: 'list', opt: 'kinds' },
  { key: 'exchange', label: 'Exchange', kind: 'list', opt: 'exchanges' },
  { key: 'result', label: 'Result', kind: 'list', opt: 'results' },
  { key: 'price', label: 'Price', kind: 'range', unit: '$', steps: [1, 5, 20, 100], ph: 'Entry price, e.g. 2.50' },
  { key: 'hold', label: 'Hold', kind: 'range', unit: 'd', steps: [7, 30, 90, 180], ph: 'Days, e.g. 45' },
  { key: 'pnl', label: 'P&L', kind: 'range', unit: '$', steps: [100, 500, 1000, 5000], ph: 'Amount, e.g. 250' },
  { key: 'qty', label: 'Qty', kind: 'range', unit: '', steps: [100, 1000, 10000, 100000], ph: 'Units, e.g. 500' },
]

export function dateLabel(): string {
  const f = filters
  if (f.from || f.to) {
    if (f.from && f.to) return f.from + ' → ' + f.to
    return f.from ? 'From ' + f.from : 'Up to ' + f.to
  }
  if (f.years.length) {
    const ys = f.years.slice().sort().reverse()
    const contiguous = ys.every((y, i) => i === 0 || +ys[i - 1] - +y === 1)
    if (ys.length > 2 && contiguous) return ys[ys.length - 1] + ' — ' + ys[0]
    return ys.join(', ')
  }
  const p = PRESETS.find((p) => p[0] === f.preset)
  return p ? p[1] : 'All time'
}

export function listSummary(key: ListKey): string {
  const on = filters.lists[key]
  if (!on.length) return ''
  const first = key === 'symbol' ? symText(on[0]) : on[0]
  return on.length === 1 ? first : first + ' +' + (on.length - 1)
}

export function rangeSummary(key: RangeKey): string {
  const r = filters.ranges[key]
  if (r.v == null || (r.v as unknown) === '') return ''
  if (key === 'hold') return r.v + ' days'
  if (key === 'qty') return Number(r.v).toLocaleString('en-US')
  return '$' + Number(r.v).toLocaleString('en-US')
}

export interface Chip {
  field: string
  key: string
  value: string
}

// The active filters, in the order the tab bar shows them: the date, the free-text
// search, then each list/range field that has a value. Matches ledger.html chips().
export function chips(): Chip[] {
  const out: Chip[] = []
  if (dateLabel() !== 'All time') out.push({ field: 'Date', value: dateLabel(), key: 'date' })
  if (filters.search) out.push({ field: 'Search', value: filters.search, key: 'search' })
  for (const f of FIELDS) {
    if (f.kind === 'list' && filters.lists[f.key as ListKey].length)
      out.push({ field: f.label + ' is', value: listSummary(f.key as ListKey), key: f.key })
    if (f.kind === 'range' && rangeSummary(f.key as RangeKey))
      out.push({ field: f.label + ' ' + filters.ranges[f.key as RangeKey].op, value: rangeSummary(f.key as RangeKey), key: f.key })
  }
  return out
}

// Clear one filter, addressed by its chip/field key. Mutation only — the caller
// reloads the model (setFilters({}) / loadModel()), as ledger.html clearField did
// via setFilters. Kept import-free of state.svelte to avoid a cycle.
export function clearField(key: string): void {
  const f = filters
  if (key === 'date') { f.preset = 'all'; f.years = []; f.from = ''; f.to = '' }
  else if (key === 'search') f.search = ''
  else if (f.lists[key as ListKey]) f.lists[key as ListKey] = []
  else if (f.ranges[key as RangeKey]) f.ranges[key as RangeKey].v = null
}
