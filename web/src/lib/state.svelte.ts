import type { Model } from './model'
import { filters } from './filters.svelte'
import { connect, disconnect, onChange } from './live'

// The reactive store. This is the whole point of the migration: state lives in
// one $state rune and the UI derives from it — no manual coreVersion/dataVersion
// bookkeeping, no reconciler. Components read store.model and Svelte tracks the
// dependency itself.
export const store = $state<{ model: Model | null; error: string | null; loading: boolean }>({
  model: null,
  error: null,
  loading: true,
})

// The page's data comes over one connection (live.ts): the whole view once, then
// only the fields of only the entities that change, written into the objects held
// here. Nothing polls and nothing is refetched to see the result of a change: a
// change made here or anywhere else reaches the page because it was committed.

/** Connect, or connect again because the filters changed. */
export function refilter(): void {
  connect(store, filters)
}

/** The server's word over an optimistic write that failed: the whole view again, reconciled. */
export function resync(): void {
  disconnect()
  connect(store, filters)
}

// The legs and fills of the trade or holding that is open: asked for once when it
// opens, and again only when a change touched that very row.
let _detailFor = ''
export async function loadDetail(id: string | null): Promise<void> {
  _detailFor = id ?? ''
  if (!id) return
  try {
    const r = await fetch('/api/trade?id=' + encodeURIComponent(id), { headers: { 'X-Bagholder': '1' } })
    const d = (await r.json()) as { ok?: boolean; legs?: unknown[]; fills?: unknown[] }
    if (!d.ok || _detailFor !== id) return
    const m = store.model
    for (const row of [m?.trades.find((t) => t.id === id), m?.positions?.find((p) => p.id === id)]) {
      if (row) Object.assign(row, { legs: d.legs ?? [], fills: d.fills ?? [] })
    }
  } catch {
    /* the rest of the page stands; the detail is asked for again when the row next changes */
  }
}
onChange((touched) => {
  if (_detailFor && touched.has(_detailFor)) loadDetail(_detailFor)
})

// Switch the benchmark the annualized-returns card compares against: persist it,
// send it on the next model load (spR is computed server-side for it), reload.
export function setBenchmark(key: string): void {
  filters.benchmark = key
  try {
    localStorage.setItem('bh2.benchmark', key)
  } catch {
    /* ignore */
  }
  refilter()
}

// Apply a partial filter change and recompute the model, like the legacy setFilters().
export function setFilters(patch: Partial<typeof filters>): void {
  Object.assign(filters, patch)
  refilter()
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
    const r = await fetch('/api/journal', { method: 'POST', headers: { 'content-type': 'application/json', 'X-Bagholder': '1' }, body: JSON.stringify(body) })
    const d = await r.json()
    if (!d.ok) throw new Error('save failed')
  } catch {
    store.error = 'Could not save journal entry.'
    resync()
  }
}

// Add a listing to the watchlist: show it at once (a stub row), then persist; the
// server's quote and sector reach that row as a change to it.
export async function addWatch(m: { symbol: string; exchange: string; name: string; currency: string }): Promise<void> {
  const mk = store.model?.markets
  if (mk && !mk.watchlist.some((w) => w.symbol === m.symbol && w.exchange === m.exchange)) {
    mk.watchlist.unshift({ symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency, last: null, priceChange: null, percentChange: null, sector: '', positionId: null })
  }
  try {
    const r = await fetch('/api/watchlist/add', { method: 'POST', headers: { 'content-type': 'application/json', 'X-Bagholder': '1' }, body: JSON.stringify(m) })
    const d = await r.json()
    if (!d.ok) throw new Error('add failed')
    // the row's quote and sector arrive as a change to that row when the server has them
  } catch {
    resync()
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
    const r = await fetch('/api/watchlist/remove', { method: 'POST', headers: { 'content-type': 'application/json', 'X-Bagholder': '1' }, body: JSON.stringify({ symbol, exchange }) })
    const d = await r.json()
    if (!d.ok) throw new Error('remove failed')
  } catch {
    resync()
  }
}
