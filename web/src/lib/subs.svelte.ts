// What the page shows, one subscription per screen (docs/architecture.md §13).
//
// The server sends each screen's data only while something on the page shows it:
// the header's status and the book as a whole always, the Dashboard while its tab
// is open, the holdings while anything that reads them is on screen, one trade
// while its page is. Each is a slot here, written into as its changes arrive
// (live.ts), and a component that reads one says so while it is mounted
// (`use`). Two components reading the same slot share one subscription.
//
// The filters reach the server as the parameters of the subscriptions that read
// them, and only when they are applied (`applied`), never while they are edited.

import type { Status } from './generated/status'
import type { BookDoc, DashboardDoc, PositionsDoc, TradesDoc, CashflowDoc, TradeDoc, ExposureDoc, MarketsDoc } from './generated/figures'
import { watchDoc, type Holder } from './live.svelte'
import { filters, type Filters } from './filters.svelte'

/** A slot: the document as last sent, or null before it arrives; `error` when the server refused it. */
export interface Slot<T> extends Holder<T> {
  error?: string
}

export const status: Slot<Status> = $state({ data: null, error: '' })
export const book: Slot<BookDoc> = $state({ data: null, error: '' })
export const dashboard: Slot<DashboardDoc> = $state({ data: null, error: '' })
export const positions: Slot<PositionsDoc> = $state({ data: null, error: '' })
export const trades: Slot<TradesDoc> = $state({ data: null, error: '' })
export const cashflow: Slot<CashflowDoc> = $state({ data: null, error: '' })
export const exposure: Slot<ExposureDoc> = $state({ data: null, error: '' })
export const markets: Slot<MarketsDoc> = $state({ data: null, error: '' })
export const trade: Slot<TradeDoc> = $state({ data: null, error: '' })

/** The filters as last applied: what the subscriptions are asked with. */
export const applied = $state<{ filters: Filters; n: number }>({ filters: $state.snapshot(filters) as Filters, n: 0 })

/** Apply the filters as they stand: every subscription that reads them is asked again. */
export function applyFilters(): void {
  applied.filters = $state.snapshot(filters) as Filters
  applied.n++
  // a long list starts again at its first rows under the new filters
  limits.trades = FIRST_ROWS
  limits.cash = FIRST_ROWS
}

// One subscription per key and parameters, however many components read it.
const held = new Map<string, { n: number; stop: () => void }>()

/**
 * Show `key` into `into` while `params` says so, for as long as the calling
 * component is mounted: its parameters are read again when what they read
 * changes (the filters applied, the sort, how far a list is scrolled). Called
 * where a component starts.
 */
export function use<T>(key: string | (() => string | null), into: Slot<T>, params: () => unknown = () => ({})): void {
  $effect(() => {
    const k = typeof key === 'function' ? key() : key
    if (!k) return
    const p = params()
    return hold(k, p, into)
  })
}

/** The filters as the subscriptions that read them send them. */
export const filtered = (): { filters: Filters } => ({ filters: applied.filters })

function hold<T>(key: string, params: unknown, into: Slot<T>): () => void {
  const id = key + '\u0000' + JSON.stringify(params)
  const h = held.get(id)
  if (h) {
    h.n++
  } else {
    held.set(id, { n: 1, stop: watchDoc(key, params, into) })
  }
  return () => {
    const h = held.get(id)
    if (!h) return
    if (--h.n === 0) {
      held.delete(id)
      h.stop()
    }
  }
}

/** The rows a long list is sent at first, and each time the person reaches its end: the server's `FIRST_ROWS`. */
export const FIRST_ROWS = 100

/**
 * How far down each long list the page shows: the rows sent. It grows as the person
 * scrolls to the end of what is there (`more`), and starts again at the first page
 * when the list is asked for under other filters or another order.
 */
export const limits = $state({ trades: FIRST_ROWS, cash: FIRST_ROWS })


/** The person reached the end of what a list shows: the next rows are asked for. */
export function more(list: keyof typeof limits, total: number): void {
  if (limits[list] < total) limits[list] += FIRST_ROWS
}
