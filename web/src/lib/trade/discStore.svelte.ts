// An instrument's regulatory filings (SEDAR+, SEC, …) for the trade/holding
// detail. Read per symbol from /api/filings and cached; each row's subject and
// summary are then read one at a time by a background sweep (/api/filings/enrich)
// that walks the issuer at a person's pace. Ported from ledger.html's disclosures
// machinery. `discVersion.n` is bumped wherever the original called render(), so
// the card re-renders as titles and summaries arrive.
import type { Trade, Filing, FilingsPayload } from '../model'
import { listingTicker } from './chart'

export const DISC_ORDER = ['Financials', 'Material events', 'Governance', 'Offerings', 'Insider & ownership', 'News releases', 'Other']

export interface DiscRec {
  loading?: boolean
  payload?: FilingsPayload
  error?: string
}

export const discStore = $state<Record<string, DiscRec>>({})
// a plain reactive counter bumped on every enrich mutation (mirrors render())
export const discVersion = $state({ n: 0 })
function bump() {
  discVersion.n++
}

const _pending: Record<string, boolean> = {}
const _open = new Set<string>()

export function discSymbol(t: Trade): string {
  return String(listingTicker(t) || t.symbol || '').toUpperCase()
}

// The card registers the symbol it is showing so the sweep knows to keep reading
// and to stop when the card is gone.
export function markDiscOpen(sym: string): void {
  _open.add(sym)
}
export function markDiscClosed(sym: string): void {
  _open.delete(sym)
}
function stillOpen(sym: string): boolean {
  return _open.has(sym)
}

export function ensureDisclosures(t: Trade): void {
  const sym = discSymbol(t)
  if (!sym || discStore[sym] || _pending[sym]) return
  _pending[sym] = true
  discStore[sym] = { loading: true }
  const q =
    'symbol=' + encodeURIComponent(sym) +
    (t.name ? '&name=' + encodeURIComponent(t.name) : '') +
    (t.exchange ? '&exchange=' + encodeURIComponent(t.exchange) : '') +
    (t.currency ? '&currency=' + encodeURIComponent(t.currency) : '')
  fetch('/api/filings?' + q, { headers: { 'X-Bagholder': '1' } })
    .then((r) => r.json())
    .catch((e) => ({ ok: false, error: String(e) }))
    .then((d: any) => {
      delete _pending[sym]
      discStore[sym] = d && d.ok ? { payload: d } : { error: (d && d.error) || 'Could not read disclosures.' }
    })
}

export function refreshDisclosures(sym: string, open: Trade | null): void {
  if (_pending[sym]) return
  delete discStore[sym]
  if (open && discSymbol(open) === sym) ensureDisclosures(open)
}

// ---- the background read of each row's subject and summary ----
const _enrichLoading: Record<string, boolean> = {}
const _enrichTried: Record<string, boolean | 'waiting'> = {}
const _enrichPasses: Record<string, number> = {}
const _sweep: Record<string, boolean> = {}
const _queue: Record<string, Set<string> | null> = {}
const _rekicks: Record<string, number> = {}
let _summaryUp = false

function sweepActive(sym: string, f: Filing): boolean {
  const q = _queue[sym]
  return !!(q && f && q.has(f.id))
}
function preparing(f: Filing): boolean {
  return f.summaryStatus === 'downloading' || f.summaryStatus === 'starting'
}
export function enrichLoading(id: string): boolean {
  return !!_enrichLoading[id]
}
export { sweepActive, preparing }

// Whether a row's title is still on its way — it wears the shimmer.
export function titleComing(f: Filing, sym: string): boolean {
  if (f.subject) return false
  return !!(_enrichLoading[f.id] || sweepActive(sym, f) || !_enrichTried[f.id] || _enrichTried[f.id] === 'waiting' || preparing(f))
}

function enrichNeed(f: Filing): boolean {
  if (f.subject && f.summary) return false
  const settled = _enrichTried[f.id]
  return settled === 'waiting' ? _summaryUp : !settled
}
function enrichSettle(f: Filing, d: any): void {
  const answered = !!(d && d.ok)
  if (answered) _summaryUp = !!d.summaryAvailable
  if (f.subject && f.summary) {
    _enrichTried[f.id] = true
    return
  }
  if (!answered) return
  if (!d.summaryAvailable) {
    _enrichTried[f.id] = 'waiting'
    return
  }
  const chances = (_enrichPasses[f.id] = (_enrichPasses[f.id] || 0) + 1)
  if (chances >= 2) _enrichTried[f.id] = true
}

function enrichOne(sym: string, f: Filing): Promise<void> {
  if (!f) return Promise.resolve()
  _enrichLoading[f.id] = true
  return fetch('/api/filings/enrich?symbol=' + encodeURIComponent(sym) + '&id=' + encodeURIComponent(f.id), { headers: { 'X-Bagholder': '1' } })
    .then((r) => r.json())
    .catch(() => ({ ok: false }))
    .then((d: any) => {
      delete _enrichLoading[f.id]
      if (d && d.ok) {
        if (d.subject) f.subject = d.subject
        if (d.summary) f.summary = d.summary
        f.summaryStatus = d.summaryStatus || ''
      }
      enrichSettle(f, d)
      if (stillOpen(sym)) bump()
    })
}

export function sweepDiscEnrich(sym: string, rows: Filing[]): void {
  if (_sweep[sym]) return
  if (!rows.some(enrichNeed)) return
  _sweep[sym] = true
  const passed = new Set<string>()
  const upNext = () => rows.find((f) => enrichNeed(f) && !passed.has(f.id))
  const queue = () => {
    _queue[sym] = new Set(rows.filter((f) => enrichNeed(f) && !passed.has(f.id)).map((f) => f.id))
  }
  queue()
  let sawPreparing = false
  let sawFailure = false
  const finish = () => {
    _sweep[sym] = false
    _queue[sym] = null
    const again = _rekicks[sym] || 0
    const unfinished = rows.some(enrichNeed)
    if (stillOpen(sym) && (sawPreparing || (unfinished && again < 6))) {
      _rekicks[sym] = again + 1
      setTimeout(
        () => {
          const r = discStore[sym]
          if (r && r.payload && stillOpen(sym)) sweepDiscEnrich(sym, r.payload.filings || [])
        },
        sawPreparing ? 15000 : sawFailure ? 4000 * (again + 1) : 1500,
      )
    } else if (!sawPreparing && !unfinished) _rekicks[sym] = 0
  }
  const step = () => {
    if (!stillOpen(sym)) {
      _sweep[sym] = false
      _queue[sym] = null
      return
    }
    const f = upNext()
    if (!f) return finish()
    queue()
    enrichOne(sym, f).then(() => {
      if (_enrichTried[f.id] !== 'waiting') passed.add(f.id)
      if (preparing(f)) sawPreparing = true
      else if (!f.subject && !f.summary) sawFailure = true
      setTimeout(step, 150)
    })
  }
  step()
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
