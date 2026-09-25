import type { Fill, Model } from './model'
import { filters } from './filters.svelte'
import { connect, disconnect, onChange, onRestart } from './live'
import { forgetHistory } from './trade/chart'
import { call } from './api'
import { leaveSub, route } from './router.svelte'
import { flash } from './ui.svelte'

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
  // a filter changed under an open trade: back to the list the filter is about
  if (store.model && route.tab === 'trades') leaveSub()
  connect(store, filters)
}

/** The server's word over an optimistic write that failed: the whole view again, reconciled. */
export function resync(): void {
  disconnect()
  connect(store, filters)
}

// The legs and fills of the trade or holding that is open: asked for once when it
// opens, and again only when a change touched that very row or the whole view came
// again. They are kept beside the model, not in its row: the view never carries them,
// so a view arriving again (the filters changed, the connection was made again) would
// take them off the row and blank the open chart.
export const detail = $state<{ id: string; fills: Fill[] | undefined; error: string }>({ id: '', fills: undefined, error: '' })
export async function loadDetail(id: string | null): Promise<void> {
  if (id !== detail.id) Object.assign(detail, { id: id ?? '', fills: undefined, error: '' })
  if (!id) return
  const d = await call('GET /api/figures/detail', { query: { id } })
  if (detail.id !== id) return
  // a failed read is said where the executions go, never left as a table waiting for ever
  if (d.error || !Array.isArray(d.fills)) {
    detail.error = d.error || 'The executions could not be read.'
    return
  }
  // the same answer is not a change: the chart and the executions stand as they are
  if (detail.fills && JSON.stringify(d.fills) === JSON.stringify(detail.fills)) return
  Object.assign(detail, { fills: d.fills, error: '' })
}
onChange((touched) => {
  if (detail.id && (touched === 'all' || touched.has(detail.id))) loadDetail(detail.id)
})

// The server this page talks to was started again: the chart's kept bars are forgotten,
// and a chart that is open asks again.
export const server = $state({ restarts: 0 })
onRestart(() => {
  forgetHistory()
  server.restarts++
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
  const d = await call('POST /api/journal', { body })
  if (!d.ok) {
    // said in the header, with the server's reason, and the row put back as the server has it
    flash('Could not save journal entry: ' + (d.error || 'no answer'), 'err')
    resync()
  }
}

// Add a listing to the watchlist: show it at once (a stub row), then persist; the
// server's quote and sector reach that row as a change to it.
export async function addWatch(m: { symbol: string; exchange: string; name: string; currency: string }): Promise<void> {
  const mk = store.model?.markets
  if (mk && !mk.watchlist.some((w) => w.symbol === m.symbol && w.exchange === m.exchange)) {
    mk.watchlist.unshift({ symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency, last: null, priceChange: null, percentChange: null, sector: '', kind: 'Shares', positionId: null })
  }
  try {
    const d = await call('POST /api/watchlist/add', { body: { symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency } })
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
    const d = await call('POST /api/watchlist/remove', { body: { symbol, exchange } })
    if (!d.ok) throw new Error('remove failed')
  } catch {
    resync()
  }
}
