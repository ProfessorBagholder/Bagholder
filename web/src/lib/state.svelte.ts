import type { Model } from './model'
import { filters } from './filters.svelte'

// The reactive store. This is the whole point of the migration: state lives in
// one $state rune and the UI derives from it — no manual coreVersion/dataVersion
// bookkeeping, no reconciler. Components read store.model and Svelte tracks the
// dependency itself.
export const store = $state<{ model: Model | null; error: string | null; loading: boolean }>({
  model: null,
  error: null,
  loading: true,
})

// The detail id in the address (trades/<id> or portfolio/<id>): the open trade's
// legs and fills travel only for it, exactly as the legacy loadModel did.
function detailParam(): string {
  if (typeof location === 'undefined') return ''
  const parts = location.hash.replace(/^#\/?/, '').split('/')
  const page = parts[0] === 'positions' ? 'portfolio' : parts[0]
  const id = parts.slice(1).join('/')
  if ((page === 'trades' || page === 'portfolio') && id) return '&trade=' + encodeURIComponent(decodeURIComponent(id))
  return ''
}

// The data layer, behavior-equivalent to the legacy loadModel(): fetch the
// server-derived model (filters + benchmark + the open trade's detail) and hand
// it to the store. Svelte tracks what each component reads.
let _loadSeq = 0
export async function loadModel(): Promise<void> {
  const seq = ++_loadSeq
  store.loading = true
  try {
    const q = encodeURIComponent(JSON.stringify(filters))
    const r = await fetch('/api/model?filters=' + q + detailParam(), { headers: { 'X-Bagholder': '1' } })
    if (!r.ok) throw new Error('HTTP ' + r.status)
    const m = (await r.json()) as Model
    if (seq !== _loadSeq) return
    store.model = m
    store.error = null
  } catch (e) {
    if (seq !== _loadSeq) return
    store.error = e instanceof Error ? e.message : String(e)
  } finally {
    if (seq === _loadSeq) store.loading = false
  }
}

// Switch the benchmark the annualized-returns card compares against: persist it,
// send it on the next model load (spR is computed server-side for it), reload.
export function setBenchmark(key: string): void {
  filters.benchmark = key
  try {
    localStorage.setItem('bh2.benchmark', key)
  } catch {
    /* ignore */
  }
  loadModel()
}

// Apply a partial filter change and recompute the model, like the legacy setFilters().
export function setFilters(patch: Partial<typeof filters>): void {
  Object.assign(filters, patch)
  loadModel()
}

// A journal edit: mutate the one trade optimistically (so the UI reflects it at
// once and nothing else re-renders), then persist. On failure, surface it and
// reload the authoritative model. This is the "update exactly what changed,
// nothing else affected" pattern the reconciler could never guarantee.
export async function saveJournal(id: string, patch: { thesis?: string; grade?: string; tags?: string[] }): Promise<void> {
  const t = store.model?.trades.find((x) => x.id === id) ?? store.model?.positions?.find((p) => p.id === id)
  if (!t) return
  Object.assign(t, patch)
  // a new tag joins the book's known tags so it offers as a suggestion at once
  if (patch.tags && store.model?.options) {
    const opts = store.model.options
    for (const x of patch.tags)
      if (opts.tags.indexOf(x) < 0) {
        opts.tags.push(x)
        opts.tags.sort()
      }
  }
  const body = { id, thesis: (t as { thesis?: string }).thesis ?? '', tags: (t as { tags?: string[] }).tags ?? [], grade: (t as { grade?: string }).grade ?? '' }
  try {
    const r = await fetch('/api/journal', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
    const d = await r.json()
    if (!d.ok) throw new Error('save failed')
  } catch {
    store.error = 'Could not save journal entry.'
    loadModel()
  }
}

// Add a listing to the watchlist: show it at once (a stub row), then persist and
// reload so the server's quote and sector fill in.
export async function addWatch(m: { symbol: string; exchange: string; name: string; currency: string }): Promise<void> {
  const mk = store.model?.markets
  if (mk && !mk.watchlist.some((w) => w.symbol === m.symbol && w.exchange === m.exchange)) {
    mk.watchlist.unshift({ symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency, last: null, priceChange: null, percentChange: null, sector: '', positionId: null })
  }
  try {
    const r = await fetch('/api/watchlist/add', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(m) })
    const d = await r.json()
    if (!d.ok) throw new Error('add failed')
  } finally {
    loadModel()
  }
}

// Remove a watchlist listing: drop it from the store at once (so the Watchlist
// card and the heatmap's watchlist universe update, and nothing else does), then
// persist. On failure, reload the authoritative model.
export async function removeWatch(symbol: string, exchange: string): Promise<void> {
  const mk = store.model?.markets
  if (!mk) return
  mk.watchlist = mk.watchlist.filter((w) => !(w.symbol === symbol && w.exchange === exchange))
  try {
    const r = await fetch('/api/watchlist/remove', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ symbol, exchange }) })
    const d = await r.json()
    if (!d.ok) throw new Error('remove failed')
  } catch {
    loadModel()
  }
}
