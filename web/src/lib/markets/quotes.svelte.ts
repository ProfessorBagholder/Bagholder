// A match's price and day change, read for the glance and remembered for a minute
// (SPEC.md §4 Markets, Watchlist: a minute's memory, nothing stored): asked for again
// the next time it is wanted once that minute is up, the one shown standing until the
// new one lands. Reactive $state so the watchlist row and the add-row suggestion fill
// in when a quote lands. The price is the source's exact decimal text, the day's
// change a fraction.
import { bareSymbol } from '../sym'
import { call } from '../api'
import type { Dec } from '../dec'

export interface Quote {
  last: Dec | null
  /** The day's change, as a fraction. */
  percentChange: number | null
}

const MEMORY_MS = 60_000

export const sugQuotes = $state<Record<string, Quote>>({})
const readAt: Record<string, number> = {}
const pending: Record<string, boolean> = {}

export function sugKey(w: { symbol: string; exchange?: string }): string {
  return bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase()
}

export function sugQuoteSchedule(rows: { symbol: string; exchange?: string; currency?: string; last?: unknown }[]): void {
  rows.forEach((w) => {
    const k = sugKey(w)
    if (w.last != null || pending[k] || (sugQuotes[k] && Date.now() - readAt[k] < MEMORY_MS)) return
    pending[k] = true
    call('GET /api/symbols/quote', { query: { symbol: w.symbol, exchange: w.exchange || '', currency: w.currency || '', name: '' } }).then((r) => {
      delete pending[k]
      // a glance with no answer shows no price: the row keeps its dash, and it is asked again when next wanted
      if (r.ok) {
        readAt[k] = Date.now()
        sugQuotes[k] = { last: r.price, percentChange: r.percentChange ?? null }
      }
    })
  })
}
