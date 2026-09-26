// The global filter state. Changing any of it asks the server for the figures in
// that scope (the stream is opened again with it), so every tab reflects the same
// scope. Accounts and instruments are held by their ids, never by a name or a
// symbol another could share; the chips and summaries show their names.

import type { Options } from './model'
import { symText } from './sym'

export type ListKey = 'account' | 'symbol' | 'grade' | 'tag' | 'kind' | 'exchange' | 'side' | 'result'
export type RangeKey = 'price' | 'hold' | 'pnl' | 'qty'

export interface Filters {
  lists: Record<ListKey, string[]>
  ranges: Record<RangeKey, { op: string; v: string | null }>
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
  { key: 'symbol', label: 'Symbol', kind: 'list', opt: 'instruments', search: true },
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

/** What the page shows for a list filter's value: an account's or an instrument's name for its id. */
export function valueLabel(key: string, v: string, options: Options | null | undefined): string {
  if (key === 'account') return options?.accounts.find((a) => a.id === v)?.name ?? v
  if (key === 'symbol') {
    const i = options?.instruments.find((x) => x.id === v)
    return i ? symText(i.symbol) : v
  }
  return v
}

export function listSummary(key: ListKey, options: Options | null | undefined): string {
  const on = filters.lists[key]
  if (!on.length) return ''
  const first = valueLabel(key, on[0], options)
  return on.length === 1 ? first : first + ' +' + (on.length - 1)
}

/** A bound as the person typed it, with its digits grouped. */
function grouped(v: string): string {
  const [whole, frac] = v.split('.')
  const sign = whole.startsWith('-') ? '-' : ''
  const digits = whole.replace('-', '').replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return sign + digits + (frac != null ? '.' + frac : '')
}

export function rangeSummary(key: RangeKey): string {
  const r = filters.ranges[key]
  if (r.v == null || r.v === '') return ''
  if (key === 'hold') return r.v + ' days'
  if (key === 'qty') return grouped(r.v)
  return r.v.startsWith('-') ? '-$' + grouped(r.v.slice(1)) : '$' + grouped(r.v)
}

export interface Chip {
  field: string
  key: string
  value: string
}

// The active filters, in the order the tab bar shows them: the date, the free-text
// search, then each list/range field that has a value. Matches ledger.html chips().
export function chips(options: Options | null | undefined): Chip[] {
  const out: Chip[] = []
  if (dateLabel() !== 'All time') out.push({ field: 'Date', value: dateLabel(), key: 'date' })
  if (filters.search) out.push({ field: 'Search', value: filters.search, key: 'search' })
  for (const f of FIELDS) {
    if (f.kind === 'list' && filters.lists[f.key as ListKey].length)
      out.push({ field: f.label + ' is', value: listSummary(f.key as ListKey, options), key: f.key })
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
