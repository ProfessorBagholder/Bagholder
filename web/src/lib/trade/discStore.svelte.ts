// An instrument's regulatory filings (SEDAR+, SEC, …) for the trade/holding
// detail. They are shown only while a card shows them, so they are sent only then
// (live.ts `watchDoc`): the list once, and after that each row's title and sentence
// as the server reads the document, written into that row. The reading itself is the
// server's work -- nothing here asks for one document after another, and there is no
// timer in this file.
import type { Trade, Filing, FilingsDoc } from '../model'
import { listingTicker } from './chart'
import { watchDoc } from '../live'
import { call } from '../api'

export const DISC_ORDER = ['Financials', 'Material events', 'Governance', 'Offerings', 'Insider & ownership', 'News releases', 'Other']

export interface DiscRec {
  loading?: boolean
  payload?: FilingsDoc
  error?: string
}

export const discStore = $state<Record<string, DiscRec>>({})

/**
 * A forced read under way, per listing: the card shows its Reading state until the
 * read answers. Kept apart from the list, which the stream replaces as it changes.
 */
export const discRereading = $state<Record<string, boolean>>({})
/** A forced read that was refused, per listing, until the next one is asked for. */
export const discRereadError = $state<Record<string, string>>({})

export function discSymbol(t: { symbol: string; kind?: string; underlying?: string }): string {
  return String(listingTicker(t as Trade) || t.symbol || '').toUpperCase()
}

function docKey(sym: string, t: { name?: string; exchange?: string; currency?: string }): string {
  return (
    'filings:symbol=' + encodeURIComponent(sym) +
    (t.name ? '&name=' + encodeURIComponent(t.name) : '') +
    (t.exchange ? '&exchange=' + encodeURIComponent(t.exchange) : '') +
    (t.currency ? '&currency=' + encodeURIComponent(t.currency) : '')
  )
}

const watching = new Map<string, { count: number; stop: () => void }>()

/**
 * Show a listing's disclosures for as long as the caller does. Several cards may show
 * the same listing; it is watched once. Returns what the caller runs when it stops.
 */
export function showDisclosures(t: { symbol: string; kind?: string; underlying?: string; name?: string; exchange?: string; currency?: string }): () => void {
  const sym = discSymbol(t)
  if (!sym) return () => {}
  const held = watching.get(sym)
  if (held) held.count++
  else {
    if (!discStore[sym]) discStore[sym] = { loading: true }
    const holder = {
      get data() { return discStore[sym]?.payload ?? null },
      set data(v: FilingsDoc | null) {
        discStore[sym] = v && v.ok ? { payload: v } : { error: 'Could not read disclosures.' }
      },
    }
    watching.set(sym, { count: 1, stop: watchDoc<FilingsDoc>(docKey(sym, t), {}, holder) })
  }
  return () => {
    const w = watching.get(sym)
    if (!w || --w.count > 0) return
    w.stop()
    watching.delete(sym)
  }
}

/**
 * Read the sources again now, whatever their age; what they say reaches the rows as
 * changes. The card reads as Reading until the read answers; a refusal is said.
 */
export async function refreshDisclosures(sym: string, t: { name?: string; exchange?: string; currency?: string } | null): Promise<void> {
  if (discRereading[sym]) return
  discRereading[sym] = true
  delete discRereadError[sym]
  try {
    const a = await call('GET /api/filings', { query: { symbol: sym, name: t?.name ?? '', exchange: t?.exchange ?? '', currency: t?.currency ?? '', refresh: true } })
    if (!a.ok) discRereadError[sym] = 'Could not read disclosures' + (a.error ? ': ' + a.error : '.')
  } finally {
    delete discRereading[sym]
  }
}

// ---- what a row wears while it waits: said by the server, which does the reading ----
const reading = (sym: string): string[] => discStore[sym]?.payload?.reading ?? []

/** The document being read this moment. */
export function enrichLoading(sym: string, id: string): boolean {
  return reading(sym)[0] === id
}
/** Still to be read in the pass under way. */
export function sweepActive(sym: string, f: Filing): boolean {
  return reading(sym).indexOf(f.id) >= 0
}
/** The local model that writes the sentences is still coming up. */
export function preparing(sym: string): boolean {
  const st = discStore[sym]?.payload?.summaryStatus
  return st === 'downloading' || st === 'starting'
}
/** Whether a row's title is still on its way -- it wears the shimmer. */
export function titleComing(f: Filing, sym: string): boolean {
  return !f.subject && (sweepActive(sym, f) || (preparing(sym) && !f.enrichFinal))
}

// ---- sorting / formatting of the list ----
const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
export function discDate(item: Filing): string {
  const iso = String(item.date || '').slice(0, 10)
  const d = new Date(iso + 'T00:00:00')
  if (isNaN(d.getTime())) return item.dateText || iso
  return d.getDate() + ' ' + MON[d.getMonth()] + ' ' + d.getFullYear()
}
function discSizeBytes(s: string): number {
  const m = /([\d.]+)\s*(KB|MB|GB|bytes)/i.exec(s || '')
  if (!m) return -1
  const n = parseFloat(m[1])
  const u = m[2].toLowerCase()
  return u === 'mb' ? n * 1e6 : u === 'gb' ? n * 1e9 : u === 'bytes' ? n : n * 1e3
}
export function discSortValue(item: Filing, key: string): unknown {
  return key === 'document'
    ? (item.type || '').toLowerCase()
    : key === 'category'
      ? item.category
      : key === 'source'
        ? item.source
        : key === 'size'
          ? discSizeBytes(item.size)
          : item.date
}
