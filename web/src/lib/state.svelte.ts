import type { Fill } from './model'
import { connect, disconnect, isUpdate, onRestart, resyncAll } from './live.svelte'
import { forgetHistory } from './trade/chart'
import { call } from './api'
import { askAgain, held, read } from './reads.svelte'
import { leaveSub, route } from './router.svelte'
import { flash } from './ui.svelte'
import { applyFilters, book, markets, positions, trade, trades } from './subs.svelte'

// The page's data comes over one connection (live.svelte.ts), one document per
// screen (subs.svelte.ts): each whole once, then only the fields of only the
// entities that change, written into the objects held there. Nothing polls and
// nothing is refetched to see the result of a change: a change made here or
// anywhere else reaches the page because it was committed.

/** Connect once: every screen's data arrives over the one stream. */
export function start(): () => void {
  connect()
  return disconnect
}

/** The filters changed: every screen that reads them is asked again under them. */
export function refilter(): void {
  // a filter changed under an open trade: back to the list the filter is about
  if (route.tab === 'trades' && route.sub) leaveSub()
  applyFilters()
}

/** The server's word over an optimistic write that failed: every document again, reconciled. */
export function resync(): void {
  resyncAll()
}

// The legs and fills of the trade or holding that is open: asked for once when it
// opens, and again only when its own document changed.
export const detail = $state<{ id: string; fills: Fill[] | undefined; error: string }>({ id: '', fills: undefined, error: '' })
export async function loadDetail(id: string | null): Promise<void> {
  if (id !== detail.id) {
    // the executions held from before are drawn at once; the server's replace them where they differ
    // its own last answer, else what the read of every trade's and holding's held of it
    const had = id ? held('GET /api/figures/detail', { query: { id } }) : undefined
    const every = id && !had?.ok ? held('GET /api/figures/details') : undefined
    const fills = had?.ok ? had.fills : every?.ok ? every.details.find((d) => d.id === id)?.fills : undefined
    Object.assign(detail, { id: id ?? '', fills, error: '' })
  }
  if (!id) return
  askAgain('GET /api/figures/detail') // asked each time the trade's own document changed
  const d = await read('GET /api/figures/detail', { query: { id } })
  if (detail.id !== id) return
  // a failed read is said where the executions go, never left as a table waiting for ever
  if (!d.ok) {
    detail.error = d.error
    return
  }
  // the same answer is not a change: the chart and the executions stand as they are
  if (detail.fills && JSON.stringify(d.fills) === JSON.stringify(detail.fills)) return
  Object.assign(detail, { fills: d.fills, error: '' })
}

// The server this page talks to was started again. After an update it runs a new
// version, or speaks another protocol: this page is the old build, and loads itself
// again from the new server (SPEC §2, Versions). Otherwise the chart's kept bars are
// forgotten, and a chart that is open asks again.
export const server = $state({ restarts: 0 })
onRestart((was, now) => {
  if (isUpdate(was, now)) {
    location.reload()
    return
  }
  forgetHistory()
  server.restarts++
})

// Switch the benchmark the annualized-returns card compares against: persist it,
// and ask for the figures under it.
import { filters } from './filters.svelte'
export function setBenchmark(key: string): void {
  filters.benchmark = key
  try {
    localStorage.setItem('bh2.benchmark', key)
  } catch {
    /* the choice holds for this visit; the browser keeps nothing */
  }
  refilter()
}

// Apply a partial filter change and ask for the figures under it.
export function setFilters(patch: Partial<typeof filters>): void {
  Object.assign(filters, patch)
  refilter()
}

/** Every copy of a trade or holding the page holds, by its id: in the list, the holdings and the open page. */
function copies(id: string): { thesis: string; grade: string; tags: string[] }[] {
  const out: { thesis: string; grade: string; tags: string[] }[] = []
  for (const t of trades.data?.trades ?? []) if (t.id === id) out.push(t)
  for (const p of positions.data?.positions ?? []) if (p.id === id) out.push(p)
  if (trade.data?.trade?.id === id) out.push(trade.data.trade)
  if (trade.data?.position?.id === id) out.push(trade.data.position)
  return out
}

// A journal edit: written into the rows that show it at once (so the UI reflects it
// and nothing else re-renders), then persisted. On failure, said, and the server's
// word taken again.
export async function saveJournal(id: string, patch: { thesis?: string; grade?: string; tags?: string[] }): Promise<void> {
  const rows = copies(id)
  if (!rows.length) return
  for (const r of rows) Object.assign(r, patch)
  // a new tag joins the book's known tags so it offers as a suggestion at once
  const opts = book.data?.options
  if (patch.tags && opts) {
    for (const x of patch.tags)
      if (opts.tags.indexOf(x) < 0) {
        opts.tags.push(x)
        opts.tags.sort()
      }
  }
  const t = rows[0]
  const d = await call('POST /api/journal', { body: { id, thesis: t.thesis ?? '', tags: t.tags ?? [], grade: t.grade ?? '' } })
  if (!d.ok) {
    // said in the header, with the server's reason, and the row put back as the server has it
    flash('Could not save journal entry: ' + d.error, 'err')
    resync()
  }
}

// Add a listing to the watchlist: show it at once (a stub row), then persist; the
// server's row, under its instrument, takes the stub's place, and its quote and
// sector reach that row as changes to it.
export async function addWatch(m: { symbol: string; exchange: string; name: string; currency: string }): Promise<void> {
  const mk = markets.data
  if (mk && !mk.watchlist.some((w) => w.symbol === m.symbol && w.exchange === m.exchange)) {
    mk.watchlist.unshift({ id: '', symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency, last: null, change: null, percentChange: null, sector: '', kind: 'Shares', positionId: null })
  }
  const d = await call('POST /api/watchlist/add', { body: { symbol: m.symbol, exchange: m.exchange, name: m.name, currency: m.currency } })
  if (!d.ok) {
    flash('Could not add ' + m.symbol + ' to the watchlist: ' + d.error, 'err')
    resync()
  }
}

// Stop watching an instrument: drop its row at once (so the Watchlist card and the
// heatmap's watchlist universe update, and nothing else does), then persist. On
// failure, said, and the server's word taken again.
export async function removeWatch(id: string, symbol: string): Promise<void> {
  const mk = markets.data
  if (!mk) return
  mk.watchlist = mk.watchlist.filter((w) => w.id !== id)
  const d = await call('POST /api/watchlist/remove', { body: { id } })
  if (!d.ok) {
    flash('Could not remove ' + symbol + ' from the watchlist: ' + d.error, 'err')
    resync()
  }
}
